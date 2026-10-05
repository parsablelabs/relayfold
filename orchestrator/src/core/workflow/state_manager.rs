use crate::core::namespace::Namespace;
use crate::core::util::unix_timestamp_ms;
use crate::core::workflow::events::{
    WorkflowEventRecord, WorkflowInstanceEvent, reduce_workflow_instance_events,
};
use crate::core::workflow::models::WorkflowInstance;
use crate::ports::storage::StoragePort;
use std::sync::Arc;

pub struct WorkflowStateManager {
    storage: Arc<dyn StoragePort + Send + Sync>,
}

// Coordinates workflow instance state changes: reduce domain events into a new
// snapshot, assign event metadata, advance the snapshot version, and ask storage
// to commit the events and snapshot atomically.
impl WorkflowStateManager {
    pub fn new(storage: Arc<dyn StoragePort + Send + Sync>) -> Self {
        Self { storage }
    }

    pub async fn commit_events(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        events: Vec<WorkflowInstanceEvent>,
    ) -> anyhow::Result<WorkflowInstance> {
        let current = self
            .storage
            .get_workflow_instance(namespace, workflow_instance_id)
            .await?;
        self.commit_events_for_current(namespace, current, events)
            .await
    }

    pub async fn commit_events_for_instance(
        &self,
        namespace: &Namespace,
        current: WorkflowInstance,
        events: Vec<WorkflowInstanceEvent>,
    ) -> anyhow::Result<WorkflowInstance> {
        self.commit_events_for_current(namespace, Some(current), events)
            .await
    }

    async fn commit_events_for_current(
        &self,
        namespace: &Namespace,
        current: Option<WorkflowInstance>,
        events: Vec<WorkflowInstanceEvent>,
    ) -> anyhow::Result<WorkflowInstance> {
        if events.is_empty() {
            anyhow::bail!("event batch must not be empty");
        }

        let expected_version = current
            .as_ref()
            .map(|instance| instance.version)
            .unwrap_or(0);
        let event_count = events.len() as u64;
        let updated = reduce_workflow_instance_events(current, &events)?;
        let updated = WorkflowInstance {
            version: expected_version + event_count,
            ..updated
        };
        let created_time = unix_timestamp_ms()?;
        let records = events
            .into_iter()
            .map(|event| WorkflowEventRecord {
                created_time,
                event,
            })
            .collect();

        self.storage
            .save_workflow_instance(namespace, expected_version, records, updated.clone())
            .await?;

        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::memory_storage::MemoryStorage;
    use crate::core::task::{TaskInstance, TaskSatisfactionStatus, TaskStatus};
    use crate::core::workflow::events::WorkflowInstanceEvent;
    use crate::core::workflow::models::{WorkflowInstance, WorkflowStatus};
    use crate::ports::storage::{
        StorageError, WorkflowEventPageRequest, WorkflowInfoPageRequest, WorkflowVersionConflict,
    };
    use std::collections::HashMap;

    fn list_all_page() -> WorkflowInfoPageRequest {
        WorkflowInfoPageRequest {
            limit: 100,
            cursor: None,
        }
    }

    fn workflow_instance(id: &str, status: WorkflowStatus) -> WorkflowInstance {
        WorkflowInstance {
            id: id.to_string(),
            workflow_def_id: "wf".to_string(),
            version: 0,
            status,
            trigger_input: None,
            pinned_worker_host: None,
            tasks: HashMap::new(),

            verifier_states: HashMap::new(),
        }
    }

    fn input_needed_instance(id: &str) -> WorkflowInstance {
        let mut instance = workflow_instance(id, WorkflowStatus::InputNeeded);
        instance.tasks.insert(
            "ask[1]".to_string(),
            TaskInstance {
                task_def_id: "ask".to_string(),
                status: TaskStatus::InputNeeded {
                    input_request: "need input".to_string(),
                },
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
            "independent[1]".to_string(),
            TaskInstance {
                task_def_id: "independent".to_string(),
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
        instance
    }

    fn workflow_version_conflict(error: &anyhow::Error) -> &WorkflowVersionConflict {
        match error.downcast_ref::<StorageError>() {
            Some(StorageError::WorkflowVersionConflict(conflict)) => conflict,
            _ => panic!("expected workflow version conflict"),
        }
    }

    #[tokio::test]
    async fn state_manager_rejects_empty_batches() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = WorkflowStateManager::new(storage);

        let result = manager
            .commit_events(&crate::core::namespace::test_namespace(), "wf-1", vec![])
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn state_manager_persists_events_snapshot_and_summary() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = WorkflowStateManager::new(storage.clone());
        let instance = workflow_instance("wf-1", WorkflowStatus::Pending);

        manager
            .commit_events(
                &crate::core::namespace::test_namespace(),
                "wf-1",
                vec![WorkflowInstanceEvent::WorkflowCreated {
                    instance: instance.clone(),
                }],
            )
            .await
            .unwrap();

        let saved = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "wf-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.workflow_def_id, "wf");
        assert_eq!(saved.version, 1);
        let events = storage
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
        assert_eq!(events.len(), 1);
        assert!(events[0].created_time > 0);
        let summaries = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_all_page(),
                vec![],
            )
            .await
            .unwrap()
            .items;
        assert_eq!(summaries[0].id, "wf-1");
        assert_eq!(summaries[0].workflow_def_id, "wf");
    }

    #[tokio::test]
    async fn state_manager_commits_events_for_existing_instance_without_loading_first() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = WorkflowStateManager::new(storage.clone());
        let instance = workflow_instance("wf-1", WorkflowStatus::Pending);

        manager
            .commit_events_for_instance(
                &crate::core::namespace::test_namespace(),
                instance,
                vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                    status: WorkflowStatus::Running,
                }],
            )
            .await
            .unwrap();

        let saved = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "wf-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, WorkflowStatus::Running);
        assert_eq!(saved.version, 1);
        let events = storage
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
        assert_eq!(events.len(), 1);
        let summaries = storage
            .list_workflow_info(
                Some(&crate::core::namespace::test_namespace()),
                list_all_page(),
                vec![],
            )
            .await
            .unwrap()
            .items;
        assert_eq!(summaries[0].status, WorkflowStatus::Running);
    }

    #[tokio::test]
    async fn stale_engine_snapshot_cannot_overwrite_newer_human_input_commit() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = WorkflowStateManager::new(storage.clone());
        manager
            .commit_events(
                &crate::core::namespace::test_namespace(),
                "wf-1",
                vec![WorkflowInstanceEvent::WorkflowCreated {
                    instance: input_needed_instance("wf-1"),
                }],
            )
            .await
            .unwrap();
        let stale_engine_snapshot = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "wf-1")
            .await
            .unwrap()
            .unwrap();

        manager
            .commit_events(
                &crate::core::namespace::test_namespace(),
                "wf-1",
                vec![WorkflowInstanceEvent::HumanInputSubmitted {
                    task_attempt_id: "ask[1]".to_string(),
                    continuation_task_attempt_id: "ask[2]".to_string(),
                    submitted_input: serde_json::json!("ready"),
                }],
            )
            .await
            .unwrap();

        let result = manager
            .commit_events_for_instance(
                &crate::core::namespace::test_namespace(),
                stale_engine_snapshot,
                vec![WorkflowInstanceEvent::TaskStatusChanged {
                    task_attempt_id: "independent[1]".to_string(),
                    status: TaskStatus::Running,
                }],
            )
            .await;

        let error = result.unwrap_err();
        let conflict = workflow_version_conflict(&error);
        assert_eq!(conflict.expected_version, 1);
        assert_eq!(conflict.actual_version, 2);

        let saved = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "wf-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, WorkflowStatus::Pending);
        assert!(saved.tasks.contains_key("ask[2]"));
        assert_eq!(saved.tasks["independent[1]"].status, TaskStatus::Pending);
        assert_eq!(saved.version, 2);
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
            2
        );
    }

    #[tokio::test]
    async fn stale_api_commit_is_rejected_and_reload_retry_can_commit() {
        let storage = Arc::new(MemoryStorage::new());
        let manager = WorkflowStateManager::new(storage.clone());
        manager
            .commit_events(
                &crate::core::namespace::test_namespace(),
                "wf-1",
                vec![WorkflowInstanceEvent::WorkflowCreated {
                    instance: workflow_instance("wf-1", WorkflowStatus::Pending),
                }],
            )
            .await
            .unwrap();
        let stale_api_snapshot = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "wf-1")
            .await
            .unwrap()
            .unwrap();

        manager
            .commit_events_for_instance(
                &crate::core::namespace::test_namespace(),
                stale_api_snapshot.clone(),
                vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                    status: WorkflowStatus::Running,
                }],
            )
            .await
            .unwrap();

        let stale_result = manager
            .commit_events_for_instance(
                &crate::core::namespace::test_namespace(),
                stale_api_snapshot,
                vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                    status: WorkflowStatus::Paused,
                }],
            )
            .await;
        let stale_error = stale_result.unwrap_err();
        workflow_version_conflict(&stale_error);

        let latest = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "wf-1")
            .await
            .unwrap()
            .unwrap();
        let retried = manager
            .commit_events_for_instance(
                &crate::core::namespace::test_namespace(),
                latest,
                vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                    status: WorkflowStatus::Paused,
                }],
            )
            .await
            .unwrap();

        assert_eq!(retried.status, WorkflowStatus::Paused);
        assert_eq!(retried.version, 3);
        let saved = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "wf-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, WorkflowStatus::Paused);
        assert_eq!(saved.version, 3);
    }
}
