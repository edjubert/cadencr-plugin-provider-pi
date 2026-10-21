//! Pure translation of Pi RPC events into ACP `session/update` payloads.
use serde_json::{json, Value};

/// Translate one Pi event (`type` + full object) into zero or more ACP
/// `update` objects. Unknown events yield an empty vec.
pub fn translate_event(kind: &str, event: &Value) -> Vec<Value> {
    match kind {
        "message_update" => message_update(event).into_iter().collect(),
        "tool_execution_start" => vec![tool_start(event)],
        "tool_execution_update" => vec![tool_update(event)],
        "tool_execution_end" => vec![tool_end(event)],
        _ => Vec::new(),
    }
}

fn message_update(event: &Value) -> Option<Value> {
    let inner = event.get("assistantMessageEvent")?;
    let update = match inner.get("type").and_then(Value::as_str)? {
        "text_delta" => "agent_message_chunk",
        "thinking_delta" => "agent_thought_chunk",
        _ => return None,
    };
    let delta = inner.get("delta").and_then(Value::as_str)?;
    Some(json!({
        "sessionUpdate": update,
        "content": { "type": "text", "text": delta },
    }))
}

fn tool_call_id(event: &Value) -> &str {
    event
        .get("toolCallId")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

fn tool_start(event: &Value) -> Value {
    let name = event
        .get("toolName")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let args = event.get("args").cloned().unwrap_or(Value::Null);
    let mut update = json!({
        "sessionUpdate": "tool_call",
        "toolCallId": tool_call_id(event),
        "title": name,
        "kind": tool_kind(name),
        "status": "in_progress",
        "rawInput": args,
    });
    if let Some(path) = args.get("path").and_then(Value::as_str) {
        update["locations"] = json!([{ "path": path }]);
    }
    update
}

fn tool_update(event: &Value) -> Value {
    let mut update = json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": tool_call_id(event),
        "status": "in_progress",
    });
    if let Some(text) = event
        .get("partialResult")
        .map(result_text)
        .filter(|t| !t.is_empty())
    {
        update["content"] = text_content(&text);
    }
    update
}

fn tool_end(event: &Value) -> Value {
    let failed = event
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut update = json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": tool_call_id(event),
        "status": if failed { "failed" } else { "completed" },
    });
    if let Some(result) = event.get("result") {
        let text = result_text(result);
        if !text.is_empty() {
            update["content"] = text_content(&text);
        }
        update["rawOutput"] = result.clone();
    }
    update
}

fn text_content(text: &str) -> Value {
    json!([{ "type": "content", "content": { "type": "text", "text": text } }])
}

/// Concatenate the `text` blocks of a tool result (`{"content":[{"type":"text","text":..}]}`).
fn result_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// ACP tool kind for a Pi built-in tool name.
pub fn tool_kind(tool_name: &str) -> &'static str {
    match tool_name {
        "read" => "read",
        "write" | "edit" => "edit",
        "grep" | "find" | "ls" => "search",
        "bash" => "execute",
        _ => "other",
    }
}

/// If `event` is the end of an assistant message, return its Pi `stopReason`.
pub fn assistant_stop_reason(kind: &str, event: &Value) -> Option<String> {
    if kind != "message_end" {
        return None;
    }
    let message = event.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    message
        .get("stopReason")
        .and_then(Value::as_str)
        .map(String::from)
}

/// If the assistant message ended with `stopReason: "error"` and carries an
/// `errorMessage`, return it so the user sees why the turn failed.
pub fn assistant_error_text(kind: &str, event: &Value) -> Option<String> {
    if assistant_stop_reason(kind, event).as_deref() != Some("error") {
        return None;
    }
    event
        .pointer("/message/errorMessage")
        .and_then(Value::as_str)
        .map(String::from)
}

/// Map a Pi assistant `stopReason` to an ACP stop reason. A missing reason
/// (no assistant message ended) is a normal `end_turn`.
pub fn map_stop_reason(pi_reason: Option<&str>) -> &'static str {
    match pi_reason {
        Some("length") => "max_tokens",
        Some("aborted") => "cancelled",
        _ => "end_turn", // "stop", "toolUse", "error" and anything unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(kind: &str, event: Value) -> Value {
        let mut updates = translate_event(kind, &event);
        assert_eq!(updates.len(), 1, "{updates:?}");
        updates.remove(0)
    }

    #[test]
    fn text_delta_becomes_agent_message_chunk() {
        let u = one(
            "message_update",
            json!({ "assistantMessageEvent":
                { "type": "text_delta", "contentIndex": 0, "delta": "Hello " } }),
        );
        assert_eq!(u["sessionUpdate"], "agent_message_chunk");
        assert_eq!(u["content"]["text"], "Hello ");
    }

    #[test]
    fn thinking_delta_becomes_agent_thought_chunk() {
        let u = one(
            "message_update",
            json!({ "assistantMessageEvent":
                { "type": "thinking_delta", "delta": "hmm" } }),
        );
        assert_eq!(u["sessionUpdate"], "agent_thought_chunk");
    }

    #[test]
    fn non_delta_message_updates_are_ignored() {
        for t in [
            "text_start",
            "text_end",
            "thinking_start",
            "toolcall_start",
            "toolcall_delta",
            "toolcall_end",
        ] {
            let ev = json!({ "assistantMessageEvent": { "type": t, "delta": "x" } });
            assert!(translate_event("message_update", &ev).is_empty(), "{t}");
        }
    }

    #[test]
    fn tool_start_carries_kind_input_and_location() {
        let u = one(
            "tool_execution_start",
            json!({ "toolCallId": "c1", "toolName": "read",
                "args": { "path": "src/a.rs" } }),
        );
        assert_eq!(u["sessionUpdate"], "tool_call");
        assert_eq!(u["kind"], "read");
        assert_eq!(u["status"], "in_progress");
        assert_eq!(u["rawInput"]["path"], "src/a.rs");
        assert_eq!(u["locations"][0]["path"], "src/a.rs");
    }

    #[test]
    fn bash_tool_is_execute_without_location() {
        let u = one(
            "tool_execution_start",
            json!({ "toolCallId": "c2", "toolName": "bash",
                "args": { "command": "ls -la" } }),
        );
        assert_eq!(u["kind"], "execute");
        assert!(u.get("locations").is_none());
    }

    #[test]
    fn tool_update_streams_partial_text() {
        let u = one(
            "tool_execution_update",
            json!({ "toolCallId": "c2", "toolName": "bash",
                "partialResult": { "content": [{ "type": "text", "text": "partial" }] } }),
        );
        assert_eq!(u["status"], "in_progress");
        assert_eq!(u["content"][0]["content"]["text"], "partial");
    }

    #[test]
    fn tool_end_success_and_failure() {
        let ok = one(
            "tool_execution_end",
            json!({ "toolCallId": "c2", "isError": false,
                "result": { "content": [{ "type": "text", "text": "total 48" }] } }),
        );
        assert_eq!(ok["status"], "completed");
        assert_eq!(ok["content"][0]["content"]["text"], "total 48");
        let bad = one(
            "tool_execution_end",
            json!({ "toolCallId": "c2", "isError": true,
                "result": { "content": [{ "type": "text", "text": "boom" }] } }),
        );
        assert_eq!(bad["status"], "failed");
    }

    #[test]
    fn unknown_events_yield_nothing() {
        for k in [
            "agent_start",
            "turn_start",
            "turn_end",
            "queue_update",
            "compaction_start",
            "weird",
        ] {
            assert!(translate_event(k, &json!({})).is_empty(), "{k}");
        }
    }

    #[test]
    fn stop_reason_comes_from_assistant_message_end_only() {
        let assistant = json!({ "message": { "role": "assistant", "stopReason": "length" } });
        let user = json!({ "message": { "role": "user", "content": "hi" } });
        assert_eq!(
            assistant_stop_reason("message_end", &assistant).as_deref(),
            Some("length")
        );
        assert_eq!(assistant_stop_reason("message_end", &user), None);
        assert_eq!(assistant_stop_reason("message_start", &assistant), None);
    }

    #[test]
    fn stop_reasons_map_to_acp() {
        assert_eq!(map_stop_reason(Some("stop")), "end_turn");
        assert_eq!(map_stop_reason(Some("toolUse")), "end_turn");
        assert_eq!(map_stop_reason(Some("error")), "end_turn");
        assert_eq!(map_stop_reason(Some("length")), "max_tokens");
        assert_eq!(map_stop_reason(Some("aborted")), "cancelled");
        assert_eq!(map_stop_reason(None), "end_turn");
    }

    #[test]
    fn error_text_only_for_error_stop_reason() {
        let err = json!({ "message": { "role": "assistant", "stopReason": "error",
            "errorMessage": "rate limited" } });
        assert_eq!(
            assistant_error_text("message_end", &err).as_deref(),
            Some("rate limited")
        );
        let ok = json!({ "message": { "role": "assistant", "stopReason": "stop",
            "errorMessage": "x" } });
        assert_eq!(assistant_error_text("message_end", &ok), None);
    }
}
