use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::{Mutex, RwLock};

use crate::core::function::models::FunctionDef;
use crate::core::namespace::Namespace;
use crate::core::util::unix_timestamp_ms;
use crate::core::workflow::events::WorkflowEventRecord;
use crate::core::workflow::models::{
    WorkflowDef, WorkflowDefSummary, WorkflowInfo, WorkflowInstance, WorkflowStatus,
};
use crate::ports::storage::{
    StoragePort, StorageResult, WorkflowEventPage, WorkflowEventPageRequest, WorkflowInfoCursor,
    WorkflowInfoPage, WorkflowInfoPageRequest, WorkflowInstanceFilter, WorkflowVersionConflict,
};

pub struct MemoryStorage {
    workflow_defs: RwLock<HashMap<ResourceKey, StoredWorkflowDef>>,
    function_defs: RwLock<HashMap<ResourceKey, FunctionDef>>,
    workflow_instances: RwLock<HashMap<ResourceKey, WorkflowInstance>>,
    workflow_instance_events: RwLock<HashMap<ResourceKey, Vec<WorkflowEventRecord>>>,
    workflow_infos: RwLock<HashMap<ResourceKey, WorkflowInfo>>,
    commit_lock: Mutex<()>,
}

type ResourceKey = (Namespace, String);

struct StoredWorkflowDef {
    definition: WorkflowDef,
    created_at_epoch_ms: u64,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            workflow_defs: RwLock::new(HashMap::new()),
            function_defs: RwLock::new(HashMap::new()),
            workflow_instances: RwLock::new(HashMap::new()),
            workflow_instance_events: RwLock::new(HashMap::new()),
            workflow_infos: RwLock::new(HashMap::new()),
            commit_lock: Mutex::new(()),
        }
    }
}

/// This implementation is intended for testing and development purposes only.
/// It is not designed for high-performance or persistent storage.
/// and should not be used in production environments.
#[async_trait]
impl StoragePort for MemoryStorage {
    async fn save_workflow_def(
        &self,
        namespace: &Namespace,
        def: WorkflowDef,
    ) -> StorageResult<()> {
        let mut map = self.workflow_defs.write().await;
        let key = resource_key(namespace, &def.id);
        let created_at_epoch_ms = map
            .get(&key)
            .map(|stored| stored.created_at_epoch_ms)
            .unwrap_or(unix_timestamp_ms()?);
        map.insert(
            key,
            StoredWorkflowDef {
                definition: def,
                created_at_epoch_ms,
            },
        );
        Ok(())
    }

    async fn get_workflow_def(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> StorageResult<Option<WorkflowDef>> {
        let map = self.workflow_defs.read().await;
        Ok(map
            .get(&resource_key(namespace, id))
            .map(|stored| stored.definition.clone()))
    }

    async fn list_workflow_def(
        &self,
        namespace: &Namespace,
    ) -> StorageResult<Vec<WorkflowDefSummary>> {
        let _commit_guard = self.commit_lock.lock().await;
        let definitions = self.workflow_defs.read().await;
        let infos = self.workflow_infos.read().await;
        let mut summaries = definitions
            .iter()
            .filter(|((definition_namespace, _), _)| definition_namespace == namespace)
            .map(|(_, stored)| WorkflowDefSummary {
                id: stored.definition.id.clone(),
                description: stored.definition.description.clone(),
                created_at_epoch_ms: stored.created_at_epoch_ms,
                last_invoked_at_epoch_ms: infos
                    .values()
                    .filter(|info| &info.namespace == namespace)
                    .filter(|info| info.workflow_def_id == stored.definition.id)
                    .filter_map(|info| info.created_at_epoch_ms)
                    .max(),
            })
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            right
                .created_at_epoch_ms
                .cmp(&left.created_at_epoch_ms)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(summaries)
    }

    async fn save_function_def(
        &self,
        namespace: &Namespace,
        def: FunctionDef,
    ) -> StorageResult<()> {
        let mut map = self.function_defs.write().await;
        map.insert(resource_key(namespace, &def.id), def);
        Ok(())
    }

    async fn get_function_def(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> StorageResult<Option<FunctionDef>> {
        let map = self.function_defs.read().await;
        Ok(map.get(&resource_key(namespace, id)).cloned())
    }

    async fn delete_function_def(&self, namespace: &Namespace, id: &str) -> StorageResult<bool> {
        let mut map = self.function_defs.write().await;
        Ok(map.remove(&resource_key(namespace, id)).is_some())
    }

    async fn get_workflow_instance(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> StorageResult<Option<WorkflowInstance>> {
        let _commit_guard = self.commit_lock.lock().await;
        let map = self.workflow_instances.read().await;
        Ok(map.get(&resource_key(namespace, id)).cloned())
    }

    async fn list_workflow_instance_events(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        page: WorkflowEventPageRequest,
    ) -> StorageResult<WorkflowEventPage> {
        let _commit_guard = self.commit_lock.lock().await;
        let map = self.workflow_instance_events.read().await;
        let all = map
            .get(&resource_key(namespace, workflow_instance_id))
            .map(Vec::as_slice)
            .unwrap_or_default();
        if page.limit == 0 {
            return Ok(WorkflowEventPage {
                items: vec![],
                next_cursor: None,
            });
        }
        let start = page.cursor.unwrap_or(0) as usize;
        let selected = all
            .iter()
            .skip(start)
            .take(page.limit + 1)
            .cloned()
            .collect::<Vec<_>>();
        let has_more = selected.len() > page.limit;
        let events = selected.into_iter().take(page.limit).collect::<Vec<_>>();
        let next_cursor = has_more.then_some((start + events.len()) as u64);
        Ok(WorkflowEventPage {
            items: events,
            next_cursor,
        })
    }

    async fn list_workflow_info(
        &self,
        namespace: Option<&Namespace>,
        page: WorkflowInfoPageRequest,
        filters: Vec<WorkflowInstanceFilter>,
    ) -> StorageResult<WorkflowInfoPage> {
        let _commit_guard = self.commit_lock.lock().await;
        let map = self.workflow_infos.read().await;
        let mut workflows: Vec<WorkflowInfo> = map
            .values()
            .filter(|info| namespace.is_none_or(|namespace| &info.namespace == namespace))
            .filter(|info| {
                filters
                    .iter()
                    .all(|filter| workflow_info_matches(info, filter))
            })
            .cloned()
            .collect();

        workflows.sort_by(|left, right| {
            right
                .modified_at_epoch_ms
                .cmp(&left.modified_at_epoch_ms)
                .then_with(|| right.id.cmp(&left.id))
                .then_with(|| right.namespace.cmp(&left.namespace))
        });

        if let Some(cursor) = &page.cursor {
            workflows.retain(|info| is_after_cursor(info, cursor));
        }

        let has_more = workflows.len() > page.limit;
        workflows.truncate(page.limit);
        let next_cursor = has_more
            .then(|| workflows.last())
            .flatten()
            .map(workflow_info_cursor);

        Ok(WorkflowInfoPage {
            items: workflows,
            next_cursor,
        })
    }

    async fn save_workflow_instance(
        &self,
        namespace: &Namespace,
        expected_version: u64,
        events: Vec<WorkflowEventRecord>,
        instance: WorkflowInstance,
    ) -> StorageResult<()> {
        let _commit_guard = self.commit_lock.lock().await;
        let workflow_instance_id = instance.id.clone();
        let key = resource_key(namespace, &workflow_instance_id);
        let actual_version = self
            .workflow_instances
            .read()
            .await
            .get(&key)
            .map(|instance| instance.version)
            .unwrap_or(0);

        if actual_version != expected_version {
            return Err(WorkflowVersionConflict {
                workflow_instance_id,
                expected_version,
                actual_version,
            }
            .into());
        }

        let created_from_events_at_epoch_ms = events
            .first()
            .map(|event| event.created_time)
            .unwrap_or(unix_timestamp_ms()?);
        let modified_at_epoch_ms = events
            .last()
            .map(|event| event.created_time)
            .unwrap_or(created_from_events_at_epoch_ms);

        let mut infos = self.workflow_infos.write().await;
        let existing_info = infos.get(&key);
        let created_at_epoch_ms = existing_info
            .and_then(|info| info.created_at_epoch_ms)
            .or(Some(created_from_events_at_epoch_ms));
        let completed_at_epoch_ms = existing_info
            .and_then(|info| info.completed_at_epoch_ms)
            .or_else(|| workflow_completed_at(&instance, modified_at_epoch_ms));
        let info = WorkflowInfo::from_instance_with_timestamps(
            namespace.clone(),
            &instance,
            created_at_epoch_ms,
            modified_at_epoch_ms,
            completed_at_epoch_ms,
        );
        infos.insert(key.clone(), info);
        drop(infos);

        let mut events_map = self.workflow_instance_events.write().await;
        events_map.entry(key.clone()).or_default().extend(events);
        drop(events_map);

        let mut instances = self.workflow_instances.write().await;
        instances.insert(key, instance);
        Ok(())
    }
}

fn resource_key(namespace: &Namespace, id: &str) -> ResourceKey {
    (namespace.clone(), id.to_string())
}

fn workflow_info_matches(info: &WorkflowInfo, filter: &WorkflowInstanceFilter) -> bool {
    match filter {
        WorkflowInstanceFilter::Statuses(statuses) => statuses.contains(&info.status),
        WorkflowInstanceFilter::WorkflowDefId(workflow_def_id) => {
            info.workflow_def_id == workflow_def_id.as_str()
        }
    }
}

fn is_after_cursor(info: &WorkflowInfo, cursor: &WorkflowInfoCursor) -> bool {
    info.modified_at_epoch_ms < cursor.modified_at_epoch_ms
        || (info.modified_at_epoch_ms == cursor.modified_at_epoch_ms
            && (info.id.as_str() < cursor.workflow_instance_id.as_str()
                || (info.id.as_str() == cursor.workflow_instance_id.as_str()
                    && info.namespace < cursor.namespace)))
}

fn workflow_info_cursor(info: &WorkflowInfo) -> WorkflowInfoCursor {
    WorkflowInfoCursor {
        namespace: info.namespace.clone(),
        modified_at_epoch_ms: info.modified_at_epoch_ms,
        workflow_instance_id: info.id.clone(),
    }
}

fn workflow_completed_at(instance: &WorkflowInstance, modified_at_epoch_ms: u64) -> Option<u64> {
    matches!(
        instance.status,
        WorkflowStatus::Completed | WorkflowStatus::Failed
    )
    .then_some(modified_at_epoch_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::function::models::FunctionDef;
    use crate::core::namespace::Namespace;
    use crate::core::task::{TaskInstance, TaskSatisfactionStatus, TaskStatus};
    use crate::core::workflow::events::{WorkflowEventRecord, WorkflowInstanceEvent};
    use crate::ports::storage::WorkflowInfoPageRequest;
    use std::collections::HashMap;

    fn namespace(value: &str) -> Namespace {
        Namespace::new(value).unwrap()
    }

    fn first_namespace() -> Namespace {
        namespace("550e8400-e29b-41d4-a716-446655440000")
    }

    fn second_namespace() -> Namespace {
        namespace("550e8400-e29b-41d4-a716-446655440001")
    }

    fn instance(id: &str, status: WorkflowStatus) -> WorkflowInstance {
        instance_for_def(id, "wf", status)
    }

    fn instance_for_def(
        id: &str,
        workflow_def_id: &str,
        status: WorkflowStatus,
    ) -> WorkflowInstance {
        WorkflowInstance {
            id: id.to_string(),
            workflow_def_id: workflow_def_id.to_string(),
            version: 0,
            status,
            trigger_input: None,
            pinned_worker_host: None,
            tasks: HashMap::new(),
            verifier_states: HashMap::new(),
        }
    }

    fn list_page() -> WorkflowInfoPageRequest {
        WorkflowInfoPageRequest {
            limit: 100,
            cursor: None,
        }
    }

    fn page_request(limit: usize, cursor: Option<WorkflowInfoCursor>) -> WorkflowInfoPageRequest {
        WorkflowInfoPageRequest { limit, cursor }
    }

    fn event_record(created_time: u64) -> WorkflowEventRecord {
        WorkflowEventRecord {
            created_time,
            event: WorkflowInstanceEvent::WorkflowStatusChanged {
                status: WorkflowStatus::Running,
            },
        }
    }

    fn workflow_def(description: &str) -> WorkflowDef {
        WorkflowDef {
            sandbox: None,
            id: "shared-def".to_string(),
            description: description.to_string(),
            tasks: vec![],
            data_bindings: vec![],
        }
    }

    fn function_def(code: &str) -> FunctionDef {
        FunctionDef {
            id: "shared-function".to_string(),
            dependencies: vec![],
            code: code.to_string(),
        }
    }

    fn task(output: &str) -> TaskInstance {
        TaskInstance {
            task_def_id: "shared-task".to_string(),
            status: TaskStatus::Completed,
            satisfaction_status: TaskSatisfactionStatus::Pending,
            human_input: None,
            input_data: vec![],
            input_mapping: vec![],
            output_data: Some(serde_json::json!(output)),
            generation_index: 1,
            verifier_metadata: None,
        }
    }

    #[tokio::test]
    async fn commits_events_snapshot_and_summary_together() {
        let storage = MemoryStorage::new();
        let mut instance = instance("wf-1", WorkflowStatus::Pending);
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance.clone(),
            )
            .await
            .unwrap();

        instance.status = WorkflowStatus::Completed;
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![WorkflowEventRecord {
                    created_time: 42,
                    event: WorkflowInstanceEvent::WorkflowStatusChanged {
                        status: WorkflowStatus::Completed,
                    },
                }],
                instance.clone(),
            )
            .await
            .unwrap();

        let saved = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "wf-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, WorkflowStatus::Completed);
        let records = storage
            .list_workflow_instance_events(
                &crate::core::namespace::test_namespace(),
                "wf-1",
                WorkflowEventPageRequest {
                    limit: 100,
                    cursor: None,
                },
            )
            .await
            .unwrap()
            .items;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].created_time, 42);
        let infos = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![WorkflowInstanceFilter::Statuses(vec![
                    WorkflowStatus::Completed,
                ])],
            )
            .await
            .unwrap()
            .items;
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].id, "wf-1");
        assert!(infos[0].created_at_epoch_ms.is_some());
        assert!(infos[0].modified_at_epoch_ms >= 42);
        assert_eq!(infos[0].completed_at_epoch_ms, Some(42));
    }

    #[tokio::test]
    async fn rejects_workflow_commit_when_expected_version_is_stale() {
        let storage = MemoryStorage::new();
        let mut instance = instance("wf-1", WorkflowStatus::Pending);
        instance.version = 1;
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(1000)],
                instance.clone(),
            )
            .await
            .unwrap();

        instance.status = WorkflowStatus::Completed;
        instance.version = 2;
        let error = storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(2000)],
                instance,
            )
            .await
            .unwrap_err();

        let crate::ports::storage::StorageError::WorkflowVersionConflict(conflict) = error else {
            panic!("expected workflow version conflict");
        };
        assert_eq!(conflict.workflow_instance_id, "wf-1");
        assert_eq!(conflict.expected_version, 0);
        assert_eq!(conflict.actual_version, 1);

        let saved = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "wf-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, WorkflowStatus::Pending);
        assert_eq!(saved.version, 1);
        assert_eq!(
            storage
                .list_workflow_instance_events(
                    &crate::core::namespace::test_namespace(),
                    "wf-1",
                    WorkflowEventPageRequest {
                        limit: 100,
                        cursor: None,
                    },
                )
                .await
                .unwrap()
                .items
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn maintains_summary_from_snapshot() {
        let storage = MemoryStorage::new();
        let mut instance = instance("wf-1", WorkflowStatus::Running);
        instance.tasks.insert(
            "task-a[1]".to_string(),
            TaskInstance {
                task_def_id: "task-a".to_string(),
                status: TaskStatus::Completed,
                satisfaction_status: TaskSatisfactionStatus::Pending,
                human_input: None,
                input_data: vec![],
                input_mapping: vec![],
                output_data: None,
                generation_index: 1,
                verifier_metadata: None,
            },
        );
        instance.tasks.insert(
            "task-b[1]".to_string(),
            TaskInstance {
                task_def_id: "task-b".to_string(),
                status: TaskStatus::Pending,
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
                instance.clone(),
            )
            .await
            .unwrap();

        let infos = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![],
            )
            .await
            .unwrap()
            .items;
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].total_task_count, 2);
        assert_eq!(infos[0].completed_task_count, 1);
    }

    #[tokio::test]
    async fn summary_timestamps_track_creation_modification_and_completion() {
        let storage = MemoryStorage::new();
        let mut instance = instance("wf-1", WorkflowStatus::Running);
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(1000)],
                instance.clone(),
            )
            .await
            .unwrap();

        instance.status = WorkflowStatus::Completed;
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(2000)],
                instance,
            )
            .await
            .unwrap();

        let infos = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![],
            )
            .await
            .unwrap()
            .items;
        assert_eq!(infos[0].created_at_epoch_ms, Some(1000));
        assert_eq!(infos[0].modified_at_epoch_ms, 2000);
        assert_eq!(infos[0].completed_at_epoch_ms, Some(2000));
    }

    #[tokio::test]
    async fn summary_creation_uses_first_event_and_modification_uses_last_event() {
        let storage = MemoryStorage::new();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(1000), event_record(1500)],
                instance("wf-1", WorkflowStatus::Running),
            )
            .await
            .unwrap();

        let infos = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![],
            )
            .await
            .unwrap()
            .items;
        assert_eq!(infos[0].created_at_epoch_ms, Some(1000));
        assert_eq!(infos[0].modified_at_epoch_ms, 1500);
    }

    #[tokio::test]
    async fn list_workflow_info_sorts_by_modified_time_desc_then_id_desc() {
        let storage = MemoryStorage::new();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(1000)],
                instance("older", WorkflowStatus::Pending),
            )
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(2000)],
                instance("same-a", WorkflowStatus::Pending),
            )
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(2000)],
                instance("same-b", WorkflowStatus::Pending),
            )
            .await
            .unwrap();

        let infos = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![],
            )
            .await
            .unwrap()
            .items;

        let ids: Vec<&str> = infos.iter().map(|info| info.id.as_str()).collect();
        assert_eq!(ids, vec!["same-b", "same-a", "older"]);
    }

    #[tokio::test]
    async fn list_workflow_info_paginates_after_cursor() {
        let storage = MemoryStorage::new();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(3000)],
                instance("newest", WorkflowStatus::Pending),
            )
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(2000)],
                instance("middle", WorkflowStatus::Pending),
            )
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(1000)],
                instance("oldest", WorkflowStatus::Pending),
            )
            .await
            .unwrap();

        let first_page = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                page_request(1, None),
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(first_page.items.len(), 1);
        assert_eq!(first_page.items[0].id, "newest");
        assert_eq!(
            first_page.next_cursor,
            Some(WorkflowInfoCursor {
                namespace: crate::core::namespace::test_namespace(),
                modified_at_epoch_ms: 3000,
                workflow_instance_id: "newest".to_string(),
            })
        );

        let second_page = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                page_request(2, first_page.next_cursor),
                vec![],
            )
            .await
            .unwrap();
        let ids: Vec<&str> = second_page
            .items
            .iter()
            .map(|info| info.id.as_str())
            .collect();
        assert_eq!(ids, vec!["middle", "oldest"]);
        assert!(second_page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn filters_summary_queries() {
        let storage = MemoryStorage::new();
        let pending = instance("pending", WorkflowStatus::Pending);
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                pending.clone(),
            )
            .await
            .unwrap();
        let completed = instance("completed", WorkflowStatus::Completed);
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                completed.clone(),
            )
            .await
            .unwrap();

        let active = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![WorkflowInstanceFilter::Statuses(vec![
                    WorkflowStatus::Pending,
                    WorkflowStatus::Running,
                ])],
            )
            .await
            .unwrap()
            .items;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "pending");

        let completed = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![WorkflowInstanceFilter::Statuses(vec![
                    WorkflowStatus::Completed,
                ])],
            )
            .await
            .unwrap()
            .items;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].id, "completed");
    }

    #[tokio::test]
    async fn filters_summary_queries_by_workflow_def_id() {
        let storage = MemoryStorage::new();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance_for_def("workflow-1-instance", "workflow-1", WorkflowStatus::Pending),
            )
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance_for_def(
                    "workflow-2-instance",
                    "workflow-2",
                    WorkflowStatus::Completed,
                ),
            )
            .await
            .unwrap();

        let infos = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![WorkflowInstanceFilter::WorkflowDefId(
                    "workflow-2".to_string(),
                )],
            )
            .await
            .unwrap()
            .items;

        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].id, "workflow-2-instance");
    }

    #[tokio::test]
    async fn combines_summary_query_filters_with_and_semantics() {
        let storage = MemoryStorage::new();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance_for_def("workflow-1-pending", "workflow-1", WorkflowStatus::Pending),
            )
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance_for_def(
                    "workflow-1-completed",
                    "workflow-1",
                    WorkflowStatus::Completed,
                ),
            )
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance_for_def("workflow-2-pending", "workflow-2", WorkflowStatus::Pending),
            )
            .await
            .unwrap();

        let infos = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![
                    WorkflowInstanceFilter::WorkflowDefId("workflow-1".to_string()),
                    WorkflowInstanceFilter::Statuses(vec![WorkflowStatus::Pending]),
                ],
            )
            .await
            .unwrap()
            .items;

        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].id, "workflow-1-pending");
    }

    #[tokio::test]
    async fn empty_statuses_filter_matches_no_summaries() {
        let storage = MemoryStorage::new();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance_for_def("workflow-1-pending", "workflow-1", WorkflowStatus::Pending),
            )
            .await
            .unwrap();

        let infos = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![WorkflowInstanceFilter::Statuses(vec![])],
            )
            .await
            .unwrap()
            .items;

        assert!(infos.is_empty());
    }

    #[tokio::test]
    async fn summary_listing_does_not_return_full_workflow_state() {
        let storage = MemoryStorage::new();
        let mut instance = instance("wf-1", WorkflowStatus::Completed);
        instance.tasks.insert(
            "task-a[1]".to_string(),
            TaskInstance {
                task_def_id: "task-a".to_string(),
                status: TaskStatus::Completed,
                satisfaction_status: TaskSatisfactionStatus::Pending,
                human_input: None,
                input_data: vec![serde_json::json!({"secret": "input"})],
                input_mapping: vec![],
                output_data: Some(serde_json::json!({"secret": "output"})),
                generation_index: 1,
                verifier_metadata: None,
            },
        );

        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance.clone(),
            )
            .await
            .unwrap();

        let infos = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_page(),
                vec![],
            )
            .await
            .unwrap()
            .items;
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].total_task_count, 1);
        assert_eq!(infos[0].completed_task_count, 1);
    }

    #[tokio::test]
    async fn event_history_uses_ordered_cursor_pages() {
        let storage = MemoryStorage::new();
        let mut workflow = instance("wf-1", WorkflowStatus::Running);
        workflow.version = 3;
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![event_record(100), event_record(200), event_record(300)],
                workflow,
            )
            .await
            .unwrap();

        let first = storage
            .list_workflow_instance_events(
                &crate::core::namespace::test_namespace(),
                "wf-1",
                WorkflowEventPageRequest {
                    limit: 2,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            first
                .items
                .iter()
                .map(|event| event.created_time)
                .collect::<Vec<_>>(),
            vec![100, 200]
        );
        assert_eq!(first.next_cursor, Some(2));

        let second = storage
            .list_workflow_instance_events(
                &crate::core::namespace::test_namespace(),
                "wf-1",
                WorkflowEventPageRequest {
                    limit: 2,
                    cursor: first.next_cursor,
                },
            )
            .await
            .unwrap();
        assert_eq!(second.items[0].created_time, 300);
        assert_eq!(second.next_cursor, None);
    }

    #[tokio::test]
    async fn isolates_identical_definition_and_function_ids() {
        let storage = MemoryStorage::new();
        let first = first_namespace();
        let second = second_namespace();

        storage
            .save_workflow_def(&first, workflow_def("first"))
            .await
            .unwrap();
        storage
            .save_workflow_def(&second, workflow_def("second"))
            .await
            .unwrap();
        storage
            .save_function_def(&first, function_def("first"))
            .await
            .unwrap();
        storage
            .save_function_def(&second, function_def("second"))
            .await
            .unwrap();

        let mut invoked =
            instance_for_def("shared-instance", "shared-def", WorkflowStatus::Running);
        invoked.version = 1;
        storage
            .save_workflow_instance(&first, 0, vec![event_record(100)], invoked)
            .await
            .unwrap();

        assert_eq!(
            storage
                .get_workflow_def(&first, "shared-def")
                .await
                .unwrap()
                .unwrap()
                .description,
            "first"
        );
        assert_eq!(
            storage
                .get_workflow_def(&second, "shared-def")
                .await
                .unwrap()
                .unwrap()
                .description,
            "second"
        );
        assert!(
            storage
                .list_workflow_def(&first)
                .await
                .unwrap()
                .first()
                .unwrap()
                .last_invoked_at_epoch_ms
                .is_some()
        );
        assert_eq!(
            storage
                .list_workflow_def(&second)
                .await
                .unwrap()
                .first()
                .unwrap()
                .last_invoked_at_epoch_ms,
            None
        );
        assert_eq!(
            storage
                .get_function_def(&first, "shared-function")
                .await
                .unwrap()
                .unwrap()
                .code,
            "first"
        );
        assert!(
            storage
                .delete_function_def(&first, "shared-function")
                .await
                .unwrap()
        );
        assert!(
            storage
                .get_function_def(&first, "shared-function")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            storage
                .get_function_def(&second, "shared-function")
                .await
                .unwrap()
                .unwrap()
                .code,
            "second"
        );
    }

    #[tokio::test]
    async fn isolates_identical_workflow_snapshots_tasks_events_and_lists() {
        let storage = MemoryStorage::new();
        let first = first_namespace();
        let second = second_namespace();
        let mut first_instance = instance("shared-instance", WorkflowStatus::Running);
        first_instance.version = 1;
        first_instance
            .tasks
            .insert("shared-task[1]".to_string(), task("first"));
        let mut second_instance = instance("shared-instance", WorkflowStatus::Completed);
        second_instance.version = 1;
        second_instance
            .tasks
            .insert("shared-task[1]".to_string(), task("second"));

        storage
            .save_workflow_instance(&first, 0, vec![event_record(100)], first_instance)
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &second,
                0,
                vec![WorkflowEventRecord {
                    created_time: 100,
                    event: WorkflowInstanceEvent::WorkflowStatusChanged {
                        status: WorkflowStatus::Completed,
                    },
                }],
                second_instance,
            )
            .await
            .unwrap();

        let first_saved = storage
            .get_workflow_instance(&first, "shared-instance")
            .await
            .unwrap()
            .unwrap();
        let second_saved = storage
            .get_workflow_instance(&second, "shared-instance")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first_saved.status, WorkflowStatus::Running);
        assert_eq!(second_saved.status, WorkflowStatus::Completed);
        assert_eq!(
            first_saved.tasks["shared-task[1]"].output_data,
            Some(serde_json::json!("first"))
        );
        assert_eq!(
            second_saved.tasks["shared-task[1]"].output_data,
            Some(serde_json::json!("second"))
        );

        let first_events = storage
            .list_workflow_instance_events(
                &first,
                "shared-instance",
                WorkflowEventPageRequest {
                    limit: 10,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        let second_events = storage
            .list_workflow_instance_events(
                &second,
                "shared-instance",
                WorkflowEventPageRequest {
                    limit: 10,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            &first_events.items[0].event,
            WorkflowInstanceEvent::WorkflowStatusChanged {
                status: WorkflowStatus::Running
            }
        ));
        assert!(matches!(
            &second_events.items[0].event,
            WorkflowInstanceEvent::WorkflowStatusChanged {
                status: WorkflowStatus::Completed
            }
        ));

        let first_page = storage
            .list_workflow_info(Some(&first), list_page(), vec![])
            .await
            .unwrap();
        assert_eq!(first_page.items.len(), 1);
        assert_eq!(first_page.items[0].namespace, first);
        assert_eq!(first_page.items[0].status, WorkflowStatus::Running);

        let recovery_first_page = storage
            .list_workflow_info(None, page_request(1, None), vec![])
            .await
            .unwrap();
        assert_eq!(recovery_first_page.items.len(), 1);
        assert!(recovery_first_page.next_cursor.is_some());
        let recovery_second_page = storage
            .list_workflow_info(
                None,
                page_request(1, recovery_first_page.next_cursor),
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(recovery_second_page.items.len(), 1);
        assert_ne!(
            recovery_first_page.items[0].namespace,
            recovery_second_page.items[0].namespace
        );
        assert_eq!(
            recovery_first_page.items[0].id,
            recovery_second_page.items[0].id
        );
    }

    #[tokio::test]
    async fn evaluates_workflow_versions_within_each_namespace() {
        let storage = MemoryStorage::new();
        let first = first_namespace();
        let second = second_namespace();
        let mut first_instance = instance("shared-instance", WorkflowStatus::Running);
        first_instance.version = 1;
        let mut second_instance = instance("shared-instance", WorkflowStatus::Running);
        second_instance.version = 1;

        storage
            .save_workflow_instance(&first, 0, vec![event_record(100)], first_instance.clone())
            .await
            .unwrap();
        storage
            .save_workflow_instance(&second, 0, vec![event_record(200)], second_instance.clone())
            .await
            .unwrap();

        first_instance.version = 2;
        let error = storage
            .save_workflow_instance(&first, 0, vec![event_record(300)], first_instance)
            .await
            .unwrap_err();
        let crate::ports::storage::StorageError::WorkflowVersionConflict(conflict) = error else {
            panic!("expected workflow version conflict");
        };
        assert_eq!(conflict.actual_version, 1);

        second_instance.version = 2;
        second_instance.status = WorkflowStatus::Completed;
        storage
            .save_workflow_instance(&second, 1, vec![event_record(400)], second_instance)
            .await
            .unwrap();

        assert_eq!(
            storage
                .get_workflow_instance(&first, "shared-instance")
                .await
                .unwrap()
                .unwrap()
                .version,
            1
        );
        assert_eq!(
            storage
                .get_workflow_instance(&second, "shared-instance")
                .await
                .unwrap()
                .unwrap()
                .version,
            2
        );
        assert_eq!(
            storage
                .list_workflow_instance_events(
                    &second,
                    "shared-instance",
                    WorkflowEventPageRequest {
                        limit: 10,
                        cursor: None,
                    },
                )
                .await
                .unwrap()
                .items
                .len(),
            2
        );
    }
}
