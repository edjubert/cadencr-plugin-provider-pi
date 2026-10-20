//! Pi RPC client: spawns `pi --mode rpc` and speaks its JSONL protocol.
//!
//! Commands are `{"id","type",...fields}` objects written one per line;
//! `{"type":"response",...}` lines answer them (matched on `id`); every
//! other line is an event `{"type":...}` forwarded as `(type, full object)`.
//! See `docs/rpc.md` in the `@earendil-works/pi-coding-agent` package.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Pending requests: id -> response sender.
type PendingRequests = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>>;

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
    cmd.arg("--mode")
        .arg("rpc")
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
             (npm install -g @earendil-works/pi-coding-agent, Node.js >= 22.19.0, \
             https://github.com/earendil-works/pi) or set {}",
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

fn read_loop<R: Read>(
    stdout: R,
    pending: PendingRequests,
    event_tx: mpsc::UnboundedSender<(String, Value)>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        if shutdown.try_recv().is_ok() {
            break;
        }
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("pi: stdout read error: {e}");
                break;
            }
        }
        // Strict JSONL: LF is the only delimiter; tolerate a trailing CR.
        if buf.last() == Some(&b'\n') {
            buf.pop();
        }
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        if buf.is_empty() {
            continue;
        }
        match serde_json::from_slice::<Value>(&buf) {
            Ok(msg) => dispatch(&msg, &pending, &event_tx),
            Err(e) => eprintln!("pi: ignoring non-JSON stdout line: {e}"),
        }
    }
    fail_pending(&pending, "pi process exited before answering");
}

/// Route one parsed stdout message: responses resolve a pending request,
/// everything else is an event.
fn dispatch(
    msg: &Value,
    pending: &PendingRequests,
    event_tx: &mpsc::UnboundedSender<(String, Value)>,
) {
    match msg.get("type").and_then(Value::as_str) {
        Some("response") => resolve_response(msg, pending),
        Some(kind) => {
            let _ = event_tx.send((kind.to_string(), msg.clone()));
        }
        None => eprintln!("pi: ignoring message without `type`: {msg}"),
    }
}

fn resolve_response(msg: &Value, pending: &PendingRequests) {
    let command = msg
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let success = msg.get("success").and_then(Value::as_bool).unwrap_or(false);
    let result = if success {
        Ok(msg.get("data").cloned().unwrap_or(Value::Null))
    } else {
        let error = msg
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        Err(format!("pi `{command}` failed: {error}"))
    };
    let Some(id) = msg.get("id").and_then(Value::as_str) else {
        // e.g. `command: "parse"` failures carry no id: nobody can be woken.
        if let Err(e) = result {
            eprintln!("{e}");
        }
        return;
    };
    let sender = pending.blocking_lock().remove(id);
    if let Some(tx) = sender {
        let _ = tx.send(result);
    }
}

/// Fail every in-flight request (the child is gone, no answer will come).
fn fail_pending(pending: &PendingRequests, reason: &str) {
    for (_, tx) in pending.blocking_lock().drain() {
        let _ = tx.send(Err(reason.to_string()));
    }
}

/// Build `{"id","type",...params}`. `params` must be an object or null.
fn envelope(kind: &str, id: &str, params: Value) -> Result<Value, String> {
    let mut object = match params {
        Value::Null => serde_json::Map::new(),
        Value::Object(map) => map,
        other => return Err(format!("pi `{kind}` params must be an object, got {other}")),
    };
    object.insert("id".to_string(), json!(id));
    object.insert("type".to_string(), json!(kind));
    Ok(Value::Object(object))
}

impl PiRpc {
    /// Write one JSON line to pi's stdin.
    async fn send(&self, message: &Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().await;
        writeln!(stdin, "{message}").map_err(|e| format!("failed to write to pi: {e}"))?;
        stdin
            .flush()
            .map_err(|e| format!("failed to flush pi stdin: {e}"))
    }

    /// Send a command and wait for its response `data`.
    pub async fn call(&self, command: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst).to_string();
        let request = envelope(command, &id, params)?;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        if let Err(e) = self.send(&request).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("pi RPC channel closed".to_string()),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("pi RPC call `{command}` timed out"))
            }
        }
    }

    /// Answer an `extension_ui_request` dialog. `response` is one of
    /// `{"value": "..."}`, `{"confirmed": bool}` or `{"cancelled": true}`.
    pub async fn respond_ui(&self, request_id: &str, response: Value) -> Result<(), String> {
        let message = envelope("extension_ui_response", request_id, response)?;
        self.send(&message).await
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
        parse_models(&self.call("get_available_models", Value::Null).await?)
    }

    /// Get the thinking levels available for the current model.
    pub async fn get_available_thinking_levels(&self) -> Result<Vec<String>, String> {
        let data = self
            .call("get_available_thinking_levels", Value::Null)
            .await?;
        serde_json::from_value(data.get("levels").cloned().unwrap_or(Value::Null))
            .map_err(|e| format!("failed to parse pi thinking levels: {e}"))
    }

    /// Get the current Pi state (includes current model, thinking level, etc.).
    pub async fn get_state(&self) -> Result<PiState, String> {
        let result = self.call("get_state", Value::Null).await?;
        let state: PiState =
            serde_json::from_value(result).map_err(|e| format!("failed to parse pi state: {e}"))?;
        Ok(state)
    }

    /// Set the active model.
    pub async fn set_model(&self, model_id: &str) -> Result<Value, String> {
        let (provider, id) = split_model_id(model_id)
            .ok_or_else(|| format!("invalid model id `{model_id}`, expected `provider/id`"))?;
        self.call("set_model", json!({ "provider": provider, "modelId": id }))
            .await
    }

    /// Set the thinking level.
    pub async fn set_thinking_level(&self, level: &str) -> Result<Value, String> {
        self.call("set_thinking_level", json!({ "level": level }))
            .await
    }

    /// Send a prompt to Pi.
    pub async fn prompt(&self, text: &str) -> Result<Value, String> {
        self.call("prompt", json!({ "message": text })).await
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
        Ok(self.get_state().await?.session_id)
    }

    /// Legacy shim, removed in task 7: maps the old allow/deny decision to a
    /// `confirm` response.
    pub async fn respond_permission(
        &self,
        request_id: &str,
        decision: &str,
    ) -> Result<Value, String> {
        let confirmed = decision.starts_with("allow");
        self.respond_ui(request_id, json!({ "confirmed": confirmed }))
            .await?;
        Ok(Value::Null)
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

/// Split an ACP model id `provider/model-id` on the first `/` (model ids may
/// themselves contain `/`, provider names never do).
pub fn split_model_id(acp_id: &str) -> Option<(&str, &str)> {
    acp_id
        .split_once('/')
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
}

fn parse_models(data: &Value) -> Result<Vec<PiModel>, String> {
    let models = data
        .get("models")
        .cloned()
        .ok_or_else(|| "pi `get_available_models` response has no `models` field".to_string())?;
    serde_json::from_value(models).map_err(|e| format!("failed to parse pi models: {e}"))
}

/// A model as returned by `get_available_models` (`Model` in rpc.md).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiModel {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub provider: Option<String>,
    /// `Model.input` is e.g. `["text","image"]`.
    #[serde(default, rename = "input", deserialize_with = "input_has_image")]
    pub supports_images: bool,
    #[serde(default, rename = "contextWindow")]
    pub max_context: Option<u64>,
    /// Filled by discovery from `get_available_thinking_levels`; not on the wire.
    #[serde(skip)]
    pub thinking_levels: Vec<String>,
    #[serde(skip)]
    #[allow(dead_code)]
    pub default_thinking_level: Option<String>,
}

fn input_has_image<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    let kinds = Vec::<String>::deserialize(deserializer)?;
    Ok(kinds.iter().any(|k| k == "image"))
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
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiState {
    #[serde(default)]
    pub model: Option<PiModel>,
    #[serde(default)]
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
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
        let data = json!({ "models": [
            { "id": "claude-sonnet-4-20250514", "name": "Claude Sonnet 4", "api": "anthropic-messages",
              "provider": "anthropic", "baseUrl": "https://api.anthropic.com", "reasoning": true,
              "input": ["text", "image"], "contextWindow": 200000, "maxTokens": 16384,
              "cost": { "input": 3.0, "output": 15.0, "cacheRead": 0.3, "cacheWrite": 3.75 } },
            { "id": "text-only", "name": "Text only", "provider": "local", "input": ["text"] }
        ]});
        let models = parse_models(&data).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(
            models[0].acp_model_id(),
            "anthropic/claude-sonnet-4-20250514"
        );
        assert!(models[0].supports_images);
        assert_eq!(models[0].max_context, Some(200000));
        assert!(!models[1].supports_images);
        assert!(models[0].thinking_levels.is_empty());
    }

    #[test]
    fn parse_models_requires_models_field() {
        assert!(parse_models(&json!({}))
            .unwrap_err()
            .contains("no `models` field"));
    }

    fn new_pending() -> PendingRequests {
        Arc::new(Mutex::new(HashMap::new()))
    }

    #[test]
    fn envelope_flattens_params_and_sets_id_and_type() {
        let msg = envelope(
            "set_model",
            "4",
            json!({ "provider": "anthropic", "modelId": "x" }),
        )
        .unwrap();
        assert_eq!(
            msg,
            json!({ "id": "4", "type": "set_model", "provider": "anthropic", "modelId": "x" })
        );
        assert_eq!(
            envelope("abort", "5", Value::Null).unwrap(),
            json!({ "id": "5", "type": "abort" })
        );
        assert!(envelope("x", "1", json!([1])).is_err());
    }

    #[test]
    fn split_model_id_splits_on_first_slash() {
        assert_eq!(
            split_model_id("anthropic/claude"),
            Some(("anthropic", "claude"))
        );
        assert_eq!(
            split_model_id("openrouter/meta/llama-3"),
            Some(("openrouter", "meta/llama-3"))
        );
        assert_eq!(split_model_id("no-slash"), None);
        assert_eq!(split_model_id("/x"), None);
    }

    #[test]
    fn response_success_resolves_pending_with_data() {
        let pending = new_pending();
        let (tx, rx) = oneshot::channel();
        pending.blocking_lock().insert("7".to_string(), tx);
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let msg = json!({ "id": "7", "type": "response", "command": "get_state", "success": true,
                          "data": { "sessionId": "abc" } });
        dispatch(&msg, &pending, &event_tx);
        assert_eq!(rx.blocking_recv().unwrap().unwrap()["sessionId"], "abc");
    }

    #[test]
    fn response_failure_resolves_pending_with_error() {
        let pending = new_pending();
        let (tx, rx) = oneshot::channel();
        pending.blocking_lock().insert("4".to_string(), tx);
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let msg = json!({ "id": "4", "type": "response", "command": "set_model", "success": false,
                          "error": "Model not found: x/y" });
        dispatch(&msg, &pending, &event_tx);
        let err = rx.blocking_recv().unwrap().unwrap_err();
        assert!(
            err.contains("set_model") && err.contains("Model not found"),
            "{err}"
        );
    }

    #[test]
    fn events_are_forwarded_with_their_type_and_full_object() {
        let pending = new_pending();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        dispatch(&json!({ "type": "agent_settled" }), &pending, &event_tx);
        dispatch(
            &json!({ "type": "tool_execution_start", "toolCallId": "c1" }),
            &pending,
            &event_tx,
        );
        assert_eq!(event_rx.try_recv().unwrap().0, "agent_settled");
        let (kind, event) = event_rx.try_recv().unwrap();
        assert_eq!(
            (kind.as_str(), event["toolCallId"].as_str()),
            ("tool_execution_start", Some("c1"))
        );
    }

    #[test]
    fn read_loop_splits_on_lf_only_and_tolerates_crlf() {
        // U+2028 inside a JSON string must not split the record.
        let input = "{\"type\":\"message_update\",\"note\":\"a\u{2028}b\"}\r\n{\"type\":\"agent_settled\"}\n";
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (_keep, shutdown_rx) = oneshot::channel::<()>();
        read_loop(
            std::io::Cursor::new(input.as_bytes().to_vec()),
            new_pending(),
            event_tx,
            shutdown_rx,
        );
        let (kind, event) = event_rx.try_recv().unwrap();
        assert_eq!(kind, "message_update");
        assert_eq!(event["note"], "a\u{2028}b");
        assert_eq!(event_rx.try_recv().unwrap().0, "agent_settled");
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn eof_fails_all_pending_requests() {
        let pending = new_pending();
        let (tx, rx) = oneshot::channel();
        pending.blocking_lock().insert("1".to_string(), tx);
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (_keep, shutdown_rx) = oneshot::channel::<()>();
        read_loop(
            std::io::Cursor::new(Vec::new()),
            pending,
            event_tx,
            shutdown_rx,
        );
        assert!(rx.blocking_recv().unwrap().unwrap_err().contains("exited"));
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
