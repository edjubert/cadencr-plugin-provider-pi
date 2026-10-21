//! ACP v1 server over stdio.
//!
//! Reads JSON-RPC 2.0 messages from stdin, dispatches them, and writes
//! responses/notifications to stdout. Each `session/new` spawns a Pi RPC
//! child; events from Pi are translated to ACP session updates by the
//! session's background event loop.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::session::{PendingPermissions, PiSession};

/// Active sessions: sessionId → PiSession.
type SessionMap = Arc<Mutex<HashMap<String, Arc<Mutex<PiSession>>>>>;

/// Run the ACP v1 server over stdio until stdin closes.
pub async fn run_acp_server(
    provider_args: &[std::ffi::OsString],
) -> Result<std::process::ExitCode, String> {
    // Use blocking stdin in a dedicated thread, fed into a channel.
    let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<String>(64);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let reader = std::io::BufReader::new(stdin.lock());
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if line_tx.blocking_send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let stdout = Arc::new(Mutex::new(std::io::stdout()));

    // Active sessions: sessionId -> PiSession.
    let sessions: SessionMap = Arc::new(Mutex::new(HashMap::new()));

    // Pending permission requests from session event loops.
    let pending_permissions: PendingPermissions = Arc::new(Mutex::new(HashMap::new()));

    let provider_args: Vec<String> = provider_args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    while let Some(line) = line_rx.recv().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).map(String::from);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        // Response to a server-initiated request (e.g. session/request_permission).
        if method.is_none() {
            if let Some(req_id) = id.and_then(|v| v.as_u64()) {
                if let Some(sender) = pending_permissions.lock().await.remove(&req_id) {
                    // Extract the result (or error) from the JSON-RPC response.
                    let result = msg.get("result").cloned().unwrap_or(Value::Null);
                    let _ = sender.send(result);
                }
            }
            continue;
        }

        let Some(method) = method else { continue };

        // `session/prompt` waits for the whole turn: run it on its own task so this
        // loop keeps reading `session/cancel` and permission responses.
        if method == "session/prompt" {
            let sessions = sessions.clone();
            let stdout = stdout.clone();
            let provider_args = provider_args.clone();
            let pending_permissions = pending_permissions.clone();
            tokio::spawn(async move {
                let response = handle_request(
                    &method,
                    &params,
                    &sessions,
                    &stdout,
                    &provider_args,
                    &pending_permissions,
                )
                .await;
                if let Some(req_id) = id {
                    write_response(&stdout, &req_id, response).await;
                }
            });
            continue;
        }

        let response = handle_request(
            &method,
            &params,
            &sessions,
            &stdout,
            &provider_args,
            &pending_permissions,
        )
        .await;

        if let Some(req_id) = id {
            write_response(&stdout, &req_id, response).await;
        }
    }

    // Cleanup: shut down all sessions.
    let mut sessions = sessions.lock().await;
    for (_, session) in sessions.drain() {
        session.lock().await.shutdown().await;
    }

    Ok(std::process::ExitCode::SUCCESS)
}

type AcpResult = Result<Value, (i64, String)>;

/// Build a JSON-RPC response object.
fn jsonrpc_response(id: &Value, response: AcpResult) -> Value {
    match response {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        }),
    }
}

async fn write_response(stdout: &Arc<Mutex<std::io::Stdout>>, id: &Value, response: AcpResult) {
    let line = jsonrpc_response(id, response);
    let mut out = stdout.lock().await;
    if let Err(e) = writeln!(out, "{line}").and_then(|_| out.flush()) {
        eprintln!("acp: failed to write response: {e}");
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_request(
    method: &str,
    params: &Value,
    sessions: &SessionMap,
    stdout: &Arc<Mutex<std::io::Stdout>>,
    provider_args: &[String],
    pending_permissions: &PendingPermissions,
) -> AcpResult {
    match method {
        "initialize" => handle_initialize(params),
        "session/new" => {
            handle_session_new(params, sessions, stdout, provider_args, pending_permissions).await
        }
        "session/prompt" => handle_session_prompt(params, sessions).await,
        "session/cancel" => handle_session_cancel(params, sessions).await,
        "session/set_config_option" => handle_set_config_option(params, sessions).await,
        "session/resume" => {
            handle_session_resume(params, sessions, stdout, provider_args, pending_permissions)
                .await
        }
        "session/close" => handle_session_close(params, sessions).await,
        "session/load" => {
            handle_session_resume(params, sessions, stdout, provider_args, pending_permissions)
                .await
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

fn handle_initialize(params: &Value) -> AcpResult {
    let protocol_version = params
        .pointer("/protocolVersion")
        .and_then(Value::as_u64)
        .unwrap_or(1);

    Ok(json!({
        "protocolVersion": protocol_version,
        "agentCapabilities": {
            "loadSession": true,
        },
        "agentInfo": {
            "name": "cadencr-plugin-provider-pi",
            "title": "Cadencr Pi connector",
            "version": env!("CARGO_PKG_VERSION"),
        },
    }))
}

async fn handle_session_new(
    params: &Value,
    sessions: &SessionMap,
    stdout: &Arc<Mutex<std::io::Stdout>>,
    provider_args: &[String],
    pending_permissions: &PendingPermissions,
) -> AcpResult {
    let cwd = params
        .pointer("/cwd")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/new requires `cwd`".to_string()))?;

    let session = PiSession::new(
        cwd,
        provider_args,
        stdout.clone(),
        pending_permissions.clone(),
    )
    .await
    .map_err(|e| (-32000, e))?;

    let session_id = session.session_id().to_string();
    let config_options = session.config_options();

    sessions
        .lock()
        .await
        .insert(session_id.clone(), Arc::new(Mutex::new(session)));

    Ok(json!({
        "sessionId": session_id,
        "configOptions": config_options,
    }))
}

async fn handle_session_prompt(params: &Value, sessions: &SessionMap) -> AcpResult {
    let session_id = params
        .pointer("/sessionId")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/prompt requires `sessionId`".to_string()))?;

    let prompt_parts = params
        .pointer("/prompt")
        .and_then(Value::as_array)
        .ok_or((-32602, "session/prompt requires `prompt` array".to_string()))?;

    // Extract text content from prompt parts.
    let mut text = String::new();
    for part in prompt_parts {
        if part.pointer("/type").and_then(Value::as_str) == Some("text") {
            if let Some(t) = part.pointer("/text").and_then(Value::as_str) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
        }
    }

    if text.is_empty() {
        return Err((
            -32602,
            "session/prompt requires at least one text part".to_string(),
        ));
    }

    // Clone the session Arc and release the sessions lock before waiting.
    let session = {
        let sessions = sessions.lock().await;
        sessions
            .get(session_id)
            .ok_or((-32602, format!("unknown session: {session_id}")))?
            .clone()
    };

    // Send the prompt to Pi and get a receiver for the turn-end signal.
    let turn_end_rx = session
        .lock()
        .await
        .prompt(&text)
        .await
        .map_err(|e| (-32000, e))?;

    // Wait for the turn to complete (event loop resolves the receiver).
    let stop_reason = turn_end_rx
        .await
        .map_err(|_| (-32000, "pi exited before the turn finished".to_string()))?;

    Ok(json!({ "stopReason": stop_reason }))
}

async fn handle_session_cancel(params: &Value, sessions: &SessionMap) -> AcpResult {
    let session_id = params
        .pointer("/sessionId")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/cancel requires `sessionId`".to_string()))?;

    let sessions = sessions.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or((-32602, format!("unknown session: {session_id}")))?;

    session
        .lock()
        .await
        .cancel()
        .await
        .map_err(|e| (-32000, e))?;

    Ok(Value::Null)
}

async fn handle_set_config_option(params: &Value, sessions: &SessionMap) -> AcpResult {
    let session_id = params
        .pointer("/sessionId")
        .and_then(Value::as_str)
        .ok_or((
            -32602,
            "session/set_config_option requires `sessionId`".to_string(),
        ))?;

    let config_id = params.pointer("/configId").and_then(Value::as_str).ok_or((
        -32602,
        "session/set_config_option requires `configId`".to_string(),
    ))?;

    let value = params.pointer("/value").and_then(Value::as_str).ok_or((
        -32602,
        "session/set_config_option requires `value`".to_string(),
    ))?;

    let sessions = sessions.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or((-32602, format!("unknown session: {session_id}")))?;

    let config_options = session
        .lock()
        .await
        .set_config_option(config_id, value)
        .await
        .map_err(|e| (-32000, e))?;

    Ok(json!({ "configOptions": config_options }))
}

async fn handle_session_resume(
    params: &Value,
    sessions: &SessionMap,
    stdout: &Arc<Mutex<std::io::Stdout>>,
    provider_args: &[String],
    pending_permissions: &PendingPermissions,
) -> AcpResult {
    let session_id = params
        .pointer("/sessionId")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/resume requires `sessionId`".to_string()))?;

    let cwd = params
        .pointer("/cwd")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/resume requires `cwd`".to_string()))?;

    let session = PiSession::resume(
        cwd,
        session_id,
        provider_args,
        stdout.clone(),
        pending_permissions.clone(),
    )
    .await
    .map_err(|e| (-32000, e))?;

    let config_options = session.config_options();

    sessions
        .lock()
        .await
        .insert(session_id.to_string(), Arc::new(Mutex::new(session)));

    Ok(json!({
        "sessionId": session_id,
        "configOptions": config_options,
    }))
}

async fn handle_session_close(params: &Value, sessions: &SessionMap) -> AcpResult {
    let session_id = params
        .pointer("/sessionId")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/close requires `sessionId`".to_string()))?;

    let mut sessions = sessions.lock().await;
    if let Some(session) = sessions.remove(session_id) {
        session.lock().await.shutdown().await;
    }

    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonrpc_response_shapes() {
        let ok = jsonrpc_response(&json!(3), Ok(json!({ "stopReason": "end_turn" })));
        assert_eq!(ok["result"]["stopReason"], "end_turn");
        assert_eq!(ok["id"], 3);
        let err = jsonrpc_response(&json!("a"), Err((-32000, "boom".to_string())));
        assert_eq!(err["error"]["code"], -32000);
        assert_eq!(err["error"]["message"], "boom");
        assert!(err.get("result").is_none());
    }
}
