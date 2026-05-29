use super::Transport;
use crate::error::{McpError, Result};
use async_trait::async_trait;
use reqwest::Client;

pub struct SseTransport {
    client: Client,
    endpoint: String,
}

impl SseTransport {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            endpoint: endpoint.into(),
        }
    }
}

#[async_trait]
impl Transport for SseTransport {
    async fn send(&self, msg: String) -> Result<()> {
        self.client
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .body(msg)
            .send()
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn recv(&self) -> Result<String> {
        let resp = self
            .client
            .get(&self.endpoint)
            .send()
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        let body = resp
            .text()
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        Ok(body)
    }
}
