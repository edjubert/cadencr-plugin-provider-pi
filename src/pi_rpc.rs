//! Pi RPC client: spawns `pi --mode rpc` and communicates over stdio
//! using newline-delimited JSON-RPC 2.0.
//!
//! Pi's RPC mode exposes methods like `get_available_models`, `get_state`,
//! `set_model`, `prompt`, `abort`, `get_session_stats`, and emits events
//! like `message`, `tool_execution_start`, `tool_execution_update`,
//! `tool_execution_end`, `permission_request`, `thinking_level_change`,
//! `model_change`, `compaction_start`, `compaction_end`, `session_stats`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Pending JSON-RPC requests: id → response sender.
type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// Shared event receiver, taken once by the session event loop.
type EventReceiver = Arc<Mutex<Option<mpsc::UnboundedReceiver<(String, Value)>>>>;

/// A handle to a running `pi --mode rpc` child process.
///
/// Cloneable: all fields are behind `Arc`, so clones share the same
/// child process, stdin, and pending-request map.
#[derive(Clone)]
pub struct PiRpc {
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: Arc<AtomicU64>,
    /// Pending requests: id -> oneshot sender for the response.
    pending: PendingRequests,
    /// Event channel: receives Pi RPC notifications (method + params).
    /// Taken by the session event loop; `None` once consumed.
    events: EventReceiver,
    /// Shutdown signal.
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

/// Spawn a Pi RPC child process.
///
/// - `cwd`: working directory for the Pi process.
/// - `session_id`: optional native session ID to resume (`--session <id>`).
/// - `discovery`: when true, adds `--no-session --no-approve` for ephemeral
///   model-discovery processes. Session processes omit these flags so Pi
///   creates a durable session and emits permission requests.
/// - `extra_args`: additional arguments passed to pi.
pub fn spawn(
    cwd: &std::path::Path,
    session_id: Option<&str>,
    discovery: bool,
    extra_args: &[String],
) -> Result<PiRpc, String> {
    let pi_cmd = crate::pi_command();
    let mut cmd = Command::new(&pi_cmd);
    cmd.arg("--mode").arg("rpc")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    if discovery {
        cmd.arg("--no-session").arg("--no-approve");
    }

    if let Some(sid) = session_id {
        cmd.arg("--session").arg(sid);
    }

    for arg in extra_args {
        cmd.arg(arg);
    }

    let mut child = cmd.spawn().map_err(|e| {
        format!(
            "failed to start `{pi_cmd}`: {e}. Install the Pi CLI \
             (https://github.com/badlogic/pi-coding-agent) or set {}",
            crate::PI_COMMAND_ENV,
        )
    })?;

    let stdin = child.stdin.take().ok_or("pi stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("pi stdout unavailable")?;

    let pending: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
    let (event_tx, event_rx) = mpsc::unbounded_channel::<(String, Value)>();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    // Reader thread: parses JSON-RPC messages from pi stdout.
    {
        let pending = Arc::clone(&pending);
        let event_tx = event_tx.clone();
        std::thread::spawn(move || {
            read_loop(stdout, pending, event_tx, shutdown_rx);
        });
    }

    Ok(PiRpc {
        child: Arc::new(Mutex::new(child)),
        stdin: Arc::new(Mutex::new(stdin)),
        next_id: Arc::new(AtomicU64::new(1)),
        pending,
        events: Arc::new(Mutex::new(Some(event_rx))),
        shutdown: Arc::new(Mutex::new(Some(shutdown_tx))),
    })
}

fn read_loop(
    stdout: ChildStdout,
    pending: PendingRequests,
    event_tx: mpsc::UnboundedSender<(String, Value)>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    loop {
        // Use a small poll: check shutdown between lines.
        if shutdown.try_recv().is_ok() {
            break;
        }
        match lines.next() {
            Some(Ok(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                // Response to a request (has "id" and "result" or "error").
                if let Some(id_val) = msg.get("id") {
                    if let Some(id) = id_val.as_u64() {
                        let sender = {
                            let mut p = pending.blocking_lock();
                            p.remove(&id)
                        };
                        if let Some(tx) = sender {
                            let result = if let Some(err) = msg.get("error") {
                                Err(format!("pi RPC error: {}", serde_json::to_string(err).unwrap_or_default()))
                            } else {
                                Ok(msg.get("result").cloned().unwrap_or(Value::Null))
                            };
                            let _ = tx.send(result);
                        }
                        continue;
                    }
                }
                // Notification / event (has "method").
                if let Some(method) = msg.get("method").and_then(Value::as_str) {
                    let params = msg.get("params").cloned().unwrap_or(Value::Null);
                    let _ = event_tx.send((method.to_string(), params));
                }
            }
            Some(Err(_)) | None => break,
        }
    }
}

impl PiRpc {
    /// Send a JSON-RPC request and wait for the response.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut p = self.pending.lock().await;
            p.insert(id, tx);
        }
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        {
            let mut stdin = self.stdin.lock().await;
            writeln!(stdin, "{request}").map_err(|e| format!("failed to write to pi: {e}"))?;
            stdin.flush().map_err(|e| format!("failed to flush pi stdin: {e}"))?;
        }
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("pi RPC channel closed".to_string()),
            Err(_) => {
                let mut p = self.pending.lock().await;
                p.remove(&id);
                Err(format!("pi RPC call `{method}` timed out"))
            }
        }
    }

    /// Take ownership of the event receiver for a background event loop.
    /// Returns `None` if the receiver was already taken.
    pub async fn take_events(&self) -> Option<mpsc::UnboundedReceiver<(String, Value)>> {
        self.events.lock().await.take()
    }

    /// Receive the next Pi RPC event (notification).
    /// Returns `None` if the receiver was taken by `take_events()`.
    #[allow(dead_code)]
    pub async fn next_event(&self) -> Option<(String, Value)> {
        // Take the receiver, recv one event, put it back.
        let mut guard = self.events.lock().await;
        let rx = guard.as_mut()?;
        rx.recv().await
    }

    /// Get available models from Pi.
    pub async fn get_available_models(&self) -> Result<Vec<PiModel>, String> {
        let result = self.call("get_available_models", Value::Null).await?;
        let models: Vec<PiModel> = serde_json::from_value(result)
            .map_err(|e| format!("failed to parse pi models: {e}"))?;
        Ok(models)
    }

    /// Get the current Pi state (includes current model, thinking level, etc.).
    pub async fn get_state(&self) -> Result<PiState, String> {
        let result = self.call("get_state", Value::Null).await?;
        let state: PiState = serde_json::from_value(result)
            .map_err(|e| format!("failed to parse pi state: {e}"))?;
        Ok(state)
    }

    /// Set the active model.
    pub async fn set_model(&self, model_id: &str) -> Result<Value, String> {
        self.call("set_model", json!({ "modelId": model_id })).await
    }

    /// Set the thinking level.
    pub async fn set_thinking_level(&self, level: &str) -> Result<Value, String> {
        self.call("set_thinking_level", json!({ "level": level })).await
    }

    /// Send a prompt to Pi.
    pub async fn prompt(&self, text: &str) -> Result<Value, String> {
        self.call("prompt", json!({ "text": text })).await
    }

    /// Abort the current Pi turn.
    pub async fn abort(&self) -> Result<Value, String> {
        self.call("abort", Value::Null).await
    }

    /// Get session stats (context usage, cost).
    #[allow(dead_code)]
    pub async fn get_session_stats(&self) -> Result<PiSessionStats, String> {
        let result = self.call("get_session_stats", Value::Null).await?;
        let stats: PiSessionStats = serde_json::from_value(result)
            .map_err(|e| format!("failed to parse pi session stats: {e}"))?;
        Ok(stats)
    }

    /// Get the native session ID (for durable resume).
    pub async fn get_session_id(&self) -> Result<Option<String>, String> {
        let result = self.call("get_session_id", Value::Null).await?;
        Ok(result.as_str().map(String::from))
    }

    /// Respond to a permission request.
    pub async fn respond_permission(&self, request_id: &str, decision: &str) -> Result<Value, String> {
        self.call("respond_to_permission_request", json!({
            "requestId": request_id,
            "decision": decision,
        })).await
    }

    /// Gracefully shut down the Pi child process.
    pub async fn shutdown(&self) {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        let mut child = self.child.lock().await;
        let _ = child.kill();
        let _ = child.wait();
    }
}

// ─── Pi RPC types ───────────────────────────────────────────────────────────

/// A model available in Pi, as returned by `get_available_models`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub thinking_levels: Vec<String>,
    #[serde(default)]
    pub default_thinking_level: Option<String>,
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default)]
    pub max_context: Option<u64>,
}

impl PiModel {
    /// The opaque model ID used in ACP: `${provider}/${id}`.
    pub fn acp_model_id(&self) -> String {
        match &self.provider {
            Some(p) if !p.is_empty() => format!("{}/{}", p, self.id),
            _ => self.id.clone(),
        }
    }
}

/// Pi state, as returned by `get_state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiState {
    #[serde(default)]
    pub model: Option<PiModel>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub session_name: Option<String>,
}

/// Pi session statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct PiSessionStats {
    #[serde(default)]
    pub context_tokens: Option<u64>,
    #[serde(default)]
    pub max_context_tokens: Option<u64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub message_count: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_model_id_with_provider() {
        let model = PiModel {
            id: "gpt-5.6-luna".to_string(),
            name: "GPT 5.6 Luna".to_string(),
            provider: Some("openai-codex".to_string()),
            thinking_levels: vec![],
            default_thinking_level: None,
            supports_images: false,
            max_context: None,
        };
        assert_eq!(model.acp_model_id(), "openai-codex/gpt-5.6-luna");
    }

    #[test]
    fn acp_model_id_without_provider() {
        let model = PiModel {
            id: "claude-sonnet-4".to_string(),
            name: "Claude Sonnet 4".to_string(),
            provider: None,
            thinking_levels: vec![],
            default_thinking_level: None,
            supports_images: false,
            max_context: None,
        };
        assert_eq!(model.acp_model_id(), "claude-sonnet-4");
    }

    #[test]
    fn parse_models_response() {
        let json = json!([
            {
                "id": "gpt-5.6-luna",
                "name": "GPT 5.6 Luna",
                "provider": "openai-codex",
                "thinkingLevels": ["low", "medium", "high", "max"],
                "defaultThinkingLevel": "medium",
                "supportsImages": true,
                "maxContext": 200000
            },
            {
                "id": "claude-sonnet-4",
                "name": "Claude Sonnet 4",
                "provider": "anthropic",
                "thinkingLevels": ["low", "high"],
                "supportsImages": true
            }
        ]);
        let models: Vec<PiModel> = serde_json::from_value(json).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].acp_model_id(), "openai-codex/gpt-5.6-luna");
        assert_eq!(models[0].thinking_levels.len(), 4);
        assert_eq!(models[1].acp_model_id(), "anthropic/claude-sonnet-4");
    }

    #[test]
    fn parse_state_response() {
        let json = json!({
            "model": {
                "id": "gpt-5.6-luna",
                "name": "GPT 5.6 Luna",
                "provider": "openai-codex"
            },
            "thinkingLevel": "medium",
            "sessionId": "abc-123",
            "sessionName": "my-session"
        });
        let state: PiState = serde_json::from_value(json).unwrap();
        assert_eq!(state.thinking_level.as_deref(), Some("medium"));
        assert_eq!(state.session_id.as_deref(), Some("abc-123"));
    }
}
