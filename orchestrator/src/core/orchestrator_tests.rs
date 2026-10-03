use super::*;
use crate::adapters::fake_task_dispatcher::FakeTaskDispatcher;
use crate::adapters::memory_storage::MemoryStorage;
use crate::adapters::memory_workflow_queue::MemoryWorkflowQueue;
use crate::adapters::worker_registry::WorkerRegistry;
use crate::core::function::function_service::FunctionService;
use crate::core::function::models::{FunctionDef, FunctionTaskDef};
use crate::core::task::{
    ExecutionMetadata, TaskDef, TaskInstance, TaskSatisfactionStatus, TaskStatus, TaskTypeDef,
    Workspace,
};
use crate::core::verifier::verifier_decision_schema;
use crate::core::worker::{WorkerHostId, WorkerId, WorkerIdentity};
use crate::core::workflow::models::{DataBinding, WorkflowDef, WorkflowInstance};
use crate::core::workflow::workflow_service::WorkflowService;
use crate::ports::storage::{StoragePort, TaskResult};
use crate::ports::task_dispatch::ExecutionResult;
use crate::ports::task_dispatch::TaskDispatchPort;
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::time::{Duration, sleep};

fn orchestrator() -> Orchestrator {
    Orchestrator::new(
        Arc::new(MemoryStorage::new()),
        Arc::new(FakeTaskDispatcher::new()),
        Arc::new(MemoryWorkflowQueue::new(10)),
    )
}

fn other_namespace() -> Namespace {
    Namespace::new("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap()
}

fn orchestrator_with_services() -> (Orchestrator, WorkflowService, FunctionService) {
    let storage = Arc::new(MemoryStorage::new());
    (
        Orchestrator::new(
            storage.clone(),
            Arc::new(FakeTaskDispatcher::new()),
            Arc::new(MemoryWorkflowQueue::new(10)),
        ),
        WorkflowService::new(storage.clone()),
        FunctionService::new(storage),
    )
}

struct CountingDispatcher {
    active: AtomicUsize,
    max_active: AtomicUsize,
    delay: Duration,
}

impl CountingDispatcher {
    fn new(delay: Duration) -> Self {
        Self {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            delay,
        }
    }

    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TaskDispatchPort for CountingDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &crate::core::namespace::Namespace,
        _workflow_inst_id: &str,
        _task: &TaskDef,
        _inputs: &[serde_json::Value],
        _metadata: &ExecutionMetadata,
        _dispatch: &crate::core::worker::TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        sleep(self.delay).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(ExecutionResult::Success(json!({})))
    }
}

#[derive(Clone, Debug)]
struct RecordedIsolatedExecution {
    namespace: crate::core::namespace::Namespace,
    workflow_inst_id: String,
    task_id: String,
}

struct RecordingIsolatedDispatcher {
    records: StdMutex<Vec<RecordedIsolatedExecution>>,
}

impl RecordingIsolatedDispatcher {
    fn new() -> Self {
        Self {
            records: StdMutex::new(vec![]),
        }
    }

    fn records(&self) -> Vec<RecordedIsolatedExecution> {
        self.records.lock().unwrap().clone()
    }
}

#[async_trait]
impl TaskDispatchPort for RecordingIsolatedDispatcher {
    async fn dispatch_task(
        &self,
        namespace: &crate::core::namespace::Namespace,
        workflow_inst_id: &str,
        task: &TaskDef,
        _inputs: &[serde_json::Value],
        _metadata: &ExecutionMetadata,
        _dispatch: &crate::core::worker::TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        self.records
            .lock()
            .unwrap()
            .push(RecordedIsolatedExecution {
                namespace: namespace.clone(),
                workflow_inst_id: workflow_inst_id.to_string(),
                task_id: task.id.clone(),
            });
        Ok(ExecutionResult::Success(json!({})))
    }
}

fn task(id: &str) -> TaskDef {
    TaskDef {
        sandbox: None,
        id: id.to_string(),
        kind: TaskTypeDef::Function(FunctionTaskDef::Inline {
            dependencies: vec![],
            code: "export default async function run() { return {}; }".to_string(),
        }),
        control: None,
        timeout_secs: None,
        input_schemas: vec![],
        output_schema: Some(json!({
            "type": "object",
            "required": ["ok"],
            "properties": {
                "ok": { "type": "boolean" }
            }
        })),
        workspace: None,
        required_credentials: vec![],
    }
}

fn function_ref_task(id: &str, reference: &str) -> TaskDef {
    TaskDef {
        sandbox: None,
        id: id.to_string(),
        kind: TaskTypeDef::Function(FunctionTaskDef::Ref {
            reference: reference.to_string(),
        }),
        control: None,
        timeout_secs: None,
        input_schemas: vec![],
        output_schema: Some(json!({
            "type": "object",
            "required": ["ok"],
            "properties": {
                "ok": { "type": "boolean" }
            }
        })),
        workspace: None,
        required_credentials: vec![],
    }
}

fn workflow(id: &str, tasks: Vec<TaskDef>) -> WorkflowDef {
    WorkflowDef {
        sandbox: None,
        id: id.to_string(),
        description: String::new(),
        tasks,
        data_bindings: vec![],
    }
}

fn workflow_instance(id: &str, workflow_def_id: &str) -> WorkflowInstance {
    WorkflowInstance {
        id: id.to_string(),
        workflow_def_id: workflow_def_id.to_string(),
        version: 0,
        status: WorkflowStatus::Pending,
        trigger_input: None,
        pinned_worker_host: None,
        tasks: HashMap::new(),
        verifier_states: HashMap::new(),
    }
}

#[tokio::test]
async fn execute_workflow_task_isolated_finds_registered_task() {
    let (orchestrator, workflow_service, _) = orchestrator_with_services();
    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![task("taska")]),
        )
        .await
        .unwrap();

    let result = orchestrator
        .execute_workflow_task_isolated(
            &crate::core::namespace::test_namespace(),
            "workflow1",
            "taska",
            &[],
        )
        .await
        .unwrap();

    assert_eq!(
        result,
        Some(ExecutionResult::Success(json!({ "ok": false })))
    );
}

#[tokio::test]
async fn execute_workflow_task_isolated_scopes_task_lookup_to_workflow_def() {
    let (orchestrator, workflow_service, _) = orchestrator_with_services();
    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![task("taska")]),
        )
        .await
        .unwrap();
    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow2", vec![task("taska")]),
        )
        .await
        .unwrap();

    let result = orchestrator
        .execute_workflow_task_isolated(
            &crate::core::namespace::test_namespace(),
            "workflow2",
            "taska",
            &[],
        )
        .await
        .unwrap();

    assert_eq!(
        result,
        Some(ExecutionResult::Success(json!({ "ok": false })))
    );
}

#[tokio::test]
async fn execute_workflow_task_isolated_resolves_registered_function_ref() {
    let (orchestrator, workflow_service, function_service) = orchestrator_with_services();
    function_service
        .create_function_def(
            &crate::core::namespace::test_namespace(),
            FunctionDef {
                id: "functiona".to_string(),
                dependencies: vec![],
                code: "export default async function run() { return {}; }".to_string(),
            },
        )
        .await
        .unwrap();
    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![function_ref_task("taska", "functiona")]),
        )
        .await
        .unwrap();

    let result = orchestrator
        .execute_workflow_task_isolated(
            &crate::core::namespace::test_namespace(),
            "workflow1",
            "taska",
            &[],
        )
        .await
        .unwrap();

    assert_eq!(
        result,
        Some(ExecutionResult::Success(json!({ "ok": false })))
    );
}

#[tokio::test]
async fn execute_workflow_task_isolated_uses_generated_isolated_execution_id() {
    let storage = Arc::new(MemoryStorage::new());
    let dispatcher = Arc::new(RecordingIsolatedDispatcher::new());
    let workflow_service = WorkflowService::new(storage.clone());
    let orchestrator = Orchestrator::new(
        storage,
        dispatcher.clone(),
        Arc::new(MemoryWorkflowQueue::new(10)),
    );

    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![task("taska")]),
        )
        .await
        .unwrap();

    orchestrator
        .execute_workflow_task_isolated(
            &crate::core::namespace::test_namespace(),
            "workflow1",
            "taska",
            &[],
        )
        .await
        .unwrap();

    let records = dispatcher.records();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].namespace,
        crate::core::namespace::test_namespace()
    );
    assert_eq!(records[0].task_id, "taska");
    assert!(
        records[0]
            .workflow_inst_id
            .starts_with("isolated-workflow1-taska-")
    );
    assert_ne!(records[0].workflow_inst_id, "123");
}

#[tokio::test]
async fn execute_workflow_task_isolated_errors_for_missing_function_ref() {
    let (orchestrator, workflow_service, _) = orchestrator_with_services();
    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow(
                "workflow1",
                vec![function_ref_task("taska", "missingfunction")],
            ),
        )
        .await
        .unwrap();

    let error = orchestrator
        .execute_workflow_task_isolated(
            &crate::core::namespace::test_namespace(),
            "workflow1",
            "taska",
            &[],
        )
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("Function definition not found: missingfunction")
    );
}

#[tokio::test]
async fn scheduler_limits_concurrent_workflow_execution() {
    let storage = Arc::new(MemoryStorage::new());
    let dispatcher = Arc::new(CountingDispatcher::new(Duration::from_millis(50)));
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator = Arc::new(Orchestrator::new(
        storage.clone(),
        dispatcher.clone(),
        queue,
    ));
    let scheduler = tokio::spawn(orchestrator.clone().run_workflow_queue(2));

    for id in ["workflow-1", "workflow-2", "workflow-3"] {
        storage
            .save_workflow_def(
                &crate::core::namespace::test_namespace(),
                workflow(
                    id,
                    vec![TaskDef {
                        output_schema: None,
                        ..task("task-a")
                    }],
                ),
            )
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                workflow_instance(id, id),
            )
            .await
            .unwrap();
        orchestrator
            .enqueue_workflow_instance(&crate::core::namespace::test_namespace(), id.to_string())
            .await
            .unwrap();
    }

    for _ in 0..20 {
        let mut completed = 0;
        for id in ["workflow-1", "workflow-2", "workflow-3"] {
            let instance = storage
                .get_workflow_instance(&crate::core::namespace::test_namespace(), id)
                .await
                .unwrap()
                .unwrap();
            if instance.status == WorkflowStatus::Completed {
                completed += 1;
            }
        }
        if completed == 3 {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(dispatcher.max_active(), 2);
    scheduler.abort();
}

#[tokio::test]
async fn isolated_workflow_task_execution_does_not_require_scheduler() {
    let (orchestrator, workflow_service, _) = orchestrator_with_services();
    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![task("taska")]),
        )
        .await
        .unwrap();

    let result = orchestrator
        .execute_workflow_task_isolated(
            &crate::core::namespace::test_namespace(),
            "workflow1",
            "taska",
            &[],
        )
        .await
        .unwrap();

    assert!(matches!(result, Some(ExecutionResult::Success(_))));
}

#[tokio::test]
async fn create_workflow_def_accepts_missing_input_schemas() {
    let storage = Arc::new(MemoryStorage::new());
    let workflow_service = WorkflowService::new(storage.clone());
    let workflow_def: WorkflowDef = serde_json::from_value(json!({
        "id": "workflow1",
        "tasks": [
            {
                "id": "taska",
                "kind": {
                    "function": {
                        "dependencies": [],
                        "code": "export default async function run() { return {}; }"
                    }
                },
                "output_schema": {
                    "type": "object"
                },
                "required_credentials": []
            }
        ],
        "data_bindings": []
    }))
    .unwrap();

    workflow_service
        .create_workflow_def(&crate::core::namespace::test_namespace(), workflow_def)
        .await
        .unwrap();

    let stored = storage
        .get_workflow_def(&crate::core::namespace::test_namespace(), "workflow1")
        .await
        .unwrap()
        .unwrap();
    assert!(stored.tasks[0].input_schemas.is_empty());
}

#[tokio::test]
async fn workflow_without_control_verifier_deserializes_and_executes() {
    let (orchestrator, workflow_service, _) = orchestrator_with_services();
    let workflow_def: WorkflowDef = serde_json::from_value(json!({
        "id": "workflow1",
        "tasks": [
            {
                "id": "taska",
                "kind": {
                    "function": {
                        "dependencies": [],
                        "code": "export default async function run() { return {}; }"
                    }
                },
                "output_schema": {
                    "type": "object"
                },
                "required_credentials": []
            }
        ],
        "data_bindings": []
    }))
    .unwrap();

    assert!(workflow_def.tasks[0].control.is_none());

    workflow_service
        .create_workflow_def(&crate::core::namespace::test_namespace(), workflow_def)
        .await
        .unwrap();
    let instance_id = workflow_service
        .create_workflow_instance_for_def(
            &crate::core::namespace::test_namespace(),
            "workflow1",
            WorkerHostId::new("test-host"),
            None,
        )
        .await
        .unwrap();

    orchestrator
        .run_workflow(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    let report = orchestrator
        .get_workflow_status(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.status, WorkflowStatus::Completed);
    assert!(report.verifier_states.is_empty());
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].task_attempt_id, "taska[1]");
    assert_eq!(report.tasks[0].task_def_id, "taska");
    assert_eq!(report.tasks[0].status, TaskStatus::Completed);
    assert!(report.tasks[0].verifier_metadata.is_none());
}

#[tokio::test]
async fn get_task_result_resolves_logical_task_id_to_generation_one() {
    let (orchestrator, workflow_service, _) = orchestrator_with_services();
    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![task("taska")]),
        )
        .await
        .unwrap();
    let instance_id = workflow_service
        .create_workflow_instance_for_def(
            &crate::core::namespace::test_namespace(),
            "workflow1",
            WorkerHostId::new("test-host"),
            None,
        )
        .await
        .unwrap();

    orchestrator
        .run_workflow(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    match workflow_service
        .get_task_result(
            &crate::core::namespace::test_namespace(),
            &instance_id,
            "taska",
        )
        .await
        .unwrap()
    {
        TaskResult::Success {
            input,
            output,
            metadata: Some(metadata),
        } => {
            assert_eq!(input, Vec::<serde_json::Value>::new());
            assert_eq!(output, json!({ "ok": false }));
            assert_eq!(metadata.task_def_id, "taska");
            assert_eq!(metadata.task_attempt_id, "taska[1]");
            assert_eq!(metadata.generation_index, 1);
        }
        result => panic!("expected success with metadata, got {result:?}"),
    }
}

#[tokio::test]
async fn list_task_results_returns_materialized_attempts() {
    let (orchestrator, workflow_service, _) = orchestrator_with_services();
    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![task("taska")]),
        )
        .await
        .unwrap();
    let instance_id = workflow_service
        .create_workflow_instance_for_def(
            &crate::core::namespace::test_namespace(),
            "workflow1",
            WorkerHostId::new("test-host"),
            None,
        )
        .await
        .unwrap();

    orchestrator
        .run_workflow(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    let tasks = workflow_service
        .list_task_results(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap();

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_attempt_id, "taska[1]");
    match &tasks[0].result {
        TaskResult::Success {
            input,
            output,
            metadata: Some(metadata),
        } => {
            assert_eq!(input, &Vec::<serde_json::Value>::new());
            assert_eq!(output, &json!({ "ok": false }));
            assert_eq!(metadata.task_def_id, "taska");
            assert_eq!(metadata.task_attempt_id, "taska[1]");
            assert_eq!(metadata.generation_index, 1);
        }
        result => panic!("expected success with metadata, got {result:?}"),
    }
}

#[tokio::test]
async fn verifier_control_accepts_function_task_and_injects_decision_schema() {
    let storage = Arc::new(MemoryStorage::new());
    let workflow_service = WorkflowService::new(storage.clone());
    let mut verifier = task("verify");
    verifier.output_schema = None;
    verifier.control = Some(crate::core::task::TaskControl {
        verifier: Some(crate::core::verifier::VerifierControlConfig {
            max_iterations: 2,
            on_exhausted_continue: false,
            rerun_from_task_id: None,
        }),
    });

    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![verifier]),
        )
        .await
        .unwrap();

    let def = storage
        .get_workflow_def(&crate::core::namespace::test_namespace(), "workflow1")
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(def.tasks[0].kind, TaskTypeDef::Function(_)));
    assert_eq!(def.tasks[0].output_schema, Some(verifier_decision_schema()));
}

#[tokio::test]
async fn verifier_control_rejects_user_output_schema() {
    let workflow_service = WorkflowService::new(Arc::new(MemoryStorage::new()));
    let mut verifier = task("verify");
    verifier.control = Some(crate::core::task::TaskControl {
        verifier: Some(crate::core::verifier::VerifierControlConfig {
            max_iterations: 2,
            on_exhausted_continue: false,
            rerun_from_task_id: None,
        }),
    });

    let error = workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![verifier]),
        )
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("control.verifier and must not declare output_schema")
    );
}

#[tokio::test]
async fn create_workflow_def_normalizes_workflow_def_task_def_and_binding_ids() {
    let storage = Arc::new(MemoryStorage::new());
    let workflow_service = WorkflowService::new(storage.clone());
    let mut task_a = task("Task_A");
    task_a.workspace = Some(Workspace {
        group_name: "Repo_Cache".to_string(),
    });
    let mut task_b = task("Task-B");
    task_b.input_schemas = vec![json!({ "type": "object" })];

    workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            WorkflowDef {
                sandbox: None,
                id: "Workflow_ABC-1".to_string(),
                description: String::new(),
                tasks: vec![task_a, task_b],
                data_bindings: vec![DataBinding {
                    source_task_id: "Task_A".to_string(),
                    target_task_id: "Task-B".to_string(),
                }],
            },
        )
        .await
        .unwrap();

    let stored = storage
        .get_workflow_def(&crate::core::namespace::test_namespace(), "workflow_abc-1")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(stored.id, "workflow_abc-1");
    assert_eq!(stored.tasks[0].id, "task_a");
    assert_eq!(
        stored.tasks[0]
            .workspace
            .as_ref()
            .map(|workspace| workspace.group_name.as_str()),
        Some("repo_cache")
    );
    assert_eq!(stored.tasks[1].id, "task-b");
    assert_eq!(stored.data_bindings[0].source_task_id, "task_a");
    assert_eq!(stored.data_bindings[0].target_task_id, "task-b");
}

#[tokio::test]
async fn create_workflow_def_rejects_invalid_identifier_characters() {
    let workflow_service = WorkflowService::new(Arc::new(MemoryStorage::new()));

    let workflow_error = workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow.1", vec![task("taska")]),
        )
        .await
        .unwrap_err();
    assert!(workflow_error.to_string().contains(
        "workflow id \"workflow.1\" must contain only ASCII alphanumeric characters, '-' or '_'"
    ));

    let task_error = workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![task("task a")]),
        )
        .await
        .unwrap_err();
    assert!(task_error.to_string().contains(
        "task id \"task a\" must contain only ASCII alphanumeric characters, '-' or '_'"
    ));

    let mut task_with_workspace = task("taska");
    task_with_workspace.workspace = Some(Workspace {
        group_name: "repo.cache".to_string(),
    });
    let workspace_error = workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            workflow("workflow1", vec![task_with_workspace]),
        )
        .await
        .unwrap_err();
    assert!(
        workspace_error
            .to_string()
            .contains("workspace group id \"repo.cache\" must contain only ASCII alphanumeric characters, '-' or '_'")
    );
}

#[tokio::test]
async fn verifier_control_rejects_invalid_rerun_from_task_id_values() {
    let workflow_service = WorkflowService::new(Arc::new(MemoryStorage::new()));

    let mut missing_target_verifier = task("verify");
    missing_target_verifier.output_schema = None;
    missing_target_verifier.control = Some(crate::core::task::TaskControl {
        verifier: Some(crate::core::verifier::VerifierControlConfig {
            max_iterations: 2,
            on_exhausted_continue: false,
            rerun_from_task_id: Some("missing".to_string()),
        }),
    });
    let missing_target_error = workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            WorkflowDef {
                sandbox: None,
                id: "workflow1".to_string(),
                description: String::new(),
                tasks: vec![task("taska"), missing_target_verifier],
                data_bindings: vec![DataBinding {
                    source_task_id: "taska".to_string(),
                    target_task_id: "verify".to_string(),
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(
        missing_target_error
            .to_string()
            .contains("verifier rerun_from_task_id references unknown task id missing")
    );

    let mut downstream_target_verifier = task("taska");
    downstream_target_verifier.output_schema = None;
    downstream_target_verifier.control = Some(crate::core::task::TaskControl {
        verifier: Some(crate::core::verifier::VerifierControlConfig {
            max_iterations: 2,
            on_exhausted_continue: false,
            rerun_from_task_id: Some("taskb".to_string()),
        }),
    });
    let downstream_target_error = workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            WorkflowDef {
                sandbox: None,
                id: "workflow2".to_string(),
                description: String::new(),
                tasks: vec![downstream_target_verifier, task("taskb")],
                data_bindings: vec![DataBinding {
                    source_task_id: "taska".to_string(),
                    target_task_id: "taskb".to_string(),
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(
        downstream_target_error
            .to_string()
            .contains("task taska verifier rerun task taskb is not an upstream ancestor")
    );

    let mut unrelated_target_verifier = task("verify");
    unrelated_target_verifier.output_schema = None;
    unrelated_target_verifier.control = Some(crate::core::task::TaskControl {
        verifier: Some(crate::core::verifier::VerifierControlConfig {
            max_iterations: 2,
            on_exhausted_continue: false,
            rerun_from_task_id: Some("taskb".to_string()),
        }),
    });
    let unrelated_target_error = workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            WorkflowDef {
                sandbox: None,
                id: "workflow3".to_string(),
                description: String::new(),
                tasks: vec![task("taska"), task("taskb"), unrelated_target_verifier],
                data_bindings: vec![DataBinding {
                    source_task_id: "taska".to_string(),
                    target_task_id: "verify".to_string(),
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(
        unrelated_target_error
            .to_string()
            .contains("task verify verifier rerun task taskb is not an upstream ancestor")
    );
}

#[tokio::test]
async fn verifier_control_rejects_overlapping_loop_slices() {
    let workflow_service = WorkflowService::new(Arc::new(MemoryStorage::new()));
    let mut verifya = task("verifya");
    verifya.output_schema = None;
    verifya.control = Some(crate::core::task::TaskControl {
        verifier: Some(crate::core::verifier::VerifierControlConfig {
            max_iterations: 2,
            on_exhausted_continue: false,
            rerun_from_task_id: Some("taska".to_string()),
        }),
    });
    let mut verifyb = task("verifyb");
    verifyb.output_schema = None;
    verifyb.control = Some(crate::core::task::TaskControl {
        verifier: Some(crate::core::verifier::VerifierControlConfig {
            max_iterations: 2,
            on_exhausted_continue: false,
            rerun_from_task_id: Some("taskb".to_string()),
        }),
    });

    let error = workflow_service
        .create_workflow_def(
            &crate::core::namespace::test_namespace(),
            WorkflowDef {
                sandbox: None,
                id: "workflow1".to_string(),
                description: String::new(),
                tasks: vec![task("taska"), task("taskb"), verifya, verifyb],
                data_bindings: vec![
                    DataBinding {
                        source_task_id: "taska".to_string(),
                        target_task_id: "taskb".to_string(),
                    },
                    DataBinding {
                        source_task_id: "taskb".to_string(),
                        target_task_id: "verifya".to_string(),
                    },
                    DataBinding {
                        source_task_id: "taskb".to_string(),
                        target_task_id: "verifyb".to_string(),
                    },
                ],
            },
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("verifier loop slices overlap"));
}

#[tokio::test]
async fn queue_status_lists_pending_workflows() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);

    let mut running = workflow_instance("running-workflow", "workflow-1");
    running.status = WorkflowStatus::Running;
    storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            running,
        )
        .await
        .unwrap();

    orchestrator
        .enqueue_workflow_instance(
            &crate::core::namespace::test_namespace(),
            "pending-workflow".to_string(),
        )
        .await
        .unwrap();

    assert_eq!(
        orchestrator
            .get_queue_status(&crate::core::namespace::test_namespace(),)
            .await
            .unwrap(),
        vec!["pending-workflow".to_string()]
    );
}

#[tokio::test]
async fn startup_discovery_finds_blocked_workflows_without_requeueing_them() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);

    for (id, status) in [
        ("pending-workflow", WorkflowStatus::Pending),
        ("running-workflow", WorkflowStatus::Running),
        ("paused-workflow", WorkflowStatus::Paused),
        ("input-needed-workflow", WorkflowStatus::InputNeeded),
        ("completed-workflow", WorkflowStatus::Completed),
        ("failed-workflow", WorkflowStatus::Failed),
    ] {
        let mut instance = workflow_instance(id, "workflow-1");
        instance.status = status;
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance,
            )
            .await
            .unwrap();
    }

    let discovery = orchestrator
        .list_nonterminal_workflow_info(None)
        .await
        .unwrap();
    let mut runnable_ids: Vec<String> =
        discovery.runnable.into_iter().map(|info| info.id).collect();
    runnable_ids.sort();
    let mut blocked_ids: Vec<String> = discovery.blocked.into_iter().map(|info| info.id).collect();
    blocked_ids.sort();

    assert_eq!(
        runnable_ids,
        vec![
            "pending-workflow".to_string(),
            "running-workflow".to_string(),
        ]
    );
    assert_eq!(
        blocked_ids,
        vec![
            "input-needed-workflow".to_string(),
            "paused-workflow".to_string(),
        ]
    );

    let requeued = orchestrator
        .enqueue_active_workflow_instances()
        .await
        .unwrap();
    let mut pending = orchestrator
        .get_queue_status(&crate::core::namespace::test_namespace())
        .await
        .unwrap();
    pending.sort();

    assert_eq!(requeued, 2);
    assert_eq!(
        pending,
        vec![
            "pending-workflow".to_string(),
            "running-workflow".to_string(),
        ]
    );
}

#[tokio::test]
async fn pause_and_resume_workflow_update_status_and_queue() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let mut instance = workflow_instance("workflow-1", "workflow-def");
    instance.status = WorkflowStatus::Pending;
    storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            instance,
        )
        .await
        .unwrap();
    orchestrator
        .enqueue_workflow_instance(
            &crate::core::namespace::test_namespace(),
            "workflow-1".to_string(),
        )
        .await
        .unwrap();

    orchestrator
        .pause_workflow_instance(&crate::core::namespace::test_namespace(), "workflow-1")
        .await
        .unwrap();

    assert_eq!(
        storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "workflow-1")
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Paused
    );
    assert!(
        orchestrator
            .get_queue_status(&crate::core::namespace::test_namespace(),)
            .await
            .unwrap()
            .is_empty()
    );

    orchestrator
        .resume_workflow_instance(&crate::core::namespace::test_namespace(), "workflow-1")
        .await
        .unwrap();

    assert_eq!(
        storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "workflow-1")
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Pending
    );
    assert_eq!(
        orchestrator
            .get_queue_status(&crate::core::namespace::test_namespace(),)
            .await
            .unwrap(),
        vec!["workflow-1".to_string()]
    );
}

#[tokio::test]
async fn bulk_pause_and_resume_update_queue_for_matching_workflows() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);

    for (id, status) in [
        ("pending-workflow", WorkflowStatus::Pending),
        ("running-workflow", WorkflowStatus::Running),
        ("paused-workflow", WorkflowStatus::Paused),
        ("input-needed-workflow", WorkflowStatus::InputNeeded),
    ] {
        let mut instance = workflow_instance(id, "workflow-def");
        instance.status = status;
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance,
            )
            .await
            .unwrap();
    }
    orchestrator
        .enqueue_workflow_instance(
            &crate::core::namespace::test_namespace(),
            "pending-workflow".to_string(),
        )
        .await
        .unwrap();
    orchestrator
        .enqueue_workflow_instance(
            &crate::core::namespace::test_namespace(),
            "running-workflow".to_string(),
        )
        .await
        .unwrap();

    let mut paused = orchestrator
        .pause_active_workflow_instances(&crate::core::namespace::test_namespace())
        .await
        .unwrap();
    paused.sort();
    assert_eq!(
        paused,
        vec![
            "pending-workflow".to_string(),
            "running-workflow".to_string(),
        ]
    );
    assert!(
        orchestrator
            .get_queue_status(&crate::core::namespace::test_namespace(),)
            .await
            .unwrap()
            .is_empty()
    );

    let mut resumed = orchestrator
        .resume_paused_workflow_instances(&crate::core::namespace::test_namespace())
        .await
        .unwrap();
    resumed.sort();
    assert_eq!(
        resumed,
        vec![
            "paused-workflow".to_string(),
            "pending-workflow".to_string(),
            "running-workflow".to_string(),
        ]
    );
    let mut pending = orchestrator
        .get_queue_status(&crate::core::namespace::test_namespace())
        .await
        .unwrap();
    pending.sort();
    assert_eq!(
        pending,
        vec![
            "paused-workflow".to_string(),
            "pending-workflow".to_string(),
            "running-workflow".to_string(),
        ]
    );
}

#[tokio::test]
async fn bulk_pause_and_resume_are_namespace_scoped() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let first = crate::core::namespace::test_namespace();
    let second = other_namespace();

    for namespace in [&first, &second] {
        let mut active = workflow_instance("shared-active", "workflow-def");
        active.status = WorkflowStatus::Pending;
        storage
            .save_workflow_instance(namespace, 0, vec![], active)
            .await
            .unwrap();

        let mut paused = workflow_instance("shared-paused", "workflow-def");
        paused.status = WorkflowStatus::Paused;
        storage
            .save_workflow_instance(namespace, 0, vec![], paused)
            .await
            .unwrap();

        orchestrator
            .enqueue_workflow_instance(namespace, "shared-active".to_string())
            .await
            .unwrap();
    }

    assert_eq!(
        orchestrator
            .pause_active_workflow_instances(&first)
            .await
            .unwrap(),
        vec!["shared-active".to_string()]
    );
    assert!(
        orchestrator
            .get_queue_status(&first)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        orchestrator.get_queue_status(&second).await.unwrap(),
        vec!["shared-active".to_string()]
    );
    assert_eq!(
        storage
            .get_workflow_instance(&second, "shared-active")
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Pending
    );

    let mut resumed = orchestrator
        .resume_paused_workflow_instances(&first)
        .await
        .unwrap();
    resumed.sort();
    assert_eq!(
        resumed,
        vec!["shared-active".to_string(), "shared-paused".to_string()]
    );
    assert_eq!(
        orchestrator.get_queue_status(&second).await.unwrap(),
        vec!["shared-active".to_string()]
    );
    assert_eq!(
        storage
            .get_workflow_instance(&second, "shared-paused")
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Paused
    );
}

#[test]
fn startup_recovery_is_cross_namespace_with_or_without_global_namespace_mode() {
    for use_global_namespace in [None, Some("true")] {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg("startup_recovery_cross_namespace_child")
            .arg("--nocapture")
            .env("RELAYFOLD_STARTUP_RECOVERY_CHILD", "1");

        match use_global_namespace {
            Some(enabled) => {
                command.env("RELAYFOLD_USE_GLOBAL_NAMESPACE", enabled);
            }
            None => {
                command.env_remove("RELAYFOLD_USE_GLOBAL_NAMESPACE");
            }
        }

        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "child recovery test failed with global mode {use_global_namespace:?}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[tokio::test]
async fn startup_recovery_cross_namespace_child() {
    if std::env::var_os("RELAYFOLD_STARTUP_RECOVERY_CHILD").is_none() {
        return;
    }

    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let first = crate::core::namespace::test_namespace();
    let second = other_namespace();
    let task_attempt_id = TaskInstance::make_task_attempt_id("taska", 1);

    for namespace in [&first, &second] {
        let mut instance = workflow_instance("shared-workflow", "workflow-def");
        instance.status = WorkflowStatus::Running;
        instance.tasks.insert(
            task_attempt_id.clone(),
            TaskInstance {
                task_def_id: "taska".to_string(),
                status: TaskStatus::Running,
                satisfaction_status: TaskSatisfactionStatus::Pending,
                human_input: None,
                input_data: vec![],
                input_mapping: vec![],
                output_data: None,
                generation_index: 1,
                verifier_metadata: None,
            },
        );
        storage
            .save_workflow_instance(namespace, 0, vec![], instance)
            .await
            .unwrap();
    }

    assert_eq!(orchestrator.synchronize_startup_tasks().await.unwrap(), 2);
    assert_eq!(
        orchestrator
            .enqueue_active_workflow_instances()
            .await
            .unwrap(),
        2
    );

    for namespace in [&first, &second] {
        let recovered = storage
            .get_workflow_instance(namespace, "shared-workflow")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.status, WorkflowStatus::Pending);
        assert_eq!(
            recovered.tasks[&task_attempt_id].status,
            TaskStatus::Pending
        );
        assert_eq!(
            orchestrator.get_queue_status(namespace).await.unwrap(),
            vec!["shared-workflow".to_string()]
        );
    }
}

#[tokio::test]
async fn startup_recovery_preserves_workflow_pins_when_reloading_runnable_work() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let pinned_host = WorkerHostId::new("host-a");

    for (id, status) in [
        ("pending-workflow", WorkflowStatus::Pending),
        ("running-workflow", WorkflowStatus::Running),
        ("paused-workflow", WorkflowStatus::Paused),
        ("input-needed-workflow", WorkflowStatus::InputNeeded),
    ] {
        let mut instance = workflow_instance(id, "workflow-1");
        instance.status = status;
        instance.pinned_worker_host = Some(pinned_host.clone());
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance,
            )
            .await
            .unwrap();
    }

    assert_eq!(orchestrator.synchronize_startup_tasks().await.unwrap(), 1);
    assert_eq!(
        orchestrator
            .enqueue_active_workflow_instances()
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        storage
            .get_workflow_instance(
                &crate::core::namespace::test_namespace(),
                "running-workflow"
            )
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Pending
    );

    for id in [
        "pending-workflow",
        "running-workflow",
        "paused-workflow",
        "input-needed-workflow",
    ] {
        let instance = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(instance.pinned_worker_host, Some(pinned_host.clone()));
    }

    assert_eq!(
        storage
            .get_workflow_instance(
                &crate::core::namespace::test_namespace(),
                "running-workflow"
            )
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Pending
    );

    let mut pending = orchestrator
        .get_queue_status(&crate::core::namespace::test_namespace())
        .await
        .unwrap();
    pending.sort();
    assert_eq!(
        pending,
        vec![
            "pending-workflow".to_string(),
            "running-workflow".to_string(),
        ]
    );
}

#[tokio::test]
async fn startup_recovery_requeues_abandoned_running_task_attempts() {
    // Seed durable storage as it would look after a crash: the workflow and one
    // task attempt were Running, but the in-memory dispatch lease is gone.
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let pinned_host = WorkerHostId::new("host-a");
    let task_attempt_id = TaskInstance::make_task_attempt_id("taska", 1);

    let mut instance = workflow_instance("running-workflow", "workflow-1");
    instance.status = WorkflowStatus::Running;
    instance.pinned_worker_host = Some(pinned_host.clone());
    instance.tasks.insert(
        task_attempt_id.clone(),
        TaskInstance {
            task_def_id: "taska".to_string(),
            status: TaskStatus::Running,
            satisfaction_status: TaskSatisfactionStatus::Pending,
            human_input: None,
            input_data: vec![],
            input_mapping: vec![],
            output_data: None,
            generation_index: 1,
            verifier_metadata: None,
        },
    );
    storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            instance,
        )
        .await
        .unwrap();

    // Startup recovery should treat the lost in-memory lease as abandoned,
    // move runnable work back to Pending, and enqueue the workflow again.
    assert_eq!(orchestrator.synchronize_startup_tasks().await.unwrap(), 1);
    assert_eq!(
        orchestrator
            .enqueue_active_workflow_instances()
            .await
            .unwrap(),
        1
    );

    // The workflow pin remains durable, while the running workflow and task
    // attempt are made eligible for redispatch.
    let recovered = storage
        .get_workflow_instance(
            &crate::core::namespace::test_namespace(),
            "running-workflow",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.status, WorkflowStatus::Pending);
    assert_eq!(recovered.pinned_worker_host, Some(pinned_host));
    assert_eq!(
        recovered.tasks.get(&task_attempt_id).unwrap().status,
        TaskStatus::Pending
    );
    assert_eq!(
        orchestrator
            .get_queue_status(&crate::core::namespace::test_namespace(),)
            .await
            .unwrap(),
        vec!["running-workflow".to_string()]
    );
}

#[tokio::test]
async fn lost_pinned_host_marks_nonterminal_workflows_failed() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let lost_host = WorkerHostId::new("host-a");
    let other_host = WorkerHostId::new("host-b");

    for (id, status, pinned_host) in [
        (
            "pending-workflow",
            WorkflowStatus::Pending,
            Some(lost_host.clone()),
        ),
        (
            "running-workflow",
            WorkflowStatus::Running,
            Some(lost_host.clone()),
        ),
        (
            "paused-workflow",
            WorkflowStatus::Paused,
            Some(lost_host.clone()),
        ),
        (
            "input-needed-workflow",
            WorkflowStatus::InputNeeded,
            Some(lost_host.clone()),
        ),
        (
            "completed-workflow",
            WorkflowStatus::Completed,
            Some(lost_host.clone()),
        ),
        (
            "already-failed-workflow",
            WorkflowStatus::Failed,
            Some(lost_host.clone()),
        ),
        (
            "other-host-workflow",
            WorkflowStatus::Pending,
            Some(other_host.clone()),
        ),
        ("unpinned-workflow", WorkflowStatus::Pending, None),
    ] {
        let mut instance = workflow_instance(id, "workflow-1");
        instance.status = status;
        instance.pinned_worker_host = pinned_host;
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance,
            )
            .await
            .unwrap();
    }

    let failed = orchestrator
        .fail_workflows_pinned_to_lost_hosts(std::slice::from_ref(&lost_host))
        .await
        .unwrap();

    assert_eq!(failed, 4);
    for id in [
        "pending-workflow",
        "running-workflow",
        "paused-workflow",
        "input-needed-workflow",
    ] {
        let instance = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(instance.status, WorkflowStatus::Failed);
        assert_eq!(instance.pinned_worker_host, Some(lost_host.clone()));
    }

    assert_eq!(
        storage
            .get_workflow_instance(
                &crate::core::namespace::test_namespace(),
                "completed-workflow"
            )
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Completed
    );
    assert_eq!(
        storage
            .get_workflow_instance(
                &crate::core::namespace::test_namespace(),
                "already-failed-workflow"
            )
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Failed
    );
    assert_eq!(
        storage
            .get_workflow_instance(
                &crate::core::namespace::test_namespace(),
                "other-host-workflow"
            )
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Pending
    );
    assert_eq!(
        storage
            .get_workflow_instance(
                &crate::core::namespace::test_namespace(),
                "unpinned-workflow"
            )
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Pending
    );
}

#[tokio::test]
async fn lost_host_reconciliation_discovers_and_updates_all_namespaces() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let first = crate::core::namespace::test_namespace();
    let second = other_namespace();
    let lost_host = WorkerHostId::new("host-a");

    for namespace in [&first, &second] {
        let mut instance = workflow_instance("shared-workflow", "workflow-def");
        instance.status = WorkflowStatus::Pending;
        instance.pinned_worker_host = Some(lost_host.clone());
        storage
            .save_workflow_instance(namespace, 0, vec![], instance)
            .await
            .unwrap();
        orchestrator
            .enqueue_workflow_instance(namespace, "shared-workflow".to_string())
            .await
            .unwrap();
    }

    assert_eq!(
        orchestrator
            .fail_workflows_pinned_to_lost_hosts(std::slice::from_ref(&lost_host))
            .await
            .unwrap(),
        2
    );

    assert_eq!(
        storage
            .get_workflow_instance(&first, "shared-workflow")
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Failed
    );
    assert_eq!(
        storage
            .get_workflow_instance(&second, "shared-workflow")
            .await
            .unwrap()
            .unwrap()
            .status,
        WorkflowStatus::Failed
    );
    assert!(
        orchestrator
            .get_queue_status(&first)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        orchestrator
            .get_queue_status(&second)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn retry_workflow_task_commits_retry_and_enqueues_workflow() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let mut instance = workflow_instance("failed-workflow", "workflow-1");
    instance.status = WorkflowStatus::Failed;
    instance.pinned_worker_host = Some(WorkerHostId::new("host-a"));
    instance.tasks.insert(
        "taska[1]".to_string(),
        TaskInstance {
            task_def_id: "taska".to_string(),
            status: TaskStatus::Failed,
            satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
            human_input: None,
            input_data: vec![],
            input_mapping: vec![],
            output_data: Some(json!({"stale": true})),
            generation_index: 1,
            verifier_metadata: None,
        },
    );
    storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            instance,
        )
        .await
        .unwrap();

    let result = orchestrator
        .retry_workflow_task(
            &crate::core::namespace::test_namespace(),
            "failed-workflow",
            "taska",
        )
        .await
        .unwrap();

    assert_eq!(result.task_attempt_id, "taska[1]");
    assert_eq!(result.pinned_host_id, Some(WorkerHostId::new("host-a")));
    assert!(!result.local_context_may_be_lost);

    let saved = storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), "failed-workflow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, WorkflowStatus::Pending);
    assert_eq!(saved.tasks["taska[1]"].status, TaskStatus::Pending);
    assert_eq!(
        orchestrator
            .get_queue_status(&crate::core::namespace::test_namespace(),)
            .await
            .unwrap(),
        vec!["failed-workflow".to_string()]
    );
}

#[tokio::test]
async fn force_retry_workflow_task_keeps_existing_host_when_it_is_available() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let worker_registry = WorkerRegistry::new();
    worker_registry
        .register_worker(WorkerIdentity {
            worker_id: WorkerId::new("worker-1"),
            host_id: WorkerHostId::new("host-a"),
        })
        .await;
    worker_registry
        .register_worker(WorkerIdentity {
            worker_id: WorkerId::new("worker-2"),
            host_id: WorkerHostId::new("host-b"),
        })
        .await;
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let mut instance = workflow_instance("failed-workflow", "workflow-1");
    instance.status = WorkflowStatus::Failed;
    instance.pinned_worker_host = Some(WorkerHostId::new("host-a"));
    instance.tasks.insert(
        "taska[1]".to_string(),
        TaskInstance {
            task_def_id: "taska".to_string(),
            status: TaskStatus::Failed,
            satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
            human_input: None,
            input_data: vec![],
            input_mapping: vec![],
            output_data: None,
            generation_index: 1,
            verifier_metadata: None,
        },
    );
    storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            instance,
        )
        .await
        .unwrap();

    let result = orchestrator
        .force_retry_workflow_task(
            &crate::core::namespace::test_namespace(),
            "failed-workflow",
            "taska",
            &worker_registry,
        )
        .await
        .unwrap();

    assert_eq!(result.pinned_host_id, Some(WorkerHostId::new("host-a")));
    assert!(!result.local_context_may_be_lost);
    let saved = storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), "failed-workflow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.pinned_worker_host, Some(WorkerHostId::new("host-a")));
}

#[tokio::test]
async fn force_retry_workflow_task_reassigns_when_existing_host_is_unavailable() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let worker_registry = WorkerRegistry::new();
    worker_registry
        .register_worker(WorkerIdentity {
            worker_id: WorkerId::new("worker-1"),
            host_id: WorkerHostId::new("host-b"),
        })
        .await;
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let mut instance = workflow_instance("failed-workflow", "workflow-1");
    instance.status = WorkflowStatus::Failed;
    instance.pinned_worker_host = Some(WorkerHostId::new("host-a"));
    instance.tasks.insert(
        "taska[1]".to_string(),
        TaskInstance {
            task_def_id: "taska".to_string(),
            status: TaskStatus::Failed,
            satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
            human_input: None,
            input_data: vec![],
            input_mapping: vec![],
            output_data: None,
            generation_index: 1,
            verifier_metadata: None,
        },
    );
    storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            instance,
        )
        .await
        .unwrap();

    let result = orchestrator
        .force_retry_workflow_task(
            &crate::core::namespace::test_namespace(),
            "failed-workflow",
            "taska",
            &worker_registry,
        )
        .await
        .unwrap();

    assert_eq!(result.pinned_host_id, Some(WorkerHostId::new("host-b")));
    assert!(result.local_context_may_be_lost);
    let saved = storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), "failed-workflow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.pinned_worker_host, Some(WorkerHostId::new("host-b")));
    assert_eq!(
        orchestrator
            .get_queue_status(&crate::core::namespace::test_namespace(),)
            .await
            .unwrap(),
        vec!["failed-workflow".to_string()]
    );
}

#[tokio::test]
async fn force_retry_workflow_task_rejects_when_no_host_is_eligible() {
    let storage = Arc::new(MemoryStorage::new());
    let queue = Arc::new(MemoryWorkflowQueue::new(10));
    let worker_registry = WorkerRegistry::new();
    let orchestrator =
        Orchestrator::new(storage.clone(), Arc::new(FakeTaskDispatcher::new()), queue);
    let mut instance = workflow_instance("failed-workflow", "workflow-1");
    instance.status = WorkflowStatus::Failed;
    instance.pinned_worker_host = Some(WorkerHostId::new("host-a"));
    instance.tasks.insert(
        "taska[1]".to_string(),
        TaskInstance {
            task_def_id: "taska".to_string(),
            status: TaskStatus::Failed,
            satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
            human_input: None,
            input_data: vec![],
            input_mapping: vec![],
            output_data: None,
            generation_index: 1,
            verifier_metadata: None,
        },
    );
    storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            instance,
        )
        .await
        .unwrap();

    let error = orchestrator
        .force_retry_workflow_task(
            &crate::core::namespace::test_namespace(),
            "failed-workflow",
            "taska",
            &worker_registry,
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("no eligible retry host"));
    assert!(
        orchestrator
            .get_queue_status(&crate::core::namespace::test_namespace(),)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn remove_and_purge_affect_pending_queue_only() {
    let orchestrator = orchestrator();

    orchestrator
        .enqueue_workflow_instance(
            &crate::core::namespace::test_namespace(),
            "workflow-1".to_string(),
        )
        .await
        .unwrap();
    orchestrator
        .enqueue_workflow_instance(
            &crate::core::namespace::test_namespace(),
            "workflow-2".to_string(),
        )
        .await
        .unwrap();

    assert!(
        orchestrator
            .remove_queued_workflow_instance(
                &crate::core::namespace::test_namespace(),
                "workflow-1"
            )
            .await
            .unwrap()
    );
    assert_eq!(
        orchestrator
            .purge_queued_workflow_instances(&crate::core::namespace::test_namespace(),)
            .await
            .unwrap(),
        vec!["workflow-2".to_string()]
    );
    assert!(
        orchestrator
            .get_queue_status(&crate::core::namespace::test_namespace(),)
            .await
            .unwrap()
            .is_empty()
    );
}
