use async_trait::async_trait;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use sqlx::{MySql, MySqlPool, Row, Transaction};
use std::collections::HashMap;

use super::sql_storage_common::*;
use crate::core::function::models::{FunctionDef, FunctionDefSummary};
use crate::core::namespace::Namespace;
use crate::core::task::TaskInstance;
use crate::core::util::unix_timestamp_ms;
use crate::core::worker::WorkerHostId;
use crate::core::workflow::events::{WorkflowEventRecord, changed_task_attempt_ids};
use crate::core::workflow::models::{
    WorkflowDef, WorkflowDefSummary, WorkflowInfo, WorkflowInstance,
};
use crate::ports::storage::{
    StoragePort, StorageResult, WorkflowEventPage, WorkflowEventPageRequest, WorkflowInfoCursor,
    WorkflowInfoPage, WorkflowInfoPageRequest, WorkflowInstanceFilter, WorkflowVersionConflict,
};

pub struct MySqlStorage {
    pool: MySqlPool,
}

pub const ENV_HOST: &str = "RELAYFOLD_STORE_MYSQL_HOST";
pub const ENV_PORT: &str = "RELAYFOLD_STORE_MYSQL_PORT";
pub const ENV_DATABASE: &str = "RELAYFOLD_STORE_MYSQL_DATABASE";
pub const ENV_USERNAME: &str = "RELAYFOLD_STORE_MYSQL_USERNAME";
pub const ENV_PASSWORD: &str = "RELAYFOLD_STORE_MYSQL_PASSWORD";

pub struct MySqlStorageConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
}

impl MySqlStorage {
    pub async fn connect(config: MySqlStorageConfig) -> anyhow::Result<Self> {
        let options = MySqlConnectOptions::new()
            .host(&config.host)
            .port(config.port)
            .database(&config.database)
            .username(&config.username)
            .password(&config.password);

        let pool = MySqlPoolOptions::new()
            .max_connections(5)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET SESSION TRANSACTION ISOLATION LEVEL REPEATABLE READ")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await?;

        let storage = Self { pool };
        storage.run_migrations().await?;
        Ok(storage)
    }

    async fn run_migrations(&self) -> anyhow::Result<()> {
        sqlx::migrate!("./migrations/mysql").run(&self.pool).await?;
        Ok(())
    }
}

#[async_trait]
impl StoragePort for MySqlStorage {
    async fn save_workflow_def(
        &self,
        namespace: &Namespace,
        def: WorkflowDef,
    ) -> StorageResult<()> {
        let now = i64_from_u64(unix_timestamp_ms()?)?;
        let definition_json = serde_json::to_string(&def)?;
        sqlx::query(
            "INSERT INTO workflow_defs (
                namespace, id, description, definition_json, created_at_epoch_ms, updated_at_epoch_ms
             )
             VALUES (?, ?, ?, ?, ?, ?) AS new
             ON DUPLICATE KEY UPDATE
                description = new.description,
                definition_json = new.definition_json,
                updated_at_epoch_ms = new.updated_at_epoch_ms",
        )
        .bind(namespace.as_str())
        .bind(&def.id)
        .bind(&def.description)
        .bind(definition_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_workflow_def(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> StorageResult<Option<WorkflowDef>> {
        let row =
            sqlx::query("SELECT definition_json FROM workflow_defs WHERE namespace = ? AND id = ?")
                .bind(namespace.as_str())
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row
            .map(|row| deserialize_json(row.get::<String, _>("definition_json").as_str()))
            .transpose()?)
    }

    async fn list_workflow_def(
        &self,
        namespace: &Namespace,
    ) -> StorageResult<Vec<WorkflowDefSummary>> {
        let rows = sqlx::query(
            "SELECT
                wd.id,
                wd.description,
                wd.created_at_epoch_ms,
                MAX(wi.created_at_epoch_ms) AS last_invoked_at_epoch_ms
             FROM workflow_defs wd
             LEFT JOIN workflow_instances wi
                ON wi.namespace = wd.namespace AND wi.workflow_def_id = wd.id
             WHERE wd.namespace = ?
             GROUP BY wd.id, wd.description, wd.created_at_epoch_ms
             ORDER BY wd.created_at_epoch_ms DESC, wd.id DESC",
        )
        .bind(namespace.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(WorkflowDefSummary {
                    id: row.get("id"),
                    description: row.get("description"),
                    created_at_epoch_ms: u64_from_i64(row.get("created_at_epoch_ms"))?,
                    last_invoked_at_epoch_ms: row
                        .get::<Option<i64>, _>("last_invoked_at_epoch_ms")
                        .map(u64_from_i64)
                        .transpose()?,
                })
            })
            .collect()
    }

    async fn save_function_def(
        &self,
        namespace: &Namespace,
        def: FunctionDef,
    ) -> StorageResult<()> {
        let now = i64_from_u64(unix_timestamp_ms()?)?;
        let definition_json = serde_json::to_string(&def)?;
        sqlx::query(
            "INSERT INTO function_defs (
                namespace, id, definition_json, created_at_epoch_ms, updated_at_epoch_ms
             )
             VALUES (?, ?, ?, ?, ?) AS new
             ON DUPLICATE KEY UPDATE
                definition_json = new.definition_json,
                updated_at_epoch_ms = new.updated_at_epoch_ms",
        )
        .bind(namespace.as_str())
        .bind(&def.id)
        .bind(definition_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_function_def(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> StorageResult<Option<FunctionDef>> {
        let row =
            sqlx::query("SELECT definition_json FROM function_defs WHERE namespace = ? AND id = ?")
                .bind(namespace.as_str())
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row
            .map(|row| deserialize_json(row.get::<String, _>("definition_json").as_str()))
            .transpose()?)
    }

    async fn list_function_def(
        &self,
        namespace: &Namespace,
    ) -> StorageResult<Vec<FunctionDefSummary>> {
        let rows = sqlx::query("SELECT id FROM function_defs WHERE namespace = ? ORDER BY id")
            .bind(namespace.as_str())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| FunctionDefSummary { id: row.get("id") })
            .collect())
    }

    async fn delete_function_def(&self, namespace: &Namespace, id: &str) -> StorageResult<bool> {
        let result = sqlx::query("DELETE FROM function_defs WHERE namespace = ? AND id = ?")
            .bind(namespace.as_str())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn get_workflow_instance(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> StorageResult<Option<WorkflowInstance>> {
        let mut tx = self.pool.begin().await?;
        let Some(row) = sqlx::query(
            "SELECT id, workflow_def_id, version, status, trigger_input_json, pinned_worker_host_id
             FROM workflow_instances
             WHERE namespace = ? AND id = ?",
        )
        .bind(namespace.as_str())
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };

        let task_rows = sqlx::query(
            "SELECT task_attempt_id, task_def_id, status_json, satisfaction_status, generation_index,
                    human_input_json, early_exit, input_data_json, input_mapping_json, output_data_json,
                    verifier_metadata_json
             FROM workflow_tasks
             WHERE namespace = ? AND workflow_instance_id = ?",
        )
        .bind(namespace.as_str())
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        let mut tasks = HashMap::new();
        for row in task_rows {
            let task_attempt_id = row.get::<String, _>("task_attempt_id");
            let task = TaskInstance {
                early_exit: row.get("early_exit"),
                task_def_id: row.get("task_def_id"),
                status: deserialize_json(&row.get::<String, _>("status_json"))?,
                satisfaction_status: deserialize_json(
                    &row.get::<String, _>("satisfaction_status"),
                )?,
                human_input: optional_json(row.get::<Option<String>, _>("human_input_json"))?,
                input_data: deserialize_json(&row.get::<String, _>("input_data_json"))?,
                input_mapping: deserialize_json(&row.get::<String, _>("input_mapping_json"))?,
                output_data: optional_json(row.get::<Option<String>, _>("output_data_json"))?,
                generation_index: u32_from_i64(row.get::<i64, _>("generation_index"))?,
                verifier_metadata: optional_json(
                    row.get::<Option<String>, _>("verifier_metadata_json"),
                )?,
            };
            tasks.insert(task_attempt_id, task);
        }

        let verifier_rows = sqlx::query(
            "SELECT verifier_task_id, state_json
             FROM workflow_verifier_states
             WHERE namespace = ? AND workflow_instance_id = ?",
        )
        .bind(namespace.as_str())
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        let mut verifier_states = HashMap::new();
        for row in verifier_rows {
            verifier_states.insert(
                row.get::<String, _>("verifier_task_id"),
                deserialize_json(&row.get::<String, _>("state_json"))?,
            );
        }

        let instance = WorkflowInstance {
            id: row.get("id"),
            workflow_def_id: row.get("workflow_def_id"),
            version: u64_from_i64(row.get::<i64, _>("version"))?,
            status: workflow_status_from_name(&row.get::<String, _>("status"))?,
            trigger_input: optional_json(row.get::<Option<String>, _>("trigger_input_json"))?,
            pinned_worker_host: row
                .get::<Option<String>, _>("pinned_worker_host_id")
                .map(WorkerHostId),
            tasks,
            verifier_states,
        };
        tx.commit().await?;
        Ok(Some(instance))
    }

    async fn list_workflow_instance_events(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        page: WorkflowEventPageRequest,
    ) -> StorageResult<WorkflowEventPage> {
        if page.limit == 0 {
            return Ok(WorkflowEventPage {
                items: vec![],
                next_cursor: None,
            });
        }
        let rows = sqlx::query(
            "SELECT event_sequence, created_at_epoch_ms, event_json
             FROM workflow_events
             WHERE namespace = ? AND workflow_instance_id = ? AND event_sequence > ?
             ORDER BY event_sequence ASC
             LIMIT ?",
        )
        .bind(namespace.as_str())
        .bind(workflow_instance_id)
        .bind(i64_from_u64(page.cursor.unwrap_or(0))?)
        .bind(i64_from_usize(page.limit + 1)?)
        .fetch_all(&self.pool)
        .await?;

        let has_more = rows.len() > page.limit;
        let selected = rows.into_iter().take(page.limit).collect::<Vec<_>>();
        let next_cursor = has_more
            .then(|| selected.last())
            .flatten()
            .map(|row| u64_from_i64(row.get::<i64, _>("event_sequence")))
            .transpose()?;
        let events = selected
            .into_iter()
            .map(|row| {
                Ok(WorkflowEventRecord {
                    created_time: u64_from_i64(row.get::<i64, _>("created_at_epoch_ms"))?,
                    event: deserialize_json(&row.get::<String, _>("event_json"))?,
                })
            })
            .collect::<StorageResult<Vec<_>>>()?;
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
        if page.limit == 0
            || filters
                .iter()
                .any(|filter| matches!(filter, WorkflowInstanceFilter::Statuses(statuses) if statuses.is_empty()))
        {
            return Ok(WorkflowInfoPage {
                items: vec![],
                next_cursor: None,
            });
        }

        let mut conditions = Vec::new();
        if namespace.is_some() {
            conditions.push("wi.namespace = ?".to_string());
        }

        for filter in &filters {
            match filter {
                WorkflowInstanceFilter::Statuses(statuses) => {
                    let placeholders = vec!["?"; statuses.len()].join(", ");
                    conditions.push(format!("wi.status IN ({placeholders})"));
                }
                WorkflowInstanceFilter::WorkflowDefId(_) => {
                    conditions.push("wi.workflow_def_id = ?".to_string());
                }
            }
        }

        if page.cursor.is_some() {
            if namespace.is_some() {
                conditions.push(
                    "(wi.modified_at_epoch_ms < ?
                      OR (wi.modified_at_epoch_ms = ? AND wi.id < ?))"
                        .to_string(),
                );
            } else {
                conditions.push(
                    "(wi.modified_at_epoch_ms < ?
                      OR (wi.modified_at_epoch_ms = ? AND (
                        wi.id < ? OR (wi.id = ? AND wi.namespace < ?)
                      )))"
                    .to_string(),
                );
            }
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT
                wi.namespace,
                wi.id,
                wi.workflow_def_id,
                wi.status,
                wi.created_at_epoch_ms,
                wi.modified_at_epoch_ms,
                wi.completed_at_epoch_ms,
                CAST(COUNT(wt.task_attempt_id) AS SIGNED) AS total_task_count,
                CAST(COALESCE(SUM(CASE WHEN wt.status = 'completed' THEN 1 ELSE 0 END), 0) AS SIGNED) AS completed_task_count
             FROM workflow_instances wi
             LEFT JOIN workflow_tasks wt
                ON wt.namespace = wi.namespace AND wt.workflow_instance_id = wi.id
             {where_clause}
             GROUP BY wi.namespace, wi.id, wi.workflow_def_id, wi.status, wi.created_at_epoch_ms,
                      wi.modified_at_epoch_ms, wi.completed_at_epoch_ms
             ORDER BY wi.modified_at_epoch_ms DESC, wi.id DESC, wi.namespace DESC
             LIMIT ?"
        );

        let mut query = sqlx::query(&sql);

        if let Some(namespace) = namespace {
            query = query.bind(namespace.as_str());
        }
        for filter in &filters {
            match filter {
                WorkflowInstanceFilter::Statuses(statuses) => {
                    for status in statuses {
                        query = query.bind(workflow_status_name(status));
                    }
                }
                WorkflowInstanceFilter::WorkflowDefId(workflow_def_id) => {
                    query = query.bind(workflow_def_id);
                }
            }
        }

        if let Some(cursor) = &page.cursor {
            query = query
                .bind(i64_from_u64(cursor.modified_at_epoch_ms)?)
                .bind(i64_from_u64(cursor.modified_at_epoch_ms)?)
                .bind(&cursor.workflow_instance_id);

            if namespace.is_none() {
                query = query
                    .bind(&cursor.workflow_instance_id)
                    .bind(cursor.namespace.as_str());
            }
        }

        query = query.bind(i64_from_usize(page.limit + 1)?);

        let rows = query.fetch_all(&self.pool).await?;
        let has_more = rows.len() > page.limit;
        let workflows: Vec<WorkflowInfo> = rows
            .into_iter()
            .take(page.limit)
            .map(workflow_info_from_row)
            .collect::<anyhow::Result<Vec<_>>>()?;

        let next_cursor =
            has_more
                .then(|| workflows.last())
                .flatten()
                .map(|info| WorkflowInfoCursor {
                    namespace: info.namespace.clone(),
                    modified_at_epoch_ms: info.modified_at_epoch_ms,
                    workflow_instance_id: info.id.clone(),
                });

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
        let mut tx = self.pool.begin().await?;
        let workflow_instance_id = instance.id.clone();

        let created_from_events_at_epoch_ms = events
            .first()
            .map(|event| event.created_time)
            .unwrap_or(unix_timestamp_ms()?);

        let modified_at_epoch_ms = events
            .last()
            .map(|event| event.created_time)
            .unwrap_or(created_from_events_at_epoch_ms);

        let is_new_instance = claim_workflow_instance(
            &mut tx,
            namespace,
            &instance,
            created_from_events_at_epoch_ms,
            modified_at_epoch_ms,
        )
        .await?;

        let existing = sqlx::query(
            "SELECT version, created_at_epoch_ms, completed_at_epoch_ms
             FROM workflow_instances
             WHERE namespace = ? AND id = ?
             FOR UPDATE",
        )
        .bind(namespace.as_str())
        .bind(&workflow_instance_id)
        .fetch_one(&mut *tx)
        .await?;

        let actual_version = u64_from_i64(existing.get::<i64, _>("version"))?;

        if actual_version != expected_version {
            return Err(WorkflowVersionConflict {
                workflow_instance_id,
                expected_version,
                actual_version,
            }
            .into());
        }

        let created_at_epoch_ms = u64_from_i64(existing.get::<i64, _>("created_at_epoch_ms"))?;

        let completed_at_epoch_ms = existing
            .get::<Option<i64>, _>("completed_at_epoch_ms")
            .map(u64_from_i64)
            .transpose()?
            .or_else(|| workflow_completed_at(&instance, modified_at_epoch_ms));

        insert_events(&mut tx, namespace, expected_version, &instance.id, &events).await?;

        upsert_workflow_instance(
            &mut tx,
            namespace,
            &instance,
            created_at_epoch_ms,
            modified_at_epoch_ms,
            completed_at_epoch_ms,
        )
        .await?;
        persist_task_changes(&mut tx, namespace, &instance, &events, is_new_instance).await?;
        replace_verifier_states(&mut tx, namespace, &instance).await?;

        tx.commit().await?;
        Ok(())
    }
}

async fn claim_workflow_instance(
    tx: &mut Transaction<'_, MySql>,
    namespace: &Namespace,
    instance: &WorkflowInstance,
    created_at_epoch_ms: u64,
    modified_at_epoch_ms: u64,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "INSERT INTO workflow_instances (
            namespace, id, workflow_def_id, version, status, trigger_input_json, pinned_worker_host_id,
            created_at_epoch_ms, modified_at_epoch_ms, completed_at_epoch_ms
         )
         VALUES (?, ?, ?, 0, ?, ?, ?, ?, ?, ?)
         ON DUPLICATE KEY UPDATE id = id",
    )
    .bind(namespace.as_str())
    .bind(&instance.id)
    .bind(&instance.workflow_def_id)
    .bind(workflow_status_name(&instance.status))
    .bind(optional_json_string(&instance.trigger_input)?)
    .bind(
        instance
            .pinned_worker_host
            .as_ref()
            .map(|host| host.0.as_str()),
    )
    .bind(i64_from_u64(created_at_epoch_ms)?)
    .bind(i64_from_u64(modified_at_epoch_ms)?)
    .bind(optional_i64_from_u64(workflow_completed_at(
        instance,
        modified_at_epoch_ms,
    ))?)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() == 1)
}

async fn insert_events(
    tx: &mut Transaction<'_, MySql>,
    namespace: &Namespace,
    expected_version: u64,
    workflow_instance_id: &str,
    events: &[WorkflowEventRecord],
) -> anyhow::Result<()> {
    for (index, event) in events.iter().enumerate() {
        sqlx::query(
            "INSERT INTO workflow_events (
                namespace, workflow_instance_id, event_sequence, created_at_epoch_ms, event_json
             )
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(namespace.as_str())
        .bind(workflow_instance_id)
        .bind(i64_from_u64(expected_version + index as u64 + 1)?)
        .bind(i64_from_u64(event.created_time)?)
        .bind(serde_json::to_string(&event.event)?)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn upsert_workflow_instance(
    tx: &mut Transaction<'_, MySql>,
    namespace: &Namespace,
    instance: &WorkflowInstance,
    created_at_epoch_ms: u64,
    modified_at_epoch_ms: u64,
    completed_at_epoch_ms: Option<u64>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO workflow_instances (
            namespace, id, workflow_def_id, version, status, trigger_input_json, pinned_worker_host_id,
            created_at_epoch_ms, modified_at_epoch_ms, completed_at_epoch_ms
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) AS new
         ON DUPLICATE KEY UPDATE
            workflow_def_id = new.workflow_def_id,
            version = new.version,
            status = new.status,
            trigger_input_json = new.trigger_input_json,

            pinned_worker_host_id = new.pinned_worker_host_id,
            modified_at_epoch_ms = new.modified_at_epoch_ms,
            completed_at_epoch_ms = new.completed_at_epoch_ms",
    )
    .bind(namespace.as_str())
    .bind(&instance.id)
    .bind(&instance.workflow_def_id)
    .bind(i64_from_u64(instance.version)?)
    .bind(workflow_status_name(&instance.status))
    .bind(optional_json_string(&instance.trigger_input)?)
    .bind(
        instance
            .pinned_worker_host
            .as_ref()
            .map(|host| host.0.as_str()),
    )
    .bind(i64_from_u64(created_at_epoch_ms)?)
    .bind(i64_from_u64(modified_at_epoch_ms)?)
    .bind(optional_i64_from_u64(completed_at_epoch_ms)?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn persist_task_changes(
    tx: &mut Transaction<'_, MySql>,
    namespace: &Namespace,
    instance: &WorkflowInstance,
    events: &[WorkflowEventRecord],
    is_new_instance: bool,
) -> anyhow::Result<()> {
    let task_attempt_ids = if is_new_instance {
        instance.tasks.keys().cloned().collect()
    } else {
        changed_task_attempt_ids(events)
    };

    for task_attempt_id in task_attempt_ids {
        let task = instance.tasks.get(&task_attempt_id).ok_or_else(|| {
            anyhow::anyhow!(
                "event identified task attempt {task_attempt_id} but it is missing from workflow instance {}",
                instance.id
            )
        })?;
        upsert_task(tx, namespace, &instance.id, &task_attempt_id, task).await?;
    }
    Ok(())
}

async fn upsert_task(
    tx: &mut Transaction<'_, MySql>,
    namespace: &Namespace,
    workflow_instance_id: &str,
    task_attempt_id: &str,
    task: &TaskInstance,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO workflow_tasks (
                namespace, workflow_instance_id, task_attempt_id, task_def_id, status, status_json,
                satisfaction_status, generation_index, human_input_json, early_exit, input_data_json,
                input_mapping_json, output_data_json, verifier_metadata_json
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) AS new
             ON DUPLICATE KEY UPDATE
                task_def_id = new.task_def_id,
                status = new.status,
                status_json = new.status_json,
                satisfaction_status = new.satisfaction_status,
                generation_index = new.generation_index,
                human_input_json = new.human_input_json,
                early_exit = new.early_exit,
                input_data_json = new.input_data_json,
                input_mapping_json = new.input_mapping_json,
                output_data_json = new.output_data_json,
                verifier_metadata_json = new.verifier_metadata_json",
    )
    .bind(namespace.as_str())
    .bind(workflow_instance_id)
    .bind(task_attempt_id)
    .bind(&task.task_def_id)
    .bind(task_status_name(&task.status))
    .bind(serde_json::to_string(&task.status)?)
    .bind(serde_json::to_string(&task.satisfaction_status)?)
    .bind(i64::from(task.generation_index))
    .bind(optional_json_string(&task.human_input)?)
    .bind(task.early_exit)
    .bind(serde_json::to_string(&task.input_data)?)
    .bind(serde_json::to_string(&task.input_mapping)?)
    .bind(optional_json_string(&task.output_data)?)
    .bind(optional_json_string(&task.verifier_metadata)?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn replace_verifier_states(
    tx: &mut Transaction<'_, MySql>,
    namespace: &Namespace,
    instance: &WorkflowInstance,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM workflow_verifier_states
         WHERE namespace = ? AND workflow_instance_id = ?",
    )
    .bind(namespace.as_str())
    .bind(&instance.id)
    .execute(&mut **tx)
    .await?;

    for (verifier_task_id, state) in &instance.verifier_states {
        sqlx::query(
            "INSERT INTO workflow_verifier_states (
                namespace, workflow_instance_id, verifier_task_id, state_json
             )
             VALUES (?, ?, ?, ?)",
        )
        .bind(namespace.as_str())
        .bind(&instance.id)
        .bind(verifier_task_id)
        .bind(serde_json::to_string(state)?)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

fn workflow_info_from_row(row: sqlx::mysql::MySqlRow) -> anyhow::Result<WorkflowInfo> {
    Ok(WorkflowInfo {
        namespace: Namespace::new(row.get::<String, _>("namespace"))?,
        id: row.get("id"),
        workflow_def_id: row.get("workflow_def_id"),
        created_at_epoch_ms: row
            .get::<Option<i64>, _>("created_at_epoch_ms")
            .map(u64_from_i64)
            .transpose()?,
        modified_at_epoch_ms: u64_from_i64(row.get::<i64, _>("modified_at_epoch_ms"))?,
        completed_at_epoch_ms: row
            .get::<Option<i64>, _>("completed_at_epoch_ms")
            .map(u64_from_i64)
            .transpose()?,
        status: workflow_status_from_name(&row.get::<String, _>("status"))?,
        total_task_count: usize_from_i64(row.get::<i64, _>("total_task_count"))?,
        completed_task_count: usize_from_i64(row.get::<i64, _>("completed_task_count"))?,
    })
}
