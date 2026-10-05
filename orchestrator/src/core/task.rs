use serde::{Deserialize, Serialize};
use serde_json::Number;
use std::collections::HashMap;

use crate::core::function::models::FunctionTaskDef;
use crate::core::verifier::{LoopExecutionContext, VerifierAttemptMetadata, VerifierControlConfig};

pub type JsonSchema = serde_json::Value;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskTypeDef {
    #[serde(rename = "apiCall")]
    ApiCall {
        url: String,
        method: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    #[serde(rename = "agent")]
    Agent {
        // Model name, e.g. sonnet, oput, gpt-5.5, gemini-2.5-flash, etc.
        model_id: String,
        provider_url: String,
        // Agent prompt
        prompt: String,
        // Allowed tools, [] - none, ["_all_"] - all available tools
        tools: Vec<String>,
        // Allowed skills, [] - none, no wildcard support
        skills: Vec<String>,
        // Gives agent allowance to pause task to get additional information if needed
        ask: bool,
        // How many times agent should re-try when output does not match expected output_schema
        schema_failure_retry_times: Number,
        // Whether or not harness session should be re-used across attempts
        #[serde(default = "default_true")]
        reuse_session: bool,
    },
    #[serde(rename = "function")]
    Function(FunctionTaskDef),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskControl {
    #[serde(default)]
    pub allow_early_exit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier: Option<VerifierControlConfig>,
}

pub fn workflow_exit_reason(output: &serde_json::Value) -> anyhow::Result<Option<&str>> {
    match output.get("workflow_exit_reason") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(reason)) if !reason.trim().is_empty() => Ok(Some(reason)),
        _ => anyhow::bail!("workflow_exit_reason must be null or a nonempty string"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub group_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDef {
    pub id: String,
    pub kind: TaskTypeDef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<TaskControl>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_schemas: Vec<JsonSchema>,
    pub output_schema: Option<JsonSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Workspace>,
    pub required_credentials: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    InputNeeded { input_request: String },
    Completed,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskSatisfactionStatus {
    #[default]
    Pending,
    Satisfied,
    Unsatisfied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskInputMapping {
    pub task_id: String,
    pub generation: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInstance {
    /// Logical task definition ID this attempt executes.
    pub task_def_id: String,
    /// Current lifecycle state for this concrete attempt.
    pub status: TaskStatus,
    /// Whether this attempt is eligible as a satisfied source for downstream data binding.
    #[serde(default)]
    pub satisfaction_status: TaskSatisfactionStatus,
    /// Whether this attempt triggered successful early workflow exit.
    #[serde(default)]
    pub early_exit: bool,
    /// Raw human response that caused this continuation attempt, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_input: Option<serde_json::Value>,
    /// Resolved upstream input values passed to this attempt when it runs.
    pub input_data: Vec<serde_json::Value>,
    /// Records which upstream task generations produced `input_data`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_mapping: Vec<TaskInputMapping>,
    /// Structured output recorded after successful execution.
    pub output_data: Option<serde_json::Value>,
    /// Concrete attempt generation for the logical task, starting at 1.
    pub generation_index: u32,
    /// Verifier result metadata when this attempt is a verifier task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier_metadata: Option<VerifierAttemptMetadata>,
}

impl TaskInstance {
    pub fn make_task_attempt_id(task_def_id: &str, generation_index: u32) -> String {
        format!("{task_def_id}[{generation_index}]")
    }
}

fn default_generation_index() -> u32 {
    1
}

/// Transient context passed through task dispatch for one concrete task attempt.
/// Durable workflow state remains on `TaskInstance`; this model carries the
/// attempt details the worker needs while executing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionMetadata {
    #[serde(default = "default_generation_index")]
    pub generation_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_context: Option<LoopExecutionContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_input_provided: Option<String>,
}

impl Default for ExecutionMetadata {
    fn default() -> Self {
        Self {
            generation_index: default_generation_index(),
            loop_context: None,
            human_input_provided: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceKey {
    Task {
        workflow_inst_id: String,
        task_id: String,
    },
    Group {
        workflow_inst_id: String,
        group_name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_reuse_session_defaults_to_true_when_omitted() {
        let task: TaskDef = serde_json::from_value(json!({
            "id": "agenttask",
            "kind": {
                "agent": {
                    "model_id": "test/model",
                    "provider_url": "",
                    "prompt": "Do the work.",
                    "tools": [],
                    "skills": [],
                    "ask": false,
                    "schema_failure_retry_times": 0
                }
            },
            "output_schema": null,
            "required_credentials": []
        }))
        .unwrap();

        assert!(matches!(
            task.kind,
            TaskTypeDef::Agent {
                reuse_session: true,
                ..
            }
        ));
    }

    #[test]
    fn agent_reuse_session_serializes_explicit_false() {
        let task: TaskDef = serde_json::from_value(json!({
            "id": "agenttask",
            "kind": {
                "agent": {
                    "model_id": "test/model",
                    "provider_url": "",
                    "prompt": "Do the work.",
                    "tools": [],
                    "skills": [],
                    "ask": false,
                    "schema_failure_retry_times": 0,
                    "reuse_session": false
                }
            },
            "output_schema": null,
            "required_credentials": []
        }))
        .unwrap();

        let serialized = serde_json::to_value(task).unwrap();
        assert_eq!(serialized["kind"]["agent"]["reuse_session"], json!(false));
    }

    #[test]
    fn task_kinds_serialize_with_lowercase_initials() {
        let api_call = serde_json::to_value(TaskTypeDef::ApiCall {
            url: "https://example.com".to_string(),
            method: "GET".to_string(),
            headers: HashMap::new(),
        })
        .unwrap();
        let function = serde_json::to_value(TaskTypeDef::Function(FunctionTaskDef::Inline {
            dependencies: vec![],
            code: "export default async function run() {}".to_string(),
        }))
        .unwrap();

        assert!(api_call.get("apiCall").is_some());
        assert!(function.get("function").is_some());
    }

    #[test]
    fn api_call_headers_default_to_empty_when_omitted_from_json() {
        let task: TaskTypeDef = serde_json::from_value(json!({
            "apiCall": {
                "url": "https://example.com/items",
                "method": "GET"
            }
        }))
        .unwrap();

        assert!(matches!(
            task,
            TaskTypeDef::ApiCall { headers, .. } if headers.is_empty()
        ));
    }

    #[test]
    fn api_call_headers_deserialize_from_yaml_and_serialize_for_worker_dispatch() {
        let yaml_value: serde_json::Value = serde_yaml::from_str(
            r#"
apiCall:
  url: https://example.com/items
  method: GET
  headers:
    Accept: application/json
    X-Client-Version: "1"
"#,
        )
        .unwrap();
        let task: TaskTypeDef = serde_json::from_value(yaml_value).unwrap();

        let serialized = serde_json::to_value(task).unwrap();
        assert_eq!(
            serialized["apiCall"]["headers"],
            json!({
                "Accept": "application/json",
                "X-Client-Version": "1"
            })
        );
    }

    #[test]
    fn task_workspace_defaults_to_none_when_omitted() {
        let task: TaskDef = serde_json::from_value(json!({
            "id": "task",
            "kind": {
                "function": {
                    "dependencies": [],
                    "code": "export default async function run() { return {}; }"
                }
            },
            "output_schema": null,
            "required_credentials": []
        }))
        .unwrap();

        assert!(task.workspace.is_none());
    }

    #[test]
    fn task_workspace_deserializes_nested_group_name() {
        let task: TaskDef = serde_json::from_value(json!({
            "id": "task",
            "kind": {
                "function": {
                    "dependencies": [],
                    "code": "export default async function run() { return {}; }"
                }
            },
            "workspace": {
                "group_name": "repo"
            },
            "output_schema": null,
            "required_credentials": []
        }))
        .unwrap();

        assert_eq!(
            task.workspace
                .as_ref()
                .map(|workspace| workspace.group_name.as_str()),
            Some("repo")
        );
    }

    #[test]
    fn task_workspace_rejects_multiple_group_declaration_fields() {
        let error = serde_json::from_value::<TaskDef>(json!({
            "id": "task",
            "kind": {
                "function": {
                    "dependencies": [],
                    "code": "export default async function run() { return {}; }"
                }
            },
            "workspace": {
                "group_name": "repo",
                "group": "other"
            },
            "output_schema": null,
            "required_credentials": []
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown field `group`"));
    }
    #[test]
    fn exit_reason_accepts_absence_null_and_nonempty_strings() {
        for output in [
            json!({}),
            json!({"workflow_exit_reason": null}),
            json!(false),
        ] {
            assert_eq!(workflow_exit_reason(&output).unwrap(), None);
        }
        let output = json!({"workflow_exit_reason": "Already processed"});
        assert_eq!(
            workflow_exit_reason(&output).unwrap(),
            Some("Already processed")
        );
    }

    #[test]
    fn exit_reason_rejects_empty_strings_and_other_types() {
        for value in [
            json!(""),
            json!(" \n\t"),
            json!(true),
            json!(1),
            json!([]),
            json!({}),
        ] {
            assert!(workflow_exit_reason(&json!({"workflow_exit_reason": value})).is_err());
        }
    }

    #[test]
    fn task_control_defaults_to_disabled_and_rejects_old_pointer_control() {
        let control: TaskControl = serde_json::from_value(json!({})).unwrap();
        assert!(!control.allow_early_exit);
        assert!(
            serde_json::from_value::<TaskControl>(json!({"exit_workflow": {"when": "/no_work"}}))
                .is_err()
        );
    }
}
