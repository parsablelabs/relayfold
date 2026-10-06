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
    #[serde(default)]
    pub description: String,
    pub tasks: Vec<TaskDef>,
    pub data_bindings: Vec<DataBinding>,
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
