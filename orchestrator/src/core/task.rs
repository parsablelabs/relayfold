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
pub struct TaskControl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_workflow: Option<ExitWorkflowControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier: Option<VerifierControlConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitWorkflowControl {
    pub when: String,
}

impl ExitWorkflowControl {
    pub fn validate(&self) -> anyhow::Result<()> {
        let pointer = &self.when;
        if !pointer.is_empty() && !pointer.starts_with('/') {
            anyhow::bail!("exit_workflow.when must be a JSON Pointer");
        }
        let mut chars = pointer.chars();
        while let Some(c) = chars.next() {
            if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
                anyhow::bail!("exit_workflow.when contains an invalid JSON Pointer escape");
            }
        }
        Ok(())
    }

    pub fn evaluate(&self, output: &serde_json::Value) -> anyhow::Result<bool> {
        self.validate()?;
        output
            .pointer(&self.when)
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "exit_workflow.when {} must select an existing boolean",
                    self.when
                )
            })
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
    fn exit_pointer_handles_nested_arrays_escapes_and_root_boolean() {
        for (pointer, output) in [
            ("/body/no_work", json!({"body":{"no_work":true}})),
            ("/items/0", json!({"items":[true]})),
            ("/a~1b/~0", json!({"a/b":{"~":true}})),
            ("", json!(true)),
        ] {
            assert!(
                ExitWorkflowControl {
                    when: pointer.into()
                }
                .evaluate(&output)
                .unwrap()
            );
        }
        for pointer in ["no_work", "/bad~", "/bad~2"] {
            assert!(
                ExitWorkflowControl {
                    when: pointer.into()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn exit_pointer_requires_a_boolean_without_coercion() {
        let control = ExitWorkflowControl {
            when: "/no_work".into(),
        };
        for output in [
            json!({}),
            json!({"no_work":"true"}),
            json!({"no_work":1}),
            json!({"no_work":null}),
        ] {
            assert!(
                control.evaluate(&output).is_err(),
                "accepted invalid output: {output}"
            );
        }
        assert!(!control.evaluate(&json!({"no_work":false})).unwrap());
    }
}
