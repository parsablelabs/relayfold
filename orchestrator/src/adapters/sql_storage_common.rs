use crate::core::task::TaskStatus;
use crate::core::workflow::models::{WorkflowInstance, WorkflowStatus};

pub(super) fn workflow_completed_at(
    instance: &WorkflowInstance,
    modified_at_epoch_ms: u64,
) -> Option<u64> {
    matches!(
        instance.status,
        WorkflowStatus::Completed | WorkflowStatus::Failed
    )
    .then_some(modified_at_epoch_ms)
}

pub(super) fn workflow_status_name(status: &WorkflowStatus) -> &'static str {
    match status {
        WorkflowStatus::Pending => "pending",
        WorkflowStatus::Running => "running",
        WorkflowStatus::Paused => "paused",
        WorkflowStatus::InputNeeded => "input_needed",
        WorkflowStatus::Completed => "completed",
        WorkflowStatus::Failed => "failed",
    }
}

pub(super) fn workflow_status_from_name(value: &str) -> anyhow::Result<WorkflowStatus> {
    match value {
        "pending" => Ok(WorkflowStatus::Pending),
        "running" => Ok(WorkflowStatus::Running),
        "paused" => Ok(WorkflowStatus::Paused),
        "input_needed" => Ok(WorkflowStatus::InputNeeded),
        "completed" => Ok(WorkflowStatus::Completed),
        "failed" => Ok(WorkflowStatus::Failed),
        _ => anyhow::bail!("unknown workflow status {value}"),
    }
}

pub(super) fn task_status_name(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::InputNeeded { .. } => "input_needed",
        TaskStatus::Completed => "completed",
        TaskStatus::Skipped => "skipped",
        TaskStatus::Failed => "failed",
    }
}

pub(super) fn optional_json_string<T>(value: &Option<T>) -> anyhow::Result<Option<String>>
where
    T: serde::Serialize,
{
    value
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(Into::into)
}

pub(super) fn optional_json<T>(value: Option<String>) -> anyhow::Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    value.as_deref().map(deserialize_json).transpose()
}

pub(super) fn deserialize_json<T>(value: &str) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    Ok(serde_json::from_str(value)?)
}

pub(super) fn i64_from_u64(value: u64) -> anyhow::Result<i64> {
    i64::try_from(value).map_err(Into::into)
}

pub(super) fn optional_i64_from_u64(value: Option<u64>) -> anyhow::Result<Option<i64>> {
    value.map(i64_from_u64).transpose()
}

pub(super) fn u64_from_i64(value: i64) -> anyhow::Result<u64> {
    u64::try_from(value).map_err(Into::into)
}

pub(super) fn u32_from_i64(value: i64) -> anyhow::Result<u32> {
    u32::try_from(value).map_err(Into::into)
}

pub(super) fn usize_from_i64(value: i64) -> anyhow::Result<usize> {
    usize::try_from(value).map_err(Into::into)
}

pub(super) fn i64_from_usize(value: usize) -> anyhow::Result<i64> {
    i64::try_from(value).map_err(Into::into)
}
