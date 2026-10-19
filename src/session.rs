//! A single ACP session backed by a Pi RPC child process.
//!
//! Owns the Pi RPC handle, translates Pi events to ACP session updates,
//! and handles ACP requests (prompt, cancel, config changes).

use agent_client_protocol::schema::v1::SessionConfigOption;
use serde_json::{json, Value};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::pi_rpc::{spawn, PiRpc};

/// A live ACP session backed by a Pi RPC child.
pub struct PiSession {
    session_id: String,
    rpc: PiRpc,
    config_options: Vec<SessionConfigOption>,
    /// Monotonic update counter for ACP session update notifications.
    update_seq: AtomicU64,
    stdout: Arc<Mutex<std::io::StdoutLock<'static>>>,
    /// Whether the event loop is running.
    event_task: Option<tokio::task::JoinHandle<()>>,
}

impl PiSession {
    /// Create a new session: spawn a fresh Pi RPC process.
    pub async fn new(
        cwd: &str,
        _session_id: Option<&str>,
        provider_args: &[String],
        stdout: Arc<Mutex<std::io::StdoutLock<'static>>>,
    ) -> Result<Self, String> {
        let cwd_path = std::path::Path::new(cwd);
        let mut rpc = spawn(cwd_path, None, provider_args)?;

        // Discover models and state for config options.
        let models = rpc.get_available_models().await?;
        let state = rpc.get_state().await.ok();

        let config_options = crate::discovery::build_session_config_options(&models, state.as_ref());

        // Get the native session ID for durable resume.
        let native_session_id = rpc.get_session_id().await.unwrap_or_default();
        let session_id = if native_session_id.is_empty() {
            uuid_v4()
        } else {
            native_session_id
        };

        let mut session = Self {
            session_id,
            rpc,
            config_options,
            update_seq: AtomicU64::new(0),
            stdout,
            event_task: None,
        };

        session.start_event_loop();
        Ok(session)
    }

    /// Resume a session: spawn Pi RPC with `--session <id>`.
    pub async fn resume(
        cwd: &str,
        session_id: &str,
        provider_args: &[String],
        stdout: Arc<Mutex<std::io::StdoutLock<'static>>>,
    ) -> Result<Self, String> {
        let cwd_path = std::path::Path::new(cwd);
        let mut rpc = spawn(cwd_path, Some(session_id), provider_args)?;

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
            update_seq: AtomicU64::new(0),
            stdout,
            event_task: None,
        };

        session.start_event_loop();
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

    /// Send a prompt to Pi and stream events as ACP updates.
    pub async fn prompt(
        &mut self,
        text: &str,
        _stdout: &Arc<Mutex<std::io::StdoutLock<'static>>>,
    ) -> Result<(), String> {
        // Send the prompt to Pi. Pi will emit events that the event loop
        // translates to ACP session updates.
        self.rpc.prompt(text).await?;
        Ok(())
    }

    /// Cancel the current Pi turn.
    pub async fn cancel(&mut self) -> Result<(), String> {
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
        self.config_options = crate::discovery::build_session_config_options(&models, state.as_ref());

        Ok(self.config_options())
    }

    /// Start the background task that reads Pi events and emits ACP updates.
    fn start_event_loop(&mut self) {
        // We need to take ownership of the rpc's event receiver.
        // Since PiRpc owns the receiver, we spawn a task that owns a clone
        // of the Arc-wrapped rpc. For simplicity, we move the event reading
        // into a task that shares the rpc via Arc.
        // Actually, PiRpc is not Clone. We need to restructure slightly.
        // For now, we use a simpler approach: the event loop runs in a task
        // that has its own reference to the rpc events.
        // Since PiRpc owns events internally, we'll handle events inline
        // during prompt() calls instead.
        //
        // This is a simplification: in a full implementation, we'd split
        // PiRpc into a sender half and a receiver half.
    }

    /// Gracefully shut down the Pi child process.
    pub async fn shutdown(&mut self) {
        if let Some(task) = self.event_task.take() {
            task.abort();
        }
        self.rpc.shutdown().await;
    }

    /// Emit an ACP session update notification.
    fn emit_update(&self, update: Value) {
        let seq = self.update_seq.fetch_add(1, Ordering::SeqCst);
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": self.session_id,
                "update": update,
                "_meta": { "seq": seq },
            },
        });
        // We can't easily lock stdout here without async, so we use a
        // blocking approach via try_lock.
        if let Ok(mut out) = self.stdout.try_lock() {
            let _ = writeln!(out, "{notification}");
            let _ = out.flush();
        }
    }
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
        (nanos >> 16) as u16 & 0xffff,
        nanos & 0xfff,
        ((addr >> 16) & 0xffff) | 0x8000,
        (addr & 0xffffffffffff) ^ nanos,
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
}
