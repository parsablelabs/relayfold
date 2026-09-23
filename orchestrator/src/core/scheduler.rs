use crate::adapters::worker_registry::WorkerRegistry;
use crate::core::consts::{RELAYFOLD_SCHEDULER_CONFIG_PATH, RELAYFOLD_SCHEDULER_ENABLED};
use crate::core::namespace::Namespace;
use crate::core::orchestrator::Orchestrator;
use crate::core::workflow::workflow_service::WorkflowService;
use anyhow::{Context, Result, anyhow, ensure};
use chrono::{DateTime, Timelike, Utc};
use croner::Cron;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{self, Duration, MissedTickBehavior};
use tracing::{error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchedulerEntry {
    namespace: Namespace,
    workflow_def_id: String,
    cron: String,
    #[serde(default = "default_run_on_startup")]
    run_on_startup: bool,
    input: Option<serde_json::Value>,
}

impl SchedulerEntry {
    fn key(&self) -> ScheduleKey {
        ScheduleKey {
            namespace: self.namespace.clone(),
            workflow_def_id: self.workflow_def_id.clone(),
        }
    }
}

fn default_run_on_startup() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SchedulerConfig {
    pub scheduler: Vec<SchedulerEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ScheduleKey {
    namespace: Namespace,
    workflow_def_id: String,
}

fn validate_scheduler_config(config: &SchedulerConfig) -> Result<()> {
    // for each entry, check if namespace exist, workflow_def_id exists, cron is valid
    let mut unique_workflows = HashSet::<ScheduleKey>::new();

    for entry in &config.scheduler {
        if unique_workflows.contains(&entry.key()) {
            return Err(anyhow!(
                "scheduler configuration contains duplicate workflows"
            ));
        }

        // validate cron expression
        if let Err(error) = parse_cron_expr(&entry.cron) {
            return Err(error);
        };

        unique_workflows.insert(entry.key());
    }
    Ok(())
}

fn parse_cron_expr(cron_expr: &str) -> Result<Cron> {
    ensure!(
        cron_expr.split_whitespace().count() == 5,
        "cron expression must contain exactly five fields"
    );

    Cron::from_str(cron_expr).with_context(|| format!("invalid cron expression: {cron_expr}"))
}

fn read_config(contents: &str) -> Result<SchedulerConfig> {
    let yaml = serde_yaml::from_str(&contents)?;
    Ok(yaml)
}

fn read_config_from_file(path: &Path) -> Result<SchedulerConfig> {
    let contents = std::fs::read_to_string(path)?;
    read_config(&contents)
}

fn normalize_to_minute(time: &DateTime<Utc>) -> DateTime<Utc> {
    time.with_nanosecond(0)
        .and_then(|time| time.with_second(0))
        .unwrap()
}

async fn run_scheduler(
    orchestrator: Arc<Orchestrator>,
    workflow_service: Arc<WorkflowService>,
    worker_registry: WorkerRegistry,
    pending_startup_entries: &mut Option<HashSet<ScheduleKey>>,
) {
    let is_enabled =
        env::var(RELAYFOLD_SCHEDULER_ENABLED).map_or(false, |v| v.parse::<bool>().unwrap_or(false));

    if !is_enabled {
        info!("scheduler disabled");
        return;
    }

    let config_path =
        env::var(RELAYFOLD_SCHEDULER_CONFIG_PATH).unwrap_or("./scheduler.yaml".to_owned());

    let config = match read_config_from_file(&PathBuf::from(&config_path)) {
        Ok(config) => config,
        Err(error) => {
            error!(
                %error,
                %config_path,
                "cannot read scheduler configuration file"
            );
            return;
        }
    };

    evaluate_scheduler_config(
        orchestrator,
        workflow_service,
        worker_registry,
        config,
        Utc::now(),
        pending_startup_entries,
    )
    .await;
}

async fn evaluate_scheduler_config(
    orchestrator: Arc<Orchestrator>,
    workflow_service: Arc<WorkflowService>,
    worker_registry: WorkerRegistry,
    config: SchedulerConfig,
    time: DateTime<Utc>,
    pending_startup_entries: &mut Option<HashSet<ScheduleKey>>,
) {
    // If configuration has some issues do not continue
    if let Err(error) = validate_scheduler_config(&config) {
        error!(%error, "scheduler configuration is not valid!");
        return;
    }

    let schedule_count = config.scheduler.len();
    info!(%schedule_count, "evaluating schedules...");

    // Create inventory of workflows that need to start,
    // we want this to happen once only on startup.
    if pending_startup_entries.is_none() {
        let mut tracker = HashSet::new();
        for schedule in &config.scheduler {
            if !schedule.run_on_startup {
                continue;
            }
            tracker.insert(schedule.key());
        }
        // indicating we already retrieved all entries
        *pending_startup_entries = Some(tracker);
    }

    for schedule in config.scheduler {
        let schedule_key = schedule.key();
        let cron = parse_cron_expr(&schedule.cron)
            .expect("this is unexpected, cron expression should have already been validated");
        let is_match = match cron.is_time_matching(&normalize_to_minute(&time)) {
            Ok(match_result) => match_result,
            Err(err) => {
                error!("failed while matching cron to current time {err}");
                return;
            }
        };

        let startup_entry_pending = match pending_startup_entries {
            Some(set) => set.contains(&schedule.key()),
            None => false,
        };

        if is_match || startup_entry_pending {
            let host = match worker_registry.select_eligible_host().await {
                Some(host) => host,
                None => {
                    error!("could not find eligible host, skipping this schedule");
                    return;
                }
            };

            // verify and schedule - only schedule if this workflow is not currently executing, if failure assume workflow may be running and skip
            let is_workflow_pending = match workflow_service
                .has_active_workflow_instance_for_def(
                    &schedule.namespace,
                    &schedule.workflow_def_id,
                )
                .await
            {
                Ok(result) => result,
                Err(err) => {
                    error!(%err, "check for pending workflow instance failed");
                    true
                }
            };

            if is_workflow_pending {
                // if this workflow is already running, we do not want to schedule another one back to back
                pending_startup_entries
                    .as_mut()
                    .unwrap()
                    .remove(&schedule_key);
            }

            if !is_workflow_pending {
                match workflow_service
                    .create_workflow_instance_for_def(
                        &schedule.namespace,
                        &schedule.workflow_def_id,
                        host,
                        schedule.input.clone(),
                    )
                    .await
                {
                    Ok(instance_id) => {
                        let enqueued = orchestrator
                            .enqueue_workflow_instance(&schedule.namespace, instance_id.clone())
                            .await;

                        pending_startup_entries
                            .as_mut()
                            .unwrap()
                            .remove(&schedule_key);

                        match enqueued {
                            Ok(()) => {
                                info!(?instance_id, ?schedule, "workflow enqueued successfully");
                            }
                            Err(error) => error!(%error, "workflow instantiation failed"),
                        }
                    }
                    Err(error) => {
                        error!(%error, "workflow instantiation fialed");
                    }
                };
            }
        }
    }
    // log if error, do not interrupt the loop
}

pub fn start_task_scheduler(
    orchestrator: Arc<Orchestrator>,
    workflow_service: Arc<WorkflowService>,
    worker_registry: WorkerRegistry,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let is_enabled = env::var(RELAYFOLD_SCHEDULER_ENABLED)
            .map_or(false, |value| value.parse::<bool>().unwrap_or(false));

        if is_enabled {
            info!("scheduler waiting for an eligible worker host");
            worker_registry.wait_for_eligible_host().await;
            info!("eligible worker host registered; starting scheduler");
        }

        let mut interval = time::interval(Duration::from_secs(60));
        let mut pending_startup_entries: Option<HashSet<ScheduleKey>> = None;
        // Prevent burst file catchup ticks due to a stall
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            run_scheduler(
                orchestrator.clone(),
                workflow_service.clone(),
                worker_registry.clone(),
                &mut pending_startup_entries,
            )
            .await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::fake_task_dispatcher::FakeTaskDispatcher;
    use crate::adapters::memory_storage::MemoryStorage;
    use crate::adapters::memory_workflow_queue::MemoryWorkflowQueue;
    use crate::core::function::models::FunctionTaskDef;
    use crate::core::task::{TaskDef, TaskTypeDef};
    use crate::core::worker::{WorkerHostId, WorkerId, WorkerIdentity};
    use crate::core::workflow::models::WorkflowDef;
    use crate::ports::workflow_queue::WorkflowQueuePort;
    use chrono::TimeZone;
    use serde_json::json;

    struct TestContext {
        namespace: Namespace,
        orchestrator: Arc<Orchestrator>,
        workflow_service: Arc<WorkflowService>,
        worker_registry: WorkerRegistry,
        queue: Arc<MemoryWorkflowQueue>,
    }

    async fn test_context(workflow_def_ids: &[&str]) -> TestContext {
        let namespace = crate::core::namespace::test_namespace();
        let storage = Arc::new(MemoryStorage::new());
        let queue = Arc::new(MemoryWorkflowQueue::new(10));
        let workflow_service = Arc::new(WorkflowService::new(storage.clone()));

        for workflow_def_id in workflow_def_ids {
            workflow_service
                .create_workflow_def(&namespace, workflow_def(workflow_def_id))
                .await
                .unwrap();
        }

        TestContext {
            namespace,
            orchestrator: Arc::new(Orchestrator::new(
                storage,
                Arc::new(FakeTaskDispatcher::new()),
                queue.clone(),
            )),
            workflow_service,
            worker_registry: WorkerRegistry::new(),
            queue,
        }
    }

    fn workflow_def(id: &str) -> WorkflowDef {
        WorkflowDef {
            id: id.to_string(),
            description: String::new(),
            tasks: vec![TaskDef {
                id: "task".to_string(),
                kind: TaskTypeDef::Function(FunctionTaskDef::Inline {
                    dependencies: vec![],
                    code: "export default async function run() { return {}; }".to_string(),
                }),
                control: None,
                timeout_secs: None,
                input_schemas: vec![],
                output_schema: Some(json!({ "type": "object" })),
                workspace: None,
                required_credentials: vec![],
            }],
            data_bindings: vec![],
        }
    }

    fn schedule(
        namespace: &Namespace,
        workflow_def_id: &str,
        run_on_startup: bool,
        cron: &str,
    ) -> SchedulerEntry {
        SchedulerEntry {
            namespace: namespace.clone(),
            workflow_def_id: workflow_def_id.to_string(),
            cron: cron.to_string(),
            run_on_startup,
            input: None,
        }
    }

    async fn register_worker(worker_registry: &WorkerRegistry) {
        worker_registry
            .register_worker(WorkerIdentity {
                worker_id: WorkerId::new("worker-1"),
                host_id: WorkerHostId::new("host-1"),
            })
            .await;
    }

    fn nonmatching_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 12, 34, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn run_on_startup_defaults_to_false_when_omitted() {
        let entry: SchedulerEntry = serde_json::from_value(json!({
            "namespace": crate::core::namespace::test_namespace(),
            "workflow_def_id": "disabled",
            "cron": "0 0 1 1 *",
            "input": null
        }))
        .unwrap();

        assert!(!entry.run_on_startup);
    }

    #[tokio::test]
    async fn startup_run_ignores_a_nonmatching_cron() {
        let context = test_context(&["workflow"]).await;
        register_worker(&context.worker_registry).await;
        let mut pending_startup_entries = None;

        evaluate_scheduler_config(
            context.orchestrator,
            context.workflow_service.clone(),
            context.worker_registry,
            SchedulerConfig {
                scheduler: vec![schedule(&context.namespace, "workflow", true, "0 0 1 1 *")],
            },
            nonmatching_time(),
            &mut pending_startup_entries,
        )
        .await;

        let workflows = context
            .workflow_service
            .list_workflows(&context.namespace, None, None, None)
            .await
            .unwrap()
            .workflows;

        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0].workflow_def_id, "workflow");
        assert!(pending_startup_entries.unwrap().is_empty());
    }

    #[tokio::test]
    async fn startup_run_survives_invalid_config_and_waits_for_a_worker() {
        let context = test_context(&["workflow"]).await;
        let mut pending_startup_entries = None;

        evaluate_scheduler_config(
            context.orchestrator.clone(),
            context.workflow_service.clone(),
            context.worker_registry.clone(),
            SchedulerConfig {
                scheduler: vec![schedule(&context.namespace, "workflow", true, "invalid")],
            },
            nonmatching_time(),
            &mut pending_startup_entries,
        )
        .await;

        assert!(pending_startup_entries.is_none());

        let valid_config = SchedulerConfig {
            scheduler: vec![schedule(&context.namespace, "workflow", true, "0 0 1 1 *")],
        };

        evaluate_scheduler_config(
            context.orchestrator.clone(),
            context.workflow_service.clone(),
            context.worker_registry.clone(),
            valid_config.clone(),
            nonmatching_time(),
            &mut pending_startup_entries,
        )
        .await;

        assert!(
            pending_startup_entries
                .as_ref()
                .unwrap()
                .contains(&valid_config.scheduler[0].key())
        );
        assert!(
            context
                .queue
                .pending_ids(&context.namespace)
                .await
                .unwrap()
                .is_empty()
        );

        register_worker(&context.worker_registry).await;
        evaluate_scheduler_config(
            context.orchestrator,
            context.workflow_service,
            context.worker_registry,
            valid_config,
            nonmatching_time(),
            &mut pending_startup_entries,
        )
        .await;

        assert_eq!(
            context
                .queue
                .pending_ids(&context.namespace)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(pending_startup_entries.unwrap().is_empty());
    }

    #[tokio::test]
    async fn matching_cron_coalesces_with_startup_and_reload_does_not_repeat_it() {
        let context = test_context(&["workflow"]).await;
        register_worker(&context.worker_registry).await;
        let mut pending_startup_entries = None;
        let matching_config = SchedulerConfig {
            scheduler: vec![schedule(&context.namespace, "workflow", true, "* * * * *")],
        };

        evaluate_scheduler_config(
            context.orchestrator.clone(),
            context.workflow_service.clone(),
            context.worker_registry.clone(),
            matching_config,
            nonmatching_time(),
            &mut pending_startup_entries,
        )
        .await;

        assert_eq!(
            context
                .queue
                .pending_ids(&context.namespace)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(pending_startup_entries.as_ref().unwrap().is_empty());

        evaluate_scheduler_config(
            context.orchestrator,
            context.workflow_service,
            context.worker_registry,
            SchedulerConfig {
                scheduler: vec![schedule(&context.namespace, "workflow", true, "0 0 1 1 *")],
            },
            nonmatching_time(),
            &mut pending_startup_entries,
        )
        .await;

        assert_eq!(
            context
                .queue
                .pending_ids(&context.namespace)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
