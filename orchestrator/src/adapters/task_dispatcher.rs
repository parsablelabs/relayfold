use crate::core::namespace::Namespace;
use crate::core::task::{ExecutionMetadata, TaskDef, TaskInstance, WorkspaceKey};
use crate::core::worker::{DispatchLease, TaskDispatchConstraints, WorkerHostId, WorkerIdentity};
use crate::ports::task_dispatch::{
    ExecutionResult, TaskDispatch, TaskDispatchPort, WorkerExecutionResult, WorkerTaskResult,
};
use anyhow::{anyhow, bail};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, Notify, oneshot};
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{debug, warn};

const TASK_TIMEOUT_MONITOR_INTERVAL: Duration = Duration::from_millis(100);
const DEFAULT_TASK_TIMEOUT: Duration = Duration::from_secs(300);
static NEXT_DISPATCH_NAMESPACE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct PendingTask {
    dispatch: TaskDispatch,
    constraints: TaskDispatchConstraints,
    result_tx: oneshot::Sender<WorkerExecutionResult>,
    claimed_tx: oneshot::Sender<()>,
    timeout: Duration,
}

#[derive(Debug)]
pub struct TaskDispatcher {
    pending_tasks: Mutex<VecDeque<PendingTask>>,
    in_flight_tasks: Mutex<HashMap<String, DispatchLease>>,
    result_waiters: Mutex<HashMap<String, oneshot::Sender<WorkerExecutionResult>>>,
    dispatch_namespace: String,
    next_dispatch_id: AtomicU64,
    task_available: Notify,
    task_timeout: Duration,
}

impl Default for TaskDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskDispatcher {
    pub fn new() -> Self {
        Self {
            pending_tasks: Mutex::new(VecDeque::new()),
            in_flight_tasks: Mutex::new(HashMap::new()),
            result_waiters: Mutex::new(HashMap::new()),
            dispatch_namespace: new_dispatch_namespace(),
            next_dispatch_id: AtomicU64::new(0),
            task_available: Notify::new(),
            task_timeout: task_timeout_from_env(),
        }
    }

    #[cfg(test)]
    fn with_task_timeout(mut self, task_timeout: Duration) -> Self {
        self.task_timeout = task_timeout;
        self
    }

    async fn enqueue_task(
        &self,
        namespace: &Namespace,
        workflow_inst_id: &str,
        task: &TaskDef,
        inputs: &[serde_json::Value],
        timeout: Duration,
        execution_metadata: ExecutionMetadata,
        constraints: TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        let task_id = format!(
            "{}-{}-{}",
            task.id,
            self.dispatch_namespace,
            self.next_dispatch_id.fetch_add(1, Ordering::Relaxed)
        );

        let (result_tx, result_rx) = oneshot::channel();
        let (claimed_tx, claimed_rx) = oneshot::channel();

        let workspace_key = workspace_key_for_task(workflow_inst_id, task);
        let workspace_path_suffix = workspace_path_suffix(namespace, &workspace_key);
        let mut dispatched_task = task.clone();
        if execution_metadata.sandbox.is_some() {
            dispatched_task.timeout_secs = Some(timeout.as_secs());
        }

        self.pending_tasks.lock().await.push_back(PendingTask {
            dispatch: TaskDispatch {
                namespace: namespace.clone(),
                workflow_inst_id: workflow_inst_id.to_string(),
                task_id: task_id.clone(),
                task: dispatched_task,
                workspace_path_suffix,
                inputs: inputs.to_vec(),
                human_input_provided: execution_metadata.human_input_provided.clone(),
                execution_metadata,
            },
            constraints,
            result_tx,
            claimed_tx,
            timeout,
        });

        self.task_available.notify_one();

        if claimed_rx.await.is_err() {
            self.remove_pending_task(&task_id).await;
            return Err(anyhow!("task {task_id} was cancelled before claim"));
        }

        match result_rx.await {
            Ok(result) => Ok(result.into()),
            Err(_) => {
                self.remove_pending_task(&task_id).await;
                Err(anyhow!("task {task_id} was cancelled before completion"))
            }
        }
    }

    pub async fn claim_task(
        &self,
        worker: WorkerIdentity,
        timeout: Duration,
    ) -> anyhow::Result<Option<TaskDispatch>> {
        let worker_id = worker.worker_id.0.clone();
        let claim = async {
            loop {
                let notified = self.task_available.notified();

                let task = {
                    let mut in_flight_tasks = self.in_flight_tasks.lock().await;
                    if Self::is_worker_in_flight(&in_flight_tasks, &worker_id) {
                        bail!("Worker has active lease")
                    }

                    let mut pending_tasks = self.pending_tasks.lock().await;
                    let Some(index) = pending_tasks.iter().position(|task| {
                        task.constraints.matches_worker(&worker)
                            && !Self::is_workflow_instance_task_in_flight(&in_flight_tasks, task)
                    }) else {
                        drop(pending_tasks);
                        drop(in_flight_tasks);
                        notified.await;
                        continue;
                    };
                    let pending_task = pending_tasks.remove(index).unwrap();

                    let claimed_at_epoch_ms = epoch_ms();
                    let expires_at_epoch_ms =
                        claimed_at_epoch_ms + pending_task.timeout.as_millis() as u64;
                    in_flight_tasks.insert(
                        pending_task.dispatch.task_id.clone(),
                        DispatchLease {
                            dispatch_id: pending_task.dispatch.task_id.clone(),
                            namespace: pending_task.dispatch.namespace.clone(),
                            workflow_instance_id: pending_task.dispatch.workflow_inst_id.clone(),
                            task_attempt_id: TaskInstance::make_task_attempt_id(
                                &pending_task.dispatch.task.id,
                                pending_task.dispatch.execution_metadata.generation_index,
                            ),
                            worker_id: worker.worker_id.clone(),
                            host_id: worker.host_id.clone(),
                            claimed_at_epoch_ms,
                            expires_at_epoch_ms,
                        },
                    );
                    pending_task
                };

                let task_id = task.dispatch.task_id.clone();
                let pinned_host_id = task.constraints.pinned_host_id.clone();
                debug!(
                    %worker_id,
                    task_id = %task_id,
                    workflow_inst_id = %task.dispatch.workflow_inst_id,
                    pinned_host_id = ?pinned_host_id,
                    "worker claimed pending task"
                );

                self.result_waiters
                    .lock()
                    .await
                    .insert(task_id.clone(), task.result_tx);

                let _ = task.claimed_tx.send(());
                return Ok(task.dispatch);
            }
        };

        match time::timeout(timeout, claim).await {
            Ok(result) => result.map(Some),
            Err(_) => Ok(None),
        }
    }

    pub async fn complete_task_result(
        &self,
        worker_id: &str,
        result: WorkerTaskResult,
    ) -> anyhow::Result<()> {
        let task_id = result.task_id.clone();
        let waiter = self.result_waiters.lock().await.remove(&task_id);
        self.in_flight_tasks.lock().await.remove(&task_id);

        if let Some(waiter) = waiter {
            let _ = waiter.send(result.result);
        } else {
            warn!(worker_id = %worker_id, task_id = %task_id, "ignoring late or untracked worker task result");
        }

        Ok(())
    }

    pub async fn worker_for_active_dispatch(&self, task_id: &str) -> Option<String> {
        self.in_flight_tasks
            .lock()
            .await
            .get(task_id)
            .map(|lease| lease.worker_id.0.clone())
    }

    pub async fn cancel_pending_tasks_for_lost_hosts(&self, lost_hosts: &[WorkerHostId]) {
        if lost_hosts.is_empty() {
            return;
        }

        let lost_hosts = lost_hosts.iter().cloned().collect::<HashSet<_>>();
        let mut pending_tasks = self.pending_tasks.lock().await;
        let mut retained_tasks = VecDeque::with_capacity(pending_tasks.len());

        while let Some(task) = pending_tasks.pop_front() {
            let pinned_host_lost = task
                .constraints
                .pinned_host_id
                .as_ref()
                .is_some_and(|host_id| lost_hosts.contains(host_id));

            if pinned_host_lost {
                warn!(
                    workflow_inst_id = %task.dispatch.workflow_inst_id,
                    task_id = %task.dispatch.task_id,
                    pinned_host_id = ?task.constraints.pinned_host_id,
                    "cancelling pending task dispatch because pinned host was declared lost"
                );
            } else {
                retained_tasks.push_back(task);
            }
        }

        *pending_tasks = retained_tasks;
    }

    async fn complete_timed_out_tasks(&self) {
        let now = epoch_ms();
        let timed_out_task_ids = {
            let in_flight_tasks = self.in_flight_tasks.lock().await;
            in_flight_tasks
                .iter()
                .filter(|(_, lease)| lease.is_expired_at(now))
                .map(|(task_id, _)| task_id.clone())
                .collect::<Vec<_>>()
        };

        for task_id in timed_out_task_ids {
            self.complete_task_timeout(&task_id).await;
        }
    }

    async fn complete_task_timeout(&self, task_id: &str) {
        let Some(lease) = self.in_flight_tasks.lock().await.remove(task_id) else {
            return;
        };

        let waiter = self.result_waiters.lock().await.remove(task_id);

        if let Some(waiter) = waiter {
            let timeout =
                Duration::from_millis(lease.expires_at_epoch_ms - lease.claimed_at_epoch_ms);
            let _ = waiter.send(WorkerExecutionResult::Failure {
                reason: format!("task {task_id} timed out after {:?}", timeout),
            });
        } else {
            warn!(worker_id = %lease.worker_id.0, task_id = %task_id, "timed out task had no result waiter");
        }
    }

    async fn remove_pending_task(&self, task_id: &str) {
        let mut pending_tasks = self.pending_tasks.lock().await;
        if let Some(index) = pending_tasks
            .iter()
            .position(|task| task.dispatch.task_id == task_id)
        {
            pending_tasks.remove(index);
        }
    }

    fn is_workflow_instance_task_in_flight(
        in_flight_tasks: &HashMap<String, DispatchLease>,
        pending_task: &PendingTask,
    ) -> bool {
        in_flight_tasks.values().any(|lease| {
            lease.namespace == pending_task.dispatch.namespace
                && lease.workflow_instance_id == pending_task.dispatch.workflow_inst_id
        })
    }

    fn is_worker_in_flight(leases: &HashMap<String, DispatchLease>, worker_id: &str) -> bool {
        leases.values().any(|lease| lease.worker_id.0 == worker_id)
    }
}

#[async_trait]
impl TaskDispatchPort for TaskDispatcher {
    async fn dispatch_task(
        &self,
        namespace: &Namespace,
        workflow_inst_id: &str,
        task: &TaskDef,
        inputs: &[serde_json::Value],
        metadata: &ExecutionMetadata,
        constraints: &TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        let task_timeout = task
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(self.task_timeout);

        self.enqueue_task(
            namespace,
            workflow_inst_id,
            task,
            inputs,
            task_timeout,
            metadata.clone(),
            constraints.clone(),
        )
        .await
    }
}

pub fn start_task_timeout_monitor(dispatcher: Arc<TaskDispatcher>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = time::interval(TASK_TIMEOUT_MONITOR_INTERVAL);
        loop {
            ticker.tick().await;
            dispatcher.complete_timed_out_tasks().await;
        }
    })
}

fn workspace_key_for_task(workflow_inst_id: &str, task: &TaskDef) -> WorkspaceKey {
    match &task.workspace {
        Some(workspace) => WorkspaceKey::Group {
            workflow_inst_id: workflow_inst_id.to_string(),
            group_name: workspace.group_name.clone(),
        },
        None => WorkspaceKey::Task {
            workflow_inst_id: workflow_inst_id.to_string(),
            task_id: task.id.clone(),
        },
    }
}

fn workspace_path_suffix(namespace: &Namespace, key: &WorkspaceKey) -> PathBuf {
    match key {
        WorkspaceKey::Task {
            workflow_inst_id,
            task_id,
        } => PathBuf::from(namespace.as_str())
            .join(workflow_inst_id)
            .join(format!("taskid-{}", task_id)),
        WorkspaceKey::Group {
            workflow_inst_id,
            group_name,
        } => PathBuf::from(namespace.as_str())
            .join(workflow_inst_id)
            .join(format!("taskgroup-{}", group_name)),
    }
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn new_dispatch_namespace() -> String {
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = NEXT_DISPATCH_NAMESPACE_ID.fetch_add(1, Ordering::Relaxed);
    format!("{}-{now_nanos:x}-{sequence:x}", std::process::id())
}

fn task_timeout_from_env() -> Duration {
    std::env::var("RELAYFOLD_TASK_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_TASK_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::namespace::Namespace;
    use crate::core::task::{TaskDef, TaskTypeDef, Workspace};
    use crate::core::worker::{WorkerHostId, WorkerId};
    use serde_json::json;
    use std::path::PathBuf;

    fn test_worker(worker_id: &str) -> WorkerIdentity {
        test_worker_for_host(worker_id, "test-host")
    }

    fn test_worker_for_host(worker_id: &str, host_id: &str) -> WorkerIdentity {
        WorkerIdentity {
            worker_id: WorkerId::new(worker_id),
            host_id: WorkerHostId::new(host_id),
        }
    }

    fn other_test_namespace() -> Namespace {
        Namespace::new("550e8400-e29b-41d4-a716-446655440001").unwrap()
    }

    async fn wait_for_pending_tasks(dispatcher: &TaskDispatcher, expected_count: usize) {
        for _ in 0..10 {
            if dispatcher.pending_tasks.lock().await.len() == expected_count {
                return;
            }
            time::sleep(Duration::from_millis(1)).await;
        }

        assert_eq!(dispatcher.pending_tasks.lock().await.len(), expected_count);
    }

    #[tokio::test]
    async fn dispatcher_waits_when_no_worker_claims_task() {
        let dispatcher =
            Arc::new(TaskDispatcher::new().with_task_timeout(Duration::from_millis(10)));
        let task = test_task("task-1");
        let dispatch = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .dispatch_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-1",
                        &task,
                        &[],
                        &ExecutionMetadata::default(),
                        &TaskDispatchConstraints::default(),
                    )
                    .await
            }
        });

        time::sleep(Duration::from_millis(30)).await;

        assert!(!dispatch.is_finished());
        dispatch.abort();
    }

    #[tokio::test]
    async fn task_timeout_overrides_dispatcher_default_timeout() {
        let dispatcher = Arc::new(TaskDispatcher::new().with_task_timeout(Duration::from_secs(60)));
        let mut task = test_task("task-1");
        task.timeout_secs = Some(0);
        let execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .dispatch_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-1",
                        &task,
                        &[],
                        &ExecutionMetadata::default(),
                        &TaskDispatchConstraints::default(),
                    )
                    .await
            }
        });

        let claimed = dispatcher
            .claim_task(test_worker("worker-1"), Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();

        dispatcher.complete_timed_out_tasks().await;

        assert_eq!(
            unwrap_failure(execution.await.unwrap().unwrap()),
            format!("task {} timed out after 0ns", claimed.task_id)
        );
    }

    #[tokio::test]
    async fn worker_claims_queued_task_and_completes_result() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        let task = test_task("task-1");
        let execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "123",
                        &task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints::default(),
                    )
                    .await
            }
        });

        let claimed = dispatcher
            .claim_task(test_worker("worker-1"), Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();

        dispatcher
            .complete_task_result(
                "worker-1",
                WorkerTaskResult {
                    task_id: claimed.task_id,
                    result: WorkerExecutionResult::Success {
                        output: json!({"worker": "worker-1"}),
                    },
                },
            )
            .await
            .unwrap();

        assert_eq!(
            unwrap_success(execution.await.unwrap().unwrap()),
            json!({"worker": "worker-1"})
        );
    }

    #[tokio::test]
    async fn identical_workflow_ids_in_different_namespaces_can_be_in_flight() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        let first_namespace = crate::core::namespace::test_namespace();
        let second_namespace = other_test_namespace();
        let mut executions = vec![];

        for namespace in [first_namespace.clone(), second_namespace.clone()] {
            let dispatcher = dispatcher.clone();
            executions.push(tokio::spawn(async move {
                dispatcher
                    .enqueue_task(
                        &namespace,
                        "shared-workflow",
                        &test_task("task-1"),
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints::default(),
                    )
                    .await
            }));
        }
        wait_for_pending_tasks(&dispatcher, 2).await;

        let first_claim = dispatcher
            .claim_task(test_worker("worker-1"), Duration::from_millis(10))
            .await
            .unwrap()
            .unwrap();
        let second_claim = dispatcher
            .claim_task(test_worker("worker-2"), Duration::from_millis(10))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(first_claim.workflow_inst_id, "shared-workflow");
        assert_eq!(second_claim.workflow_inst_id, "shared-workflow");
        assert_ne!(first_claim.namespace, second_claim.namespace);
        assert_eq!(
            HashSet::from([first_claim.namespace, second_claim.namespace]),
            HashSet::from([first_namespace, second_namespace])
        );

        for execution in executions {
            execution.abort();
        }
    }

    #[tokio::test]
    async fn worker_with_active_lease_cannot_claim_another_task() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        let first_task = test_task("task-1");
        let first_execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-1",
                        &first_task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints::default(),
                    )
                    .await
            }
        });

        let first_claim = dispatcher
            .claim_task(test_worker("worker-1"), Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();

        let second_task = test_task("task-2");
        let second_execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-2",
                        &second_task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints::default(),
                    )
                    .await
            }
        });
        wait_for_pending_tasks(&dispatcher, 1).await;

        let error = dispatcher
            .claim_task(test_worker("worker-1"), Duration::from_millis(10))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("active lease"));

        dispatcher
            .complete_task_result(
                "worker-1",
                WorkerTaskResult {
                    task_id: first_claim.task_id,
                    result: WorkerExecutionResult::Success {
                        output: json!({"task": "first"}),
                    },
                },
            )
            .await
            .unwrap();

        let second_claim = dispatcher
            .claim_task(test_worker("worker-1"), Duration::from_millis(10))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second_claim.workflow_inst_id, "workflow-2");

        dispatcher
            .complete_task_result(
                "worker-1",
                WorkerTaskResult {
                    task_id: second_claim.task_id,
                    result: WorkerExecutionResult::Success {
                        output: json!({"task": "second"}),
                    },
                },
            )
            .await
            .unwrap();

        assert_eq!(
            unwrap_success(first_execution.await.unwrap().unwrap()),
            json!({"task": "first"})
        );
        assert_eq!(
            unwrap_success(second_execution.await.unwrap().unwrap()),
            json!({"task": "second"})
        );
    }

    #[tokio::test]
    async fn queued_task_does_not_time_out_before_claim() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        let task = test_task("task-1");
        let execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "123",
                        &task,
                        &[],
                        Duration::from_millis(10),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints::default(),
                    )
                    .await
            }
        });

        time::sleep(Duration::from_millis(30)).await;
        assert!(!execution.is_finished());

        let claimed = dispatcher
            .claim_task(test_worker("worker-1"), Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        dispatcher
            .complete_task_result(
                "worker-1",
                WorkerTaskResult {
                    task_id: claimed.task_id,
                    result: WorkerExecutionResult::Success {
                        output: json!({"worker": "worker-1"}),
                    },
                },
            )
            .await
            .unwrap();

        assert_eq!(
            unwrap_success(execution.await.unwrap().unwrap()),
            json!({"worker": "worker-1"})
        );
    }

    #[tokio::test]
    async fn claimed_task_dispatch_includes_logical_workspace_metadata() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        let task = test_task("task-1");
        let execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "123",
                        &task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints::default(),
                    )
                    .await
            }
        });

        let claimed = dispatcher
            .claim_task(test_worker("worker-1"), Duration::from_millis(10))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            serde_json::to_value(&claimed).unwrap()["workspace_path_suffix"],
            json!("550e8400-e29b-41d4-a716-446655440000/123/taskid-task-1")
        );
        execution.abort();
    }

    #[tokio::test]
    async fn shared_workspace_tasks_reuse_suffix_and_workflow_pin() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        let mut first_task = test_task("first");
        first_task.workspace = Some(Workspace {
            group_name: "repo".to_string(),
        });
        let first_execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-1",
                        &first_task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints {
                            pinned_host_id: Some(WorkerHostId::new("host-a")),
                        },
                    )
                    .await
            }
        });

        let mut second_task = test_task("second");
        second_task.workspace = Some(Workspace {
            group_name: "repo".to_string(),
        });
        let second_execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-1",
                        &second_task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints {
                            pinned_host_id: Some(WorkerHostId::new("host-a")),
                        },
                    )
                    .await
            }
        });
        wait_for_pending_tasks(&dispatcher, 2).await;

        let mismatched_claim = dispatcher
            .claim_task(
                test_worker_for_host("worker-b", "host-b"),
                Duration::from_millis(10),
            )
            .await
            .unwrap();
        assert!(mismatched_claim.is_none());

        let first_claim = dispatcher
            .claim_task(
                test_worker_for_host("worker-a", "host-a"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            first_claim.workspace_path_suffix,
            PathBuf::from("550e8400-e29b-41d4-a716-446655440000/workflow-1/taskgroup-repo")
        );

        dispatcher
            .complete_task_result(
                "worker-a",
                WorkerTaskResult {
                    task_id: first_claim.task_id,
                    result: WorkerExecutionResult::Success {
                        output: json!({"task": "first"}),
                    },
                },
            )
            .await
            .unwrap();

        let second_claim = dispatcher
            .claim_task(
                test_worker_for_host("worker-a", "host-a"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            second_claim.workspace_path_suffix,
            PathBuf::from("550e8400-e29b-41d4-a716-446655440000/workflow-1/taskgroup-repo")
        );

        first_execution.abort();
        second_execution.abort();
    }

    #[tokio::test]
    async fn multiple_workers_sharing_host_can_claim_pinned_workflows() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        for (workflow_id, task_id) in [("workflow-a", "task-a"), ("workflow-b", "task-b")] {
            let task = test_task(task_id);
            tokio::spawn({
                let dispatcher = dispatcher.clone();
                async move {
                    dispatcher
                        .enqueue_task(
                            &crate::core::namespace::test_namespace(),
                            workflow_id,
                            &task,
                            &[],
                            Duration::from_secs(5),
                            ExecutionMetadata::default(),
                            TaskDispatchConstraints {
                                pinned_host_id: Some(WorkerHostId::new("host-a")),
                            },
                        )
                        .await
                }
            });
        }
        wait_for_pending_tasks(&dispatcher, 2).await;

        let first_claim = dispatcher
            .claim_task(
                test_worker_for_host("worker-1", "host-a"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();
        let second_claim = dispatcher
            .claim_task(
                test_worker_for_host("worker-2", "host-a"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(first_claim.workflow_inst_id, "workflow-a");
        assert_eq!(second_claim.workflow_inst_id, "workflow-b");
    }

    #[tokio::test]
    async fn claimed_task_records_dispatch_lease_metadata() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        let task = test_task("task-1");
        let execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-1",
                        &task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata {
                            sandbox: Some(Default::default()),
                            generation_index: 3,
                            loop_context: None,
                            human_input_provided: None,
                        },
                        TaskDispatchConstraints {
                            pinned_host_id: Some(WorkerHostId::new("host-a")),
                        },
                    )
                    .await
            }
        });

        let before_claim_epoch_ms = epoch_ms();
        let claimed = dispatcher
            .claim_task(
                test_worker_for_host("worker-1", "host-a"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();
        let after_claim_epoch_ms = epoch_ms();

        assert!(claimed.execution_metadata.sandbox.is_some());
        assert_eq!(claimed.task.timeout_secs, Some(5));

        let in_flight_tasks = dispatcher.in_flight_tasks.lock().await;
        let lease = in_flight_tasks.get(&claimed.task_id).unwrap();

        assert_eq!(lease.dispatch_id, claimed.task_id);
        assert_eq!(lease.namespace, crate::core::namespace::test_namespace());
        assert_eq!(lease.workflow_instance_id, "workflow-1");
        assert_eq!(lease.task_attempt_id, "task-1[3]");
        assert_eq!(lease.worker_id, WorkerId::new("worker-1"));
        assert_eq!(lease.host_id, WorkerHostId::new("host-a"));
        assert!(lease.claimed_at_epoch_ms >= before_claim_epoch_ms);
        assert!(lease.claimed_at_epoch_ms <= after_claim_epoch_ms);
        assert_eq!(lease.expires_at_epoch_ms - lease.claimed_at_epoch_ms, 5_000);

        execution.abort();
    }

    #[tokio::test]
    async fn stale_pre_restart_dispatch_result_does_not_complete_recovered_dispatch() {
        let old_dispatcher = Arc::new(TaskDispatcher::new());
        let task = test_task("task-1");
        let old_execution = tokio::spawn({
            let old_dispatcher = old_dispatcher.clone();
            let task = task.clone();
            async move {
                old_dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-1",
                        &task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints {
                            pinned_host_id: Some(WorkerHostId::new("host-a")),
                        },
                    )
                    .await
            }
        });

        let old_claim = old_dispatcher
            .claim_task(
                test_worker_for_host("old-worker", "host-a"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();
        let stale_task_id = old_claim.task_id;
        old_execution.abort();

        let recovered_dispatcher = Arc::new(TaskDispatcher::new());
        let recovered_execution = tokio::spawn({
            let recovered_dispatcher = recovered_dispatcher.clone();
            async move {
                recovered_dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-1",
                        &task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints {
                            pinned_host_id: Some(WorkerHostId::new("host-a")),
                        },
                    )
                    .await
            }
        });

        let recovered_claim = recovered_dispatcher
            .claim_task(
                test_worker_for_host("new-worker", "host-a"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();

        assert_ne!(stale_task_id, recovered_claim.task_id);

        recovered_dispatcher
            .complete_task_result(
                "old-worker",
                WorkerTaskResult {
                    task_id: stale_task_id,
                    result: WorkerExecutionResult::Success {
                        output: json!({"source": "stale"}),
                    },
                },
            )
            .await
            .unwrap();

        assert_eq!(
            recovered_dispatcher
                .worker_for_active_dispatch(&recovered_claim.task_id)
                .await,
            Some("new-worker".to_string())
        );

        recovered_dispatcher
            .complete_task_result(
                "new-worker",
                WorkerTaskResult {
                    task_id: recovered_claim.task_id,
                    result: WorkerExecutionResult::Success {
                        output: json!({"source": "recovered"}),
                    },
                },
            )
            .await
            .unwrap();

        assert_eq!(
            unwrap_success(recovered_execution.await.unwrap().unwrap()),
            json!({"source": "recovered"})
        );
    }

    #[tokio::test]
    async fn worker_claim_scans_past_nonmatching_pinned_tasks() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        for (workflow_id, task_id, host_id) in [
            ("workflow-a", "task-a", "host-a"),
            ("workflow-b", "task-b", "host-b"),
        ] {
            let task = test_task(task_id);
            tokio::spawn({
                let dispatcher = dispatcher.clone();
                async move {
                    dispatcher
                        .enqueue_task(
                            &crate::core::namespace::test_namespace(),
                            workflow_id,
                            &task,
                            &[],
                            Duration::from_secs(5),
                            ExecutionMetadata::default(),
                            TaskDispatchConstraints {
                                pinned_host_id: Some(WorkerHostId::new(host_id)),
                            },
                        )
                        .await
                }
            });
        }
        wait_for_pending_tasks(&dispatcher, 2).await;

        let claimed = dispatcher
            .claim_task(
                test_worker_for_host("worker-1", "host-b"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(claimed.workflow_inst_id, "workflow-b");
        assert_eq!(dispatcher.pending_tasks.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn worker_claim_skips_workflow_with_active_dispatch() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        let active_task = test_task("active-task");
        let active_execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-1",
                        &active_task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints {
                            pinned_host_id: Some(WorkerHostId::new("host-a")),
                        },
                    )
                    .await
            }
        });
        let active_claim = dispatcher
            .claim_task(
                test_worker_for_host("worker-1", "host-a"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();

        for (workflow_id, task_id) in [("workflow-1", "blocked-task"), ("workflow-2", "other-task")]
        {
            let task = test_task(task_id);
            tokio::spawn({
                let dispatcher = dispatcher.clone();
                async move {
                    dispatcher
                        .enqueue_task(
                            &crate::core::namespace::test_namespace(),
                            workflow_id,
                            &task,
                            &[],
                            Duration::from_secs(5),
                            ExecutionMetadata::default(),
                            TaskDispatchConstraints {
                                pinned_host_id: Some(WorkerHostId::new("host-a")),
                            },
                        )
                        .await
                }
            });
        }
        wait_for_pending_tasks(&dispatcher, 2).await;

        let other_claim = dispatcher
            .claim_task(
                test_worker_for_host("worker-2", "host-a"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(other_claim.workflow_inst_id, "workflow-2");
        assert_eq!(dispatcher.pending_tasks.lock().await.len(), 1);

        dispatcher
            .complete_task_result(
                "worker-1",
                WorkerTaskResult {
                    task_id: active_claim.task_id,
                    result: WorkerExecutionResult::Success {
                        output: json!({"task": "active"}),
                    },
                },
            )
            .await
            .unwrap();

        let blocked_claim = dispatcher
            .claim_task(
                test_worker_for_host("worker-1", "host-a"),
                Duration::from_millis(10),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(blocked_claim.workflow_inst_id, "workflow-1");

        active_execution.abort();
    }

    #[tokio::test]
    async fn claimed_task_times_out_through_result_waiter() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        let task = test_task("task-1");
        let execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "123",
                        &task,
                        &[],
                        Duration::from_millis(10),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints::default(),
                    )
                    .await
            }
        });

        let claimed = dispatcher
            .claim_task(test_worker("worker-1"), Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();

        time::sleep(Duration::from_millis(20)).await;
        dispatcher.complete_timed_out_tasks().await;

        assert_eq!(
            unwrap_failure(execution.await.unwrap().unwrap()),
            format!("task {} timed out after 10ms", claimed.task_id)
        );
        assert_eq!(
            dispatcher
                .worker_for_active_dispatch(&claimed.task_id)
                .await,
            None
        );
    }

    #[tokio::test]
    async fn lost_host_cancels_pending_dispatches_pinned_to_that_host() {
        let dispatcher = Arc::new(TaskDispatcher::new());
        let task = test_task("task-1");
        let execution = tokio::spawn({
            let dispatcher = dispatcher.clone();
            async move {
                dispatcher
                    .enqueue_task(
                        &crate::core::namespace::test_namespace(),
                        "workflow-1",
                        &task,
                        &[],
                        Duration::from_secs(5),
                        ExecutionMetadata::default(),
                        TaskDispatchConstraints {
                            pinned_host_id: Some(WorkerHostId::new("host-a")),
                        },
                    )
                    .await
            }
        });
        wait_for_pending_tasks(&dispatcher, 1).await;

        dispatcher
            .cancel_pending_tasks_for_lost_hosts(&[WorkerHostId::new("host-a")])
            .await;

        assert!(dispatcher.pending_tasks.lock().await.is_empty());
        let error = execution.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("cancelled before claim"));
    }

    fn test_task(id: &str) -> TaskDef {
        TaskDef {
            sandbox: None,
            id: id.to_string(),
            kind: TaskTypeDef::Function(crate::core::function::models::FunctionTaskDef::Inline {
                dependencies: vec![],
                code: "return 1".to_string(),
            }),
            control: None,
            timeout_secs: None,
            input_schemas: vec![],
            output_schema: None,
            workspace: None,
            required_credentials: vec![],
        }
    }

    fn unwrap_success(result: ExecutionResult) -> serde_json::Value {
        match result {
            ExecutionResult::Success(output) => output,
            other => panic!("expected success, got {other:?}"),
        }
    }

    fn unwrap_failure(result: ExecutionResult) -> String {
        match result {
            ExecutionResult::Failure(reason) => reason,
            other => panic!("expected failure, got {other:?}"),
        }
    }
}
