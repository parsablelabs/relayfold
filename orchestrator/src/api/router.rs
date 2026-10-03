use axum::{
    Router,
    routing::{delete, get, post},
};
use std::sync::Arc;

use super::handlers;

use crate::adapters::task_dispatcher::TaskDispatcher;
use crate::adapters::worker_registry::WorkerRegistry;
use crate::core::function::function_service::FunctionService;
use crate::core::namespace::NamespaceResolverPort;
use crate::core::orchestrator::Orchestrator;
use crate::core::workflow::workflow_service::WorkflowService;

#[derive(Clone)]
pub struct PublicAppState {
    pub orchestrator: Arc<Orchestrator>,
    pub workflow_service: Arc<WorkflowService>,
    pub function_service: Arc<FunctionService>,
    pub worker_registry: WorkerRegistry,
    pub namespace_resolver: Arc<dyn NamespaceResolverPort + Send + Sync>,
}

#[derive(Clone)]
pub struct WorkerAppState {
    pub worker_registry: WorkerRegistry,
    pub task_dispatcher: Arc<TaskDispatcher>,
}

pub fn create_public_router(
    orchestrator: Arc<Orchestrator>,
    workflow_service: Arc<WorkflowService>,
    function_service: Arc<FunctionService>,
    worker_registry: WorkerRegistry,
    namespace_resolver: Arc<dyn NamespaceResolverPort + Send + Sync>,
) -> Router {
    let state = PublicAppState {
        orchestrator,
        workflow_service,
        function_service,
        worker_registry,
        namespace_resolver,
    };

    Router::new()
        .route("/health", get(handlers::health_check))
        .route("/function-def", post(handlers::create_function_def))
        .route(
            "/function-def/{def_id}",
            delete(handlers::delete_function_def),
        )
        .route(
            "/workflow-def",
            get(handlers::list_workflow_defs).post(handlers::create_workflow_def),
        )
        .route(
            "/workflow-def/{def_id}/tasks/{task_id}",
            post(handlers::invoke_workflow_task_isolated),
        )
        .route(
            "/workflow-def/{def_id}",
            get(handlers::get_workflow_def).post(handlers::trigger_workflow_instance),
        )
        .route(
            "/workflow-queue",
            get(handlers::get_queue).delete(handlers::purge_queue),
        )
        .route("/workflow-queue/{id}", delete(handlers::delete_queue_item))
        .route("/workflows", get(handlers::list_workflows))
        .route("/workflows/pause", post(handlers::pause_active_workflows))
        .route("/workflows/resume", post(handlers::resume_paused_workflows))
        .route("/workflows/{id}/events", get(handlers::get_workflow_events))
        .route("/workflows/{id}/pause", post(handlers::pause_workflow))
        .route("/workflows/{id}/resume", post(handlers::resume_workflow))
        .route("/workflows/{id}", get(handlers::get_workflow_instance))
        .route(
            "/workflows/{workflow_instance_id}/tasks",
            get(handlers::list_task_results),
        )
        .route(
            "/workflows/{workflow_instance_id}/tasks/{task_id}/{generation}",
            get(handlers::get_task_result_generation),
        )
        .route(
            "/workflows/{workflow_instance_id}/tasks/{task_id}/human-input",
            post(handlers::submit_human_input),
        )
        .route(
            "/workflows/{workflow_instance_id}/tasks/{task_id}/retry",
            post(handlers::retry_task),
        )
        .route(
            "/workflows/{workflow_instance_id}/tasks/{task_id}",
            get(handlers::get_task_result),
        )
        .fallback(handlers::not_found)
        .with_state(state)
}

pub fn create_worker_router(
    worker_registry: WorkerRegistry,
    task_dispatcher: Arc<TaskDispatcher>,
) -> Router {
    let state = WorkerAppState {
        worker_registry,
        task_dispatcher,
    };

    Router::new()
        .route("/health", get(handlers::health_check))
        .route("/workers/register", post(handlers::register_worker))
        .route("/workers/heartbeat", post(handlers::heartbeat_worker))
        .route("/workers/tasks/claim", post(handlers::claim_worker_task))
        .route(
            "/workers/tasks/{task_id}/result",
            post(handlers::complete_worker_task),
        )
        .fallback(handlers::not_found)
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::fake_task_dispatcher::FakeTaskDispatcher;
    use crate::adapters::memory_storage::MemoryStorage;
    use crate::adapters::memory_workflow_queue::MemoryWorkflowQueue;
    use crate::core::function::models::FunctionDef;
    use crate::core::namespace::{Namespace, NamespaceResolverPort};
    use crate::core::task::{TaskInstance, TaskSatisfactionStatus, TaskStatus, TaskTypeDef};
    use crate::core::workflow::events::{WorkflowEventRecord, WorkflowInstanceEvent};
    use crate::core::workflow::models::{WorkflowDef, WorkflowInstance, WorkflowStatus};
    use crate::ports::storage::StoragePort;
    use crate::ports::workflow_queue::WorkflowQueuePort;
    use async_trait::async_trait;
    use axum::{
        body::{Body, Bytes, to_bytes},
        http::{Method, Request, StatusCode, header::AUTHORIZATION},
    };
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    const NAMESPACE_A: &str = "550e8400-e29b-41d4-a716-446655440000";
    const NAMESPACE_B: &str = "6ba7b811-9dad-11d1-80b4-00c04fd430c8";

    struct TestNamespaceResolver {
        global_namespace: Option<Namespace>,
        calls: Mutex<Vec<Option<String>>>,
    }

    impl TestNamespaceResolver {
        fn with_global_namespace(namespace: Namespace) -> Self {
            Self {
                global_namespace: Some(namespace),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn without_global_namespace() -> Self {
            Self {
                global_namespace: None,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl NamespaceResolverPort for TestNamespaceResolver {
        async fn resolve(&self, api_key: Option<&str>) -> anyhow::Result<Namespace> {
            self.calls.lock().await.push(api_key.map(str::to_string));

            if let Some(namespace) = &self.global_namespace {
                return Ok(namespace.clone());
            }

            match api_key {
                Some("namespace-a") => Ok(Namespace::new(NAMESPACE_A).unwrap()),
                Some("namespace-b") => Ok(Namespace::new(NAMESPACE_B).unwrap()),
                _ => anyhow::bail!("bearer credential is required"),
            }
        }
    }

    fn test_router(
        storage: Arc<MemoryStorage>,
        queue: Arc<MemoryWorkflowQueue>,
        resolver: Arc<dyn NamespaceResolverPort + Send + Sync>,
    ) -> Router {
        let orchestrator = Arc::new(Orchestrator::new(
            storage.clone(),
            Arc::new(FakeTaskDispatcher::new()),
            queue,
        ));

        create_public_router(
            orchestrator,
            Arc::new(WorkflowService::new(storage.clone())),
            Arc::new(FunctionService::new(storage)),
            WorkerRegistry::new(),
            resolver,
        )
    }

    async fn request(
        router: &Router,
        method: Method,
        uri: &str,
        authorization: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Bytes) {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(authorization) = authorization {
            request = request.header(AUTHORIZATION, authorization);
        }
        if body.is_some() {
            request = request.header("content-type", "application/json");
        }

        let response = router
            .clone()
            .oneshot(
                request
                    .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, body)
    }

    fn task(status: TaskStatus) -> TaskInstance {
        TaskInstance {
            task_def_id: "task-a".to_string(),
            satisfaction_status: if status == TaskStatus::Completed {
                TaskSatisfactionStatus::Satisfied
            } else if status == TaskStatus::Failed {
                TaskSatisfactionStatus::Unsatisfied
            } else {
                TaskSatisfactionStatus::Pending
            },
            status,
            human_input: None,
            input_data: vec![],
            input_mapping: vec![],
            output_data: Some(json!({"owner": "namespace-b"})),
            generation_index: 1,
            verifier_metadata: None,
        }
    }

    fn workflow(id: &str, status: WorkflowStatus, task_status: TaskStatus) -> WorkflowInstance {
        WorkflowInstance {
            id: id.to_string(),
            workflow_def_id: "shared-def".to_string(),
            version: 0,
            status,
            trigger_input: Some(json!({"owner": "namespace-b"})),
            pinned_worker_host: None,
            tasks: HashMap::from([("task-a[1]".to_string(), task(task_status))]),
            verifier_states: HashMap::new(),
        }
    }

    #[test]
    fn public_router_accepts_task_action_route_shapes() {
        let storage = Arc::new(MemoryStorage::new());
        let _router = test_router(
            storage,
            Arc::new(MemoryWorkflowQueue::new(10)),
            Arc::new(TestNamespaceResolver::without_global_namespace()),
        );
    }

    #[tokio::test]
    async fn public_health_check_does_not_require_namespace_context() {
        let storage = Arc::new(MemoryStorage::new());
        let resolver = Arc::new(TestNamespaceResolver::without_global_namespace());
        let router = test_router(
            storage,
            Arc::new(MemoryWorkflowQueue::new(10)),
            resolver.clone(),
        );

        let (status, body) = request(&router, Method::GET, "/health", None, None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "OK");
        assert!(resolver.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn public_router_uses_global_namespace_and_ignores_authorization() {
        let storage = Arc::new(MemoryStorage::new());
        let resolver = Arc::new(TestNamespaceResolver::with_global_namespace(
            Namespace::new(crate::core::namespace::GLOBAL_NAMESPACE).unwrap(),
        ));
        let router = test_router(
            storage,
            Arc::new(MemoryWorkflowQueue::new(10)),
            resolver.clone(),
        );

        let (missing_status, _) = request(&router, Method::GET, "/workflow-def", None, None).await;
        let (malformed_status, _) = request(
            &router,
            Method::GET,
            "/workflow-def",
            Some("not-even-a-bearer-value"),
            None,
        )
        .await;
        let (unrelated_status, _) = request(
            &router,
            Method::GET,
            "/workflow-def",
            Some("Bearer unrelated-key"),
            None,
        )
        .await;

        assert_eq!(missing_status, StatusCode::OK);
        assert_eq!(malformed_status, StatusCode::OK);
        assert_eq!(unrelated_status, StatusCode::OK);
        assert_eq!(
            *resolver.calls.lock().await,
            vec![None, None, Some("unrelated-key".to_string())]
        );
    }

    #[tokio::test]
    async fn public_router_requires_bearer_and_invokes_resolver_without_global_mode() {
        let storage = Arc::new(MemoryStorage::new());
        let resolver = Arc::new(TestNamespaceResolver::without_global_namespace());
        let router = test_router(
            storage,
            Arc::new(MemoryWorkflowQueue::new(10)),
            resolver.clone(),
        );

        for authorization in [None, Some("Basic api-key"), Some("Bearer ")] {
            let (status, _) =
                request(&router, Method::GET, "/workflow-def", authorization, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{authorization:?}");
        }

        let (status, _) = request(
            &router,
            Method::GET,
            "/workflow-def",
            Some("Bearer namespace-a"),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            *resolver.calls.lock().await,
            vec![None, None, None, Some("namespace-a".to_string())]
        );
    }

    #[tokio::test]
    async fn public_api_isolates_resources_and_cross_namespace_absence_contracts() {
        let storage = Arc::new(MemoryStorage::new());
        let queue = Arc::new(MemoryWorkflowQueue::new(10));
        let resolver = Arc::new(TestNamespaceResolver::without_global_namespace());
        let router = test_router(storage.clone(), queue.clone(), resolver);
        let namespace_a = Namespace::new(NAMESPACE_A).unwrap();
        let namespace_b = Namespace::new(NAMESPACE_B).unwrap();

        storage
            .save_workflow_def(
                &namespace_b,
                WorkflowDef {
                    sandbox: None,
                    id: "shared-def".to_string(),
                    description: "namespace-b".to_string(),
                    tasks: vec![crate::core::task::TaskDef {
                        sandbox: None,
                        id: "task-a".to_string(),
                        kind: TaskTypeDef::Agent {
                            model_id: "model".to_string(),
                            provider_url: "provider".to_string(),
                            prompt: "prompt".to_string(),
                            tools: vec![],
                            skills: vec![],
                            ask: true,
                            schema_failure_retry_times: 0.into(),
                            reuse_session: true,
                        },
                        control: None,
                        timeout_secs: None,
                        input_schemas: vec![],
                        output_schema: None,
                        workspace: None,
                        required_credentials: vec![],
                    }],
                    data_bindings: vec![],
                },
            )
            .await
            .unwrap();
        storage
            .save_function_def(
                &namespace_b,
                FunctionDef {
                    id: "foreign-function".to_string(),
                    dependencies: vec![],
                    code: "export default async function run() {}".to_string(),
                },
            )
            .await
            .unwrap();

        for instance in [
            workflow(
                "foreign-pending",
                WorkflowStatus::Pending,
                TaskStatus::Completed,
            ),
            workflow(
                "foreign-paused",
                WorkflowStatus::Paused,
                TaskStatus::Pending,
            ),
            workflow("foreign-failed", WorkflowStatus::Failed, TaskStatus::Failed),
            workflow(
                "foreign-input",
                WorkflowStatus::InputNeeded,
                TaskStatus::InputNeeded {
                    input_request: "approval needed".to_string(),
                },
            ),
        ] {
            storage
                .save_workflow_instance(&namespace_b, 0, vec![], instance)
                .await
                .unwrap();
        }
        storage
            .save_workflow_instance(
                &namespace_b,
                0,
                vec![WorkflowEventRecord {
                    created_time: 100,
                    event: WorkflowInstanceEvent::WorkflowStatusChanged {
                        status: WorkflowStatus::Pending,
                    },
                }],
                {
                    let mut instance = workflow(
                        "foreign-events",
                        WorkflowStatus::Pending,
                        TaskStatus::Completed,
                    );
                    instance.version = 1;
                    instance
                },
            )
            .await
            .unwrap();
        queue
            .enqueue(&namespace_b, "foreign-pending".to_string())
            .await
            .unwrap();

        let authorization = Some("Bearer namespace-a");
        for (uri, collection_key) in [
            ("/workflow-def", "workflow_defs"),
            ("/workflows", "workflows"),
            ("/workflow-queue", "pending"),
        ] {
            let (status, body) = request(&router, Method::GET, uri, authorization, None).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body[collection_key], json!([]), "{uri}");
        }

        let cross_namespace_cases = [
            (
                Method::GET,
                "/workflow-def/shared-def",
                "/workflow-def/missing-def",
                None,
            ),
            (
                Method::POST,
                "/workflow-def/shared-def/tasks/task-a",
                "/workflow-def/missing-def/tasks/task-a",
                Some(json!({"inputs": []})),
            ),
            (
                Method::GET,
                "/workflows/foreign-pending",
                "/workflows/missing",
                None,
            ),
            (
                Method::GET,
                "/workflows/foreign-events/events",
                "/workflows/missing/events",
                None,
            ),
            (
                Method::GET,
                "/workflows/foreign-pending/tasks",
                "/workflows/missing/tasks",
                None,
            ),
            (
                Method::GET,
                "/workflows/foreign-pending/tasks/task-a",
                "/workflows/missing/tasks/task-a",
                None,
            ),
            (
                Method::GET,
                "/workflows/foreign-pending/tasks/task-a/1",
                "/workflows/missing/tasks/task-a/1",
                None,
            ),
            (
                Method::POST,
                "/workflows/foreign-pending/pause",
                "/workflows/missing/pause",
                None,
            ),
            (
                Method::POST,
                "/workflows/foreign-paused/resume",
                "/workflows/missing/resume",
                None,
            ),
            (
                Method::POST,
                "/workflows/foreign-failed/tasks/task-a/retry",
                "/workflows/missing/tasks/task-a/retry",
                None,
            ),
            (
                Method::POST,
                "/workflows/foreign-input/tasks/task-a/human-input",
                "/workflows/missing/tasks/task-a/human-input",
                Some(json!({"input": {"approved": true}})),
            ),
            (
                Method::DELETE,
                "/workflow-queue/foreign-pending",
                "/workflow-queue/missing",
                None,
            ),
            (
                Method::DELETE,
                "/function-def/foreign-function",
                "/function-def/missing",
                None,
            ),
        ];

        for (method, foreign_uri, missing_uri, body) in cross_namespace_cases {
            let foreign = request(
                &router,
                method.clone(),
                foreign_uri,
                authorization,
                body.clone(),
            )
            .await;
            let missing = request(&router, method, missing_uri, authorization, body).await;

            assert_eq!(foreign, missing, "{foreign_uri}");
            assert_eq!(foreign.0, StatusCode::NOT_FOUND, "{foreign_uri}");
        }

        let (status, body) = request(
            &router,
            Method::DELETE,
            "/workflow-queue",
            authorization,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["purged"],
            json!([])
        );

        let (status, _) = request(
            &router,
            Method::POST,
            "/workflow-def",
            authorization,
            Some(json!({
                "id": "shared-def",
                "description": "namespace-a",
                "namespace": NAMESPACE_B,
                "tasks": [],
                "data_bindings": []
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            storage
                .get_workflow_def(&namespace_a, "shared-def")
                .await
                .unwrap()
                .unwrap()
                .description,
            "namespace-a"
        );
        assert_eq!(
            storage
                .get_workflow_def(&namespace_b, "shared-def")
                .await
                .unwrap()
                .unwrap()
                .description,
            "namespace-b"
        );

        assert_eq!(
            queue.pending_ids(&namespace_b).await.unwrap(),
            vec!["foreign-pending".to_string()]
        );
        assert!(
            storage
                .get_function_def(&namespace_b, "foreign-function")
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            storage
                .get_workflow_instance(&namespace_b, "foreign-pending")
                .await
                .unwrap()
                .unwrap()
                .status,
            WorkflowStatus::Pending
        );
        assert_eq!(
            storage
                .get_workflow_instance(&namespace_b, "foreign-paused")
                .await
                .unwrap()
                .unwrap()
                .status,
            WorkflowStatus::Paused
        );
        assert_eq!(
            storage
                .get_workflow_instance(&namespace_b, "foreign-failed")
                .await
                .unwrap()
                .unwrap()
                .status,
            WorkflowStatus::Failed
        );
        assert_eq!(
            storage
                .get_workflow_instance(&namespace_b, "foreign-input")
                .await
                .unwrap()
                .unwrap()
                .status,
            WorkflowStatus::InputNeeded
        );
    }
}
