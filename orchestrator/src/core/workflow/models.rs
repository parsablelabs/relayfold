use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::core::namespace::Namespace;
use crate::core::task::{
    TaskDef, TaskInputMapping, TaskInstance, TaskSatisfactionStatus, TaskStatus,
};
use crate::core::verifier::VerifierAttemptMetadata;
use crate::core::worker::WorkerHostId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkflowStatus {
    Pending,
    Running,
    Paused,
    InputNeeded,
    Completed,
    Failed,
}

/// A lightweight read model describing the current state of a workflow instance.
/// Intended for status queries - does not expose raw input/output data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStatusReport {
    pub instance_id: String,
    pub workflow_def_id: String,
    pub status: WorkflowStatus,
    pub tasks: Vec<TaskStatusReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verifier_states: Vec<VerifierStatusReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatusReport {
    pub early_exit: bool,
    pub task_attempt_id: String,
    pub task_def_id: String,
    pub status: TaskStatus,
    pub satisfaction: TaskSatisfactionStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_mapping: Vec<TaskInputMapping>,
    pub generation_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier_metadata: Option<VerifierAttemptMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierStatusReport {
    pub verifier_task_id: String,
    pub rerun_start_task_id: String,
    pub latest_generation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_generation: Option<u32>,
    pub status: VerifierStateStatus,
    pub feedback_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInstance {
    pub id: String,
    pub workflow_def_id: String,
    #[serde(default)]
    pub version: u64,
    pub status: WorkflowStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_input: Option<serde_json::Value>,
    // TODO: make this non Optional if possible
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_worker_host: Option<WorkerHostId>,
    // Keyed by task_attempt_id, e.g. "task-a[2]".
    pub tasks: HashMap<String, TaskInstance>,
    #[serde(default)]
    pub verifier_states: HashMap<String, VerifierGenerationState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowInfo {
    #[serde(skip_serializing)]
    pub namespace: Namespace,
    pub id: String,
    pub workflow_def_id: String,
    pub created_at_epoch_ms: Option<u64>,
    pub modified_at_epoch_ms: u64,
    pub completed_at_epoch_ms: Option<u64>,
    pub status: WorkflowStatus,
    pub total_task_count: usize,
    pub completed_task_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowListPage {
    pub workflows: Vec<WorkflowInfo>,
    pub next_cursor: Option<String>,
}

impl WorkflowInfo {
    pub fn from_instance_with_timestamps(
        namespace: Namespace,
        instance: &WorkflowInstance,
        created_at_epoch_ms: Option<u64>,
        modified_at_epoch_ms: u64,
        completed_at_epoch_ms: Option<u64>,
    ) -> Self {
        Self {
            namespace,
            id: instance.id.clone(),
            workflow_def_id: instance.workflow_def_id.clone(),
            created_at_epoch_ms,
            modified_at_epoch_ms,
            completed_at_epoch_ms,
            status: instance.status.clone(),
            total_task_count: instance.tasks.len(),
            completed_task_count: instance
                .tasks
                .values()
                .filter(|task| task.status == TaskStatus::Completed)
                .count(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierGenerationState {
    pub verifier_task_id: String,
    pub rerun_start_task_id: String,
    pub latest_generation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_generation: Option<u32>,
    #[serde(default)]
    pub feedback_history: Vec<VerifierFeedbackEntry>,
    pub status: VerifierStateStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierFeedbackEntry {
    pub generation_index: u32,
    pub feedback: String,
    pub verifier_output: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerifierStateStatus {
    Running,
    Accepted,
    ExhaustedAccepted,
    ExhaustedFailed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBinding {
    pub target_task_id: String,
    pub source_task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub id: String,
    /// Example trigger input for clients; never used as a runtime default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example_input: Option<serde_json::Value>,
    #[serde(default)]
    pub description: String,
    pub tasks: Vec<TaskDef>,
    #[serde(deserialize_with = "deserialize_data_bindings")]
    pub data_bindings: Vec<DataBinding>,
}

/// Expand authoring shorthand into the individual edges used throughout the runtime.
fn deserialize_data_bindings<'de, D>(deserializer: D) -> Result<Vec<DataBinding>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct Binding {
        source_task_id: String,
        target_task_id: serde_json::Value,
    }

    let bindings = Vec::<Binding>::deserialize(deserializer)?;
    let mut expanded = Vec::new();
    for binding in bindings {
        let targets = match binding.target_task_id {
            serde_json::Value::String(target) => vec![target],
            serde_json::Value::Array(targets) if !targets.is_empty() => targets
                .into_iter()
                .enumerate()
                .map(|(index, target)| match target {
                    serde_json::Value::String(target) => Ok(target),
                    _ => Err(serde::de::Error::custom(format!(
                        "data binding target_task_id[{index}] must be a task ID string"
                    ))),
                })
                .collect::<Result<Vec<_>, D::Error>>()?,
            _ => {
                return Err(serde::de::Error::custom(
                    "data binding target_task_id must be a task ID string or a non-empty list of task ID strings",
                ));
            }
        };
        expanded.extend(targets.into_iter().map(|target_task_id| DataBinding {
            source_task_id: binding.source_task_id.clone(),
            target_task_id,
        }));
    }
    Ok(expanded)
}

/// Compact workflow definition metadata for discovery and selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowDefSummary {
    pub id: String,
    pub description: String,
    pub created_at_epoch_ms: u64,
    pub last_invoked_at_epoch_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct StartupWorkflowDiscovery {
    pub runnable: Vec<WorkflowInfo>,
    #[allow(dead_code)]
    pub blocked: Vec<WorkflowInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_bindings_expand_scalar_list_and_mixed_syntax_in_order() {
        let yaml = "id: example\ntasks: []\ndata_bindings:\n  - source_task_id: a\n    target_task_id: [b, c]\n  - source_task_id: b\n    target_task_id: c\n  - source_task_id: c\n    target_task_id: [d]\n";
        let expected = serde_json::json!([
            {"source_task_id": "a", "target_task_id": "b"},
            {"source_task_id": "a", "target_task_id": "c"},
            {"source_task_id": "b", "target_task_id": "c"},
            {"source_task_id": "c", "target_task_id": "d"},
        ]);
        let from_yaml: WorkflowDef = serde_yaml::from_str(yaml).unwrap();
        let value: serde_json::Value = serde_yaml::from_str(yaml).unwrap();
        let from_json: WorkflowDef = serde_json::from_value(value).unwrap();
        for definition in [from_yaml, from_json] {
            let serialized = serde_json::to_value(definition).unwrap();
            assert_eq!(serialized["data_bindings"], expected);
            let round_trip: WorkflowDef = serde_json::from_value(serialized).unwrap();
            assert_eq!(
                serde_json::to_value(round_trip.data_bindings).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn data_bindings_reject_empty_lists_and_non_string_targets() {
        for target in [
            "[]",
            "null",
            "42",
            "true",
            "{}",
            "[b, 42]",
            "[b, true]",
            "[b, null]",
            "[b, {}]",
            "[b, [c]]",
        ] {
            let yaml = format!(
                "id: example\ntasks: []\ndata_bindings:\n  - source_task_id: a\n    target_task_id: {target}\n"
            );
            let error = serde_yaml::from_str::<WorkflowDef>(&yaml).unwrap_err();
            assert!(
                error.to_string().contains("target_task_id"),
                "{target}: {error}"
            );
            let value: serde_json::Value = serde_yaml::from_str(&yaml).unwrap();
            let error = serde_json::from_value::<WorkflowDef>(value).unwrap_err();
            assert!(
                error.to_string().contains("target_task_id"),
                "{target}: {error}"
            );
        }
    }

    #[test]
    fn workflow_example_input_round_trips_json_values_and_omits_absent_examples() {
        for example in [
            serde_json::json!({"nested": [false, 0, "text"]}),
            serde_json::json!([1, 2]),
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::json!(""),
            serde_json::Value::Null,
        ] {
            let value = serde_json::json!({
                "id": "example", "tasks": [], "data_bindings": [],
                "example_input": example,
            });
            let definition: WorkflowDef = serde_json::from_value(value).unwrap();
            let serialized = serde_json::to_value(&definition).unwrap();
            if example.is_null() {
                assert!(serialized.get("example_input").is_none());
            } else {
                assert_eq!(serialized["example_input"], example);
            }
        }
        let definition: WorkflowDef = serde_json::from_value(serde_json::json!({
            "id": "example", "tasks": [], "data_bindings": [],
        }))
        .unwrap();
        assert!(definition.example_input.is_none());
        assert!(
            serde_json::to_value(definition)
                .unwrap()
                .get("example_input")
                .is_none()
        );
    }

    #[test]
    fn workflow_info_omits_storage_namespace_from_public_serialization() {
        let info = WorkflowInfo {
            namespace: crate::core::namespace::test_namespace(),
            id: "workflow-1".to_string(),
            workflow_def_id: "workflow".to_string(),
            created_at_epoch_ms: Some(1),
            modified_at_epoch_ms: 2,
            completed_at_epoch_ms: None,
            status: WorkflowStatus::Running,
            total_task_count: 1,
            completed_task_count: 0,
        };

        let value = serde_json::to_value(info).unwrap();

        assert!(value.get("namespace").is_none());
    }
}
