pub mod sse;
pub mod stdio;

use crate::error::Result;
use async_trait::async_trait;

#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, msg: String) -> Result<()>;
    async fn recv(&self) -> Result<String>;
}
