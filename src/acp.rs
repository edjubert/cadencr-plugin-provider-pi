//! ACP v1 server over stdio.
//!
//! Reads JSON-RPC 2.0 messages from stdin, dispatches them, and writes
//! responses/notifications to stdout. Each `session/new` spawns a Pi RPC
//! child; events from Pi are translated to ACP session updates.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::session::PiSession;

/// Run the ACP v1 server over stdio until stdin closes.
pub async fn run_acp_server(provider_args: &[std::ffi::OsString]) -> Result<std::process::ExitCode, String> {
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    let stdout = Arc::new(Mutex::new(std::io::stdout().lock()));

    // Active sessions: sessionId -> PiSession.
    let sessions: Arc<Mutex<HashMap<String, Arc<Mutex<PiSession>>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let provider_args: Vec<String> = provider_args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).map(String::from);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        let Some(method) = method else {
            // Response to a server-initiated request — not expected in this connector.
            continue;
        };

        let response = handle_request(
            &method,
            &params,
            &sessions,
            &stdout,
            &provider_args,
        ).await;

        if let Some(req_id) = id {
            let mut out = stdout.lock().await;
            let resp = match response {
                Ok(result) => json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": result,
                }),
                Err((code, message)) => json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": { "code": code, "message": message },
                }),
            };
            let _ = writeln!(out, "{resp}");
            let _ = out.flush();
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

async fn handle_request(
    method: &str,
    params: &Value,
    sessions: &Arc<Mutex<HashMap<String, Arc<Mutex<PiSession>>>>>,
    stdout: &Arc<Mutex<std::io::StdoutLock<'static>>>,
    provider_args: &[String],
) -> AcpResult {
    match method {
        "initialize" => handle_initialize(params),
        "session/new" => handle_session_new(params, sessions, stdout, provider_args).await,
        "session/prompt" => handle_session_prompt(params, sessions, stdout).await,
        "session/cancel" => handle_session_cancel(params, sessions).await,
        "session/set_config_option" => handle_set_config_option(params, sessions).await,
        "session/resume" => handle_session_resume(params, sessions, stdout, provider_args).await,
        "session/close" => handle_session_close(params, sessions).await,
        "session/load" => handle_session_load(params, sessions, stdout, provider_args).await,
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
    sessions: &Arc<Mutex<HashMap<String, Arc<Mutex<PiSession>>>>>,
    stdout: &Arc<Mutex<std::io::StdoutLock<'static>>>,
    provider_args: &[String],
) -> AcpResult {
    let cwd = params
        .pointer("/cwd")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/new requires `cwd`".to_string()))?;

    let session = PiSession::new(cwd, None, provider_args, stdout.clone())
        .await
        .map_err(|e| (-32000, e))?;

    let session_id = session.session_id().to_string();
    let config_options = session.config_options();

    sessions.lock().await.insert(session_id.clone(), Arc::new(Mutex::new(session)));

    Ok(json!({
        "sessionId": session_id,
        "configOptions": config_options,
    }))
}

async fn handle_session_prompt(
    params: &Value,
    sessions: &Arc<Mutex<HashMap<String, Arc<Mutex<PiSession>>>>>,
    stdout: &Arc<Mutex<std::io::StdoutLock<'static>>>,
) -> AcpResult {
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
        return Err((-32602, "session/prompt requires at least one text part".to_string()));
    }

    let sessions = sessions.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or((-32602, format!("unknown session: {session_id}")))?;

    session
        .lock()
        .await
        .prompt(&text, stdout)
        .await
        .map_err(|e| (-32000, e))?;

    Ok(Value::Null)
}

async fn handle_session_cancel(
    params: &Value,
    sessions: &Arc<Mutex<HashMap<String, Arc<Mutex<PiSession>>>>>,
) -> AcpResult {
    let session_id = params
        .pointer("/sessionId")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/cancel requires `sessionId`".to_string()))?;

    let sessions = sessions.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or((-32602, format!("unknown session: {session_id}")))?;

    session.lock().await.cancel().await.map_err(|e| (-32000, e))?;

    Ok(Value::Null)
}

async fn handle_set_config_option(
    params: &Value,
    sessions: &Arc<Mutex<HashMap<String, Arc<Mutex<PiSession>>>>>,
) -> AcpResult {
    let session_id = params
        .pointer("/sessionId")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/set_config_option requires `sessionId`".to_string()))?;

    let config_id = params
        .pointer("/configId")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/set_config_option requires `configId`".to_string()))?;

    let value = params
        .pointer("/value")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/set_config_option requires `value`".to_string()))?;

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
    sessions: &Arc<Mutex<HashMap<String, Arc<Mutex<PiSession>>>>>,
    stdout: &Arc<Mutex<std::io::StdoutLock<'static>>>,
    provider_args: &[String],
) -> AcpResult {
    let session_id = params
        .pointer("/sessionId")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/resume requires `sessionId`".to_string()))?;

    let cwd = params
        .pointer("/cwd")
        .and_then(Value::as_str)
        .ok_or((-32602, "session/resume requires `cwd`".to_string()))?;

    let session = PiSession::resume(cwd, session_id, provider_args, stdout.clone())
        .await
        .map_err(|e| (-32000, e))?;

    let config_options = session.config_options();

    sessions.lock().await.insert(session_id.to_string(), Arc::new(Mutex::new(session)));

    Ok(json!({
        "sessionId": session_id,
        "configOptions": config_options,
    }))
}

async fn handle_session_load(
    params: &Value,
    sessions: &Arc<Mutex<HashMap<String, Arc<Mutex<PiSession>>>>>,
    stdout: &Arc<Mutex<std::io::StdoutLock<'static>>>,
    provider_args: &[String],
) -> AcpResult {
    // Legacy load: same as resume but with the legacy method name.
    handle_session_resume(params, sessions, stdout, provider_args).await
}

async fn handle_session_close(
    params: &Value,
    sessions: &Arc<Mutex<HashMap<String, Arc<Mutex<PiSession>>>>>,
) -> AcpResult {
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
