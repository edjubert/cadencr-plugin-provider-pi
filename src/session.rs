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

use crate::pi_rpc::{spawn, PiRpc, PiState};
use crate::translate::{
    assistant_error_text, assistant_stop_reason, map_stop_reason, translate_event,
};
use crate::ui_bridge::handle_ui_request;

/// Shared map of pending permission requests: request ID → response sender.
/// The ACP server fills this when the event loop asks for a permission decision;
/// the main stdin loop resolves it when the host responds.
pub type PendingPermissions = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// A live ACP session backed by a Pi RPC child.
pub struct PiSession {
    session_id: String,
    rpc: PiRpc,
    config_options: Vec<SessionConfigOption>,
    context_window: Arc<AtomicU64>,
    stdout: Arc<Mutex<std::io::Stdout>>,
    next_update_seq: Arc<AtomicU64>,
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

        let config_options = match crate::discovery::load_config_options(&rpc).await {
            Ok(options) => options,
            Err(error) => {
                rpc.shutdown().await;
                return Err(error);
            }
        };
        let state = match rpc.get_state().await {
            Ok(state) => state,
            Err(error) => {
                rpc.shutdown().await;
                return Err(error);
            }
        };
        let session_id = match state.session_id.clone() {
            Some(session_id) => session_id,
            None => {
                rpc.shutdown().await;
                return Err("pi did not report a sessionId (get_state)".to_string());
            }
        };

        Ok(Self {
            session_id,
            rpc,
            config_options,
            context_window: Arc::new(AtomicU64::new(
                context_window_for_state(&state).unwrap_or(0),
            )),
            stdout,
            next_update_seq: Arc::new(AtomicU64::new(0)),
            turn_end_tx: Arc::new(Mutex::new(None)),
            pending_permissions,
            permission_req_id: Arc::new(AtomicU64::new(1)),
            event_task: None,
        })
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

        let state = match rpc.get_state().await {
            Ok(state) => state,
            Err(error) => {
                rpc.shutdown().await;
                return Err(error);
            }
        };
        if state.session_id.as_deref() != Some(session_id) {
            rpc.shutdown().await;
            return Err(format!(
                "pi session mismatch: expected `{session_id}`, got `{}`",
                state.session_id.unwrap_or_default()
            ));
        }
        let config_options = match crate::discovery::load_config_options(&rpc).await {
            Ok(options) => options,
            Err(error) => {
                rpc.shutdown().await;
                return Err(error);
            }
        };

        Ok(Self {
            session_id: session_id.to_string(),
            rpc,
            config_options,
            context_window: Arc::new(AtomicU64::new(
                context_window_for_state(&state).unwrap_or(0),
            )),
            stdout,
            next_update_seq: Arc::new(AtomicU64::new(0)),
            turn_end_tx: Arc::new(Mutex::new(None)),
            pending_permissions,
            permission_req_id: Arc::new(AtomicU64::new(1)),
            event_task: None,
        })
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

    /// Start forwarding Pi events after the ACP session/new or session/resume
    /// response has been written, so the host knows this session's ID first.
    pub async fn activate(&mut self) {
        let context_window = self.context_window.load(Ordering::SeqCst);
        if context_window > 0 {
            self.emit_context_usage_update(Some(0), Some(context_window))
                .await;
        }
        self.start_event_loop().await;
    }

    /// Send a prompt to Pi. Returns a receiver that resolves with the
    /// stop reason when the turn completes.
    pub async fn prompt(&self, text: &str) -> Result<oneshot::Receiver<String>, String> {
        let (tx, rx) = oneshot::channel();
        {
            let mut slot = self.turn_end_tx.lock().await;
            if slot.is_some() {
                return Err("a prompt is already running in this session".to_string());
            }
            *slot = Some(tx);
        }
        if let Err(e) = self.rpc.prompt(text).await {
            self.turn_end_tx.lock().await.take();
            return Err(e);
        }
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

        self.config_options = crate::discovery::load_config_options(&self.rpc).await?;
        if config_id == "model" {
            let state = self.rpc.get_state().await?;
            let context_window = context_window_for_state(&state).unwrap_or(0);
            self.context_window.store(context_window, Ordering::SeqCst);
            self.emit_context_usage_update(Some(0), Some(context_window))
                .await;
        }
        Ok(self.config_options())
    }

    async fn emit_context_usage_update(&self, used: Option<u64>, size: Option<u64>) {
        let mut update = json!({ "sessionUpdate": "usage_update" });
        if let Some(used) = used {
            update["used"] = json!(used);
        }
        if let Some(size) = size {
            update["size"] = json!(size);
        }
        write_session_update(
            &self.stdout,
            &self.next_update_seq,
            &self.session_id,
            update,
        )
        .await;
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
        let next_update_seq = self.next_update_seq.clone();
        let context_window = self.context_window.clone();

        let task = tokio::spawn(async move {
            let mut last_stop_reason: Option<String> = None;
            while let Some((kind, event)) = events.recv().await {
                if let Some(reason) = assistant_stop_reason(&kind, &event) {
                    last_stop_reason = Some(reason);
                }

                match kind.as_str() {
                    // The whole prompt (retries, compaction, queued follow-ups
                    // included) is over: resolve the pending `session/prompt`.
                    "agent_settled" => {
                        match rpc.get_context_usage().await {
                            Ok(Some(usage)) => {
                                let size = usage
                                    .context_window
                                    .filter(|window| *window > 0)
                                    .or_else(|| {
                                        let window = context_window.load(Ordering::SeqCst);
                                        (window > 0).then_some(window)
                                    });
                                if usage.tokens.is_some() || size.is_some() {
                                    let mut update = json!({ "sessionUpdate": "usage_update" });
                                    if let Some(used) = usage.tokens {
                                        update["used"] = json!(used);
                                    }
                                    if let Some(size) = size {
                                        update["size"] = json!(size);
                                    }
                                    write_session_update(
                                        &stdout,
                                        &next_update_seq,
                                        &session_id,
                                        update,
                                    )
                                    .await;
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                eprintln!(
                                    "pi session {session_id}: failed to read context usage: {error}"
                                );
                            }
                        }
                        let reason = map_stop_reason(last_stop_reason.take().as_deref());
                        if let Some(tx) = turn_end_tx.lock().await.take() {
                            let _ = tx.send(reason.to_string());
                        }
                        continue;
                    }
                    "extension_ui_request" => {
                        tokio::spawn(handle_ui_request(
                            rpc.clone(),
                            stdout.clone(),
                            session_id.clone(),
                            event,
                            pending_permissions.clone(),
                            permission_req_id.clone(),
                        ));
                        continue;
                    }
                    _ => {}
                }

                let mut updates = translate_event(&kind, &event);
                if let Some(text) = assistant_error_text(&kind, &event) {
                    updates.push(json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": format!("Pi error: {text}") },
                    }));
                }
                for update in updates {
                    write_session_update(&stdout, &next_update_seq, &session_id, update).await;
                }
            }
            // The event channel closed: pi exited. Dropping the sender wakes the
            // pending `prompt()` receiver with an error (turned into a JSON-RPC
            // error by acp.rs).
            turn_end_tx.lock().await.take();
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

fn context_window_for_state(state: &PiState) -> Option<u64> {
    state
        .model
        .as_ref()
        .and_then(|model| model.max_context)
        .filter(|window| *window > 0)
}

async fn write_session_update(
    stdout: &Arc<Mutex<std::io::Stdout>>,
    next_update_seq: &AtomicU64,
    session_id: &str,
    update: Value,
) {
    let mut out = stdout.lock().await;
    let seq = next_update_seq.fetch_add(1, Ordering::SeqCst);
    let notification = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": update,
            "_meta": { "seq": seq },
        },
    });
    if let Err(error) = writeln!(out, "{notification}").and_then(|_| out.flush()) {
        eprintln!("pi session {session_id}: failed to write session/update: {error}");
    }
}
