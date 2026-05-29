use super::types::{EdgeId, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
    pub condition: Option<String>,
    pub mapping: Option<Value>,
}
