use super::*;
use crate::adapters::fake_task_dispatcher::FakeTaskDispatcher;
use crate::adapters::memory_storage::MemoryStorage;
use crate::core::function::models::FunctionTaskDef;
use crate::core::task::*;
use crate::core::verifier::verifier_decision_schema;
use crate::core::worker::{TaskDispatchConstraints, WorkerHostId};
use crate::core::workflow::models::DataBinding;
use crate::core::workflow::state_manager::WorkflowStateManager;
use crate::ports::storage::WorkflowEventPageRequest;
use crate::ports::task_dispatch::ExecutionResult;
use crate::ports::task_dispatch::TaskDispatchPort;
use async_trait::async_trait;
use serde_json::Number;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

fn make_engine() -> WorkflowEngine {
    WorkflowEngine::new(
        Arc::new(MemoryStorage::new()),
        Arc::new(FakeTaskDispatcher::new()),
    )
}

fn make_engine_with_dispatcher(
    dispatcher: Arc<dyn TaskDispatchPort + Send + Sync>,
) -> WorkflowEngine {
    WorkflowEngine::new(Arc::new(MemoryStorage::new()), dispatcher)
}

struct ContinueThenCompleteDispatcher;

#[async_trait]
impl TaskDispatchPort for ContinueThenCompleteDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &crate::core::namespace::Namespace,
        _workflow_inst_id: &str,
        task: &TaskDef,
        _inputs: &[serde_json::Value],
        metadata: &ExecutionMetadata,
        _dispatch: &crate::core::worker::TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        if task_verifier(task).is_some() {
            let generation = metadata
                .loop_context
                .as_ref()
                .map(|context| context.generation)
                .unwrap_or(1);
            if generation == 1 {
                return Ok(ExecutionResult::Success(json!({
                    "decision": "continue",
                    "feedback": "try again"
                })));
            }
            return Ok(ExecutionResult::Success(json!({ "decision": "complete" })));
        }

        Ok(ExecutionResult::Success(json!({})))
    }
}

struct CompleteVerifierDispatcher;

#[async_trait]
impl TaskDispatchPort for CompleteVerifierDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &crate::core::namespace::Namespace,
        _workflow_inst_id: &str,
        task: &TaskDef,
        _inputs: &[serde_json::Value],
        _metadata: &ExecutionMetadata,
        _dispatch: &crate::core::worker::TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        if task_verifier(task).is_some() {
            return Ok(ExecutionResult::Success(json!({ "decision": "complete" })));
        }

        Ok(ExecutionResult::Success(json!({})))
    }
}

struct AlwaysContinueVerifierDispatcher;

#[async_trait]
impl TaskDispatchPort for AlwaysContinueVerifierDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &crate::core::namespace::Namespace,
        _workflow_inst_id: &str,
        task: &TaskDef,
        _inputs: &[serde_json::Value],
        _metadata: &ExecutionMetadata,
        _dispatch: &crate::core::worker::TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        if task_verifier(task).is_some() {
            return Ok(ExecutionResult::Success(json!({
                "decision": "continue",
                "feedback": "try again"
            })));
        }

        Ok(ExecutionResult::Success(json!({})))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedExecution {
    workflow_inst_id: String,
    task_id: String,
    generation_index: u32,
    pinned_host_id: Option<WorkerHostId>,
    reuse_session: Option<bool>,
    feedback_count: usize,
    human_input_provided: Option<String>,
}

struct RecordingContinueDispatcher {
    records: StdMutex<Vec<RecordedExecution>>,
}

impl RecordingContinueDispatcher {
    fn new() -> Self {
        Self {
            records: StdMutex::new(vec![]),
        }
    }

    fn records(&self) -> Vec<RecordedExecution> {
        self.records.lock().unwrap().clone()
    }
}

#[async_trait]
impl TaskDispatchPort for RecordingContinueDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &crate::core::namespace::Namespace,
        workflow_inst_id: &str,
        task: &TaskDef,
        _inputs: &[serde_json::Value],
        metadata: &ExecutionMetadata,
        dispatch: &TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        let reuse_session = match &task.kind {
            TaskTypeDef::Agent { reuse_session, .. } => Some(*reuse_session),
            _ => None,
        };
        self.records.lock().unwrap().push(RecordedExecution {
            workflow_inst_id: workflow_inst_id.to_string(),
            task_id: task.id.clone(),
            generation_index: metadata.generation_index,
            pinned_host_id: dispatch.pinned_host_id.clone(),
            reuse_session,
            feedback_count: metadata
                .loop_context
                .as_ref()
                .map(|context| context.feedback_history.len())
                .unwrap_or(0),
            human_input_provided: metadata.human_input_provided.clone(),
        });

        if task_verifier(task).is_some() {
            if metadata.generation_index == 1 {
                return Ok(ExecutionResult::Success(json!({
                    "decision": "continue",
                    "feedback": "revise task-a"
                })));
            }
            return Ok(ExecutionResult::Success(json!({ "decision": "complete" })));
        }

        Ok(ExecutionResult::Success(json!({ "ok": true })))
    }
}

struct InputNeededDispatcher;

#[async_trait]
impl TaskDispatchPort for InputNeededDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &crate::core::namespace::Namespace,
        _workflow_inst_id: &str,
        _task: &TaskDef,
        _inputs: &[serde_json::Value],
        _metadata: &ExecutionMetadata,
        _dispatch: &TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        Ok(ExecutionResult::InputNeeded(
            "Need human input before continuing.".to_string(),
        ))
    }
}

struct ApiResponseDispatcher {
    response: serde_json::Value,
    calls: StdMutex<Vec<(String, Vec<serde_json::Value>)>>,
}

impl ApiResponseDispatcher {
    fn new(response: serde_json::Value) -> Self {
        Self {
            response,
            calls: StdMutex::new(vec![]),
        }
    }

    fn calls(&self) -> Vec<(String, Vec<serde_json::Value>)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl TaskDispatchPort for ApiResponseDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &crate::core::namespace::Namespace,
        _workflow_inst_id: &str,
        task: &TaskDef,
        inputs: &[serde_json::Value],
        _metadata: &ExecutionMetadata,
        _dispatch: &TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        self.calls
            .lock()
            .unwrap()
            .push((task.id.clone(), inputs.to_vec()));

        if task.id == "fetch-data" {
            return Ok(ExecutionResult::Success(self.response.clone()));
        }

        Ok(ExecutionResult::Success(json!({ "done": true })))
    }
}

struct InputNeededForTaskDispatcher {
    input_needed_task_id: String,
    calls: StdMutex<Vec<String>>,
}

impl InputNeededForTaskDispatcher {
    fn new(input_needed_task_id: &str) -> Self {
        Self {
            input_needed_task_id: input_needed_task_id.to_string(),
            calls: StdMutex::new(vec![]),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl TaskDispatchPort for InputNeededForTaskDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &crate::core::namespace::Namespace,
        _workflow_inst_id: &str,
        task: &TaskDef,
        _inputs: &[serde_json::Value],
        _metadata: &ExecutionMetadata,
        _dispatch: &TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        self.calls.lock().unwrap().push(task.id.clone());
        if task.id == self.input_needed_task_id {
            return Ok(ExecutionResult::InputNeeded(
                "Need human input before continuing.".to_string(),
            ));
        }

        Ok(ExecutionResult::Success(json!({ "ok": true })))
    }
}

struct PauseDuringTaskDispatcher {
    storage: Arc<MemoryStorage>,
    workflow_instance_id: String,
    calls: StdMutex<Vec<String>>,
}

impl PauseDuringTaskDispatcher {
    fn new(storage: Arc<MemoryStorage>, workflow_instance_id: &str) -> Self {
        Self {
            storage,
            workflow_instance_id: workflow_instance_id.to_string(),
            calls: StdMutex::new(vec![]),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl TaskDispatchPort for PauseDuringTaskDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &crate::core::namespace::Namespace,
        _workflow_inst_id: &str,
        task: &TaskDef,
        _inputs: &[serde_json::Value],
        _metadata: &ExecutionMetadata,
        _dispatch: &TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        self.calls.lock().unwrap().push(task.id.clone());
        WorkflowStateManager::new(self.storage.clone())
            .commit_events(
                &crate::core::namespace::test_namespace(),
                &self.workflow_instance_id,
                vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                    status: WorkflowStatus::Paused,
                }],
            )
            .await?;

        Ok(ExecutionResult::Success(json!({ "ok": true })))
    }
}

fn task_def(id: &str, output_schema: serde_json::Value) -> TaskDef {
    TaskDef {
        id: id.to_string(),
        kind: TaskTypeDef::ApiCall {
            url: "http://example.com".to_string(),
            method: "GET".to_string(),
            headers: HashMap::new(),
        },
        control: None,
        timeout_secs: None,
        input_schemas: vec![],
        output_schema: Some(output_schema),
        workspace: None,
        required_credentials: vec![],
    }
}

fn agent_task(id: &str, reuse_session: bool) -> TaskDef {
    TaskDef {
        id: id.to_string(),
        kind: TaskTypeDef::Agent {
            model_id: "test/model".to_string(),
            provider_url: "".to_string(),
            prompt: "draft".to_string(),
            tools: vec![],
            skills: vec![],
            ask: false,
            schema_failure_retry_times: Number::from(0),
            reuse_session,
        },
        control: None,
        timeout_secs: None,
        input_schemas: vec![],
        output_schema: Some(json!({ "type": "object" })),
        workspace: None,
        required_credentials: vec![],
    }
}

fn task_def_with_workspace_group(id: &str, group_name: &str) -> TaskDef {
    let mut task = task_def(id, json!({ "type": "object" }));
    task.workspace = Some(Workspace {
        group_name: group_name.to_string(),
    });
    task
}

fn pending_task_instance(task_def_id: &str) -> TaskInstance {
    TaskInstance {
        early_exit: false,
        task_def_id: task_def_id.to_string(),
        status: TaskStatus::Pending,
        satisfaction_status: TaskSatisfactionStatus::Pending,
        human_input: None,
        input_data: vec![],
        input_mapping: vec![],
        output_data: None,
        generation_index: 1,
        verifier_metadata: None,
    }
}

async fn setup(engine: &WorkflowEngine, def: WorkflowDef) -> String {
    setup_with_pin(engine, def, None).await
}

async fn setup_with_pin(
    engine: &WorkflowEngine,
    def: WorkflowDef,
    pinned_worker_host: Option<WorkerHostId>,
) -> String {
    let instance_id = "inst-1".to_string();
    let instance = WorkflowInstance {
        id: instance_id.clone(),
        workflow_def_id: def.id.clone(),
        version: 0,
        status: WorkflowStatus::Pending,
        trigger_input: None,
        pinned_worker_host,
        tasks: HashMap::new(),

        verifier_states: HashMap::new(),
    };
    engine
        .storage
        .save_workflow_def(&crate::core::namespace::test_namespace(), def)
        .await
        .unwrap();
    engine
        .storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            instance,
        )
        .await
        .unwrap();
    instance_id
}

fn agent_verifier_task(id: &str, rerun_from_task_id: Option<&str>) -> TaskDef {
    let mut task = task_def(id, json!({ "type": "object" }));
    task.kind = TaskTypeDef::Agent {
        model_id: "test/model".to_string(),
        provider_url: "".to_string(),
        prompt: "verify".to_string(),
        tools: vec![],
        skills: vec![],
        ask: false,
        schema_failure_retry_times: Number::from(0),
        reuse_session: false,
    };
    task.output_schema = None;
    task.control = Some(TaskControl {
        verifier: Some(VerifierControlConfig {
            max_iterations: 2,
            on_exhausted_continue: false,
            rerun_from_task_id: rerun_from_task_id.map(str::to_string),
        }),
    });
    task.output_schema = Some(verifier_decision_schema());
    task
}

fn agent_verifier_task_with_policy(
    id: &str,
    rerun_from_task_id: Option<&str>,
    max_iterations: u32,
    on_exhausted_continue: bool,
) -> TaskDef {
    let mut task = agent_verifier_task(id, rerun_from_task_id);
    task.control = Some(TaskControl {
        verifier: Some(VerifierControlConfig {
            max_iterations,
            on_exhausted_continue,
            rerun_from_task_id: rerun_from_task_id.map(str::to_string),
        }),
    });
    task
}

fn function_verifier_task(id: &str, rerun_from_task_id: Option<&str>) -> TaskDef {
    let mut task = task_def(id, json!({ "type": "object" }));
    task.kind = TaskTypeDef::Function(FunctionTaskDef::Inline {
        dependencies: vec![],
        code: "export default async function run() { return { decision: 'complete' }; }"
            .to_string(),
    });
    task.output_schema = Some(verifier_decision_schema());
    task.control = Some(TaskControl {
        verifier: Some(VerifierControlConfig {
            max_iterations: 2,
            on_exhausted_continue: false,
            rerun_from_task_id: rerun_from_task_id.map(str::to_string),
        }),
    });
    task
}

#[test]
fn test_workspace_group_does_not_create_scheduling_dependency() {
    let engine = make_engine();
    let task_a = task_def_with_workspace_group("task-a", "repo");
    let task_b = task_def_with_workspace_group("task-b", "repo");
    let def = WorkflowDef {
        example_input: None,
        id: "def-workspace-group-no-edge".to_string(),
        description: String::new(),
        tasks: vec![task_a.clone(), task_b.clone()],
        data_bindings: vec![],
    };
    let loop_slices = engine.compute_loop_slices(&def);
    let instance = WorkflowInstance {
        id: "inst-workspace-group-no-edge".to_string(),
        workflow_def_id: def.id.clone(),
        version: 0,
        status: WorkflowStatus::Running,
        trigger_input: None,
        pinned_worker_host: None,
        tasks: HashMap::from([
            ("task-a[1]".to_string(), pending_task_instance("task-a")),
            ("task-b[1]".to_string(), pending_task_instance("task-b")),
        ]),

        verifier_states: HashMap::new(),
    };

    let task_a_inputs = engine
        .resolve_inputs(
            &instance,
            &def,
            &instance.tasks["task-a[1]"],
            &task_a,
            &loop_slices,
        )
        .unwrap();
    let task_b_inputs = engine
        .resolve_inputs(
            &instance,
            &def,
            &instance.tasks["task-b[1]"],
            &task_b,
            &loop_slices,
        )
        .unwrap();

    assert!(task_a_inputs.values.is_empty());
    assert!(task_a_inputs.mapping.is_empty());
    assert!(task_b_inputs.values.is_empty());
    assert!(task_b_inputs.mapping.is_empty());
}

#[test]
fn test_root_task_uses_trigger_input_during_initial_materialization() {
    let mut task = task_def("task-a", json!({ "type": "object" }));
    task.input_schemas = vec![json!({ "type": "object" })];
    let def = WorkflowDef {
        example_input: None,
        id: "def-root-input".to_string(),
        description: String::new(),
        tasks: vec![task.clone()],
        data_bindings: vec![],
    };
    let instance = WorkflowInstance {
        id: "inst-root-input".to_string(),
        workflow_def_id: def.id.clone(),
        version: 0,
        status: WorkflowStatus::Running,
        trigger_input: Some(json!({ "repository": "parsablelabs/relayfold" })),
        pinned_worker_host: None,
        tasks: HashMap::new(),

        verifier_states: HashMap::new(),
    };

    assert_eq!(
        initial_task_input_data(&instance, &def, &task),
        vec![json!({ "repository": "parsablelabs/relayfold" })]
    );
}

#[test]
fn test_workspace_group_tasks_still_wait_for_data_binding() {
    let engine = make_engine();
    let task_a = task_def_with_workspace_group("task-a", "repo");
    let mut task_b = task_def_with_workspace_group("task-b", "repo");
    task_b.input_schemas = vec![json!({ "type": "object" })];
    let def = WorkflowDef {
        example_input: None,
        id: "def-workspace-group-data-binding".to_string(),
        description: String::new(),
        tasks: vec![task_a.clone(), task_b.clone()],
        data_bindings: vec![DataBinding {
            source_task_id: "task-a".to_string(),
            target_task_id: "task-b".to_string(),
        }],
    };
    let loop_slices = engine.compute_loop_slices(&def);
    let mut instance = WorkflowInstance {
        id: "inst-workspace-group-data-binding".to_string(),
        workflow_def_id: def.id.clone(),
        version: 0,
        status: WorkflowStatus::Running,
        trigger_input: None,
        pinned_worker_host: None,
        tasks: HashMap::from([
            ("task-a[1]".to_string(), pending_task_instance("task-a")),
            ("task-b[1]".to_string(), pending_task_instance("task-b")),
        ]),

        verifier_states: HashMap::new(),
    };

    assert!(
        engine
            .resolve_inputs(
                &instance,
                &def,
                &instance.tasks["task-b[1]"],
                &task_b,
                &loop_slices,
            )
            .is_none()
    );

    let task_a_instance = instance.tasks.get_mut("task-a[1]").unwrap();
    task_a_instance.status = TaskStatus::Completed;
    task_a_instance.satisfaction_status = TaskSatisfactionStatus::Satisfied;
    task_a_instance.output_data = Some(json!({ "value": "from-json-output" }));

    let task_b_inputs = engine
        .resolve_inputs(
            &instance,
            &def,
            &instance.tasks["task-b[1]"],
            &task_b,
            &loop_slices,
        )
        .unwrap();

    assert_eq!(
        task_b_inputs.values,
        vec![json!({ "value": "from-json-output" })]
    );
    assert_eq!(
        task_b_inputs.mapping,
        vec![TaskInputMapping {
            task_id: "task-a".to_string(),
            generation: 1,
        }]
    );
}

#[tokio::test]
async fn test_input_needed_workflow_retains_pinned_host() {
    let engine = make_engine_with_dispatcher(Arc::new(InputNeededDispatcher));
    let def = WorkflowDef {
        example_input: None,
        id: "def-input-needed-pin-retention".to_string(),
        description: String::new(),
        tasks: vec![agent_task("ask-user", true)],
        data_bindings: vec![],
    };
    let pinned_host = WorkerHostId::new("host-a");
    let instance_id = setup_with_pin(&engine, def, Some(pinned_host.clone())).await;

    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(instance.status, WorkflowStatus::InputNeeded);
    assert_eq!(instance.pinned_worker_host, Some(pinned_host));
    assert!(matches!(
        instance.tasks["ask-user[1]"].status,
        TaskStatus::InputNeeded { .. }
    ));
}

#[tokio::test]
async fn test_input_needed_stops_current_engine_pass() {
    let dispatcher = Arc::new(InputNeededForTaskDispatcher::new("ask-user"));
    let engine = make_engine_with_dispatcher(dispatcher.clone());
    let def = WorkflowDef {
        example_input: None,
        id: "def-input-needed-stops-pass".to_string(),
        description: String::new(),
        tasks: vec![
            agent_task("ask-user", true),
            task_def("independent-work", json!({ "type": "object" })),
        ],
        data_bindings: vec![],
    };
    let instance_id = setup(&engine, def).await;

    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(instance.status, WorkflowStatus::InputNeeded);
    assert!(matches!(
        instance.tasks["ask-user[1]"].status,
        TaskStatus::InputNeeded { .. }
    ));
    assert_eq!(
        instance.tasks["independent-work[1]"].status,
        TaskStatus::Pending
    );
    assert_eq!(dispatcher.calls(), vec!["ask-user".to_string()]);
}

#[test]
fn test_verifier_without_rerun_from_task_id_self_reruns_only() {
    let engine = make_engine();
    let def = WorkflowDef {
        example_input: None,
        id: "def-self-rerun".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            agent_verifier_task("verify", None),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "task-a".to_string(),
            target_task_id: "verify".to_string(),
        }],
    };

    let slices = engine.compute_loop_slices(&def);
    assert_eq!(slices["verify"], vec!["verify".to_string()]);
}

#[test]
fn test_verifier_with_rerun_from_task_id_reruns_upstream_slice() {
    let engine = make_engine();
    let def = WorkflowDef {
        example_input: None,
        id: "def-upstream-rerun".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            task_def("task-b", json!({ "type": "object" })),
            agent_verifier_task("verify", Some("task-a")),
        ],
        data_bindings: vec![
            DataBinding {
                source_task_id: "task-a".to_string(),
                target_task_id: "task-b".to_string(),
            },
            DataBinding {
                source_task_id: "task-b".to_string(),
                target_task_id: "verify".to_string(),
            },
        ],
    };

    let slices = engine.compute_loop_slices(&def);
    assert_eq!(
        slices["verify"],
        vec![
            "task-a".to_string(),
            "task-b".to_string(),
            "verify".to_string()
        ]
    );
}

#[test]
fn test_loop_execution_metadata_includes_feedback_history() {
    let engine = make_engine();
    let def = WorkflowDef {
        example_input: None,
        id: "def-loop-metadata".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            agent_verifier_task("verify", Some("task-a")),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "task-a".to_string(),
            target_task_id: "verify".to_string(),
        }],
    };
    let task_instance = TaskInstance {
        early_exit: false,
        task_def_id: "task-a".to_string(),
        status: TaskStatus::Pending,
        satisfaction_status: TaskSatisfactionStatus::Pending,
        human_input: None,
        input_data: vec![],
        input_mapping: vec![],
        output_data: None,
        generation_index: 2,
        verifier_metadata: None,
    };
    let mut instance = WorkflowInstance {
        id: "inst-loop-metadata".to_string(),
        workflow_def_id: def.id.clone(),
        version: 0,
        status: WorkflowStatus::Running,
        trigger_input: None,
        pinned_worker_host: None,
        tasks: HashMap::from([
            (
                "task-a[1]".to_string(),
                TaskInstance {
                    early_exit: false,
                    task_def_id: "task-a".to_string(),
                    status: TaskStatus::Completed,
                    satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                    human_input: None,
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: Some(json!({ "draft": "first" })),
                    generation_index: 1,
                    verifier_metadata: None,
                },
            ),
            ("task-a[2]".to_string(), task_instance.clone()),
        ]),

        verifier_states: HashMap::new(),
    };
    instance.verifier_states.insert(
        "verify".to_string(),
        VerifierGenerationState {
            verifier_task_id: "verify".to_string(),
            rerun_start_task_id: "task-a".to_string(),
            latest_generation: 2,
            selected_generation: None,
            feedback_history: vec![VerifierFeedbackEntry {
                generation_index: 1,
                feedback: "Add citations.".to_string(),
                verifier_output: json!({ "decision": "continue" }),
            }],
            status: VerifierStateStatus::Running,
            exit_reason: None,
        },
    );

    let metadata = engine.execution_metadata(&instance, &def, &task_instance);
    let loop_context = metadata.loop_context.unwrap();

    assert_eq!(
        loop_context.feedback_history,
        vec![LoopFeedbackEntry {
            generation: 1,
            feedback: "Add citations.".to_string(),
        }]
    );
    assert_eq!(
        loop_context.previous_output,
        Some(json!({ "draft": "first" }))
    );
}

#[test]
fn test_execution_metadata_default_generation_index_is_one() {
    assert_eq!(ExecutionMetadata::default().generation_index, 1);
}

#[test]
fn test_execution_metadata_includes_task_instance_generation_index() {
    let engine = make_engine();
    let def = WorkflowDef {
        example_input: None,
        id: "def-generation-metadata".to_string(),
        description: String::new(),
        tasks: vec![task_def("task-a", json!({ "type": "object" }))],
        data_bindings: vec![],
    };
    let task_instance = TaskInstance {
        early_exit: false,
        task_def_id: "task-a".to_string(),
        status: TaskStatus::Pending,
        satisfaction_status: TaskSatisfactionStatus::Pending,
        human_input: None,
        input_data: vec![],
        input_mapping: vec![],
        output_data: None,
        generation_index: 2,
        verifier_metadata: None,
    };
    let instance = WorkflowInstance {
        id: "inst-generation-metadata".to_string(),
        workflow_def_id: def.id.clone(),
        version: 0,
        status: WorkflowStatus::Running,
        trigger_input: None,
        pinned_worker_host: None,
        tasks: HashMap::from([("task-a[2]".to_string(), task_instance.clone())]),

        verifier_states: HashMap::new(),
    };

    let metadata = engine.execution_metadata(&instance, &def, &task_instance);

    assert_eq!(metadata.generation_index, 2);
}

/// A single task with no dependencies should run and complete the workflow.
#[tokio::test]
async fn test_single_task_workflow_completes() {
    let engine = make_engine();

    let def = WorkflowDef {
        example_input: None,
        id: "def-1".to_string(),
        description: String::new(),
        tasks: vec![task_def("task-a", json!({ "type": "object" }))],
        data_bindings: vec![],
    };

    let instance_id = setup(&engine, def).await;
    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    let result = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status, WorkflowStatus::Completed);
    assert_eq!(result.tasks["task-a[1]"].status, TaskStatus::Completed);
    assert_eq!(result.tasks["task-a[1]"].task_def_id, "task-a");
    assert_eq!(result.tasks["task-a[1]"].generation_index, 1);
}

#[tokio::test]
async fn paused_workflow_records_in_flight_nonfinal_task_and_stops() {
    let storage = Arc::new(MemoryStorage::new());
    let dispatcher = Arc::new(PauseDuringTaskDispatcher::new(
        storage.clone(),
        "inst-paused",
    ));
    let engine = WorkflowEngine::new(storage.clone(), dispatcher.clone());
    let def = WorkflowDef {
        example_input: None,
        id: "def-paused-nonfinal".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            task_def("task-b", json!({ "type": "object" })),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "task-a".to_string(),
            target_task_id: "task-b".to_string(),
        }],
    };
    storage
        .save_workflow_def(&crate::core::namespace::test_namespace(), def.clone())
        .await
        .unwrap();
    storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            WorkflowInstance {
                id: "inst-paused".to_string(),
                workflow_def_id: def.id,
                version: 0,
                status: WorkflowStatus::Pending,
                trigger_input: None,
                pinned_worker_host: None,
                tasks: HashMap::new(),

                verifier_states: HashMap::new(),
            },
        )
        .await
        .unwrap();

    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            "inst-paused".to_string(),
        )
        .await
        .unwrap();

    let saved = storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), "inst-paused")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, WorkflowStatus::Paused);
    assert_eq!(saved.tasks["task-a[1]"].status, TaskStatus::Completed);
    assert_eq!(saved.tasks["task-b[1]"].status, TaskStatus::Pending);
    assert_eq!(dispatcher.calls(), vec!["task-a".to_string()]);
}

#[tokio::test]
async fn paused_workflow_records_in_flight_final_task_and_completes() {
    let storage = Arc::new(MemoryStorage::new());
    let dispatcher = Arc::new(PauseDuringTaskDispatcher::new(
        storage.clone(),
        "inst-paused",
    ));
    let engine = WorkflowEngine::new(storage.clone(), dispatcher.clone());
    let def = WorkflowDef {
        example_input: None,
        id: "def-paused-final".to_string(),
        description: String::new(),
        tasks: vec![task_def("task-a", json!({ "type": "object" }))],
        data_bindings: vec![],
    };
    storage
        .save_workflow_def(&crate::core::namespace::test_namespace(), def.clone())
        .await
        .unwrap();
    storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            WorkflowInstance {
                id: "inst-paused".to_string(),
                workflow_def_id: def.id,
                version: 0,
                status: WorkflowStatus::Pending,
                trigger_input: None,
                pinned_worker_host: None,
                tasks: HashMap::new(),

                verifier_states: HashMap::new(),
            },
        )
        .await
        .unwrap();

    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            "inst-paused".to_string(),
        )
        .await
        .unwrap();

    let saved = storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), "inst-paused")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, WorkflowStatus::Completed);
    assert_eq!(saved.tasks["task-a[1]"].status, TaskStatus::Completed);
    assert_eq!(dispatcher.calls(), vec!["task-a".to_string()]);
}

/// Two independent tasks (A and B) feed into a third (C) via data bindings.
/// C should only run after both A and B complete (Fan-In).
#[tokio::test]
async fn test_fan_in_workflow_completes_with_propagation() {
    let engine = make_engine();

    let def = WorkflowDef {
        example_input: None,
        id: "def-2".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            task_def("task-b", json!({ "type": "object" })),
            TaskDef {
                id: "task-c".to_string(),
                kind: TaskTypeDef::ApiCall {
                    url: "http://example.com".to_string(),
                    method: "POST".to_string(),
                    headers: HashMap::new(),
                },
                control: None,
                timeout_secs: None,
                input_schemas: vec![
                    json!({ "type": "object" }), // from task-a
                    json!({ "type": "object" }), // from task-b
                ],
                output_schema: Some(json!({ "type": "object" })),
                workspace: None,
                required_credentials: vec![],
            },
        ],
        data_bindings: vec![
            DataBinding {
                source_task_id: "task-a".to_string(),
                target_task_id: "task-c".to_string(),
            },
            DataBinding {
                source_task_id: "task-b".to_string(),
                target_task_id: "task-c".to_string(),
            },
        ],
    };

    let instance_id = setup(&engine, def).await;
    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    let result = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status, WorkflowStatus::Completed);
    assert_eq!(result.tasks["task-a[1]"].status, TaskStatus::Completed);
    assert_eq!(result.tasks["task-b[1]"].status, TaskStatus::Completed);
    assert_eq!(result.tasks["task-c[1]"].status, TaskStatus::Completed);

    // task-c should have received propagated inputs at both index slots
    let task_c = &result.tasks["task-c[1]"];
    assert_eq!(task_c.input_data.len(), 2);
    assert_eq!(
        task_c.input_mapping,
        vec![
            TaskInputMapping {
                task_id: "task-a".to_string(),
                generation: 1,
            },
            TaskInputMapping {
                task_id: "task-b".to_string(),
                generation: 1,
            },
        ]
    );
}

#[tokio::test]
async fn test_verifier_continue_marks_rejected_slice_unsatisfied() {
    let engine = make_engine_with_dispatcher(Arc::new(ContinueThenCompleteDispatcher));
    let def = WorkflowDef {
        example_input: None,
        id: "def-loop-satisfaction".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            task_def("task-b", json!({ "type": "object" })),
            agent_verifier_task("verify", Some("task-a")),
        ],
        data_bindings: vec![
            DataBinding {
                source_task_id: "task-a".to_string(),
                target_task_id: "task-b".to_string(),
            },
            DataBinding {
                source_task_id: "task-b".to_string(),
                target_task_id: "verify".to_string(),
            },
        ],
    };

    let instance_id = setup(&engine, def).await;
    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();
    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    let events = engine
        .storage
        .list_workflow_instance_events(
            &crate::core::namespace::test_namespace(),
            &instance_id,
            WorkflowEventPageRequest {
                limit: 100,
                cursor: None,
            },
        )
        .await
        .unwrap()
        .items;

    assert!(events.iter().any(|record| matches!(
        &record.event,
        WorkflowInstanceEvent::VerifierFeedbackRecorded {
            verifier_task_id,
            ..
        } if verifier_task_id == "verify"
    )));
    assert!(events.iter().any(|record| matches!(
        &record.event,
        WorkflowInstanceEvent::TaskMaterialized {
            task_attempt_id,
            ..
        } if task_attempt_id == "task-a[2]"
    )));

    for task_id in ["task-a[1]", "task-b[1]", "verify[1]"] {
        assert_eq!(
            instance.tasks[task_id].satisfaction_status,
            TaskSatisfactionStatus::Unsatisfied
        );
    }
    for task_id in ["task-a[2]", "task-b[2]", "verify[2]"] {
        assert_eq!(
            instance.tasks[task_id].satisfaction_status,
            TaskSatisfactionStatus::Satisfied
        );
    }
    for task_id in ["task-a[1]", "task-b[1]", "task-a[2]", "task-b[2]"] {
        assert!(instance.tasks[task_id].verifier_metadata.is_none());
    }
    assert!(instance.tasks["verify[1]"].verifier_metadata.is_some());
    assert!(instance.tasks["verify[2]"].verifier_metadata.is_some());
    assert_eq!(
        instance.tasks["task-b[2]"].input_mapping,
        vec![TaskInputMapping {
            task_id: "task-a".to_string(),
            generation: 2,
        }]
    );
}

#[tokio::test]
async fn test_verifier_rerun_dispatches_same_logical_agent_identity() {
    let dispatcher = Arc::new(RecordingContinueDispatcher::new());
    let engine = make_engine_with_dispatcher(dispatcher.clone());
    let def = WorkflowDef {
        example_input: None,
        id: "def-agent-session-identity".to_string(),
        description: String::new(),
        tasks: vec![
            agent_task("task-a", true),
            agent_verifier_task("verify", Some("task-a")),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "task-a".to_string(),
            target_task_id: "verify".to_string(),
        }],
    };

    let pinned_host = WorkerHostId::new("host-a");
    let instance_id = setup_with_pin(&engine, def, Some(pinned_host.clone())).await;
    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    let agent_records: Vec<_> = dispatcher
        .records()
        .into_iter()
        .filter(|record| record.task_id == "task-a")
        .collect();

    assert_eq!(agent_records.len(), 2);
    assert_eq!(
        agent_records[0],
        RecordedExecution {
            workflow_inst_id: instance_id.clone(),
            task_id: "task-a".to_string(),
            generation_index: 1,
            pinned_host_id: Some(pinned_host.clone()),
            reuse_session: Some(true),
            feedback_count: 0,
            human_input_provided: None,
        }
    );
    assert_eq!(
        agent_records[1],
        RecordedExecution {
            workflow_inst_id: instance_id.clone(),
            task_id: "task-a".to_string(),
            generation_index: 2,
            pinned_host_id: Some(pinned_host),
            reuse_session: Some(true),
            feedback_count: 1,
            human_input_provided: None,
        }
    );
}

#[tokio::test]
async fn test_human_input_continuation_dispatches_same_logical_agent_identity() {
    let dispatcher = Arc::new(RecordingContinueDispatcher::new());
    let engine = make_engine_with_dispatcher(dispatcher.clone());
    let def = WorkflowDef {
        example_input: None,
        id: "def-human-input-continuation".to_string(),
        description: String::new(),
        tasks: vec![agent_task("task-a", true)],
        data_bindings: vec![],
    };
    let pinned_host = WorkerHostId::new("host-a");
    let instance_id = "inst-human-input-continuation".to_string();
    let instance = WorkflowInstance {
        id: instance_id.clone(),
        workflow_def_id: def.id.clone(),
        version: 0,
        status: WorkflowStatus::Pending,
        trigger_input: None,
        pinned_worker_host: Some(pinned_host.clone()),
        tasks: HashMap::from([
            (
                "task-a[1]".to_string(),
                TaskInstance {
                    early_exit: false,
                    task_def_id: "task-a".to_string(),
                    status: TaskStatus::InputNeeded {
                        input_request: "need clarification".to_string(),
                    },
                    satisfaction_status: TaskSatisfactionStatus::Pending,
                    human_input: None,
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: None,
                    generation_index: 1,
                    verifier_metadata: None,
                },
            ),
            (
                "task-a[2]".to_string(),
                TaskInstance {
                    early_exit: false,
                    task_def_id: "task-a".to_string(),
                    status: TaskStatus::Pending,
                    satisfaction_status: TaskSatisfactionStatus::Pending,
                    human_input: Some(json!("The customer prefers a concise answer.")),
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: None,
                    generation_index: 2,
                    verifier_metadata: None,
                },
            ),
        ]),

        verifier_states: HashMap::new(),
    };
    engine
        .storage
        .save_workflow_def(&crate::core::namespace::test_namespace(), def)
        .await
        .unwrap();
    engine
        .storage
        .save_workflow_instance(
            &crate::core::namespace::test_namespace(),
            0,
            vec![],
            instance,
        )
        .await
        .unwrap();

    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    assert_eq!(
        dispatcher.records(),
        vec![RecordedExecution {
            workflow_inst_id: instance_id.clone(),
            task_id: "task-a".to_string(),
            generation_index: 2,
            pinned_host_id: Some(pinned_host),
            reuse_session: Some(true),
            feedback_count: 0,
            human_input_provided: Some("The customer prefers a concise answer.".to_string()),
        }]
    );

    let saved = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, WorkflowStatus::Completed);
    assert!(matches!(
        saved.tasks["task-a[1]"].status,
        TaskStatus::InputNeeded { .. }
    ));
    assert_eq!(saved.tasks["task-a[2]"].status, TaskStatus::Completed);
    assert_eq!(saved.tasks["task-a[2]"].generation_index, 2);
}

#[test]
fn test_verifier_slice_uses_latest_materialized_completed_source_attempt() {
    let engine = make_engine();
    let verifier_task = agent_verifier_task("verify", Some("task-b"));
    let def = WorkflowDef {
        example_input: None,
        id: "def-verifier-latest-completed-source".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-b", json!({ "type": "object" })),
            verifier_task.clone(),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "task-b".to_string(),
            target_task_id: "verify".to_string(),
        }],
    };
    let loop_slices = engine.compute_loop_slices(&def);
    let instance = WorkflowInstance {
        id: "inst-verifier-latest-completed-source".to_string(),
        workflow_def_id: def.id.clone(),
        version: 0,
        status: WorkflowStatus::Running,
        trigger_input: None,
        pinned_worker_host: None,
        tasks: HashMap::from([
            (
                "task-b[1]".to_string(),
                TaskInstance {
                    early_exit: false,
                    task_def_id: "task-b".to_string(),
                    status: TaskStatus::InputNeeded {
                        input_request: "need clarification".to_string(),
                    },
                    satisfaction_status: TaskSatisfactionStatus::Pending,
                    human_input: None,
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: None,
                    generation_index: 1,
                    verifier_metadata: None,
                },
            ),
            (
                "task-b[2]".to_string(),
                TaskInstance {
                    early_exit: false,
                    task_def_id: "task-b".to_string(),
                    status: TaskStatus::Completed,
                    satisfaction_status: TaskSatisfactionStatus::Pending,
                    human_input: None,
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: Some(json!({ "value": "after-human-input" })),
                    generation_index: 2,
                    verifier_metadata: None,
                },
            ),
            (
                "verify[1]".to_string(),
                TaskInstance {
                    early_exit: false,
                    task_def_id: "verify".to_string(),
                    status: TaskStatus::Pending,
                    satisfaction_status: TaskSatisfactionStatus::Pending,
                    human_input: None,
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: None,
                    generation_index: 1,
                    verifier_metadata: None,
                },
            ),
        ]),

        verifier_states: HashMap::from([(
            "verify".to_string(),
            VerifierGenerationState {
                verifier_task_id: "verify".to_string(),
                rerun_start_task_id: "task-b".to_string(),
                latest_generation: 1,
                selected_generation: None,
                feedback_history: vec![],
                status: VerifierStateStatus::Running,
                exit_reason: None,
            },
        )]),
    };

    let resolved = engine
        .resolve_inputs(
            &instance,
            &def,
            &instance.tasks["verify[1]"],
            &verifier_task,
            &loop_slices,
        )
        .unwrap();

    assert_eq!(
        resolved.values,
        vec![json!({ "value": "after-human-input" })]
    );
    assert_eq!(
        resolved.mapping,
        vec![TaskInputMapping {
            task_id: "task-b".to_string(),
            generation: 2,
        }]
    );
}

#[test]
fn test_verifier_slice_waits_for_latest_materialized_source_attempt() {
    let engine = make_engine();
    let verifier_task = agent_verifier_task("verify", Some("task-b"));
    let def = WorkflowDef {
        example_input: None,
        id: "def-verifier-waits-current-source".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-b", json!({ "type": "object" })),
            verifier_task.clone(),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "task-b".to_string(),
            target_task_id: "verify".to_string(),
        }],
    };
    let loop_slices = engine.compute_loop_slices(&def);
    let instance = WorkflowInstance {
        id: "inst-verifier-waits-current-source".to_string(),
        workflow_def_id: def.id.clone(),
        version: 0,
        status: WorkflowStatus::Running,
        trigger_input: None,
        pinned_worker_host: None,
        tasks: HashMap::from([
            (
                "task-b[1]".to_string(),
                TaskInstance {
                    early_exit: false,
                    task_def_id: "task-b".to_string(),
                    status: TaskStatus::Completed,
                    satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                    human_input: None,
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: Some(json!({ "value": "rejected" })),
                    generation_index: 1,
                    verifier_metadata: None,
                },
            ),
            (
                "task-b[2]".to_string(),
                TaskInstance {
                    early_exit: false,
                    task_def_id: "task-b".to_string(),
                    status: TaskStatus::Pending,
                    satisfaction_status: TaskSatisfactionStatus::Pending,
                    human_input: None,
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: None,
                    generation_index: 2,
                    verifier_metadata: None,
                },
            ),
            (
                "verify[2]".to_string(),
                TaskInstance {
                    early_exit: false,
                    task_def_id: "verify".to_string(),
                    status: TaskStatus::Pending,
                    satisfaction_status: TaskSatisfactionStatus::Pending,
                    human_input: None,
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: None,
                    generation_index: 2,
                    verifier_metadata: None,
                },
            ),
        ]),

        verifier_states: HashMap::from([(
            "verify".to_string(),
            VerifierGenerationState {
                verifier_task_id: "verify".to_string(),
                rerun_start_task_id: "task-b".to_string(),
                latest_generation: 2,
                selected_generation: None,
                feedback_history: vec![],
                status: VerifierStateStatus::Running,
                exit_reason: None,
            },
        )]),
    };

    let resolved = engine.resolve_inputs(
        &instance,
        &def,
        &instance.tasks["verify[2]"],
        &verifier_task,
        &loop_slices,
    );

    assert!(resolved.is_none());
}

#[tokio::test]
async fn test_verifier_complete_accepts_first_generation() {
    let engine = make_engine_with_dispatcher(Arc::new(CompleteVerifierDispatcher));
    let def = WorkflowDef {
        example_input: None,
        id: "def-first-generation-accepted".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            task_def("task-b", json!({ "type": "object" })),
            task_def("task-c", json!({ "type": "object" })),
            agent_verifier_task("verify", Some("task-b")),
        ],
        data_bindings: vec![
            DataBinding {
                source_task_id: "task-a".to_string(),
                target_task_id: "task-b".to_string(),
            },
            DataBinding {
                source_task_id: "task-b".to_string(),
                target_task_id: "task-c".to_string(),
            },
            DataBinding {
                source_task_id: "task-c".to_string(),
                target_task_id: "verify".to_string(),
            },
        ],
    };

    let instance_id = setup(&engine, def).await;
    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();
    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Completed);
    assert!(!instance.tasks.contains_key("task-b[2]"));
    assert!(!instance.tasks.contains_key("task-c[2]"));
    assert!(!instance.tasks.contains_key("verify[2]"));

    for task_id in ["task-a[1]", "task-b[1]", "task-c[1]", "verify[1]"] {
        assert_eq!(instance.tasks[task_id].status, TaskStatus::Completed);
        assert_eq!(
            instance.tasks[task_id].satisfaction_status,
            TaskSatisfactionStatus::Satisfied
        );
    }
    assert_eq!(
        instance.verifier_states["verify"].status,
        VerifierStateStatus::Accepted
    );
    assert_eq!(
        instance.verifier_states["verify"].selected_generation,
        Some(1)
    );
}

#[tokio::test]
async fn test_function_verifier_can_drive_bounded_rerun() {
    let engine = make_engine_with_dispatcher(Arc::new(ContinueThenCompleteDispatcher));
    let def = WorkflowDef {
        example_input: None,
        id: "def-function-verifier".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            function_verifier_task("verify", Some("task-a")),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "task-a".to_string(),
            target_task_id: "verify".to_string(),
        }],
    };

    let instance_id = setup(&engine, def).await;
    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();
    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Completed);
    assert_eq!(
        instance.tasks["verify[1]"].satisfaction_status,
        TaskSatisfactionStatus::Unsatisfied
    );
    assert_eq!(
        instance.tasks["verify[2]"].satisfaction_status,
        TaskSatisfactionStatus::Satisfied
    );
    assert_eq!(
        instance.verifier_states["verify"].status,
        VerifierStateStatus::Accepted
    );
}

#[tokio::test]
async fn test_exhausted_verifier_fails_when_continue_policy_is_false() {
    let engine = make_engine_with_dispatcher(Arc::new(AlwaysContinueVerifierDispatcher));
    let def = WorkflowDef {
        example_input: None,
        id: "def-exhaustion-fail".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            agent_verifier_task_with_policy("verify", Some("task-a"), 2, false),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "task-a".to_string(),
            target_task_id: "verify".to_string(),
        }],
    };

    let instance_id = setup(&engine, def).await;
    let error = engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("exhausted iteration budget"));

    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    let state = &instance.verifier_states["verify"];
    assert_eq!(instance.status, WorkflowStatus::Failed);
    assert_eq!(state.status, VerifierStateStatus::ExhaustedFailed);
    assert_eq!(state.latest_generation, 2);
    assert_eq!(state.selected_generation, None);
    assert_eq!(
        state.exit_reason.as_deref(),
        Some("max_iterations_exhausted")
    );
    assert_eq!(instance.tasks["verify[2]"].status, TaskStatus::Failed);
    assert_eq!(
        instance.tasks["verify[2]"]
            .verifier_metadata
            .as_ref()
            .unwrap()
            .status,
        VerifierAttemptStatus::ExhaustedFailed
    );
}

#[tokio::test]
async fn test_exhausted_verifier_accepts_latest_generation_when_continue_policy_is_true() {
    let engine = make_engine_with_dispatcher(Arc::new(AlwaysContinueVerifierDispatcher));
    let def = WorkflowDef {
        example_input: None,
        id: "def-exhaustion-accept".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            agent_verifier_task_with_policy("verify", Some("task-a"), 2, true),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "task-a".to_string(),
            target_task_id: "verify".to_string(),
        }],
    };

    let instance_id = setup(&engine, def).await;
    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    let state = &instance.verifier_states["verify"];
    assert_eq!(instance.status, WorkflowStatus::Completed);
    assert_eq!(state.status, VerifierStateStatus::ExhaustedAccepted);
    assert_eq!(state.latest_generation, 2);
    assert_eq!(state.selected_generation, Some(2));
    assert_eq!(
        state.exit_reason.as_deref(),
        Some("max_iterations_exhausted")
    );
    assert_eq!(
        instance.tasks["verify[2]"].satisfaction_status,
        TaskSatisfactionStatus::Satisfied
    );
    assert_eq!(
        instance.tasks["verify[2]"]
            .verifier_metadata
            .as_ref()
            .unwrap()
            .status,
        VerifierAttemptStatus::ExhaustedAccepted
    );
}

#[test]
fn test_exhausted_continue_fails_without_schema_valid_latest_output() {
    let engine = make_engine();
    let mut verifier_task = agent_verifier_task_with_policy("verify", Some("task-a"), 1, true);
    verifier_task.output_schema = None;
    let def = WorkflowDef {
        example_input: None,
        id: "def-exhaustion-no-valid-output".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            verifier_task,
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "task-a".to_string(),
            target_task_id: "verify".to_string(),
        }],
    };
    let loop_slices = engine.compute_loop_slices(&def);
    let instance = WorkflowInstance {
        id: "inst-exhaustion-no-valid-output".to_string(),
        workflow_def_id: def.id.clone(),
        version: 0,
        status: WorkflowStatus::Running,
        trigger_input: None,
        pinned_worker_host: None,
        tasks: HashMap::from([(
            "verify[1]".to_string(),
            TaskInstance {
                early_exit: false,
                task_def_id: "verify".to_string(),
                status: TaskStatus::Completed,
                satisfaction_status: TaskSatisfactionStatus::Pending,
                human_input: None,
                input_data: vec![],
                input_mapping: vec![],
                output_data: None,
                generation_index: 1,
                verifier_metadata: None,
            },
        )]),

        verifier_states: HashMap::from([(
            "verify".to_string(),
            VerifierGenerationState {
                verifier_task_id: "verify".to_string(),
                rerun_start_task_id: "task-a".to_string(),
                latest_generation: 1,
                selected_generation: None,
                feedback_history: vec![],
                status: VerifierStateStatus::Running,
                exit_reason: None,
            },
        )]),
    };

    let transition = engine
        .verifier_result_transition(
            &instance,
            &def,
            &loop_slices,
            "verify[1]",
            &serde_json::Value::Null,
            VerifierExecutionResult {
                decision: VerifierDecision::Continue,
                feedback: Some("try again".to_string()),
                output: json!({
                    "decision": "continue",
                    "feedback": "try again"
                }),
            },
        )
        .unwrap();

    assert!(
        transition
            .error_message
            .as_ref()
            .unwrap()
            .contains("no schema-valid output")
    );
    let instance = crate::core::workflow::events::reduce_workflow_instance_events(
        Some(instance),
        &transition.events,
    )
    .unwrap();
    assert_eq!(instance.status, WorkflowStatus::Failed);
    assert_eq!(
        instance.verifier_states["verify"].status,
        VerifierStateStatus::Failed
    );
    assert_eq!(instance.tasks["verify[1]"].status, TaskStatus::Failed);
    assert_eq!(
        instance.tasks["verify[1]"]
            .verifier_metadata
            .as_ref()
            .unwrap()
            .status,
        VerifierAttemptStatus::ExhaustedFailed
    );
}

#[tokio::test]
async fn test_downstream_uses_latest_satisfied_generation_after_verifier() {
    let engine = make_engine_with_dispatcher(Arc::new(ContinueThenCompleteDispatcher));
    let mut task_c = task_def("task-c", json!({ "type": "object" }));
    task_c.input_schemas = vec![json!({ "type": "object" }), json!({ "type": "object" })];
    let def = WorkflowDef {
        example_input: None,
        id: "def-downstream-latest-satisfied".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            task_def("task-b", json!({ "type": "object" })),
            agent_verifier_task("verify", Some("task-b")),
            task_c,
        ],
        data_bindings: vec![
            DataBinding {
                source_task_id: "task-a".to_string(),
                target_task_id: "task-b".to_string(),
            },
            DataBinding {
                source_task_id: "task-b".to_string(),
                target_task_id: "verify".to_string(),
            },
            DataBinding {
                source_task_id: "task-b".to_string(),
                target_task_id: "task-c".to_string(),
            },
            DataBinding {
                source_task_id: "verify".to_string(),
                target_task_id: "task-c".to_string(),
            },
        ],
    };

    let instance_id = setup(&engine, def).await;
    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();
    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        instance.tasks["task-c[1]"].input_mapping,
        vec![
            TaskInputMapping {
                task_id: "task-b".to_string(),
                generation: 2,
            },
            TaskInputMapping {
                task_id: "verify".to_string(),
                generation: 2,
            },
        ]
    );
    assert_eq!(
        instance.tasks["task-c[1]"].satisfaction_status,
        TaskSatisfactionStatus::Satisfied
    );
}

/// A task whose output fails schema validation should mark the workflow as Failed.
#[tokio::test]
async fn test_schema_validation_failure_marks_workflow_failed() {
    let engine = make_engine();

    // FakeTaskDispatcher cannot satisfy a `const` schema — it returns `{}` for unknown constructs,
    // which will always fail this constraint.
    let strict_schema = json!({
        "const": "only-this-value"
    });

    let def = WorkflowDef {
        example_input: None,
        id: "def-3".to_string(),
        description: String::new(),
        tasks: vec![task_def("task-strict", strict_schema)],
        data_bindings: vec![],
    };

    let instance_id = setup(&engine, def).await;
    let run_result = engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await;
    assert!(run_result.is_err());

    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(instance.status, WorkflowStatus::Failed);
    assert_eq!(instance.tasks["task-strict[1]"].status, TaskStatus::Failed);
}

#[tokio::test]
async fn api_call_complete_response_satisfies_schema_and_propagates_downstream() {
    let response = json!({
        "status": 200,
        "headers": { "content-type": "application/json" },
        "body": { "id": "item-1" }
    });
    let dispatcher = Arc::new(ApiResponseDispatcher::new(response.clone()));
    let engine = make_engine_with_dispatcher(dispatcher.clone());
    let def = WorkflowDef {
        example_input: None,
        id: "api-response-valid".to_string(),
        description: String::new(),
        tasks: vec![
            TaskDef {
                id: "fetch-data".to_string(),
                kind: TaskTypeDef::ApiCall {
                    url: "http://example.com/items".to_string(),
                    method: "GET".to_string(),
                    headers: HashMap::from([(
                        "Accept".to_string(),
                        "application/json".to_string(),
                    )]),
                },
                control: None,
                timeout_secs: None,
                input_schemas: vec![],
                output_schema: Some(api_response_schema()),
                workspace: None,
                required_credentials: vec![],
            },
            task_def("consume-data", json!({ "type": "object" })),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "fetch-data".to_string(),
            target_task_id: "consume-data".to_string(),
        }],
    };

    let instance_id = setup(&engine, def).await;
    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(instance.status, WorkflowStatus::Completed);
    assert_eq!(
        instance.tasks["fetch-data[1]"].output_data,
        Some(response.clone())
    );
    assert_eq!(
        dispatcher.calls(),
        vec![
            ("fetch-data".to_string(), vec![]),
            ("consume-data".to_string(), vec![response]),
        ]
    );
}

#[tokio::test]
async fn api_call_complete_response_that_violates_schema_is_not_propagated() {
    let dispatcher = Arc::new(ApiResponseDispatcher::new(json!({
        "status": 200,
        "headers": { "content-type": "application/json" },
        "body": { "id": 42 }
    })));
    let engine = make_engine_with_dispatcher(dispatcher.clone());
    let def = WorkflowDef {
        example_input: None,
        id: "api-response-invalid".to_string(),
        description: String::new(),
        tasks: vec![
            TaskDef {
                id: "fetch-data".to_string(),
                kind: TaskTypeDef::ApiCall {
                    url: "http://example.com/items".to_string(),
                    method: "GET".to_string(),
                    headers: HashMap::new(),
                },
                control: None,
                timeout_secs: None,
                input_schemas: vec![],
                output_schema: Some(api_response_schema()),
                workspace: None,
                required_credentials: vec![],
            },
            task_def("consume-data", json!({ "type": "object" })),
        ],
        data_bindings: vec![DataBinding {
            source_task_id: "fetch-data".to_string(),
            target_task_id: "consume-data".to_string(),
        }],
    };

    let instance_id = setup(&engine, def).await;
    let result = engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await;
    assert!(result.is_err());

    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(instance.status, WorkflowStatus::Failed);
    assert_eq!(instance.tasks["fetch-data[1]"].status, TaskStatus::Failed);
    assert_eq!(instance.tasks["fetch-data[1]"].output_data, None);
    assert_eq!(dispatcher.calls(), vec![("fetch-data".to_string(), vec![])]);
}

fn api_response_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "required": ["status", "headers", "body"],
        "properties": {
            "status": { "type": "integer" },
            "headers": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            },
            "body": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" }
                }
            }
        }
    })
}

#[tokio::test]
async fn test_input_schema_failure_marks_workflow_failed() {
    let engine = make_engine();
    let mut downstream = task_def("task-b", json!({ "type": "object" }));
    downstream.input_schemas = vec![json!({ "type": "string" })];
    let def = WorkflowDef {
        example_input: None,
        id: "def-input-schema".to_string(),
        description: String::new(),
        tasks: vec![task_def("task-a", json!({ "type": "object" })), downstream],
        data_bindings: vec![DataBinding {
            source_task_id: "task-a".to_string(),
            target_task_id: "task-b".to_string(),
        }],
    };

    let instance_id = setup(&engine, def).await;
    let run_result = engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await;
    assert!(run_result.is_err());
    assert!(
        run_result
            .unwrap_err()
            .to_string()
            .contains("input 0 failed schema validation")
    );

    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(instance.status, WorkflowStatus::Failed);
    assert_eq!(instance.tasks["task-b[1]"].status, TaskStatus::Failed);
}

/// After a successful run, get_workflow_status should return a report reflecting
/// the completed state without exposing raw input/output data.
#[tokio::test]
async fn test_get_workflow_status_after_completion() {
    let engine = make_engine();

    let def = WorkflowDef {
        example_input: None,
        id: "def-status".to_string(),
        description: String::new(),
        tasks: vec![
            task_def("task-a", json!({ "type": "object" })),
            task_def("task-b", json!({ "type": "object" })),
        ],
        data_bindings: vec![],
    };

    let instance_id = setup(&engine, def).await;
    engine
        .run_workflow_instance(
            &crate::core::namespace::test_namespace(),
            instance_id.clone(),
        )
        .await
        .unwrap();

    let report = engine
        .get_workflow_status(&crate::core::namespace::test_namespace(), &instance_id)
        .await
        .unwrap()
        .expect("report should be present");

    assert_eq!(report.instance_id, instance_id);
    assert_eq!(report.status, WorkflowStatus::Completed);
    assert_eq!(report.tasks.len(), 2);

    // Tasks are sorted by task attempt id, so task-a[1] comes first.
    assert_eq!(report.tasks[0].task_attempt_id, "task-a[1]");
    assert_eq!(report.tasks[0].task_def_id, "task-a");
    assert_eq!(report.tasks[0].status, TaskStatus::Completed);

    assert_eq!(report.tasks[1].task_attempt_id, "task-b[1]");
    assert_eq!(report.tasks[1].task_def_id, "task-b");
    assert_eq!(report.tasks[1].status, TaskStatus::Completed);
}

/// get_workflow_status should return None for an unknown instance id.
#[tokio::test]
async fn test_get_workflow_status_unknown_instance() {
    let engine = make_engine();
    let report = engine
        .get_workflow_status(&crate::core::namespace::test_namespace(), "does-not-exist")
        .await
        .unwrap();
    assert!(report.is_none());
}

struct ExitOutputDispatcher {
    output: serde_json::Value,
    calls: StdMutex<Vec<String>>,
}

#[async_trait]
impl TaskDispatchPort for ExitOutputDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &Namespace,
        _workflow_inst_id: &str,
        task: &TaskDef,
        _inputs: &[serde_json::Value],
        _metadata: &ExecutionMetadata,
        _dispatch: &TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        self.calls.lock().unwrap().push(task.id.clone());
        Ok(ExecutionResult::Success(self.output.clone()))
    }
}

async fn run_exit_case(
    output: serde_json::Value,
    schema: Option<serde_json::Value>,
) -> (WorkflowInstance, Vec<String>, bool, Vec<(String, bool)>) {
    let dispatcher = Arc::new(ExitOutputDispatcher {
        output,
        calls: StdMutex::new(vec![]),
    });
    let engine = make_engine_with_dispatcher(dispatcher.clone());
    let mut exit = task_def("a-exit", json!({"type": "object"}));
    exit.output_schema = schema;
    let id = setup(
        &engine,
        WorkflowDef {
            example_input: None,
            id: "exit-test".into(),
            description: String::new(),
            tasks: vec![
                exit,
                task_def("b-independent", json!({"type":"object"})),
                task_def("c-downstream", json!({"type":"object"})),
            ],
            data_bindings: vec![DataBinding {
                source_task_id: "a-exit".into(),
                target_task_id: "c-downstream".into(),
            }],
        },
    )
    .await;
    let failed = engine
        .run_workflow_instance(&crate::core::namespace::test_namespace(), id.clone())
        .await
        .is_err();
    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &id)
        .await
        .unwrap()
        .unwrap();
    let records = engine
        .storage
        .list_workflow_instance_events(
            &crate::core::namespace::test_namespace(),
            &id,
            WorkflowEventPageRequest {
                limit: 100,
                cursor: None,
            },
        )
        .await
        .unwrap()
        .items;
    let exit_events = records
        .into_iter()
        .filter_map(|record| match record.event {
            WorkflowInstanceEvent::TaskEarlyExitSet {
                task_attempt_id,
                early_exit,
            } => Some((task_attempt_id, early_exit)),
            _ => None,
        })
        .collect();
    let calls = dispatcher.calls.lock().unwrap().clone();
    (instance, calls, failed, exit_events)
}

#[tokio::test]
async fn early_exit_skips_independent_and_downstream_tasks_and_preserves_unschematized_output() {
    let output =
        json!({"_workflow_exit_reason":"already processed", "explanation":"already processed"});
    let (instance, calls, failed, exit_events) = run_exit_case(output.clone(), None).await;
    assert!(!failed);
    assert_eq!(calls, vec!["a-exit"]);
    assert_eq!(instance.status, WorkflowStatus::Completed);
    assert_eq!(instance.tasks["a-exit[1]"].status, TaskStatus::Completed);
    assert_eq!(instance.tasks["a-exit[1]"].output_data, Some(output));
    assert!(instance.tasks["a-exit[1]"].early_exit);
    for id in ["b-independent[1]", "c-downstream[1]"] {
        assert_eq!(instance.tasks[id].status, TaskStatus::Skipped);
        assert!(!instance.tasks[id].early_exit);
        assert_eq!(
            instance.tasks[id].satisfaction_status,
            TaskSatisfactionStatus::Unsatisfied
        );
        assert_eq!(instance.tasks[id].output_data, None);
    }
    assert_eq!(exit_events, vec![("a-exit[1]".into(), true)]);
}

#[tokio::test]
async fn execution_continues_without_reserved_reason() {
    for output in [
        json!({}),
        json!({"_workflow_exit_reason":null}),
        json!({"workflow_exit_reason":"ordinary application data"}),
        json!({"workflow_exit_reason":false}),
    ] {
        let (instance, calls, failed, exit_events) = run_exit_case(output, None).await;
        assert!(!failed);
        assert_eq!(calls, vec!["a-exit", "b-independent", "c-downstream"]);
        assert_eq!(instance.status, WorkflowStatus::Completed);
        assert!(exit_events.is_empty());
    }
}

#[tokio::test]
async fn invalid_exit_reason_fails_without_skipping_pending_tasks() {
    for output in [
        json!({"_workflow_exit_reason":""}),
        json!({"_workflow_exit_reason":"  "}),
        json!({"_workflow_exit_reason":true}),
    ] {
        let (instance, calls, failed, exit_events) = run_exit_case(output, None).await;
        assert!(exit_events.is_empty());
        assert!(failed);
        assert_eq!(calls, vec!["a-exit"]);
        assert_eq!(instance.status, WorkflowStatus::Failed);
        assert_eq!(instance.tasks["a-exit[1]"].status, TaskStatus::Failed);
        assert_eq!(
            instance.tasks["b-independent[1]"].status,
            TaskStatus::Pending
        );
    }
}

#[tokio::test]
async fn invalid_output_schema_cannot_request_early_exit() {
    let (instance, calls, failed, exit_events) = run_exit_case(
        json!({"_workflow_exit_reason":"already processed"}),
        Some(json!({"type":"string"})),
    )
    .await;
    assert_eq!(calls, vec!["a-exit"]);
    assert!(exit_events.is_empty());
    assert_eq!(
        instance.tasks["b-independent[1]"].status,
        TaskStatus::Pending
    );
    assert!(failed);
    assert_eq!(instance.status, WorkflowStatus::Failed);
}

#[tokio::test]
async fn early_exit_retains_prior_outputs_and_running_attempts_and_waits_for_them() {
    let dispatcher = Arc::new(ExitOutputDispatcher {
        output: json!({"_workflow_exit_reason":"already processed"}),
        calls: StdMutex::new(vec![]),
    });
    let engine = make_engine_with_dispatcher(dispatcher.clone());
    let exit = task_def("exit", json!({"type":"object"}));
    let def = WorkflowDef {
        example_input: None,
        id: "running-exit".into(),
        description: String::new(),
        tasks: vec![
            exit,
            task_def("running", json!({"type":"object"})),
            task_def("prior", json!({"type":"object"})),
            task_def("pending", json!({"type":"object"})),
        ],
        data_bindings: vec![],
    };
    let id = setup(&engine, def).await;
    let manager = WorkflowStateManager::new(engine.storage.clone());
    manager
        .commit_events(
            &crate::core::namespace::test_namespace(),
            &id,
            vec![
                WorkflowInstanceEvent::TaskMaterialized {
                    task_attempt_id: "exit[1]".into(),
                    task: pending_task_instance("exit"),
                },
                WorkflowInstanceEvent::TaskMaterialized {
                    task_attempt_id: "pending[1]".into(),
                    task: pending_task_instance("pending"),
                },
                WorkflowInstanceEvent::TaskMaterialized {
                    task_attempt_id: "running[1]".into(),
                    task: TaskInstance {
                        early_exit: false,
                        status: TaskStatus::Running,
                        ..pending_task_instance("running")
                    },
                },
                WorkflowInstanceEvent::TaskMaterialized {
                    task_attempt_id: "prior[1]".into(),
                    task: TaskInstance {
                        early_exit: false,
                        status: TaskStatus::Completed,
                        output_data: Some(json!({"retained":true})),
                        satisfaction_status: TaskSatisfactionStatus::Satisfied,
                        ..pending_task_instance("prior")
                    },
                },
            ],
        )
        .await
        .unwrap();
    engine
        .run_workflow_instance(&crate::core::namespace::test_namespace(), id.clone())
        .await
        .unwrap();
    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(instance.status, WorkflowStatus::Running);
    assert_eq!(instance.tasks["pending[1]"].status, TaskStatus::Skipped);
    assert_eq!(instance.tasks["running[1]"].status, TaskStatus::Running);
    assert_eq!(
        instance.tasks["prior[1]"].output_data,
        Some(json!({"retained":true}))
    );
    assert_eq!(*dispatcher.calls.lock().unwrap(), vec!["exit"]);
    manager
        .commit_events(
            &crate::core::namespace::test_namespace(),
            &id,
            vec![
                WorkflowInstanceEvent::TaskStatusChanged {
                    task_attempt_id: "running[1]".into(),
                    status: TaskStatus::InputNeeded {
                        input_request: "confirm report context".into(),
                    },
                },
                WorkflowInstanceEvent::WorkflowStatusChanged {
                    status: WorkflowStatus::InputNeeded,
                },
            ],
        )
        .await
        .unwrap();
    manager
        .commit_events(
            &crate::core::namespace::test_namespace(),
            &id,
            vec![WorkflowInstanceEvent::HumanInputSubmitted {
                task_attempt_id: "running[1]".into(),
                continuation_task_attempt_id: "running[2]".into(),
                submitted_input: json!("confirmed"),
            }],
        )
        .await
        .unwrap();
    engine
        .run_workflow_instance(&crate::core::namespace::test_namespace(), id.clone())
        .await
        .unwrap();
    let finished = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finished.status, WorkflowStatus::Completed);
    assert_eq!(finished.tasks["pending[1]"].status, TaskStatus::Skipped);
    assert_eq!(finished.tasks["running[2]"].status, TaskStatus::Completed);
    // The continuation also returns the reserved reason and triggers exit.
    assert!(finished.tasks["running[2]"].early_exit);
    assert_eq!(*dispatcher.calls.lock().unwrap(), vec!["exit", "running"]);
}

#[tokio::test]
async fn skipped_verifier_slices_complete_without_acceptance_or_new_generations() {
    for initialized_before_exit in [true, false] {
        let dispatcher = Arc::new(ExitOutputDispatcher {
            output: json!({"_workflow_exit_reason":"already processed"}),
            calls: StdMutex::new(vec![]),
        });
        let engine = make_engine_with_dispatcher(dispatcher.clone());
        let exit = task_def("a-exit", json!({"type":"object"}));
        let mut bindings = vec![DataBinding {
            source_task_id: "b-work".into(),
            target_task_id: "c-verify".into(),
        }];
        if !initialized_before_exit {
            bindings.push(DataBinding {
                source_task_id: "a-exit".into(),
                target_task_id: "b-work".into(),
            });
        }
        let def = WorkflowDef {
            example_input: None,
            id: "skip-loop".into(),
            description: String::new(),
            tasks: vec![
                exit,
                task_def("b-work", json!({"type":"object"})),
                agent_verifier_task("c-verify", Some("b-work")),
            ],
            data_bindings: bindings,
        };
        let id = setup(&engine, def).await;
        engine
            .run_workflow_instance(&crate::core::namespace::test_namespace(), id.clone())
            .await
            .unwrap();
        let instance = engine
            .storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(instance.status, WorkflowStatus::Completed);
        assert_eq!(instance.tasks["c-verify[1]"].status, TaskStatus::Skipped);
        assert_eq!(instance.tasks.len(), 3);
        assert_eq!(
            instance.verifier_states.contains_key("c-verify"),
            initialized_before_exit
        );
        assert_eq!(*dispatcher.calls.lock().unwrap(), vec!["a-exit"]);
    }
}

#[tokio::test]
async fn verifier_can_trigger_exit_without_retrying_or_exhaustion_failure() {
    let dispatcher = Arc::new(ExitOutputDispatcher {
        output: json!({"decision":"continue", "feedback":"retry normally", "_workflow_exit_reason":"report no longer needed"}),
        calls: StdMutex::new(vec![]),
    });
    let engine = make_engine_with_dispatcher(dispatcher.clone());
    let verifier = agent_verifier_task_with_policy("verify", None, 1, false);
    let id = setup(
        &engine,
        WorkflowDef {
            example_input: None,
            id: "verifier-exit".into(),
            description: String::new(),
            tasks: vec![verifier],
            data_bindings: vec![],
        },
    )
    .await;
    engine
        .run_workflow_instance(&crate::core::namespace::test_namespace(), id.clone())
        .await
        .unwrap();
    let instance = engine
        .storage
        .get_workflow_instance(&crate::core::namespace::test_namespace(), &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(instance.status, WorkflowStatus::Completed);
    let attempt = &instance.tasks["verify[1]"];
    assert!(attempt.early_exit);
    assert_eq!(attempt.status, TaskStatus::Completed);
    assert_eq!(
        attempt.satisfaction_status,
        TaskSatisfactionStatus::Unsatisfied
    );
    assert_eq!(
        attempt.verifier_metadata.as_ref().unwrap().decision,
        Some(VerifierDecision::Continue)
    );
    assert_eq!(instance.verifier_states["verify"].latest_generation, 1);
    assert_eq!(*dispatcher.calls.lock().unwrap(), vec!["verify"]);
}

#[tokio::test]
async fn independent_verifier_continues_with_skipped_consumer_after_exit() {
    for has_consumer in [true, false] {
        let engine = make_engine_with_dispatcher(Arc::new(ContinueThenCompleteDispatcher));
        let def = WorkflowDef {
            example_input: None,
            id: "continue-after-exit".into(),
            description: String::new(),
            tasks: vec![
                task_def("exit", json!({"type":"object"})),
                agent_verifier_task("verify", None),
                task_def("consumer", json!({"type":"object"})),
            ],
            data_bindings: if has_consumer {
                vec![DataBinding {
                    source_task_id: "verify".into(),
                    target_task_id: "consumer".into(),
                }]
            } else {
                vec![]
            },
        };
        let id = setup(&engine, def).await;
        let manager = WorkflowStateManager::new(engine.storage.clone());
        manager
            .commit_events(
                &crate::core::namespace::test_namespace(),
                &id,
                vec![
                    WorkflowInstanceEvent::TaskMaterialized {
                        task_attempt_id: "exit[1]".into(),
                        task: TaskInstance {
                            status: TaskStatus::Completed,
                            early_exit: true,
                            ..pending_task_instance("exit")
                        },
                    },
                    WorkflowInstanceEvent::TaskMaterialized {
                        task_attempt_id: "consumer[1]".into(),
                        task: TaskInstance {
                            status: TaskStatus::Skipped,
                            ..pending_task_instance("consumer")
                        },
                    },
                    WorkflowInstanceEvent::TaskMaterialized {
                        task_attempt_id: "verify[1]".into(),
                        task: pending_task_instance("verify"),
                    },
                ],
            )
            .await
            .unwrap();
        // Model the retry work permitted to finish an in-flight independent verifier.
        engine
            .run_workflow_instance(&crate::core::namespace::test_namespace(), id.clone())
            .await
            .unwrap();
        let instance = engine
            .storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(instance.status, WorkflowStatus::Completed);
        assert_eq!(instance.tasks["consumer[1]"].status, TaskStatus::Skipped);
        assert_eq!(instance.tasks["verify[2]"].status, TaskStatus::Completed);
        assert_eq!(
            instance.verifier_states["verify"].status,
            VerifierStateStatus::Accepted
        );
    }
}
