use serde::{Deserialize, Serialize};
use tracing::{error, info};
use crate::adapters::worker_registry::WorkerRegistry;
use crate::core::namespace::Namespace;
use crate::core::orchestrator::Orchestrator;
use crate::core::workflow::workflow_service::WorkflowService;
use crate::core::consts::{RELAYFOLD_SCHEDULER_CONFIG_PATH, RELAYFOLD_SCHEDULER_ENABLED};
use anyhow::{Context, Result, ensure};
use croner::Cron;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{self, Duration};
use std::{env};
use chrono::{DateTime, Timelike, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchedulerEntry {
    namespace: Namespace,
    workflow_def_id: String,
    cron: String,
    #[serde(default = "default_run_on_startup")]
    run_on_startup: bool,
    input: Option<serde_json::Value>
}

fn default_run_on_startup() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SchedulerConfig {
    pub scheduler: Vec<SchedulerEntry>
}

fn validate_scheduler_config(config: &SchedulerConfig) -> bool {
    // for each entry, check if namespace exist, workflow_def_id exists, cron is valid
    for entry in &config.scheduler {
        // validate
        if let Err(_) = parse_cron_expr(&entry.cron) {
            return false;
        };
    }
    true
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
        .and_then(|time| time.with_second(0)).unwrap()
}

pub async fn run_scheduler(orchestrator: Arc<Orchestrator>, workflow_service: Arc<WorkflowService>, worker_registry: WorkerRegistry) {
        let is_enabled = env::var(RELAYFOLD_SCHEDULER_ENABLED)
            .map_or(false, |v| v.parse::<bool>().unwrap_or(false));

        if !is_enabled {
            info!("scheduler disabled");
            return;
        }

        let config_path = env::var(RELAYFOLD_SCHEDULER_CONFIG_PATH)
            .unwrap_or("./scheduler.yaml".to_owned());

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

        // If configuration has some issues
        if !validate_scheduler_config(&config) {
            error!("scheduler configuration is not valid!");
            return;
        }

        let schedule_count = config.scheduler.len();
        info!(%schedule_count, "evaluating schedules...");

        let time = Utc::now();

        for schedule in &config.scheduler {
            let cron = parse_cron_expr(&schedule.cron).expect("this is unexpected, cron expression should have already been validated");
            let is_match = match cron.is_time_matching(&normalize_to_minute(&time)) {
                Ok(match_result) => match_result,
                Err(err) => {
                    error!("failed while matching cron to current time {err}");
                    return;
                }
            };

            if is_match {
                let host = match worker_registry.select_eligible_host().await {
                    Some(host) => host,
                    None => {
                        error!("could not find eligible host, skipping this schedule");
                        return;
                    }
                };

                // verify and schedule - only schedule if this workflow is not currently executing, if failure assume workflow may be running and skip
                let is_workflow_pending = workflow_service.has_active_workflow_instance_for_def(&schedule.namespace, &schedule.workflow_def_id)
                    .await
                    .unwrap_or(true);

                if !is_workflow_pending {
                    match workflow_service.create_workflow_instance_for_def(&schedule.namespace, &schedule.workflow_def_id, host, schedule.input.clone()).await {
                        Ok(instance_id) => {
                            let enqueued = orchestrator
                                .enqueue_workflow_instance(&schedule.namespace, instance_id.clone())
                                .await;

                            match enqueued {
                                Ok(()) => info!(?instance_id, ?schedule, "workflow enqueued successfully"),
                                Err(error) => error!(%error, "workflow instantiation fialed")
                            }
                        },
                        Err(error) => {
                            error!(%error, "workflow instantiation fialed");
                        }
                    };

                }
            }
        }
        // log if error, do not interrupt the loop
}

pub fn start_task_scheduler(orchestrator: Arc<Orchestrator>, workflow_service: Arc<WorkflowService>, worker_registry: WorkerRegistry) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { 
        let mut interval = time::interval(Duration::from_secs(60));

        loop {
            interval.tick().await;
            run_scheduler(orchestrator.clone(), workflow_service.clone(), worker_registry.clone()).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron() {
        let cron = parse_cron_expr("0 12 * * *");
    }
}