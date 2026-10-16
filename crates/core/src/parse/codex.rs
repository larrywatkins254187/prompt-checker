//! Parser for OpenAI Codex CLI rollouts (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`).
//!
//! Current format wraps each record as `{"timestamp", "type", "payload"}` with types
//! `session_meta`, `turn_context`, `response_item`, `event_msg`. Older rollouts put the
//! session meta on the first line and response items directly on following lines;
//! both are handled.

use std::collections::HashMap;

use serde_json::Value;

use super::claude::summarize_tool_input;
use crate::model::{MessageKind, ParsedMessage, ParsedSession, SourceKind, Usage};

/// User-role messages Codex injects itself (AGENTS.md, environment info).
const INJECTED_PREFIXES: &[&str] = &[
    "<environment_context>",
    "<user_instructions>",
    "# AGENTS.md instructions",
    "<permissions instructions>",
    "<user_shell_command>",
];

pub fn parse(content: &str, fallback_id: &str) -> Option<ParsedSession> {
    let mut session = ParsedSession::new(SourceKind::Codex, fallback_id.to_string());
    let mut model: Option<String> = None;
    let mut call_names: HashMap<String, String> = HashMap::new();
    let mut any = false;

    for (line_no, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<Value>(line) else { continue };
        any = true;
        let timestamp = rec.get("timestamp").and_then(Value::as_str).map(String::from);

        let (rtype, payload) = match (rec.get("type").and_then(Value::as_str), rec.get("payload")) {
            (Some(t), Some(p)) => (t.to_string(), p.clone()),
            // Legacy: first line is bare session meta.
            (None, None) if line_no == 0 || rec.get("instructions").is_some() => ("session_meta".to_string(), rec.clone()),
            // Legacy: bare response item.
            (Some(_), None) => ("response_item".to_string(), rec.clone()),
            _ => continue,
        };

        match rtype.as_str() {
            "session_meta" => {
                if let Some(id) = payload.get("id").and_then(Value::as_str) {
                    session.external_id = id.to_string();
                }
                if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
                    session.cwd = Some(cwd.to_string());
                }
                if let Some(v) = payload.get("cli_version").and_then(Value::as_str) {
                    session.cli_version = Some(v.to_string());
                }
                if let Some(b) = payload.pointer("/git/branch").and_then(Value::as_str) {
                    session.git_branch = Some(b.to_string());
                }
                if let Some(m) = payload.get("model").and_then(Value::as_str) {
                    model = Some(m.to_string());
                }
            }
            "turn_context" => {
                if let Some(m) = payload.get("model").and_then(Value::as_str) {
                    model = Some(m.to_string());
                }
                if session.cwd.is_none() {
                    session.cwd = payload.get("cwd").and_then(Value::as_str).map(String::from);
                }
            }
            "event_msg" => {
                if payload.get("type").and_then(Value::as_str) == Some("token_count") {
                    if let Some(t) = payload.pointer("/info/total_token_usage") {
                        let input = t.get("input_tokens").and_then(Value::as_i64).unwrap_or(0);
                        let cached = t.get("cached_input_tokens").and_then(Value::as_i64).unwrap_or(0);
                        session.usage_total = Some(Usage {
                            input_tokens: (input - cached).max(0),
                            output_tokens: t.get("output_tokens").and_then(Value::as_i64).unwrap_or(0),
                            cache_read_tokens: cached,
                            cache_creation_tokens: 0,
                        });
                    }
                }
            }
            "response_item" => {
                let uuid = format!("c{line_no}");
                if let Some(mut m) = parse_item(&payload, uuid, &mut call_names) {
                    m.timestamp = timestamp;
                    if m.kind == MessageKind::Response {
                        m.model = model.clone();
                    }
                    session.messages.push(m);
                }
            }
            _ => {}
        }
    }
    any.then_some(session)
}

fn content_text(payload: &Value) -> String {
    match payload.get("content") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|it| match it.get("type").and_then(Value::as_str) {
                Some("input_text") | Some("output_text") | Some("text") => {
                    it.get("text").and_then(Value::as_str).map(String::from)
                }
                Some("input_image") => Some("[image]".to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn parse_item(p: &Value, uuid: String, call_names: &mut HashMap<String, String>) -> Option<ParsedMessage> {
    let ptype = p.get("type").and_then(Value::as_str).unwrap_or("");
    match ptype {
        "message" => {
            let role = p.get("role").and_then(Value::as_str).unwrap_or("");
            let text = content_text(p);
            if text.trim().is_empty() {
                return None;
            }
            let kind = match role {
                "user" => {
                    let t = text.trim_start();
                    if INJECTED_PREFIXES.iter().any(|pre| t.starts_with(pre)) {
                        MessageKind::System
                    } else {
                        MessageKind::Prompt
                    }
                }
                "assistant" => MessageKind::Response,
                _ => return None, // developer / system instructions
            };
            Some(ParsedMessage::new(uuid, kind, text))
        }
        "function_call" | "custom_tool_call" => {
            let name = p.get("name").and_then(Value::as_str).unwrap_or("tool").to_string();
            if let Some(id) = p.get("call_id").and_then(Value::as_str) {
                call_names.insert(id.to_string(), name.clone());
            }
            let raw = p
                .get("arguments")
                .or_else(|| p.get("input"))
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::String(raw.clone()));
            let summary = match &parsed {
                Value::Object(_) => {
                    // shell's command is an argv array
                    if let Some(Value::Array(argv)) = parsed.get("command") {
                        argv.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")
                    } else {
                        summarize_tool_input(&name, &parsed)
                    }
                }
                Value::String(s) => s.lines().next().unwrap_or("").to_string(),
                other => other.to_string(),
            };
            let mut m = ParsedMessage::new(uuid, MessageKind::ToolCall, summary);
            m.tool_name = Some(name);
            m.tool_input = Some(raw);
            Some(m)
        }
        "local_shell_call" => {
            let cmd = p
                .pointer("/action/command")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            if let Some(id) = p.get("call_id").and_then(Value::as_str) {
                call_names.insert(id.to_string(), "shell".to_string());
            }
            let mut m = ParsedMessage::new(uuid, MessageKind::ToolCall, cmd);
            m.tool_name = Some("shell".to_string());
            m.tool_input = p.get("action").map(Value::to_string);
            Some(m)
        }
        "function_call_output" | "custom_tool_call_output" | "local_shell_call_output" => {
            let name = p
                .get("call_id")
                .and_then(Value::as_str)
                .and_then(|id| call_names.get(id).cloned());
            let (text, is_error) = tool_output(p.get("output"));
            let mut m = ParsedMessage::new(uuid, MessageKind::ToolResult, text);
            m.tool_name = name;
            m.is_error = is_error;
            Some(m)
        }
        "web_search_call" => {
            let q = p.pointer("/action/query").and_then(Value::as_str).unwrap_or("").to_string();
            let mut m = ParsedMessage::new(uuid, MessageKind::ToolCall, q);
            m.tool_name = Some("web_search".to_string());
            Some(m)
        }
        _ => None, // reasoning etc.
    }
}

/// Codex tool output is a string that is often itself JSON `{"output": "...", "metadata": {"exit_code": N}}`.
fn tool_output(v: Option<&Value>) -> (String, bool) {
    let v = match v {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).unwrap_or(Value::String(s.clone())),
        Some(other) => other.clone(),
        None => return (String::new(), false),
    };
    match &v {
        Value::Object(_) => {
            let text = v
                .get("output")
                .or_else(|| v.get("content"))
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| v.to_string());
            let exit = v.pointer("/metadata/exit_code").and_then(Value::as_i64).unwrap_or(0);
            let failed = v.get("success").and_then(Value::as_bool) == Some(false);
            (text, exit != 0 || failed)
        }
        Value::String(s) => (s.clone(), false),
        other => (other.to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_format() {
        let src = r#"
{"timestamp":"2026-01-02T09:00:00Z","type":"session_meta","payload":{"id":"abc-123","cwd":"/home/me/app","cli_version":"0.50.0","git":{"branch":"dev"}}}
{"timestamp":"2026-01-02T09:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}}
{"timestamp":"2026-01-02T09:00:01Z","type":"turn_context","payload":{"cwd":"/home/me/app","model":"gpt-5-codex"}}
{"timestamp":"2026-01-02T09:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"add a README"}]}}
{"timestamp":"2026-01-02T09:00:02Z","type":"response_item","payload":{"type":"reasoning","summary":[]}}
{"timestamp":"2026-01-02T09:00:03Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"ls\",\"-la\"]}","call_id":"call_1"}}
{"timestamp":"2026-01-02T09:00:04Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":"{\"output\":\"oops\",\"metadata\":{\"exit_code\":2}}"}}
{"timestamp":"2026-01-02T09:00:05Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done."}]}}
{"timestamp":"2026-01-02T09:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":50}}}}
"#;
        let s = parse(src, "fallback").unwrap();
        assert_eq!(s.external_id, "abc-123");
        assert_eq!(s.cwd.as_deref(), Some("/home/me/app"));
        assert_eq!(s.git_branch.as_deref(), Some("dev"));
        let kinds: Vec<_> = s.messages.iter().map(|m| (m.kind, m.text.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (MessageKind::System, "<environment_context>cwd</environment_context>"),
                (MessageKind::Prompt, "add a README"),
                (MessageKind::ToolCall, "ls -la"),
                (MessageKind::ToolResult, "oops"),
                (MessageKind::Response, "Done."),
            ]
        );
        assert!(s.messages[3].is_error);
        assert_eq!(s.messages[3].tool_name.as_deref(), Some("shell"));
        assert_eq!(s.messages[4].model.as_deref(), Some("gpt-5-codex"));
        let u = s.usage_total.unwrap();
        assert_eq!((u.input_tokens, u.cache_read_tokens, u.output_tokens), (600, 400, 50));
    }

    #[test]
    fn parses_legacy_format() {
        let src = r#"{"id":"old-1","timestamp":"2025-05-01T00:00:00Z","instructions":null}
{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}
{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}"#;
        let s = parse(src, "fallback").unwrap();
        assert_eq!(s.external_id, "old-1");
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0].kind, MessageKind::Prompt);
    }
}
