use std::collections::HashMap;

use orchestrator::adapters::mysql_storage::{MySqlStorage, MySqlStorageConfig};
use orchestrator::core::function::models::FunctionDef;
use orchestrator::core::namespace::Namespace;
use orchestrator::core::task::{TaskInstance, TaskSatisfactionStatus, TaskStatus};
use orchestrator::core::util::unix_timestamp_ms;
use orchestrator::core::worker::WorkerHostId;
use orchestrator::core::workflow::events::{WorkflowEventRecord, WorkflowInstanceEvent};
use orchestrator::core::workflow::models::{WorkflowDef, WorkflowInstance, WorkflowStatus};
use orchestrator::ports::storage::{
    StorageError, StoragePort, WorkflowEventPageRequest, WorkflowInfoPageRequest,
    WorkflowInstanceFilter, WorkflowVersionConflict,
};
use serde_json::json;

const TEST_ENV_HOST: &str = "RELAYFOLD_STORE_MYSQL_TEST_HOST";
const TEST_ENV_PORT: &str = "RELAYFOLD_STORE_MYSQL_TEST_PORT";
const TEST_ENV_DATABASE: &str = "RELAYFOLD_STORE_MYSQL_TEST_DATABASE";
const TEST_ENV_USERNAME: &str = "RELAYFOLD_STORE_MYSQL_TEST_USERNAME";
const TEST_ENV_PASSWORD: &str = "RELAYFOLD_STORE_MYSQL_TEST_PASSWORD";

fn required_test_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set"))
}

#[tokio::test]
#[ignore = "requires RELAYFOLD_STORE_MYSQL_TEST_* and a dedicated MySQL database"]
async fn persists_and_reconstructs_workflow_state() {
    let port = std::env::var(TEST_ENV_PORT)
        .map(|value| {
            value
                .parse::<u16>()
                .unwrap_or_else(|_| panic!("invalid {TEST_ENV_PORT}"))
        })
        .unwrap_or(3306);
    let config = MySqlStorageConfig {
        host: required_test_env(TEST_ENV_HOST),
        port,
        database: required_test_env(TEST_ENV_DATABASE),
        username: required_test_env(TEST_ENV_USERNAME),
        password: required_test_env(TEST_ENV_PASSWORD),
    };
    let storage = MySqlStorage::connect(config).await.unwrap();
    let namespace = Namespace::new("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let other_namespace = Namespace::new("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
    let suffix = unix_timestamp_ms().unwrap();
    let workflow_def_id = format!("mysqlcontract{suffix}");
    let workflow_instance_id = format!("{workflow_def_id}-{suffix}");
    let function_def_id = format!("function{suffix}");

    let mut workflow_def = WorkflowDef {
        sandbox: None,
        id: workflow_def_id.clone(),
        description: "MySQL contract test".to_string(),
        tasks: vec![],
        data_bindings: vec![],
    };
    storage
        .save_workflow_def(&namespace, workflow_def.clone())
        .await
        .unwrap();
    assert_eq!(
        storage
            .get_workflow_def(&namespace, &workflow_def_id)
            .await
            .unwrap()
            .unwrap()
            .description,
        workflow_def.description
    );
    workflow_def.description = "Updated MySQL contract test".to_string();
    storage
        .save_workflow_def(&namespace, workflow_def.clone())
        .await
        .unwrap();
    let mut other_workflow_def = workflow_def.clone();
    other_workflow_def.description = "Other namespace".to_string();
    storage
        .save_workflow_def(&other_namespace, other_workflow_def.clone())
        .await
        .unwrap();
    assert_eq!(
        storage
            .get_workflow_def(&other_namespace, &workflow_def_id)
            .await
            .unwrap()
            .unwrap()
            .description,
        other_workflow_def.description
    );

    let mut function_def = FunctionDef {
        id: function_def_id.clone(),
        dependencies: vec![],
        code: "export default () => 1".to_string(),
    };
    storage
        .save_function_def(&namespace, function_def.clone())
        .await
        .unwrap();
    assert_eq!(
        storage
            .get_function_def(&namespace, &function_def_id)
            .await
            .unwrap()
            .unwrap()
            .code,
        function_def.code
    );
    function_def.code = "export default () => 2".to_string();
    storage
        .save_function_def(&namespace, function_def.clone())
        .await
        .unwrap();

    let event = WorkflowEventRecord {
        created_time: suffix,
        event: WorkflowInstanceEvent::WorkflowStatusChanged {
            status: WorkflowStatus::Running,
        },
    };
    let task = TaskInstance {
        task_def_id: "taska".to_string(),
        status: TaskStatus::Completed,
        satisfaction_status: TaskSatisfactionStatus::Satisfied,
        human_input: None,
        input_data: vec![json!({"input": 1})],
        input_mapping: vec![],
        output_data: Some(json!({"output": 2})),
        generation_index: 1,
        verifier_metadata: None,
    };
    let mut tasks = HashMap::new();
    tasks.insert("taska[1]".to_string(), task);
    let instance = WorkflowInstance {
        id: workflow_instance_id.clone(),
        workflow_def_id: workflow_def_id.clone(),
        version: 1,
        status: WorkflowStatus::Running,
        trigger_input: Some(json!({"request": true})),
        pinned_worker_host: Some(WorkerHostId("mysql-host".to_string())),
        tasks,
        verifier_states: HashMap::new(),
    };

    storage
        .save_workflow_instance(&namespace, 0, vec![event.clone()], instance.clone())
        .await
        .unwrap();
    let mut other_instance = instance.clone();
    other_instance.trigger_input = Some(json!({"request": "other namespace"}));
    other_instance
        .tasks
        .get_mut("taska[1]")
        .unwrap()
        .output_data = Some(json!({"output": 99}));
    storage
        .save_workflow_instance(&other_namespace, 0, vec![event.clone()], other_instance)
        .await
        .unwrap();
    let saved = storage
        .get_workflow_instance(&namespace, &workflow_instance_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.version, 1);
    assert_eq!(
        saved.tasks["taska[1]"].output_data,
        Some(json!({"output": 2}))
    );
    let other_saved = storage
        .get_workflow_instance(&other_namespace, &workflow_instance_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        other_saved.tasks["taska[1]"].output_data,
        Some(json!({"output": 99}))
    );
    let saved_events = storage
        .list_workflow_instance_events(
            &namespace,
            &workflow_instance_id,
            WorkflowEventPageRequest {
                limit: 10,
                cursor: None,
            },
        )
        .await
        .unwrap()
        .items;
    assert_eq!(saved_events.len(), 1);
    assert_eq!(saved_events[0].created_time, event.created_time);

    let mut update_a = saved.clone();
    update_a.version = 3;
    update_a.status = WorkflowStatus::Completed;
    update_a.tasks.get_mut("taska[1]").unwrap().output_data = Some(json!({"output": 3}));
    let mut update_b = saved;
    update_b.version = 3;
    update_b.status = WorkflowStatus::Failed;
    update_b.tasks.get_mut("taska[1]").unwrap().output_data = Some(json!({"output": 4}));
    let (result_a, result_b) = tokio::join!(
        storage.save_workflow_instance(
            &namespace,
            1,
            vec![
                WorkflowEventRecord {
                    created_time: suffix + 1,
                    event: WorkflowInstanceEvent::TaskOutputRecorded {
                        task_attempt_id: "taska[1]".to_string(),
                        output_data: Some(json!({"output": 3})),
                    },
                },
                WorkflowEventRecord {
                    created_time: suffix + 2,
                    event: WorkflowInstanceEvent::WorkflowStatusChanged {
                        status: WorkflowStatus::Completed,
                    },
                },
            ],
            update_a,
        ),
        storage.save_workflow_instance(
            &namespace,
            1,
            vec![
                WorkflowEventRecord {
                    created_time: suffix + 1,
                    event: WorkflowInstanceEvent::TaskOutputRecorded {
                        task_attempt_id: "taska[1]".to_string(),
                        output_data: Some(json!({"output": 4})),
                    },
                },
                WorkflowEventRecord {
                    created_time: suffix + 2,
                    event: WorkflowInstanceEvent::WorkflowStatusChanged {
                        status: WorkflowStatus::Failed,
                    },
                },
            ],
            update_b,
        ),
    );
    assert_eq!(
        usize::from(result_a.is_ok()) + usize::from(result_b.is_ok()),
        1
    );
    let concurrent_conflict = result_a.err().or_else(|| result_b.err()).unwrap();
    assert!(matches!(
        concurrent_conflict,
        StorageError::WorkflowVersionConflict(WorkflowVersionConflict {
            actual_version: 3,
            ..
        })
    ));

    let conflict = storage
        .save_workflow_instance(&namespace, 0, vec![], instance)
        .await
        .unwrap_err();
    assert!(matches!(
        conflict,
        StorageError::WorkflowVersionConflict(WorkflowVersionConflict {
            actual_version: 3,
            ..
        })
    ));

    let summaries = storage
        .list_workflow_info(
            Some(&namespace),
            WorkflowInfoPageRequest {
                limit: 10,
                cursor: None,
            },
            vec![WorkflowInstanceFilter::WorkflowDefId(workflow_def_id)],
        )
        .await
        .unwrap();
    assert_eq!(summaries.items.len(), 1);
    assert_eq!(summaries.items[0].namespace, namespace);
    assert_eq!(summaries.items[0].completed_task_count, 1);
    let first_cross_namespace_page = storage
        .list_workflow_info(
            None,
            WorkflowInfoPageRequest {
                limit: 1,
                cursor: None,
            },
            vec![WorkflowInstanceFilter::WorkflowDefId(
                workflow_def.id.clone(),
            )],
        )
        .await
        .unwrap();
    assert_eq!(first_cross_namespace_page.items.len(), 1);
    let second_cross_namespace_page = storage
        .list_workflow_info(
            None,
            WorkflowInfoPageRequest {
                limit: 1,
                cursor: first_cross_namespace_page.next_cursor,
            },
            vec![WorkflowInstanceFilter::WorkflowDefId(
                workflow_def.id.clone(),
            )],
        )
        .await
        .unwrap();
    assert_eq!(second_cross_namespace_page.items.len(), 1);
    assert_ne!(
        first_cross_namespace_page.items[0].namespace,
        second_cross_namespace_page.items[0].namespace
    );
    let definition_summaries = storage.list_workflow_def(&namespace).await.unwrap();
    let definition_summary = definition_summaries
        .iter()
        .find(|summary| summary.id == workflow_def.id)
        .unwrap();
    assert_eq!(definition_summary.description, workflow_def.description);
    assert_eq!(definition_summary.last_invoked_at_epoch_ms, Some(suffix));
    assert!(
        storage
            .delete_function_def(&namespace, &function_def_id)
            .await
            .unwrap()
    );
}
