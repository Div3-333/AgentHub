use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GraphId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Pending,
    Running,
    Yielded,
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionState {
    pub graph_id: GraphId,
    pub node_states: HashMap<NodeId, NodeStatus>,
    pub outputs: HashMap<NodeId, serde_json::Value>,
}

impl ExecutionState {
    pub fn new(graph_id: GraphId) -> Self {
        Self {
            graph_id,
            node_states: HashMap::new(),
            outputs: HashMap::new(),
        }
    }
}
