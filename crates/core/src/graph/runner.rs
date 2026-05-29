use super::edge::Edge;
use super::node::NodeExecutor;
use super::types::{GraphId, NodeId, NodeStatus};
use crate::error::{GraphError, Result};
use crate::graph::types::ExecutionState;
use petgraph::algo::is_cyclic_directed;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum GraphEvent {
    NodeStarted(NodeId),
    NodeCompleted(NodeId),
    NodeFailed(NodeId, String),
}

pub struct GraphRunner {
    graph: DiGraph<NodeId, ()>,
    node_indices: HashMap<NodeId, NodeIndex>,
    executors: HashMap<NodeId, Arc<dyn NodeExecutor>>,
    edges: Vec<Edge>,
    state: Arc<RwLock<ExecutionState>>,
    tx: broadcast::Sender<GraphEvent>,
}

impl GraphRunner {
    pub fn new(
        graph_id: GraphId,
        nodes: Vec<(NodeId, Arc<dyn NodeExecutor>)>,
        edges: Vec<Edge>,
        tx: broadcast::Sender<GraphEvent>,
    ) -> Result<Self> {
        let mut graph = DiGraph::new();
        let mut node_indices = HashMap::new();
        let mut executors = HashMap::new();

        for (id, executor) in nodes {
            let idx = graph.add_node(id);
            node_indices.insert(id, idx);
            executors.insert(id, executor);
        }

        for edge in &edges {
            let from = *node_indices
                .get(&edge.source)
                .ok_or_else(|| GraphError::NodeNotFound(edge.source.0.to_string()))?;
            let to = *node_indices
                .get(&edge.target)
                .ok_or_else(|| GraphError::NodeNotFound(edge.target.0.to_string()))?;
            graph.add_edge(from, to, ());
        }

        if is_cyclic_directed(&graph) {
            return Err(GraphError::CycleDetected.into());
        }

        Ok(Self {
            graph,
            node_indices,
            executors,
            edges,
            state: Arc::new(RwLock::new(ExecutionState::new(graph_id))),
            tx,
        })
    }

    pub fn topological_sort(&self) -> Result<Vec<NodeId>> {
        let mut indegree: HashMap<NodeIndex, usize> = HashMap::new();
        for idx in self.graph.node_indices() {
            indegree.insert(idx, self.graph.neighbors_directed(idx, Direction::Incoming).count());
        }

        let mut queue: VecDeque<NodeIndex> = indegree
            .iter()
            .filter_map(|(&idx, &deg)| if deg == 0 { Some(idx) } else { None })
            .collect();

        let mut order = Vec::new();
        while let Some(idx) = queue.pop_front() {
            order.push(self.graph[idx]);
            for neighbor in self.graph.neighbors_directed(idx, Direction::Outgoing) {
                let entry = indegree.get_mut(&neighbor).unwrap();
                *entry -= 1;
                if *entry == 0 {
                    queue.push_back(neighbor);
                }
            }
        }

        if order.len() != self.graph.node_count() {
            return Err(GraphError::CycleDetected.into());
        }

        Ok(order)
    }

    pub fn get_ready_nodes(&self) -> Vec<NodeId> {
        let state = self.state.read().expect("state lock poisoned");
        let mut ready = Vec::new();

        for node_id in self.graph.node_indices().map(|i| self.graph[i]) {
            let status = state
                .node_states
                .get(&node_id)
                .cloned()
                .unwrap_or(NodeStatus::Pending);

            if !matches!(status, NodeStatus::Pending) {
                continue;
            }

            let idx = self.node_indices[&node_id];
            let deps_done = self
                .graph
                .neighbors_directed(idx, Direction::Incoming)
                .all(|dep| {
                    matches!(
                        state.node_states.get(&self.graph[dep]),
                        Some(NodeStatus::Completed)
                    )
                });

            if deps_done {
                ready.push(node_id);
            }
        }

        ready
    }

    pub async fn execute_node(&self, id: NodeId) -> Result<()> {
        let executor = self
            .executors
            .get(&id)
            .cloned()
            .ok_or_else(|| GraphError::NodeNotFound(id.0.to_string()))?;

        {
            let mut state = self.state.write().expect("state lock poisoned");
            state.node_states.insert(id, NodeStatus::Running);
        }
        let _ = self.tx.send(GraphEvent::NodeStarted(id));

        let inputs = serde_json::json!({});
        match executor.execute(&super::node::ExecutionContext {
            graph_id: self.state.read().unwrap().graph_id,
            node_id: id,
        }, inputs).await {
            Ok(output) => {
                let mut state = self.state.write().expect("state lock poisoned");
                state.outputs.insert(id, output);
                state.node_states.insert(id, NodeStatus::Completed);
                let _ = self.tx.send(GraphEvent::NodeCompleted(id));
            }
            Err(e) => {
                let msg = e.to_string();
                let mut state = self.state.write().expect("state lock poisoned");
                state
                    .node_states
                    .insert(id, NodeStatus::Failed(msg.clone()));
                let _ = self.tx.send(GraphEvent::NodeFailed(id, msg));
            }
        }

        Ok(())
    }

    pub async fn step(&mut self) -> Result<bool> {
        let ready = self.get_ready_nodes();
        if ready.is_empty() {
            return Ok(false);
        }
        for node in ready {
            self.execute_node(node).await?;
        }
        Ok(true)
    }

    pub async fn start(&mut self) -> Result<()> {
        let order = self.topological_sort()?;
        for node in order {
            if !matches!(
                self.state.read().unwrap().node_states.get(&node),
                Some(NodeStatus::Completed)
            ) {
                self.execute_node(node).await?;
            }
        }
        Ok(())
    }

    pub fn evaluate_edge_condition(&self, condition: &str, output: &serde_json::Value) -> bool {
        if condition.is_empty() {
            return true;
        }
        if condition.contains("true") {
            return output.as_bool().unwrap_or(true);
        }
        if condition.contains("false") {
            return output.as_bool().unwrap_or(false);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::node::{LlmConfig, NodeType};
    use async_trait::async_trait;
    use serde_json::Value;
    use uuid::Uuid;

    struct EchoExecutor {
        node_type: NodeType,
    }

    #[async_trait]
    impl NodeExecutor for EchoExecutor {
        async fn execute(&self, _ctx: &super::super::node::ExecutionContext, inputs: Value) -> Result<Value> {
            Ok(inputs)
        }

        fn node_type(&self) -> NodeType {
            self.node_type.clone()
        }
    }

    fn nid() -> NodeId {
        NodeId(Uuid::new_v4())
    }

    fn eid() -> super::super::types::EdgeId {
        super::super::types::EdgeId(Uuid::new_v4())
    }

    #[tokio::test]
    async fn test_linear_graph_execution() {
        let a = nid();
        let b = nid();
        let (tx, _rx) = broadcast::channel(16);
        let nodes = vec![
            (
                a,
                Arc::new(EchoExecutor {
                    node_type: NodeType::Llm(LlmConfig {
                        provider: "test".into(),
                        model: "test".into(),
                        system_prompt: None,
                    }),
                }),
            ),
            (
                b,
                Arc::new(EchoExecutor {
                    node_type: NodeType::Llm(LlmConfig {
                        provider: "test".into(),
                        model: "test".into(),
                        system_prompt: None,
                    }),
                }),
            ),
        ];
        let edges = vec![Edge {
            id: eid(),
            source: a,
            target: b,
            condition: None,
            mapping: None,
        }];
        let mut runner = GraphRunner::new(GraphId(Uuid::new_v4()), nodes, edges, tx).unwrap();
        runner.start().await.unwrap();
        assert!(matches!(
            runner.state.read().unwrap().node_states.get(&b),
            Some(NodeStatus::Completed)
        ));
    }

    #[tokio::test]
    async fn test_cycle_detection_fails_validation() {
        let a = nid();
        let b = nid();
        let (tx, _rx) = broadcast::channel(16);
        let nodes = vec![
            (
                a,
                Arc::new(EchoExecutor {
                    node_type: NodeType::Llm(LlmConfig {
                        provider: "t".into(),
                        model: "t".into(),
                        system_prompt: None,
                    }),
                }),
            ),
            (
                b,
                Arc::new(EchoExecutor {
                    node_type: NodeType::Llm(LlmConfig {
                        provider: "t".into(),
                        model: "t".into(),
                        system_prompt: None,
                    }),
                }),
            ),
        ];
        let edges = vec![
            Edge {
                id: eid(),
                source: a,
                target: b,
                condition: None,
                mapping: None,
            },
            Edge {
                id: eid(),
                source: b,
                target: a,
                condition: None,
                mapping: None,
            },
        ];
        assert!(GraphRunner::new(GraphId(Uuid::new_v4()), nodes, edges, tx).is_err());
    }
}
