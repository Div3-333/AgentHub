use thiserror::Error;

pub type Result<T> = std::result::Result<T, ProviderError>;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("http: {0}")]
    Http(String),
    #[error("api: {0}")]
    Api(String),
}
