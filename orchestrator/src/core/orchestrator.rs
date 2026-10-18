use crate::adapters::worker_registry::WorkerRegistry;
use crate::core::engine::WorkflowEngine;
use crate::core::function::function_service::resolve_task_function_ref;
use crate::core::namespace::Namespace;
use crate::core::task::{ExecutionMetadata, TaskDef, TaskStatus};
use crate::core::worker::{TaskDispatchConstraints, WorkerHostId};
use crate::core::workflow::events::WorkflowInstanceEvent;
use crate::core::workflow::models::{
    StartupWorkflowDiscovery, WorkflowInfo, WorkflowStatus, WorkflowStatusReport,
};
use crate::core::workflow::state_manager::WorkflowStateManager;
use crate::core::workflow::workflow_service::WorkflowService;
use crate::ports::storage::{
    StoragePort, WorkflowInfoCursor, WorkflowInfoPageRequest, WorkflowInstanceFilter,
};
use crate::ports::task_dispatch::TaskDispatchPort;
use crate::ports::workflow_queue::WorkflowQueuePort;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

#[cfg(test)]
#[path = "orchestrator_tests.rs"]
mod tests;

/// The application layer for the orchestrator.
/// It coordinates between the workflow engine, storage, task dispatch, and queues.
pub struct Orchestrator {
    engine: WorkflowEngine,
    storage: Arc<dyn StoragePort + Send + Sync>,
    task_dispatcher: Arc<dyn TaskDispatchPort + Send + Sync>,
    workflow_queue: Arc<dyn WorkflowQueuePort + Send + Sync>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryWorkflowTaskResult {
    pub task_attempt_id: String,
    pub pinned_host_id: Option<WorkerHostId>,
    pub local_context_may_be_lost: bool,
}

impl Orchestrator {
    pub fn new(
        storage: Arc<dyn StoragePort + Send + Sync>,
        task_dispatcher: Arc<dyn TaskDispatchPort + Send + Sync>,
        workflow_queue: Arc<dyn WorkflowQueuePort + Send + Sync>,
    ) -> Self {
        let engine = WorkflowEngine::new(storage.clone(), task_dispatcher.clone());

        Self {
            engine,
            storage,
            task_dispatcher,
            workflow_queue,
        }
    }

    /// Finds a task in a registered workflow definition and executes it directly.
    pub async fn execute_workflow_task_isolated(
        &self,
        namespace: &Namespace,
        workflow_def_id: &str,
        task_id: &str,
        inputs: &[serde_json::Value],
    ) -> anyhow::Result<Option<crate::ports::task_dispatch::ExecutionResult>> {
        let Some(def) = self
            .storage
            .get_workflow_def(namespace, workflow_def_id)
            .await?
        else {
            return Ok(None);
        };

        let Some(task) = def.tasks.into_iter().find(|task| task.id == task_id) else {
            return Ok(None);
        };

        self.execute_task_isolated_with_id(
            namespace,
            isolated_workflow_task_execution_id(workflow_def_id, task_id)?,
            &task,
            inputs,
        )
        .await
        .map(Some)
    }

    /// Adds a workflow instance to the execution queue.
    pub async fn enqueue_workflow_instance(
        &self,
        namespace: &Namespace,
        instance_id: String,
    ) -> anyhow::Result<()> {
        self.workflow_queue.enqueue(namespace, instance_id).await
    }

    pub async fn retry_workflow_task(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        task_id: &str,
    ) -> anyhow::Result<RetryWorkflowTaskResult> {
        let workflow_service = WorkflowService::new(Arc::clone(&self.storage));
        let pinned_worker_host = workflow_service
            .load_retryable_task_pin(namespace, workflow_instance_id, task_id)
            .await?;
        let task_attempt_id = workflow_service
            .retry_task(namespace, workflow_instance_id, task_id)
            .await?;
        self.enqueue_workflow_instance(namespace, workflow_instance_id.to_string())
            .await?;

        Ok(RetryWorkflowTaskResult {
            task_attempt_id,
            pinned_host_id: pinned_worker_host,
            local_context_may_be_lost: false,
        })
    }

    pub async fn force_retry_workflow_task(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        task_id: &str,
        worker_registry: &WorkerRegistry,
    ) -> anyhow::Result<RetryWorkflowTaskResult> {
        let workflow_service = WorkflowService::new(Arc::clone(&self.storage));
        let pinned_worker_host = workflow_service
            .load_retryable_task_pin(namespace, workflow_instance_id, task_id)
            .await?;

        let Some(target_host_id) = worker_registry
            .select_force_retry_host(pinned_worker_host.as_ref())
            .await
        else {
            anyhow::bail!("no eligible retry host available");
        };

        let local_context_may_be_lost = pinned_worker_host.as_ref() != Some(&target_host_id);

        let task_attempt_id = workflow_service
            .force_retry_task(
                namespace,
                workflow_instance_id,
                task_id,
                target_host_id.clone(),
            )
            .await?;

        self.enqueue_workflow_instance(namespace, workflow_instance_id.to_string())
            .await?;

        Ok(RetryWorkflowTaskResult {
            task_attempt_id,
            pinned_host_id: Some(target_host_id),
            local_context_may_be_lost,
        })
    }

    pub async fn pause_workflow_instance(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
    ) -> anyhow::Result<()> {
        let workflow_service = WorkflowService::new(Arc::clone(&self.storage));
        workflow_service
            .pause_workflow(namespace, workflow_instance_id)
            .await?;
        self.remove_queued_workflow_instance(namespace, workflow_instance_id)
            .await?;
        Ok(())
    }

    pub async fn resume_workflow_instance(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
    ) -> anyhow::Result<()> {
        let workflow_service = WorkflowService::new(Arc::clone(&self.storage));
        workflow_service
            .resume_workflow(namespace, workflow_instance_id)
            .await?;
        self.enqueue_workflow_instance(namespace, workflow_instance_id.to_string())
            .await
    }

    pub async fn pause_active_workflow_instances(
        &self,
        namespace: &Namespace,
    ) -> anyhow::Result<Vec<String>> {
        let workflow_service = WorkflowService::new(Arc::clone(&self.storage));
        let paused = workflow_service.pause_active_workflows(namespace).await?;
        for workflow_instance_id in &paused {
            self.remove_queued_workflow_instance(namespace, workflow_instance_id)
                .await?;
        }
        Ok(paused)
    }

    pub async fn resume_paused_workflow_instances(
        &self,
        namespace: &Namespace,
    ) -> anyhow::Result<Vec<String>> {
        let workflow_service = WorkflowService::new(Arc::clone(&self.storage));
        let resumed = workflow_service.resume_paused_workflows(namespace).await?;
        for workflow_instance_id in &resumed {
            self.enqueue_workflow_instance(namespace, workflow_instance_id.clone())
                .await?;
        }
        Ok(resumed)
    }

    /// Returns queued and currently running workflow instance IDs.
    pub async fn get_queue_status(&self, namespace: &Namespace) -> anyhow::Result<Vec<String>> {
        self.workflow_queue.pending_ids(namespace).await
    }

    /// Removes a pending workflow instance from the queue.
    pub async fn remove_queued_workflow_instance(
        &self,
        namespace: &Namespace,
        instance_id: &str,
    ) -> anyhow::Result<bool> {
        self.workflow_queue.remove(namespace, instance_id).await
    }

    /// Removes all pending workflow instances from the queue.
    pub async fn purge_queued_workflow_instances(
        &self,
        namespace: &Namespace,
    ) -> anyhow::Result<Vec<String>> {
        self.workflow_queue.purge(namespace).await
    }

    /// Returns a status report for a workflow instance.
    pub async fn get_workflow_status(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> anyhow::Result<Option<WorkflowStatusReport>> {
        self.engine.get_workflow_status(namespace, id).await
    }

    /// Starts or resumes execution of a workflow instance.
    pub async fn run_workflow(
        &self,
        namespace: &Namespace,
        instance_id: String,
    ) -> anyhow::Result<()> {
        self.engine
            .run_workflow_instance(namespace, instance_id)
            .await
    }

    /// Continuously consumes queued workflow instances and runs up to `max_concurrent_workflows`.
    pub async fn run_workflow_queue(self: Arc<Self>, max_concurrent_workflows: usize) {
        let max_concurrent_workflows = max_concurrent_workflows.max(1);
        let permits = Arc::new(Semaphore::new(max_concurrent_workflows));
        info!(max_concurrent_workflows, "workflow scheduler started");

        loop {
            let permit = match Arc::clone(&permits).acquire_owned().await {
                Ok(permit) => permit,
                Err(error) => {
                    error!(%error, "workflow scheduler semaphore closed");
                    break;
                }
            };

            let item = match self.workflow_queue.dequeue().await {
                Ok(item) => item,
                Err(error) => {
                    error!(%error, "workflow scheduler failed to dequeue workflow instance");
                    drop(permit);
                    break;
                }
            };

            let orchestrator = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(error) = orchestrator
                    .run_workflow(&item.namespace, item.workflow_instance_id.clone())
                    .await
                {
                    error!(
                        workflow_instance_id = %item.workflow_instance_id,
                        error = ?error,
                        "workflow execution failed"
                    );
                }
                if let Err(error) = orchestrator
                    .workflow_queue
                    .complete(&item.namespace, &item.workflow_instance_id)
                    .await
                {
                    error!(
                        workflow_instance_id = %item.workflow_instance_id,
                        error = ?error,
                        "workflow scheduler failed to mark workflow instance complete"
                    );
                }
                drop(permit);
            });
        }
    }

    /// Reconciles in-flight workflow state after an orchestrator restart.
    ///
    /// Storage is the source of truth. Any task left Running from a previous
    /// process is moved back to Pending so it can be dispatched again.
    pub async fn synchronize_startup_tasks(&self) -> anyhow::Result<usize> {
        let mut recovered = 0;

        let state_manager = WorkflowStateManager::new(Arc::clone(&self.storage));

        for info in self.list_nonterminal_workflow_info(None).await?.runnable {
            let Some(instance) = self
                .storage
                .get_workflow_instance(&info.namespace, &info.id)
                .await?
            else {
                continue;
            };

            // Tasks that need recovery
            let task_attempt_ids: Vec<String> = instance
                .tasks
                .iter()
                .filter(|(_, task)| task.status == TaskStatus::Running)
                .map(|(task_attempt_id, _)| task_attempt_id.clone())
                .collect();

            // If workflow itself or at least one task is still in Running state, we need to recover those tasks.
            let needs_recovery =
                instance.status == WorkflowStatus::Running || !task_attempt_ids.is_empty();

            if needs_recovery {
                state_manager
                    .commit_events(
                        &info.namespace,
                        &info.id,
                        vec![WorkflowInstanceEvent::StartupRecoveryApplied { task_attempt_ids }],
                    )
                    .await?;
                recovered += 1;
            }
        }

        Ok(recovered)
    }

    /// Requeues all active workflow instances found in storage.
    pub async fn enqueue_active_workflow_instances(&self) -> anyhow::Result<usize> {
        let infos = self.list_nonterminal_workflow_info(None).await?.runnable;
        let count = infos.len();

        for info in infos {
            self.enqueue_workflow_instance(&info.namespace, info.id)
                .await?;
        }

        Ok(count)
    }

    /// Fails non-terminal workflow instances pinned to hosts that heartbeat
    /// policy has declared lost. The existing pin remains on the snapshot so
    /// later retry behavior can decide whether to preserve or reassign it.
    ///
    /// Worker hosts are deployment-scoped, so discovery spans namespaces. Each
    /// returned workflow namespace is retained for authoritative reads,
    /// transitions, and queue removal.
    pub async fn fail_workflows_pinned_to_lost_hosts(
        &self,
        lost_hosts: &[WorkerHostId],
    ) -> anyhow::Result<usize> {
        if lost_hosts.is_empty() {
            return Ok(0);
        }

        let lost_hosts = lost_hosts.iter().cloned().collect::<HashSet<_>>();
        let state_manager = WorkflowStateManager::new(Arc::clone(&self.storage));
        let active = self.list_nonterminal_workflow_info(None).await?;
        let mut failed = 0;

        for info in active.runnable.into_iter().chain(active.blocked) {
            let Some(instance) = self
                .storage
                .get_workflow_instance(&info.namespace, &info.id)
                .await?
            else {
                continue;
            };
            let Some(pinned_host) = &instance.pinned_worker_host else {
                continue;
            };
            if is_terminal_workflow_status(&instance.status) || !lost_hosts.contains(pinned_host) {
                continue;
            }
            let pinned_host_id = pinned_host.0.clone();

            state_manager
                .commit_events_for_instance(
                    &info.namespace,
                    instance,
                    vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                        status: WorkflowStatus::Failed,
                    }],
                )
                .await?;
            self.remove_queued_workflow_instance(&info.namespace, &info.id)
                .await?;
            failed += 1;
            warn!(
                workflow_instance_id = %info.id,
                pinned_host_id = %pinned_host_id,
                "marked workflow failed because pinned host was declared lost"
            );
        }

        Ok(failed)
    }

    async fn execute_task_isolated_with_id(
        &self,
        namespace: &Namespace,
        isolated_execution_id: String,
        task: &TaskDef,
        inputs: &[serde_json::Value],
    ) -> anyhow::Result<crate::ports::task_dispatch::ExecutionResult> {
        let mut resolved_task = self.resolve_task_function_ref(namespace, task).await?;
        resolved_task.output_schema = task.effective_output_schema()?;
        self.task_dispatcher
            .dispatch_task(
                namespace,
                &isolated_execution_id,
                &resolved_task,
                inputs,
                &ExecutionMetadata::default(),
                &TaskDispatchConstraints::default(),
            )
            .await
    }

    async fn resolve_task_function_ref(
        &self,
        namespace: &Namespace,
        task: &TaskDef,
    ) -> anyhow::Result<TaskDef> {
        resolve_task_function_ref(self.storage.as_ref(), namespace, task).await
    }

    async fn list_nonterminal_workflow_info(
        &self,
        namespace: Option<&Namespace>,
    ) -> anyhow::Result<StartupWorkflowDiscovery> {
        let mut cursor: Option<WorkflowInfoCursor> = None;
        let mut runnable_infos: Vec<WorkflowInfo> = Vec::new();
        let mut blocked_infos: Vec<WorkflowInfo> = Vec::new();

        loop {
            let page = self
                .storage
                .list_workflow_info(
                    namespace,
                    WorkflowInfoPageRequest { limit: 100, cursor },
                    all_nonterminal_workflow_filter(),
                )
                .await?;

            let runnable: Vec<WorkflowInfo> = page
                .items
                .iter()
                .filter(|x| matches!(x.status, WorkflowStatus::Running | WorkflowStatus::Pending))
                .map(|x| x.clone())
                .collect();
            let blocked: Vec<WorkflowInfo> = page
                .items
                .iter()
                .filter(|x| {
                    matches!(
                        x.status,
                        WorkflowStatus::Paused | WorkflowStatus::InputNeeded
                    )
                })
                .map(|x| x.clone())
                .collect();

            runnable_infos.extend(runnable);
            blocked_infos.extend(blocked);
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        Ok(StartupWorkflowDiscovery {
            runnable: runnable_infos,
            blocked: blocked_infos,
        })
    }
}

fn all_nonterminal_workflow_filter() -> Vec<WorkflowInstanceFilter> {
    vec![WorkflowInstanceFilter::Statuses(vec![
        WorkflowStatus::Pending,
        WorkflowStatus::Running,
        WorkflowStatus::Paused,
        WorkflowStatus::InputNeeded,
    ])]
}

fn is_terminal_workflow_status(status: &WorkflowStatus) -> bool {
    matches!(status, WorkflowStatus::Completed | WorkflowStatus::Failed)
}

fn isolated_workflow_task_execution_id(
    workflow_def_id: &str,
    task_id: &str,
) -> anyhow::Result<String> {
    Ok(format!(
        "isolated-{workflow_def_id}-{task_id}-{}",
        timestamp_nanos()?
    ))
}

fn timestamp_nanos() -> anyhow::Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())
}
