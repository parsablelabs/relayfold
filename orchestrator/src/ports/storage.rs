use crate::core::function::models::{FunctionDef, FunctionDefSummary};
use crate::core::namespace::Namespace;
use crate::core::task::{TaskInputMapping, TaskSatisfactionStatus};
use crate::core::verifier::VerifierAttemptMetadata;
use crate::core::workflow::events::WorkflowEventRecord;
use crate::core::workflow::models::{
    WorkflowDef, WorkflowDefSummary, WorkflowInfo, WorkflowInstance, WorkflowStatus,
};
use async_trait::async_trait;
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowVersionConflict {
    pub workflow_instance_id: String,
    pub expected_version: u64,
    pub actual_version: u64,
}

impl Display for WorkflowVersionConflict {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "workflow instance {} version conflict: expected {}, actual {}",
            self.workflow_instance_id, self.expected_version, self.actual_version
        )
    }
}

impl Error for WorkflowVersionConflict {}

#[derive(Debug)]
pub enum StorageError {
    WorkflowVersionConflict(WorkflowVersionConflict),
    Backend(anyhow::Error),
}

pub type StorageResult<T> = Result<T, StorageError>;

impl Display for StorageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkflowVersionConflict(conflict) => conflict.fmt(f),
            Self::Backend(error) => write!(f, "storage backend error: {error}"),
        }
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WorkflowVersionConflict(conflict) => Some(conflict),
            Self::Backend(error) => error.source(),
        }
    }
}

impl From<WorkflowVersionConflict> for StorageError {
    fn from(value: WorkflowVersionConflict) -> Self {
        Self::WorkflowVersionConflict(value)
    }
}

impl From<anyhow::Error> for StorageError {
    fn from(value: anyhow::Error) -> Self {
        Self::Backend(value)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(value: serde_json::Error) -> Self {
        Self::Backend(value.into())
    }
}

impl From<sqlx::Error> for StorageError {
    fn from(value: sqlx::Error) -> Self {
        Self::Backend(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowInstanceFilter {
    Statuses(Vec<WorkflowStatus>),
    WorkflowDefId(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest<C> {
    pub limit: usize,
    pub cursor: Option<C>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowInfoCursor {
    pub namespace: Namespace,
    pub modified_at_epoch_ms: u64,
    pub workflow_instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T, C> {
    pub items: Vec<T>,
    pub next_cursor: Option<C>,
}

pub type WorkflowInfoPageRequest = PageRequest<WorkflowInfoCursor>;
pub type WorkflowInfoPage = Page<WorkflowInfo, WorkflowInfoCursor>;
pub type WorkflowEventPageRequest = PageRequest<u64>;
pub type WorkflowEventPage = Page<WorkflowEventRecord, u64>;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskResult {
    Success {
        input: Vec<serde_json::Value>,
        output: serde_json::Value,
        metadata: Option<TaskResultMetadata>,
    },
    Failure {
        input: Vec<serde_json::Value>,
        error_message: String,
        metadata: Option<TaskResultMetadata>,
    },
    Skipped {
        input: Vec<serde_json::Value>,
        metadata: Option<TaskResultMetadata>,
    },
    Pending {
        input: Vec<serde_json::Value>,
        metadata: Option<TaskResultMetadata>,
    },
    Running {
        input: Vec<serde_json::Value>,
        metadata: Option<TaskResultMetadata>,
    },
    InputNeeded {
        input: Vec<serde_json::Value>,
        input_request: String,
        metadata: Option<TaskResultMetadata>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowTaskResult {
    pub task_attempt_id: String,
    pub result: TaskResult,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskResultMetadata {
    pub early_exit: bool,
    pub task_def_id: String,
    pub task_attempt_id: String,
    pub satisfaction: TaskSatisfactionStatus,
    pub input_mapping: Vec<TaskInputMapping>,
    pub generation_index: u32,
    pub verifier_metadata: Option<VerifierAttemptMetadata>,
}

fn serialize_metadata<S>(
    map: &mut S,
    metadata: &TaskResultMetadata,
) -> Result<(), <S as SerializeMap>::Error>
where
    S: SerializeMap,
{
    map.serialize_entry("early_exit", &metadata.early_exit)?;
    map.serialize_entry("task_def_id", &metadata.task_def_id)?;
    map.serialize_entry("task_attempt_id", &metadata.task_attempt_id)?;
    map.serialize_entry("satisfaction", &metadata.satisfaction)?;
    if !metadata.input_mapping.is_empty() {
        map.serialize_entry("input_mapping", &metadata.input_mapping)?;
    }
    map.serialize_entry("generation_index", &metadata.generation_index)?;
    if let Some(verifier_metadata) = &metadata.verifier_metadata {
        map.serialize_entry("verifier_metadata", verifier_metadata)?;
    }
    Ok(())
}

// TODO: consider moving this into API layer
impl Serialize for TaskResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            TaskResult::Success {
                input,
                output,
                metadata,
            } => {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("status", "success")?;
                map.serialize_entry("input", input)?;
                map.serialize_entry("output", output)?;
                if let Some(metadata) = metadata {
                    serialize_metadata(&mut map, metadata)?;
                }
                map.end()
            }
            TaskResult::Failure {
                input,
                error_message,
                metadata,
            } => {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("status", "failure")?;
                map.serialize_entry("input", input)?;
                map.serialize_entry("error_message", error_message)?;
                if let Some(metadata) = metadata {
                    serialize_metadata(&mut map, metadata)?;
                }
                map.end()
            }
            TaskResult::Skipped { input, metadata } => {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("status", "skipped")?;
                map.serialize_entry("input", input)?;
                if let Some(metadata) = metadata {
                    serialize_metadata(&mut map, metadata)?;
                }
                map.end()
            }
            TaskResult::Pending { input, metadata } => {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("status", "pending")?;
                map.serialize_entry("input", input)?;
                if let Some(metadata) = metadata {
                    serialize_metadata(&mut map, metadata)?;
                }
                map.end()
            }
            TaskResult::Running { input, metadata } => {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("status", "running")?;
                map.serialize_entry("input", input)?;
                if let Some(metadata) = metadata {
                    serialize_metadata(&mut map, metadata)?;
                }
                map.end()
            }
            TaskResult::InputNeeded {
                input,
                input_request,
                metadata,
            } => {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("status", "input_needed")?;
                map.serialize_entry("input", input)?;
                map.serialize_entry("input_request", input_request)?;
                if let Some(metadata) = metadata {
                    serialize_metadata(&mut map, metadata)?;
                }
                map.end()
            }
        }
    }
}

#[async_trait]
/// Persistence boundary for workflow definitions, function definitions, and workflow execution
/// state.
///
/// Point reads return authoritative committed state. Collection and history reads may be backed by
/// asynchronous projections, so callers must tolerate recently committed changes being absent or
/// stale and must re-read an entity by ID before making a state transition. List pagination is not
/// a snapshot: concurrent writes may move entries between pages. Except for internal recovery and
/// reconciliation [`StoragePort::list_workflow_info`] calls with `None`, every operation requires
/// the namespace that owns the resource.
pub trait StoragePort {
    /// Returns the authoritative committed workflow definition for `id`, or `None` when it does not
    /// exist.
    ///
    /// A successful save completed before this call must be visible to this point read.
    async fn get_workflow_def(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> StorageResult<Option<WorkflowDef>>;

    /// Returns lightweight summaries of all workflow definitions, ordered by creation time
    /// descending and then definition ID descending.
    ///
    /// This discovery read may lag recent definition saves and workflow invocations. In particular,
    /// `last_invoked_at_epoch_ms` is projection data and is not an authoritative workflow-instance
    /// existence check.
    async fn list_workflow_def(
        &self,
        namespace: &Namespace,
    ) -> StorageResult<Vec<WorkflowDefSummary>>;

    /// Returns the authoritative committed function definition for `id`, or `None` when it does not
    /// exist.
    ///
    /// A successful save completed before this call must be visible to this point read.
    async fn get_function_def(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> StorageResult<Option<FunctionDef>>;

    /// Lists lightweight function metadata in this namespace, ordered by ID ascending.
    async fn list_function_def(
        &self,
        namespace: &Namespace,
    ) -> StorageResult<Vec<FunctionDefSummary>>;

    /// Returns the latest committed workflow-instance snapshot for `id`, or `None` when it does not
    /// exist.
    ///
    /// This is the authoritative read for workflow state and must provide read-after-write
    /// visibility. Callers must use the returned version when attempting a subsequent transition.
    async fn get_workflow_instance(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> StorageResult<Option<WorkflowInstance>>;

    /// Returns a bounded page of lightweight workflow-instance summaries.
    ///
    /// `Some(namespace)` is required for normal service and resource operations. `None` is
    /// reserved for internal startup recovery and lost-host reconciliation across namespaces; in
    /// that mode every returned [`WorkflowInfo`] and pagination cursor retains its owning
    /// namespace.
    ///
    /// Filters of different kinds are combined with logical AND. Status filters match any supplied
    /// status. Results are ordered by modification time descending and then workflow-instance ID
    /// descending. `next_cursor` is present only when another page may exist, and the cursor resumes
    /// strictly after the final returned item in that ordering.
    ///
    /// Summaries may lag recent transition commits and pagination is not snapshot-isolated. A caller
    /// making a state-changing decision must fetch the authoritative instance with
    /// [`StoragePort::get_workflow_instance`] before committing the transition.
    async fn list_workflow_info(
        &self,
        namespace: Option<&Namespace>,
        page: WorkflowInfoPageRequest,
        filters: Vec<WorkflowInstanceFilter>,
    ) -> StorageResult<WorkflowInfoPage>;

    /// Returns a bounded page of persisted events for one workflow instance in ascending sequence
    /// order.
    ///
    /// The optional cursor is an exclusive event-sequence cursor. `next_cursor` is present only when
    /// another page may exist. Event discovery may lag a successful transition commit, but returned
    /// events must retain their committed order and contents.
    async fn list_workflow_instance_events(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        page: WorkflowEventPageRequest,
    ) -> StorageResult<WorkflowEventPage>;

    /// Creates or replaces a workflow definition and makes it available to authoritative point
    /// reads before returning.
    ///
    /// This storage operation does not enforce the business rule that definitions with existing
    /// workflow instances cannot be replaced; callers are currently responsible for that rule.
    async fn save_workflow_def(&self, namespace: &Namespace, def: WorkflowDef)
    -> StorageResult<()>;

    /// Creates or replaces a function definition and makes it available to authoritative point
    /// reads before returning.
    async fn save_function_def(&self, namespace: &Namespace, def: FunctionDef)
    -> StorageResult<()>;

    /// Removes the function definition identified by `id` from authoritative reads.
    ///
    /// Returns `true` when a stored definition was removed and `false` when it did not exist.
    /// Backends may retain unreachable immutable payload data after removing its authoritative
    /// metadata.
    async fn delete_function_def(&self, namespace: &Namespace, id: &str) -> StorageResult<bool>;

    /// Atomically commits an ordered event batch and its already-reduced workflow-instance snapshot.
    ///
    /// `expected_version` is the version from the authoritative snapshot on which the transition was
    /// based. The supplied `instance.version` must equal `expected_version + events.len()`, and the
    /// events must be in the same order in which core reduced them. Storage persists, as one logical
    /// transition, the new authoritative snapshot, ordered event records, changed task state, and
    /// summary data derived from the snapshot.
    ///
    /// If the authoritative version differs from `expected_version`, the method returns
    /// [`StorageError::WorkflowVersionConflict`] and none of the supplied data becomes authoritative.
    /// On success, a subsequent [`StoragePort::get_workflow_instance`] must return the new snapshot;
    /// collection and history reads may observe it later.
    async fn save_workflow_instance(
        &self,
        namespace: &Namespace,
        expected_version: u64,
        events: Vec<WorkflowEventRecord>,
        instance: WorkflowInstance,
    ) -> StorageResult<()>;
}
