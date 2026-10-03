use crate::core::namespace::Namespace;
use crate::core::task::{ExecutionMetadata, TaskDef};
use crate::core::worker::TaskDispatchConstraints;
use crate::ports::task_dispatch::{ExecutionResult, TaskDispatchPort};
use async_trait::async_trait;
use serde_json::Value;

/// A deterministic, in-process dispatcher that generates schema-conformant default
/// output from a `TaskDef`'s `output_schema`. Used in unit tests and dry-runs.
#[allow(dead_code)]
pub struct FakeTaskDispatcher;

impl FakeTaskDispatcher {
    #[allow(dead_code)]
    pub fn new() -> Self {
        FakeTaskDispatcher
    }
}

impl Default for FakeTaskDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Recursively walk a JSON Schema and produce a default value that conforms to it.
///
/// Rules:
/// - `"type": "object"` → `{}` unless `required` fields are listed, in which case
///   each required field is generated from its entry in `properties`.
/// - `"type": "string"` → `""`
/// - `"type": "integer"` | `"number"` → `0`
/// - `"type": "boolean"` → `false`
/// - `"type": "array"` → `[]`
/// - `"type": "null"` → `null`
/// - No `type` field, or unsupported constructs (`oneOf`, `anyOf`, `$ref`) → `{}`
fn schema_default(schema: &Value) -> Value {
    if let Some(first_enum_value) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    {
        return first_enum_value.clone();
    }

    let type_str = schema.get("type").and_then(Value::as_str);

    match type_str {
        Some("object") => {
            let required = schema
                .get("required")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            if required.is_empty() {
                return Value::Object(serde_json::Map::new());
            }

            let properties = schema.get("properties");
            let mut map = serde_json::Map::new();

            for field in &required {
                if let Some(field_name) = field.as_str() {
                    let field_schema = properties
                        .and_then(|p| p.get(field_name))
                        .cloned()
                        .unwrap_or(Value::Object(serde_json::Map::new()));
                    map.insert(field_name.to_string(), schema_default(&field_schema));
                }
            }

            Value::Object(map)
        }
        Some("string") => Value::String(String::new()),
        Some("integer") | Some("number") => Value::Number(0.into()),
        Some("boolean") => Value::Bool(false),
        Some("array") => Value::Array(vec![]),
        Some("null") => Value::Null,
        // No type or unsupported constructs (oneOf, anyOf, $ref, …) → graceful fallback
        _ => Value::Object(serde_json::Map::new()),
    }
}

#[async_trait]
impl TaskDispatchPort for FakeTaskDispatcher {
    async fn dispatch_task(
        &self,
        _namespace: &Namespace,
        _workflow_inst_id: &str,
        task: &TaskDef,
        _inputs: &[Value],
        _metadata: &ExecutionMetadata,
        _dispatch: &TaskDispatchConstraints,
    ) -> anyhow::Result<ExecutionResult> {
        Ok(ExecutionResult::Success(match &task.output_schema {
            Some(schema) => schema_default(schema),
            None => Value::Null,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fake() -> FakeTaskDispatcher {
        FakeTaskDispatcher::new()
    }

    fn task_with_schema(schema: Value) -> TaskDef {
        TaskDef {
            sandbox: None,
            id: "test-task".to_string(),
            kind: crate::core::task::TaskTypeDef::ApiCall {
                url: "http://example.com".to_string(),
                method: "GET".to_string(),
                headers: std::collections::HashMap::new(),
            },
            control: None,
            timeout_secs: None,
            input_schemas: vec![],
            output_schema: Some(schema),
            workspace: None,
            required_credentials: vec![],
        }
    }

    #[tokio::test]
    async fn test_empty_object_schema() {
        let task = task_with_schema(json!({"type": "object"}));
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match result {
            ExecutionResult::Success(val) => assert_eq!(val, json!({})),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_object_with_required_fields() {
        let task = task_with_schema(json!({
            "type": "object",
            "required": ["name", "count"],
            "properties": {
                "name": {"type": "string"},
                "count": {"type": "integer"}
            }
        }));
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match result {
            ExecutionResult::Success(val) => {
                assert_eq!(val, json!({"name": "", "count": 0}))
            }
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_string_schema() {
        let task = task_with_schema(json!({"type": "string"}));
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match result {
            ExecutionResult::Success(val) => assert_eq!(val, json!("")),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_integer_schema() {
        let task = task_with_schema(json!({"type": "integer"}));
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match result {
            ExecutionResult::Success(val) => assert_eq!(val, json!(0)),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_number_schema() {
        let task = task_with_schema(json!({"type": "number"}));
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match result {
            ExecutionResult::Success(val) => assert_eq!(val, json!(0)),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_boolean_schema() {
        let task = task_with_schema(json!({"type": "boolean"}));
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match result {
            ExecutionResult::Success(val) => assert_eq!(val, json!(false)),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_array_schema() {
        let task = task_with_schema(json!({"type": "array"}));
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match result {
            ExecutionResult::Success(val) => assert_eq!(val, json!([])),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_null_schema() {
        let task = task_with_schema(json!({"type": "null"}));
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match result {
            ExecutionResult::Success(val) => assert_eq!(val, Value::Null),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_no_type_schema() {
        let task = task_with_schema(json!({}));
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match result {
            ExecutionResult::Success(val) => assert_eq!(val, json!({})),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_one_of_fallback() {
        let task = task_with_schema(json!({
            "oneOf": [{"type": "string"}, {"type": "integer"}]
        }));
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match result {
            ExecutionResult::Success(val) => assert_eq!(val, json!({})),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_inputs_do_not_affect_output() {
        let task = task_with_schema(json!({"type": "string"}));
        let res1 = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        let res2 = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[json!("anything"), json!(42)],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        match (res1, res2) {
            (ExecutionResult::Success(v1), ExecutionResult::Success(v2)) => assert_eq!(v1, v2),
            _ => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_output_valid_against_strict_schema() {
        let schema = json!({
            "type": "object",
            "required": ["label", "score"],
            "properties": {
                "label": {"type": "string"},
                "score": {"type": "number"}
            },
            "additionalProperties": false
        });
        let task = task_with_schema(schema.clone());
        let result = fake()
            .dispatch_task(
                &crate::core::namespace::test_namespace(),
                "123",
                &task,
                &[],
                &ExecutionMetadata::default(),
                &crate::core::worker::TaskDispatchConstraints::default(),
            )
            .await
            .unwrap();
        let output = match result {
            ExecutionResult::Success(val) => val,
            _ => panic!("Expected Success"),
        };

        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(
            validator.is_valid(&output),
            "output did not satisfy schema: {output}"
        );
    }
}
