use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDependency {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FunctionTaskDef {
    Inline {
        // Task will attempt to download these dependencies
        dependencies: Vec<FunctionDependency>,
        code: String,
    },
    Ref {
        #[serde(rename = "ref")]
        reference: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub id: String,
    pub dependencies: Vec<FunctionDependency>,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefSummary {
    pub id: String,
}
