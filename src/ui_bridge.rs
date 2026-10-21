//! Bridge Pi extension dialogs (`extension_ui_request`) to ACP
//! `session/request_permission`.
use serde_json::{json, Value};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

use crate::pi_rpc::PiRpc;
use crate::session::PendingPermissions;

/// What the user is asked, derived from a Pi dialog request.
#[derive(Debug, PartialEq)]
pub enum Dialog {
    Confirm { title: String },
    Select { title: String, options: Vec<String> },
}

/// Classification of an `extension_ui_request`.
#[derive(Debug, PartialEq)]
pub enum UiRequest {
    /// `select` / `confirm`: ask the ACP host.
    Ask(Dialog),
    /// `input` / `editor`: no ACP equivalent, answer `cancelled`.
    Unsupported,
    /// `notify`, `setStatus`, ...: no response expected.
    Ignore,
}

pub fn classify(event: &Value) -> UiRequest {
    let title = event
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Pi extension")
        .to_string();
    match event.get("method").and_then(Value::as_str) {
        Some("confirm") => UiRequest::Ask(Dialog::Confirm { title }),
        Some("select") => {
            let options = event
                .get("options")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|o| o.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if options.is_empty() {
                UiRequest::Unsupported
            } else {
                UiRequest::Ask(Dialog::Select { title, options })
            }
        }
        Some("input") | Some("editor") => UiRequest::Unsupported,
        _ => UiRequest::Ignore,
    }
}

impl Dialog {
    pub fn title(&self) -> &str {
        match self {
            Dialog::Confirm { title } | Dialog::Select { title, .. } => title,
        }
    }

    /// ACP permission options as `(optionId, name, kind)`.
    pub fn options(&self) -> Vec<(String, String, &'static str)> {
        match self {
            Dialog::Confirm { .. } => vec![
                ("yes".into(), "Yes".into(), "allow_once"),
                ("no".into(), "No".into(), "reject_once"),
            ],
            Dialog::Select { options, .. } => options
                .iter()
                .enumerate()
                .map(|(i, name)| (i.to_string(), name.clone(), "allow_once"))
                .collect(),
        }
    }

    /// Pi response body for the option the host selected (`None` = cancelled).
    pub fn response_payload(&self, selected: Option<&str>) -> Value {
        match (self, selected) {
            (_, None) => json!({ "cancelled": true }),
            (Dialog::Confirm { .. }, Some(id)) => json!({ "confirmed": id == "yes" }),
            (Dialog::Select { options, .. }, Some(id)) => {
                match id.parse::<usize>().ok().and_then(|i| options.get(i)) {
                    Some(value) => json!({ "value": value }),
                    None => json!({ "cancelled": true }),
                }
            }
        }
    }
}

/// Extract the selected `optionId` from an ACP permission response, if any.
pub fn selected_option(response: &Value) -> Option<String> {
    if response.pointer("/outcome/outcome").and_then(Value::as_str) == Some("cancelled") {
        return None;
    }
    response
        .pointer("/outcome/optionId")
        .and_then(Value::as_str)
        .map(String::from)
}

/// Handle one `extension_ui_request`. Meant to be `tokio::spawn`ed: it waits
/// for the host's decision (up to 300 s) without blocking the event loop.
pub async fn handle_ui_request(
    rpc: PiRpc,
    stdout: Arc<Mutex<std::io::Stdout>>,
    session_id: String,
    event: Value,
    pending: PendingPermissions,
    counter: Arc<AtomicU64>,
) {
    let request_id = event
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let dialog = match classify(&event) {
        UiRequest::Ignore => return,
        UiRequest::Unsupported => {
            respond(&rpc, &request_id, json!({ "cancelled": true })).await;
            return;
        }
        UiRequest::Ask(dialog) => dialog,
    };

    let req_id = counter.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = oneshot::channel::<Value>();
    pending.lock().await.insert(req_id, tx);

    let options: Vec<Value> = dialog
        .options()
        .into_iter()
        .map(|(id, name, kind)| json!({ "optionId": id, "name": name, "kind": kind }))
        .collect();
    let request = json!({
        "jsonrpc": "2.0",
        "id": req_id,
        "method": "session/request_permission",
        "params": {
            "sessionId": session_id,
            "toolCall": { "toolCallId": request_id, "title": dialog.title() },
            "options": options,
        },
    });
    {
        let mut out = stdout.lock().await;
        if let Err(e) = writeln!(out, "{request}").and_then(|_| out.flush()) {
            eprintln!("pi ui-bridge: failed to write permission request: {e}");
        }
    }

    let selected = match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
        Ok(Ok(response)) => selected_option(&response),
        _ => {
            pending.lock().await.remove(&req_id);
            None
        }
    };
    respond(
        &rpc,
        &request_id,
        dialog.response_payload(selected.as_deref()),
    )
    .await;
}

async fn respond(rpc: &PiRpc, request_id: &str, payload: Value) {
    if let Err(e) = rpc.respond_ui(request_id, payload).await {
        eprintln!("pi ui-bridge: failed to answer dialog `{request_id}`: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_select_confirm_input_and_fire_and_forget() {
        let select =
            json!({ "method": "select", "title": "Allow?", "options": ["Allow", "Block"] });
        assert_eq!(
            classify(&select),
            UiRequest::Ask(Dialog::Select {
                title: "Allow?".into(),
                options: vec!["Allow".into(), "Block".into()]
            })
        );
        let confirm = json!({ "method": "confirm", "title": "Clear?", "message": "m" });
        assert_eq!(
            classify(&confirm),
            UiRequest::Ask(Dialog::Confirm {
                title: "Clear?".into()
            })
        );
        assert_eq!(
            classify(&json!({ "method": "input", "title": "x" })),
            UiRequest::Unsupported
        );
        assert_eq!(
            classify(&json!({ "method": "editor" })),
            UiRequest::Unsupported
        );
        assert_eq!(
            classify(&json!({ "method": "notify", "message": "hi" })),
            UiRequest::Ignore
        );
        assert_eq!(
            classify(&json!({ "method": "select", "options": [] })),
            UiRequest::Unsupported
        );
    }

    #[test]
    fn confirm_options_and_payloads() {
        let d = Dialog::Confirm {
            title: "Clear?".into(),
        };
        let ids: Vec<String> = d.options().into_iter().map(|(id, _, _)| id).collect();
        assert_eq!(ids, vec!["yes", "no"]);
        assert_eq!(
            d.response_payload(Some("yes")),
            json!({ "confirmed": true })
        );
        assert_eq!(
            d.response_payload(Some("no")),
            json!({ "confirmed": false })
        );
        assert_eq!(d.response_payload(None), json!({ "cancelled": true }));
    }

    #[test]
    fn select_option_ids_are_indexes_and_map_back_to_values() {
        let d = Dialog::Select {
            title: "t".into(),
            options: vec!["Allow".into(), "Block".into()],
        };
        let opts = d.options();
        assert_eq!((opts[1].0.as_str(), opts[1].1.as_str()), ("1", "Block"));
        assert_eq!(d.response_payload(Some("0")), json!({ "value": "Allow" }));
        assert_eq!(d.response_payload(Some("9")), json!({ "cancelled": true }));
        assert_eq!(d.response_payload(Some("x")), json!({ "cancelled": true }));
    }

    #[test]
    fn selected_option_reads_acp_outcome() {
        assert_eq!(
            selected_option(&json!({ "outcome": { "outcome": "selected", "optionId": "1" } }))
                .as_deref(),
            Some("1")
        );
        assert_eq!(
            selected_option(&json!({ "outcome": { "outcome": "cancelled" } })),
            None
        );
        assert_eq!(selected_option(&Value::Null), None);
    }
}
