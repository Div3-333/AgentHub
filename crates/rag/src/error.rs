use thiserror::Error;

pub type Result<T> = std::result::Result<T, RagError>;

#[derive(Debug, Error)]
pub enum RagError {
    #[error("index: {0}")]
    Index(String),
    #[error("embed: {0}")]
    Embed(String),
}
