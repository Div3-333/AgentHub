use super::Transport;
use crate::error::{McpError, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

pub struct StdioTransport {
    stdin: Arc<Mutex<ChildStdin>>,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    _child: Child,
}

impl StdioTransport {
    pub fn new(child: Child, stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
            _child: child,
        }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send(&self, msg: String) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(msg.as_bytes())
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        Ok(())
    }

    async fn recv(&self) -> Result<String> {
        let mut stdout = self.stdout.lock().await;
        let mut line = String::new();
        stdout
            .read_line(&mut line)
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        Ok(line)
    }
}
