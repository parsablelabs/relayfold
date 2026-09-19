use std::path::PathBuf;
use std::sync::Arc;
use orchestrator::core::scheduler::start_task_scheduler;
use tokio::net::TcpListener;
use tokio::time::{self, Duration};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use orchestrator::adapters::memory_storage::MemoryStorage;
use orchestrator::adapters::memory_workflow_queue::MemoryWorkflowQueue;
use orchestrator::adapters::mysql_storage::{self, MySqlStorage, MySqlStorageConfig};
use orchestrator::adapters::sqlite_storage::{self, SqliteStorage};
use orchestrator::adapters::task_dispatcher::{self, TaskDispatcher};
use orchestrator::adapters::worker_registry::WorkerRegistry;
use orchestrator::api::router;
use orchestrator::core::function::function_service::FunctionService;
use orchestrator::core::namespace::NamespaceResolver;
use orchestrator::core::orchestrator::Orchestrator;
use orchestrator::core::workflow::workflow_service::WorkflowService;
use orchestrator::ports::storage::StoragePort;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();
    info!("Starting RelayFold Orchestrator...");

    // Initialize dependencies (Adapters)
    let storage = create_storage().await?;
    let worker_registry = WorkerRegistry::new();
    let task_dispatcher = Arc::new(TaskDispatcher::new());
    let workflow_queue = Arc::new(MemoryWorkflowQueue::new(workflow_queue_capacity()));
    let workflow_service = Arc::new(WorkflowService::new(storage.clone()));

    // Initialize Orchestrator (Application Layer)
    let orchestrator = Arc::new(Orchestrator::new(
        storage.clone(),
        task_dispatcher.clone(),
        workflow_queue,
    ));

    let namespace_resolver = Arc::new(NamespaceResolver::new(storage.clone()));

    let recovered = orchestrator.synchronize_startup_tasks().await?;
    info!(recovered, "Startup task synchronization complete");

    let requeued = orchestrator.enqueue_active_workflow_instances().await?;
    info!(requeued, "Active workflow requeue complete");

    tokio::spawn(
        orchestrator
            .clone()
            .run_workflow_queue(max_concurrent_workflows()),
    );

    // Setup API (Interface Layer)
    let public_app = router::create_public_router(
        orchestrator.clone(),
        workflow_service.clone(),
        Arc::new(FunctionService::new(storage)),
        worker_registry.clone(),
        namespace_resolver,
    );
    let worker_app = router::create_worker_router(worker_registry.clone(), task_dispatcher.clone());

    let public_addr = resolve_public_http_addr();
    let worker_addr = resolve_worker_http_addr();
    let public_listener = TcpListener::bind(&public_addr).await?;
    let worker_listener = TcpListener::bind(&worker_addr).await?;

    info!("Public API listening on {}", public_listener.local_addr()?);
    info!("Worker API listening on {}", worker_listener.local_addr()?);

    // TODO: consider handling task failures, with restarts
    let _ = task_dispatcher::start_task_timeout_monitor(task_dispatcher.clone());
    let _ = start_task_pinned_host_loss_monitor(
        orchestrator.clone(),
        worker_registry.clone(),
        task_dispatcher.clone(),
    );
    let _ = start_task_scheduler(orchestrator.clone(), workflow_service.clone(), worker_registry.clone());

    tokio::try_join!(
        axum::serve(public_listener, public_app),
        axum::serve(worker_listener, worker_app),
    )?;

    Ok(())
}

fn max_concurrent_workflows() -> usize {
    std::env::var("RELAYFOLD_MAX_CONCURRENT_WORKFLOWS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1)
}

fn workflow_queue_capacity() -> usize {
    std::env::var("RELAYFOLD_WORKFLOW_QUEUE_CAPACITY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1024)
}

async fn create_storage() -> anyhow::Result<Arc<dyn StoragePort + Send + Sync>> {
    match load_storage_config()? {
        StorageConfig::Memory => Ok(Arc::new(MemoryStorage::new())),
        StorageConfig::Sqlite { database_path } => {
            Ok(Arc::new(SqliteStorage::connect(database_path).await?))
        }
        StorageConfig::MySql(config) => Ok(Arc::new(MySqlStorage::connect(config).await?)),
    }
}

enum StorageConfig {
    Memory,
    Sqlite { database_path: PathBuf },
    MySql(MySqlStorageConfig),
}

fn load_storage_config() -> anyhow::Result<StorageConfig> {
    load_storage_config_with(|name| std::env::var(name).ok())
}

fn load_storage_config_with(
    mut read: impl FnMut(&str) -> Option<String>,
) -> anyhow::Result<StorageConfig> {
    match read("RELAYFOLD_STORAGE").as_deref().unwrap_or("memory") {
        "memory" => Ok(StorageConfig::Memory),
        "sqlite" => Ok(StorageConfig::Sqlite {
            database_path: required_config_value(&mut read, sqlite_storage::ENV_PATH)?.into(),
        }),
        "mysql" => {
            let port_env = mysql_storage::ENV_PORT;
            let port = match read(port_env) {
                None => 3306,
                Some(value) if value.trim().is_empty() => {
                    anyhow::bail!("{port_env} must not be empty")
                }
                Some(value) => value.parse::<u16>().map_err(|_| {
                    anyhow::anyhow!("{port_env} must be an integer from 1 to 65535")
                })?,
            };
            if port == 0 {
                anyhow::bail!("{port_env} must be an integer from 1 to 65535");
            }

            let database_env = mysql_storage::ENV_DATABASE;
            let database = match read(database_env) {
                None => "relayfold".to_string(),
                Some(value) if value.trim().is_empty() => {
                    anyhow::bail!("{database_env} must not be empty")
                }
                Some(value) => value,
            };

            Ok(StorageConfig::MySql(MySqlStorageConfig {
                host: required_config_value(&mut read, mysql_storage::ENV_HOST)?,
                port,
                database,
                username: required_config_value(&mut read, mysql_storage::ENV_USERNAME)?,
                password: required_config_value(&mut read, mysql_storage::ENV_PASSWORD)?,
            }))
        }
        value => anyhow::bail!(
            "unsupported RELAYFOLD_STORAGE value {value}; expected memory, sqlite, or mysql"
        ),
    }
}

fn required_config_value(
    read: &mut impl FnMut(&str) -> Option<String>,
    name: &str,
) -> anyhow::Result<String> {
    let value = read(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))?;
    if value.trim().is_empty() {
        anyhow::bail!("{name} must not be empty");
    }
    Ok(value)
}

#[cfg(test)]
mod storage_config_tests {
    use super::*;
    use std::collections::HashMap;

    fn parse(values: &[(&str, &str)]) -> anyhow::Result<StorageConfig> {
        let values = values
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect::<HashMap<_, _>>();
        load_storage_config_with(|name| values.get(name).cloned())
    }

    fn mysql_required_values() -> Vec<(&'static str, &'static str)> {
        vec![
            ("RELAYFOLD_STORAGE", "mysql"),
            (mysql_storage::ENV_HOST, "mysql.internal"),
            (mysql_storage::ENV_USERNAME, "relayfold-user"),
            (mysql_storage::ENV_PASSWORD, "secret"),
        ]
    }

    #[test]
    fn defaults_to_memory_storage() {
        assert!(matches!(parse(&[]).unwrap(), StorageConfig::Memory));
    }

    #[test]
    fn selects_memory_storage_explicitly() {
        assert!(matches!(
            parse(&[("RELAYFOLD_STORAGE", "memory")]).unwrap(),
            StorageConfig::Memory
        ));
    }

    #[test]
    fn configures_sqlite_from_path() {
        let config = parse(&[
            ("RELAYFOLD_STORAGE", "sqlite"),
            (sqlite_storage::ENV_PATH, "/data/relayfold.db"),
        ])
        .unwrap();

        let StorageConfig::Sqlite { database_path } = config else {
            panic!("expected SQLite storage");
        };
        assert_eq!(database_path, PathBuf::from("/data/relayfold.db"));
    }

    #[test]
    fn sqlite_requires_a_non_empty_path() {
        for values in [
            vec![("RELAYFOLD_STORAGE", "sqlite")],
            vec![
                ("RELAYFOLD_STORAGE", "sqlite"),
                (sqlite_storage::ENV_PATH, "  "),
            ],
        ] {
            let error = parse(&values).err().unwrap().to_string();
            assert!(error.contains(sqlite_storage::ENV_PATH));
        }
    }

    #[test]
    fn mysql_uses_database_and_port_defaults() {
        let StorageConfig::MySql(config) = parse(&mysql_required_values()).unwrap() else {
            panic!("expected MySQL storage");
        };

        assert_eq!(config.host, "mysql.internal");
        assert_eq!(config.port, 3306);
        assert_eq!(config.database, "relayfold");
        assert_eq!(config.username, "relayfold-user");
        assert_eq!(config.password, "secret");
    }

    #[test]
    fn mysql_accepts_explicit_database_and_port() {
        let mut values = mysql_required_values();
        values.extend([
            (mysql_storage::ENV_PORT, "4406"),
            (mysql_storage::ENV_DATABASE, "custom"),
        ]);
        let StorageConfig::MySql(config) = parse(&values).unwrap() else {
            panic!("expected MySQL storage");
        };

        assert_eq!(config.port, 4406);
        assert_eq!(config.database, "custom");
    }

    #[test]
    fn mysql_requires_non_empty_connection_values() {
        for variable in [
            mysql_storage::ENV_HOST,
            mysql_storage::ENV_USERNAME,
            mysql_storage::ENV_PASSWORD,
        ] {
            let missing = mysql_required_values()
                .into_iter()
                .filter(|(name, _)| *name != variable)
                .collect::<Vec<_>>();
            assert!(
                parse(&missing)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains(variable)
            );

            let mut empty = mysql_required_values();
            let value = empty
                .iter_mut()
                .find(|(name, _)| *name == variable)
                .unwrap();
            value.1 = " ";
            assert!(parse(&empty).err().unwrap().to_string().contains(variable));
        }
    }

    #[test]
    fn mysql_rejects_invalid_ports() {
        for port in ["", "0", "abc", "65536"] {
            let mut values = mysql_required_values();
            values.push((mysql_storage::ENV_PORT, port));
            let error = parse(&values).err().unwrap().to_string();
            assert!(error.contains(mysql_storage::ENV_PORT));
        }
    }

    #[test]
    fn mysql_rejects_an_empty_database() {
        let mut values = mysql_required_values();
        values.push((mysql_storage::ENV_DATABASE, ""));
        let error = parse(&values).err().unwrap().to_string();
        assert!(error.contains(mysql_storage::ENV_DATABASE));
    }

    #[test]
    fn rejects_unknown_and_legacy_storage_values() {
        for value in ["postgres", "sql", ""] {
            let error = parse(&[("RELAYFOLD_STORAGE", value)])
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("unsupported RELAYFOLD_STORAGE"));
        }
    }
}

fn resolve_public_http_addr() -> String {
    std::env::var("RELAYFOLD_PUBLIC_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string())
}

fn resolve_worker_http_addr() -> String {
    std::env::var("RELAYFOLD_WORKER_HTTP_ADDR").unwrap_or_else(|_| "127.0.0.1:3001".to_string())
}

fn start_task_pinned_host_loss_monitor(
    orchestrator: Arc<Orchestrator>,
    worker_registry: WorkerRegistry,
    task_dispatcher: Arc<TaskDispatcher>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = time::interval(Duration::from_millis(100));
        loop {
            ticker.tick().await;
            let lost_hosts = worker_registry.update_worker_liveness().await;
            if lost_hosts.is_empty() {
                continue;
            }
            task_dispatcher
                .cancel_pending_tasks_for_lost_hosts(&lost_hosts)
                .await;

            match orchestrator
                .fail_workflows_pinned_to_lost_hosts(&lost_hosts)
                .await
            {
                Ok(failed) => {
                    info!(failed, lost_hosts = ?lost_hosts, "Pinned host loss reconciliation complete");
                }
                Err(error) => {
                    error!(%error, lost_hosts = ?lost_hosts, "Pinned host loss reconciliation failed");
                }
            }
        }
    })
}
