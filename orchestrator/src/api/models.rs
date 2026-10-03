use crate::core::{
    worker::{WorkerHostId, WorkerId, WorkerIdentity},
    workflow::events::WorkflowEventRecord,
    workflow::models::{WorkflowDefSummary, WorkflowInfo},
};
use crate::ports::task_dispatch::TaskDispatch;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Default, Deserialize)]
pub struct WorkflowDefFormatQuery {
    #[serde(default)]
    pub(crate) format: DefinitionFormat,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DefinitionFormat {
    #[default]
    Json,
    Yaml,
}

#[derive(Debug, Deserialize)]
pub struct InvokeTaskRequest {
    pub(crate) inputs: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowEventListQuery {
    pub(crate) limit: Option<usize>,
    pub(crate) after_sequence: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitHumanInputRequest {
    pub(crate) input: Value,
}

#[derive(Debug, Deserialize)]
pub struct RetryTaskQuery {
    pub(crate) force: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowListQuery {
    pub(crate) status: Option<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorkerRegistrationRequest {
    pub(crate) worker_id: String,
    pub(crate) host_id: WorkerHostId,
}

impl WorkerRegistrationRequest {
    pub fn into_identity(self) -> WorkerIdentity {
        WorkerIdentity {
            worker_id: WorkerId::new(self.worker_id),
            host_id: self.host_id,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkerClaimRequest {
    pub(crate) worker_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowList {
    pub workflows: Vec<WorkflowInfo>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowDefList {
    pub workflow_defs: Vec<WorkflowDefSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEvents {
    pub workflow_instance_id: String,
    pub events: Vec<WorkflowEventRecord>,
    pub next_sequence: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowQueueStatus {
    pub pending: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerResponse {
    RegistrationAck {
        worker_id: String,
        heartbeat_interval_ms: u64,
    },
    NoTask,
    TaskDispatch(TaskDispatch),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::function::models::FunctionTaskDef;
    use crate::core::task::{ExecutionMetadata, TaskDef, TaskTypeDef};
    use crate::ports::task_dispatch::WorkerExecutionResult;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn worker_registration_request_converts_to_domain_identity() {
        let request: WorkerRegistrationRequest = serde_json::from_value(json!({
            "worker_id": "worker-1",
            "host_id": "host-a"
        }))
        .unwrap();

        assert_eq!(
            request.into_identity(),
            WorkerIdentity {
                worker_id: WorkerId::new("worker-1"),
                host_id: WorkerHostId::new("host-a"),
            }
        );
    }

    #[test]
    fn worker_response_wire_shapes_remain_stable() {
        assert_eq!(
            serde_json::to_value(WorkerResponse::RegistrationAck {
                worker_id: "worker-1".to_string(),
                heartbeat_interval_ms: 5_000,
            })
            .unwrap(),
            json!({
                "type": "registration_ack",
                "worker_id": "worker-1",
                "heartbeat_interval_ms": 5_000
            })
        );
        assert_eq!(
            serde_json::to_value(WorkerResponse::NoTask).unwrap(),
            json!({ "type": "no_task" })
        );

        let response = WorkerResponse::TaskDispatch(TaskDispatch {
            namespace: crate::core::namespace::test_namespace(),
            workflow_inst_id: "workflow-1".to_string(),
            task_id: "dispatch-1".to_string(),
            task: TaskDef {
                sandbox: None,
                id: "hello".to_string(),
                kind: TaskTypeDef::Function(FunctionTaskDef::Inline {
                    dependencies: vec![],
                    code: "export default async function run() {}".to_string(),
                }),
                control: None,
                timeout_secs: None,
                input_schemas: vec![],
                output_schema: None,
                workspace: None,
                required_credentials: vec![],
            },
            workspace_path_suffix: PathBuf::from(
                "550e8400-e29b-41d4-a716-446655440000/workflow-1/taskid-hello",
            ),
            inputs: vec![json!({ "name": "Ada" })],
            execution_metadata: ExecutionMetadata::default(),
            human_input_provided: None,
        });

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "type": "task_dispatch",
                "namespace": "550e8400-e29b-41d4-a716-446655440000",
                "workflow_inst_id": "workflow-1",
                "task_id": "dispatch-1",
                "task": {
                    "id": "hello",
                    "kind": {
                        "function": {
                            "dependencies": [],
                            "code": "export default async function run() {}"
                        }
                    },
                    "timeout_secs": null,
                    "output_schema": null,
                    "required_credentials": []
                },
                "workspace_path_suffix": "550e8400-e29b-41d4-a716-446655440000/workflow-1/taskid-hello",
                "inputs": [{ "name": "Ada" }],
                "execution_metadata": { "generation_index": 1 }
            })
        );
    }

    #[test]
    fn worker_execution_result_wire_shapes_remain_stable() {
        let cases = [
            (
                WorkerExecutionResult::Success {
                    output: json!({ "response": "ok" }),
                },
                json!({ "kind": "success", "output": { "response": "ok" } }),
            ),
            (
                WorkerExecutionResult::InputNeeded {
                    description: "Which channel?".to_string(),
                },
                json!({ "kind": "input_needed", "description": "Which channel?" }),
            ),
            (
                WorkerExecutionResult::Failure {
                    reason: "missing credential".to_string(),
                },
                json!({ "kind": "failure", "reason": "missing credential" }),
            ),
        ];

        for (result, expected) in cases {
            assert_eq!(serde_json::to_value(result).unwrap(), expected);
        }
    }
}
