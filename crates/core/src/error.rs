use thiserror::Error;

pub type Result<T> = std::result::Result<T, AgentHubError>;

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("cycle detected in graph")]
    CycleDetected,
    #[error("node not found: {0}")]
    NodeNotFound(String),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
}

#[derive(Debug, Error)]
pub enum McpError {
    #[error("mcp error: {0}")]
    Message(String),
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider error: {0}")]
    Message(String),
}

#[derive(Debug, Error)]
pub enum AgentHubError {
    #[error("config: {0}")]
    Config(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Mcp(#[from] McpError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
