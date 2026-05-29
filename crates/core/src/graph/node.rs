use super::types::NodeId;
use crate::error::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    pub mcp_server: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    pub expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanConfig {
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeType {
    Llm(LlmConfig),
    Tool(ToolConfig),
    Router(RouterConfig),
    Human(HumanConfig),
}

pub struct ExecutionContext {
    pub graph_id: super::types::GraphId,
    pub node_id: NodeId,
}

#[async_trait]
pub trait NodeExecutor: Send + Sync {
    async fn execute(&self, ctx: &ExecutionContext, inputs: Value) -> Result<Value>;
    fn node_type(&self) -> NodeType;
    fn validate_inputs(&self, inputs: &Value) -> Result<()> {
        let _ = inputs;
        Ok(())
    }
}
