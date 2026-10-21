//! A single ACP session backed by a Pi RPC child process.
//!
//! Owns the Pi RPC handle, translates Pi events to ACP session updates,
//! and handles ACP requests (prompt, cancel, config changes).
//!
//! The event loop runs in a background tokio task: it reads Pi RPC
//! notifications, translates them to ACP `session/update` notifications,
//! and writes them to stdout. Permission requests from Pi are bridged
//! to ACP `session/request_permission` requests.

use agent_client_protocol::schema::v1::SessionConfigOption;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

use crate::pi_rpc::{spawn, PiRpc};

/// Shared map of pending permission requests: request ID → response sender.
/// The ACP server fills this when the event loop asks for a permission decision;
/// the main stdin loop resolves it when the host responds.
pub type PendingPermissions = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// A live ACP session backed by a Pi RPC child.
pub struct PiSession {
    session_id: String,
    rpc: PiRpc,
    config_options: Vec<SessionConfigOption>,
    stdout: Arc<Mutex<std::io::Stdout>>,
    /// Notified with the stop reason when the current turn ends.
    turn_end_tx: Arc<Mutex<Option<oneshot::Sender<String>>>>,
    /// Shared pending-permission map (owned by the ACP server).
    pending_permissions: PendingPermissions,
    /// Counter for generating unique permission request IDs.
    permission_req_id: Arc<AtomicU64>,
    /// Background event loop task handle.
    event_task: Option<tokio::task::JoinHandle<()>>,
}

impl PiSession {
    /// Create a new session: spawn a fresh Pi RPC process.
    pub async fn new(
        cwd: &str,
        provider_args: &[String],
        stdout: Arc<Mutex<std::io::Stdout>>,
        pending_permissions: PendingPermissions,
    ) -> Result<Self, String> {
        let cwd_path = std::path::Path::new(cwd);
        let rpc = spawn(cwd_path, None, false, provider_args)?;

        // Discover models and state for config options.
        let models = rpc.get_available_models().await?;
        let state = rpc.get_state().await.ok();

        let config_options =
            crate::discovery::build_session_config_options(&models, state.as_ref());

        // Get the native session ID for durable resume.
        let native_session_id = rpc
            .get_session_id()
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let session_id = if native_session_id.is_empty() {
            uuid_v4()
        } else {
            native_session_id
        };

        let mut session = Self {
            session_id,
            rpc,
            config_options,
            stdout,
            turn_end_tx: Arc::new(Mutex::new(None)),
            pending_permissions,
            permission_req_id: Arc::new(AtomicU64::new(1)),
            event_task: None,
        };

        session.start_event_loop().await;
        Ok(session)
    }

    /// Resume a session: spawn Pi RPC with `--session <id>`.
    pub async fn resume(
        cwd: &str,
        session_id: &str,
        provider_args: &[String],
        stdout: Arc<Mutex<std::io::Stdout>>,
        pending_permissions: PendingPermissions,
    ) -> Result<Self, String> {
        let cwd_path = std::path::Path::new(cwd);
        let rpc = spawn(cwd_path, Some(session_id), false, provider_args)?;

        // Verify the session loaded correctly.
        let models = rpc.get_available_models().await?;
        let state = rpc.get_state().await?;

        if state.session_id.as_deref() != Some(session_id) {
            rpc.shutdown().await;
            return Err(format!(
                "pi session mismatch: expected `{session_id}`, got `{}`",
                state.session_id.unwrap_or_default()
            ));
        }

        let config_options = crate::discovery::build_session_config_options(&models, Some(&state));

        let mut session = Self {
            session_id: session_id.to_string(),
            rpc,
            config_options,
            stdout,
            turn_end_tx: Arc::new(Mutex::new(None)),
            pending_permissions,
            permission_req_id: Arc::new(AtomicU64::new(1)),
            event_task: None,
        };

        session.start_event_loop().await;
        Ok(session)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn config_options(&self) -> Vec<Value> {
        self.config_options
            .iter()
            .map(|opt| serde_json::to_value(opt).unwrap_or(Value::Null))
            .collect()
    }

    /// Send a prompt to Pi. Returns a receiver that resolves with the
    /// stop reason when the turn completes.
    pub async fn prompt(&self, text: &str) -> Result<oneshot::Receiver<String>, String> {
        let (tx, rx) = oneshot::channel();
        *self.turn_end_tx.lock().await = Some(tx);
        self.rpc.prompt(text).await?;
        Ok(rx)
    }

    /// Cancel the current Pi turn.
    pub async fn cancel(&self) -> Result<(), String> {
        self.rpc.abort().await?;
        Ok(())
    }

    /// Set a config option (model or thinking level).
    pub async fn set_config_option(
        &mut self,
        config_id: &str,
        value: &str,
    ) -> Result<Vec<Value>, String> {
        match config_id {
            "model" => {
                self.rpc.set_model(value).await?;
            }
            "thinking" => {
                self.rpc.set_thinking_level(value).await?;
            }
            _ => {
                return Err(format!("unknown config option: {config_id}"));
            }
        }

        // Refresh config options from Pi state.
        let models = self.rpc.get_available_models().await?;
        let state = self.rpc.get_state().await.ok();
        self.config_options =
            crate::discovery::build_session_config_options(&models, state.as_ref());

        Ok(self.config_options())
    }

    /// Start the background task that reads Pi events and emits ACP updates.
    async fn start_event_loop(&mut self) {
        let Some(mut events) = self.rpc.take_events().await else {
            return;
        };

        let rpc = self.rpc.clone();
        let stdout = self.stdout.clone();
        let session_id = self.session_id.clone();
        let turn_end_tx = self.turn_end_tx.clone();
        let pending_permissions = self.pending_permissions.clone();
        let permission_req_id = self.permission_req_id.clone();
        let seq = Arc::new(AtomicU64::new(0));

        let task = tokio::spawn(async move {
            while let Some((method, params)) = events.recv().await {
                let seq_val = seq.fetch_add(1, Ordering::SeqCst);

                // Handle turn end: resolve the prompt receiver.
                if method == "turn_end" {
                    let reason = params
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("end_turn");
                    let stop_reason = map_stop_reason(reason);
                    if let Some(tx) = turn_end_tx.lock().await.take() {
                        let _ = tx.send(stop_reason);
                    }
                    continue;
                }

                // Handle permission requests: bridge to ACP.
                if method == "permission_request" {
                    handle_permission_request(
                        &rpc,
                        &stdout,
                        &session_id,
                        &params,
                        &pending_permissions,
                        &permission_req_id,
                    )
                    .await;
                    continue;
                }

                // Translate other events to ACP session updates.
                if let Some(update) = translate_pi_event(&method, &params) {
                    let notification = json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {
                            "sessionId": session_id,
                            "update": update,
                            "_meta": { "seq": seq_val },
                        },
                    });
                    let mut out = stdout.lock().await;
                    let _ = writeln!(out, "{notification}");
                    let _ = out.flush();
                }
            }
        });

        self.event_task = Some(task);
    }

    /// Gracefully shut down the Pi child process.
    pub async fn shutdown(&mut self) {
        if let Some(task) = self.event_task.take() {
            task.abort();
        }
        self.rpc.shutdown().await;
    }
}

/// Map a Pi turn-end reason to an ACP stop reason.
fn map_stop_reason(pi_reason: &str) -> String {
    match pi_reason {
        "stop" | "end_turn" => "end_turn".to_string(),
        "max_tokens" | "length" => "max_tokens".to_string(),
        "tool_calls" => "end_turn".to_string(),
        "abort" | "cancel" | "cancelled" => "cancelled".to_string(),
        "error" => "end_turn".to_string(),
        other => other.to_string(),
    }
}

/// Translate a Pi RPC event to an ACP session update (the `update` field).
/// Returns `None` for events that don't map to an ACP update.
fn translate_pi_event(method: &str, params: &Value) -> Option<Value> {
    match method {
        // Text/thinking message chunks.
        "message" | "message_update" => {
            let role = params
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("assistant");
            let content = params.get("content");
            let text = extract_text(content);
            if text.is_empty() {
                return None;
            }
            let session_update = match role {
                "thinking" | "reasoning" => "agent_thought_chunk",
                _ => "agent_message_chunk",
            };
            Some(json!({
                "sessionUpdate": session_update,
                "content": { "type": "text", "text": text },
            }))
        }

        // Tool call lifecycle.
        "tool_execution_start" => {
            let tool_call_id = params
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let tool_name = params
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or("tool");
            Some(json!({
                "sessionUpdate": "tool_call",
                "toolCallId": tool_call_id,
                "title": tool_name,
                "kind": "execute",
                "status": "in_progress",
                "rawInput": params.get("args").cloned().unwrap_or(Value::Null),
            }))
        }

        "tool_execution_update" => {
            let tool_call_id = params
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let mut update = json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": tool_call_id,
            });
            if let Some(status) = params.get("status").and_then(Value::as_str) {
                let acp_status = match status {
                    "executing" | "running" => "in_progress",
                    "success" | "completed" => "completed",
                    "error" | "failed" => "failed",
                    other => other,
                };
                update["status"] = json!(acp_status);
            }
            if let Some(output) = params.get("output") {
                update["content"] = json!([{
                    "type": "content",
                    "content": { "type": "text", "text": output },
                }]);
            }
            Some(update)
        }

        "tool_execution_end" => {
            let tool_call_id = params
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let success = params
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let status = if success { "completed" } else { "failed" };
            let mut update = json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": tool_call_id,
                "status": status,
            });
            if let Some(output) = params.get("output") {
                update["content"] = json!([{
                    "type": "content",
                    "content": { "type": "text", "text": output },
                }]);
            }
            if let Some(error) = params.get("error") {
                update["rawOutput"] = error.clone();
            }
            Some(update)
        }

        // Session stats → usage info (sent as a custom update).
        "session_stats" => {
            let used = params
                .get("contextTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let size = params
                .get("maxContextTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if used == 0 && size == 0 {
                return None;
            }
            Some(json!({
                "sessionUpdate": "session_info_update",
                "contextUsage": { "used": used, "size": size },
            }))
        }

        // Thinking level / model changes → config option update.
        "thinking_level_change" | "model_change" => None, // handled via set_config_option

        // Compaction events — no direct ACP equivalent, skip.
        "compaction_start" | "compaction_end" => None,

        // Command responses (slash commands) — skip for now.
        "command_response" => None,

        _ => None,
    }
}

/// Extract text from a Pi message content field (string or array of blocks).
fn extract_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    block.get("text").and_then(Value::as_str)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Bridge a Pi permission request to an ACP `session/request_permission`.
///
/// Sends the request to the host, waits for the response, and forwards
/// the decision back to Pi.
async fn handle_permission_request(
    rpc: &PiRpc,
    stdout: &Arc<Mutex<std::io::Stdout>>,
    session_id: &str,
    params: &Value,
    pending_permissions: &PendingPermissions,
    permission_req_id: &Arc<AtomicU64>,
) {
    let request_id = params
        .get("requestId")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let tool_name = params
        .get("toolName")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let description = params
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");

    let req_id = permission_req_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = oneshot::channel::<Value>();
    pending_permissions.lock().await.insert(req_id, tx);

    let request = json!({
        "jsonrpc": "2.0",
        "id": req_id,
        "method": "session/request_permission",
        "params": {
            "sessionId": session_id,
            "toolCall": {
                "toolCallId": request_id,
                "title": tool_name,
            },
            "options": [
                { "optionId": "allow_once", "name": "Allow once" },
                { "optionId": "allow_always", "name": "Always allow" },
                { "optionId": "deny", "name": "Deny" },
            ],
        },
    });

    {
        let mut out = stdout.lock().await;
        let _ = writeln!(out, "{request}");
        let _ = out.flush();
    }

    // Add a description note if available.
    if !description.is_empty() {
        eprintln!("pi permission request: {tool_name} — {description}");
    }

    // Wait for the host's decision (with a generous timeout).
    let decision = match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
        Ok(Ok(response)) => {
            // Extract the selected optionId from the ACP response.
            response
                .pointer("/outcome/optionId")
                .and_then(Value::as_str)
                .unwrap_or("deny")
                .to_string()
        }
        _ => "deny".to_string(),
    };

    // Map ACP option IDs to Pi decisions.
    let pi_decision = match decision.as_str() {
        "allow_once" => "allow_once",
        "allow_always" => "allow_always",
        _ => "deny",
    };

    let _ = rpc.respond_permission(request_id, pi_decision).await;
}

/// Generate a simple UUID v4 (without external crates).
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = now.as_nanos() as u64;
    let addr = &nanos as *const u64 as u64;
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (nanos >> 32) as u32,
        (nanos >> 16) as u16,
        nanos & 0xfff,
        (((addr >> 16) & 0xffff) | 0x8000) as u16,
        ((addr & 0xffffffffffff) ^ nanos) & 0xffffffffffff,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_v4_format() {
        let id = uuid_v4();
        assert_eq!(id.len(), 36);
        assert_eq!(id.chars().nth(14), Some('4')); // UUID v4 marker
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn map_stop_reason_known() {
        assert_eq!(map_stop_reason("stop"), "end_turn");
        assert_eq!(map_stop_reason("max_tokens"), "max_tokens");
        assert_eq!(map_stop_reason("abort"), "cancelled");
        assert_eq!(map_stop_reason("error"), "end_turn");
    }

    #[test]
    fn map_stop_reason_unknown_passthrough() {
        assert_eq!(map_stop_reason("custom_reason"), "custom_reason");
    }

    #[test]
    fn translate_message_text() {
        let params = json!({
            "role": "assistant",
            "content": [{ "type": "text", "text": "Hello world" }],
        });
        let update = translate_pi_event("message", &params).unwrap();
        assert_eq!(update["sessionUpdate"], "agent_message_chunk");
        assert_eq!(update["content"]["text"], "Hello world");
    }

    #[test]
    fn translate_message_thinking() {
        let params = json!({
            "role": "thinking",
            "content": [{ "type": "text", "text": "Let me think..." }],
        });
        let update = translate_pi_event("message", &params).unwrap();
        assert_eq!(update["sessionUpdate"], "agent_thought_chunk");
    }

    #[test]
    fn translate_message_string_content() {
        let params = json!({
            "role": "assistant",
            "content": "Plain string",
        });
        let update = translate_pi_event("message", &params).unwrap();
        assert_eq!(update["content"]["text"], "Plain string");
    }

    #[test]
    fn translate_tool_execution_start() {
        let params = json!({
            "toolCallId": "call-1",
            "toolName": "bash",
            "args": { "command": "ls -la" },
        });
        let update = translate_pi_event("tool_execution_start", &params).unwrap();
        assert_eq!(update["sessionUpdate"], "tool_call");
        assert_eq!(update["toolCallId"], "call-1");
        assert_eq!(update["title"], "bash");
        assert_eq!(update["status"], "in_progress");
    }

    #[test]
    fn translate_tool_execution_end_success() {
        let params = json!({
            "toolCallId": "call-1",
            "success": true,
            "output": "file1.txt\nfile2.txt",
        });
        let update = translate_pi_event("tool_execution_end", &params).unwrap();
        assert_eq!(update["sessionUpdate"], "tool_call_update");
        assert_eq!(update["status"], "completed");
        assert_eq!(
            update["content"][0]["content"]["text"],
            "file1.txt\nfile2.txt"
        );
    }

    #[test]
    fn translate_tool_execution_end_failure() {
        let params = json!({
            "toolCallId": "call-1",
            "success": false,
            "error": "command not found",
        });
        let update = translate_pi_event("tool_execution_end", &params).unwrap();
        assert_eq!(update["status"], "failed");
        assert_eq!(update["rawOutput"], "command not found");
    }

    #[test]
    fn translate_session_stats() {
        let params = json!({
            "contextTokens": 5000,
            "maxContextTokens": 200000,
            "costUsd": 0.05,
        });
        let update = translate_pi_event("session_stats", &params).unwrap();
        assert_eq!(update["contextUsage"]["used"], 5000);
        assert_eq!(update["contextUsage"]["size"], 200000);
    }

    #[test]
    fn translate_unknown_event_returns_none() {
        assert!(translate_pi_event("unknown_event", &json!({})).is_none());
        assert!(translate_pi_event("compaction_start", &json!({})).is_none());
    }

    #[test]
    fn extract_text_from_array() {
        let content = json!([
            { "type": "text", "text": "Hello " },
            { "type": "text", "text": "world" },
            { "type": "image", "data": "..." },
        ]);
        assert_eq!(extract_text(Some(&content)), "Hello world");
    }

    #[test]
    fn extract_text_from_string() {
        let content = json!("direct string");
        assert_eq!(extract_text(Some(&content)), "direct string");
    }

    #[test]
    fn extract_text_from_null() {
        assert_eq!(extract_text(None), "");
        assert_eq!(extract_text(Some(&Value::Null)), "");
    }
}
