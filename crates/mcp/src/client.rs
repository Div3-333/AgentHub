use crate::error::{McpError, Result};
use crate::jsonrpc::{Request, Response};
use crate::transport::Transport;
use crate::types::{
    CallToolRequestParams, CallToolResult, InitializeRequestParams, InitializeResult, Tool,
};
use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::oneshot;

pub struct McpClient {
    transport: Box<dyn Transport>,
    request_id: AtomicU64,
    pending_requests: Arc<DashMap<u64, oneshot::Sender<Response>>>,
}

impl McpClient {
    pub async fn connect(transport: Box<dyn Transport>) -> Result<Self> {
        Ok(Self {
            transport,
            request_id: AtomicU64::new(1),
            pending_requests: Arc::new(DashMap::new()),
        })
    }

    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id();
        let req = Request {
            jsonrpc: "2.0".into(),
            id: json!(id),
            method: method.into(),
            params,
        };
        let payload = serde_json::to_string(&req).map_err(|e| McpError::Rpc(e.to_string()))?;
        self.transport.send(payload).await?;
        let line = self.transport.recv().await?;
        let resp: Response =
            serde_json::from_str(&line).map_err(|e| McpError::Rpc(e.to_string()))?;
        resp.result
            .ok_or_else(|| McpError::Rpc("missing result".into()))
    }

    pub async fn initialize(&self) -> Result<InitializeResult> {
        let params = InitializeRequestParams {
            protocol_version: "2024-11-05".into(),
            capabilities: Default::default(),
            client_info: crate::types::Implementation {
                name: "agenthub".into(),
                version: "0.1.0".into(),
            },
        };
        let value = self
            .call("initialize", Some(serde_json::to_value(params).unwrap()))
            .await?;
        serde_json::from_value(value).map_err(|e| McpError::Rpc(e.to_string()))
    }

    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        let value = self.call("tools/list", None).await?;
        let tools = value
            .get("tools")
            .cloned()
            .unwrap_or(Value::Array(vec![]));
        serde_json::from_value(tools).map_err(|e| McpError::Rpc(e.to_string()))
    }

    pub async fn call_tool(&self, name: &str, args: Value) -> Result<CallToolResult> {
        let params = CallToolRequestParams {
            name: name.into(),
            arguments: args,
        };
        let value = self
            .call("tools/call", Some(serde_json::to_value(params).unwrap()))
            .await?;
        serde_json::from_value(value).map_err(|e| McpError::Rpc(e.to_string()))
    }
}
