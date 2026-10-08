use std::sync::Arc;

use crate::core::function::models::{FunctionDef, FunctionDefSummary, FunctionTaskDef};
use crate::core::namespace::Namespace;
use crate::core::task::{TaskDef, TaskTypeDef};
use crate::ports::storage::StoragePort;

pub struct FunctionService {
    storage: Arc<dyn StoragePort + Send + Sync>,
}

impl FunctionService {
    pub fn new(storage: Arc<dyn StoragePort + Send + Sync>) -> Self {
        Self { storage }
    }

    pub async fn list_function_defs(
        &self,
        namespace: &Namespace,
    ) -> anyhow::Result<Vec<FunctionDefSummary>> {
        Ok(self.storage.list_function_def(namespace).await?)
    }

    pub async fn get_function_def(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> anyhow::Result<Option<FunctionDef>> {
        Ok(self.storage.get_function_def(namespace, id).await?)
    }

    pub async fn create_function_def(
        &self,
        namespace: &Namespace,
        def: FunctionDef,
    ) -> anyhow::Result<()> {
        self.storage.save_function_def(namespace, def).await?;
        Ok(())
    }

    pub async fn delete_function_def(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> anyhow::Result<bool> {
        Ok(self.storage.delete_function_def(namespace, id).await?)
    }
}

pub async fn resolve_task_function_ref(
    storage: &(dyn StoragePort + Send + Sync),
    namespace: &Namespace,
    task: &TaskDef,
) -> anyhow::Result<TaskDef> {
    let TaskTypeDef::Function(FunctionTaskDef::Ref { reference }) = &task.kind else {
        return Ok(task.clone());
    };

    let Some(function_def) = storage.get_function_def(namespace, reference).await? else {
        anyhow::bail!("Function definition not found: {reference}");
    };

    let mut resolved = task.clone();
    resolved.kind = TaskTypeDef::Function(FunctionTaskDef::Inline {
        dependencies: function_def.dependencies,
        code: function_def.code,
    });
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::memory_storage::MemoryStorage;
    use crate::adapters::sqlite_storage::SqliteStorage;
    use crate::core::function::models::FunctionDependency;

    #[tokio::test]
    async fn lists_summaries_and_fetches_content_with_namespace_isolation() {
        let stores: Vec<Arc<dyn StoragePort + Send + Sync>> = vec![
            Arc::new(MemoryStorage::new()),
            Arc::new(SqliteStorage::connect_in_memory().await.unwrap()),
        ];
        let first = Namespace::new("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let second = Namespace::new("550e8400-e29b-41d4-a716-446655440001").unwrap();
        for storage in stores {
            let service = FunctionService::new(storage);
            assert!(service.list_function_defs(&first).await.unwrap().is_empty());
            for (namespace, id, code) in [
                (&first, "z-last", "old"),
                (&first, "a-first", "first"),
                (&second, "z-last", "foreign"),
            ] {
                service
                    .create_function_def(
                        namespace,
                        FunctionDef {
                            id: id.to_string(),
                            dependencies: vec![],
                            code: code.to_string(),
                        },
                    )
                    .await
                    .unwrap();
            }
            service
                .create_function_def(
                    &first,
                    FunctionDef {
                        id: "z-last".to_string(),
                        dependencies: vec![FunctionDependency {
                            name: "lodash-es".to_string(),
                            version: "4.17.21".to_string(),
                        }],
                        code: "updated".to_string(),
                    },
                )
                .await
                .unwrap();
            let definitions = service.list_function_defs(&first).await.unwrap();
            assert_eq!(
                definitions
                    .iter()
                    .map(|def| def.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["a-first", "z-last"]
            );
            let definition = service
                .get_function_def(&first, "z-last")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(definition.code, "updated");
            assert_eq!(definition.dependencies[0].name, "lodash-es");
            assert_eq!(definition.dependencies[0].version, "4.17.21");
            assert!(
                service
                    .get_function_def(&second, "a-first")
                    .await
                    .unwrap()
                    .is_none()
            );
            let definitions = service.list_function_defs(&second).await.unwrap();
            assert_eq!(definitions.len(), 1);
            assert_eq!(
                service
                    .get_function_def(&second, "z-last")
                    .await
                    .unwrap()
                    .unwrap()
                    .code,
                "foreign"
            );
        }
    }
}
