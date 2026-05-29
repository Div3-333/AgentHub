use thiserror::Error;

pub type Result<T> = std::result::Result<T, McpError>;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("rpc: {0}")]
    Rpc(String),
    #[error("transport: {0}")]
    Transport(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
