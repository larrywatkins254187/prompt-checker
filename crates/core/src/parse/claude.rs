//! Parser for Claude Code session transcripts (`~/.claude/projects/<dir>/<session>.jsonl`).
//!
//! Each line is a JSON record. The ones we care about:
//! - `type: "user"`: human prompts (string content or `text` blocks) and `tool_result` blocks.
//! - `type: "assistant"`: one content block per line; several lines share one `message.id`.
//! - `type: "summary"`: auto-generated session title.
//!
//! Unknown record types are ignored so newer Claude Code versions keep working.

use std::collections::HashMap;

use serde_json::Value;

use crate::model::{MessageKind, ParsedMessage, ParsedSession, SourceKind, Usage};

/// Tools whose results carry the human's own words (answers to clarifying questions).
const HUMAN_INPUT_TOOLS: &[&str] = &["AskUserQuestion", "ExitPlanMode"];

pub fn parse(content: &str) -> Vec<ParsedSession> {
    let mut sessions: Vec<ParsedSession> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut seen_uuids: HashMap<String, ()> = HashMap::new();
    let mut summaries: Vec<(Option<String>, String)> = Vec::new();

    for (line_no, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<Value>(line) else {
            continue; // partially written last line, or garbage
        };
        let rtype = rec.get("type").and_then(Value::as_str).unwrap_or("");

        if rtype == "summary" {
            if let Some(s) = rec.get("summary").and_then(Value::as_str) {
                let sid = rec.get("sessionId").and_then(Value::as_str).map(String::from);
                summaries.push((sid, s.to_string()));
            }
            continue;
        }
        if rtype != "user" && rtype != "assistant" {
            continue;
        }
        let Some(session_id) = rec.get("sessionId").and_then(Value::as_str) else {
            continue;
        };
        let uuid = rec
            .get("uuid")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| format!("line{line_no}"));
        if seen_uuids.insert(format!("{session_id}/{uuid}"), ()).is_some() {
            continue;
        }

        let idx = *index.entry(session_id.to_string()).or_insert_with(|| {
            sessions.push(ParsedSession::new(SourceKind::Claude, session_id.to_string()));
            sessions.len() - 1
        });
        let session = &mut sessions[idx];
        if session.cwd.is_none() {
            session.cwd = str_field(&rec, "cwd");
        }
        if session.git_branch.is_none() {
            session.git_branch = str_field(&rec, "gitBranch").filter(|b| !b.is_empty());
        }
        if let Some(v) = str_field(&rec, "version") {
            session.cli_version = Some(v);
        }

        let timestamp = str_field(&rec, "timestamp");
        let is_sidechain = rec.get("isSidechain").and_then(Value::as_bool).unwrap_or(false);
        let is_meta = rec.get("isMeta").and_then(Value::as_bool).unwrap_or(false);
        let Some(message) = rec.get("message") else { continue };

        let mut out: Vec<ParsedMessage> = Vec::new();
        if rtype == "user" {
            if is_meta {
                continue; // injected skill text, caveats, etc.
            }
            let compact = rec.get("isCompactSummary").and_then(Value::as_bool).unwrap_or(false);
            parse_user(message, &uuid, compact, &tool_names, &mut out);
        } else {
            parse_assistant(message, &uuid, &mut tool_names, &mut out);
        }
        for mut m in out {
            m.timestamp = timestamp.clone();
            m.is_sidechain = is_sidechain;
            session.messages.push(m);
        }
    }

    for (sid, summary) in summaries {
        let target = match sid {
            Some(sid) => index.get(&sid).copied(),
            None => (sessions.len() == 1).then_some(0),
        };
        if let Some(i) = target {
            sessions[i].title = Some(summary);
        }
    }
    sessions
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(String::from)
}

fn parse_user(
    message: &Value,
    uuid: &str,
    compact: bool,
    tool_names: &HashMap<String, String>,
    out: &mut Vec<ParsedMessage>,
) {
    let content = message.get("content");
    let mut text_parts: Vec<String> = Vec::new();
    match content {
        Some(Value::String(s)) => text_parts.push(s.clone()),
        Some(Value::Array(blocks)) => {
            for (i, block) in blocks.iter().enumerate() {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            text_parts.push(t.to_string());
                        }
                    }
                    Some("image") => text_parts.push("[image]".to_string()),
                    Some("document") => text_parts.push("[document]".to_string()),
                    Some("tool_result") => {
                        let tool_id = block.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                        let tool_name = tool_names.get(tool_id).cloned();
                        let text = tool_result_text(block.get("content"));
                        let human = tool_name
                            .as_deref()
                            .map(|n| HUMAN_INPUT_TOOLS.contains(&n))
                            .unwrap_or(false);
                        let is_error = block.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                        let kind = if human && !is_error { MessageKind::Prompt } else { MessageKind::ToolResult };
                        let mut m = ParsedMessage::new(format!("{uuid}#{i}"), kind, text);
                        m.tool_name = tool_name;
                        m.is_error = is_error;
                        out.push(m);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    if text_parts.is_empty() {
        return;
    }
    let text = text_parts.join("\n");
    let kind = if compact { MessageKind::System } else { classify_user_text(&text) };
    // Text goes first so a prompt precedes tool results from the same record.
    out.insert(0, ParsedMessage::new(uuid.to_string(), kind, text));
}

/// Decide whether a user text record is a real prompt or harness noise.
pub fn classify_user_text(text: &str) -> MessageKind {
    let t = text.trim_start();
    const COMMAND_PREFIXES: &[&str] = &["<command-name>", "<command-message>", "<bash-input>", "<user-memory-input>"];
    const SYSTEM_PREFIXES: &[&str] = &[
        "<local-command-stdout>",
        "<local-command-stderr>",
        "<bash-stdout>",
        "<bash-stderr>",
        "<system-reminder>",
        "[Request interrupted by user",
        "Caveat: The messages below were generated",
        "This session is being continued from a previous conversation",
    ];
    if COMMAND_PREFIXES.iter().any(|p| t.starts_with(p)) {
        MessageKind::Command
    } else if SYSTEM_PREFIXES.iter().any(|p| t.starts_with(p)) || t.is_empty() {
        MessageKind::System
    } else {
        MessageKind::Prompt
    }
}

fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|it| match it.get("type").and_then(Value::as_str) {
                Some("text") => it.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                Some("image") => "[image]".to_string(),
                Some(other) => format!("[{other}]"),
                None => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn parse_assistant(
    message: &Value,
    uuid: &str,
    tool_names: &mut HashMap<String, String>,
    out: &mut Vec<ParsedMessage>,
) {
    let model = message
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.starts_with('<')) // "<synthetic>"
        .map(String::from);
    let api_id = message.get("id").and_then(Value::as_str).map(String::from);
    let usage = message.get("usage").map(|u| Usage {
        input_tokens: int(u, "input_tokens"),
        output_tokens: int(u, "output_tokens"),
        cache_read_tokens: int(u, "cache_read_input_tokens"),
        cache_creation_tokens: int(u, "cache_creation_input_tokens"),
    });

    let blocks: Vec<Value> = match message.get("content") {
        Some(Value::Array(b)) => b.clone(),
        Some(Value::String(s)) => vec![serde_json::json!({"type": "text", "text": s})],
        _ => vec![],
    };
    for (i, block) in blocks.iter().enumerate() {
        let id = format!("{uuid}#{i}");
        let msg = match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let t = block.get("text").and_then(Value::as_str).unwrap_or("");
                if t.trim().is_empty() {
                    continue;
                }
                ParsedMessage::new(id, MessageKind::Response, t.to_string())
            }
            Some("tool_use") | Some("server_tool_use") => {
                let name = block.get("name").and_then(Value::as_str).unwrap_or("tool").to_string();
                if let Some(tid) = block.get("id").and_then(Value::as_str) {
                    tool_names.insert(tid.to_string(), name.clone());
                }
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                let mut m = ParsedMessage::new(id, MessageKind::ToolCall, summarize_tool_input(&name, &input));
                m.tool_name = Some(name);
                m.tool_input = Some(input.to_string());
                m
            }
            _ => continue, // thinking / redacted_thinking
        };
        out.push(msg);
    }
    // Usage rides on the first row of each line; db aggregation takes MAX per api id,
    // so repeated (streamed) usage on several lines is counted once.
    if let Some(first) = out.first_mut() {
        first.model = model.clone();
        first.api_msg_id = api_id.clone();
        first.usage = usage;
    }
    for m in out.iter_mut() {
        if m.model.is_none() {
            m.model = model.clone();
        }
    }
}

fn int(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// One-line human readable summary of a tool call, used as the row's text.
pub fn summarize_tool_input(name: &str, input: &Value) -> String {
    let pick = |keys: &[&str]| -> Option<String> {
        keys.iter()
            .find_map(|k| input.get(*k).and_then(Value::as_str))
            .map(String::from)
    };
    let s = match name {
        "Bash" | "shell" | "exec_command" => pick(&["command", "cmd"]),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => pick(&["file_path", "notebook_path"]),
        "Grep" | "Glob" => pick(&["pattern"]),
        "WebFetch" => pick(&["url"]),
        "WebSearch" => pick(&["query"]),
        "Task" | "Agent" => pick(&["description", "prompt"]),
        "TodoWrite" => Some("update todo list".to_string()),
        _ => pick(&["description", "command", "file_path", "path", "query", "prompt"]),
    };
    match s {
        Some(s) => s,
        None => {
            let raw = input.to_string();
            if raw.chars().count() > 200 {
                format!("{}…", raw.chars().take(200).collect::<String>())
            } else {
                raw
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
{"type":"user","sessionId":"s1","uuid":"u1","timestamp":"2026-01-01T10:00:00Z","cwd":"C:\\Users\\me\\proj","gitBranch":"main","version":"2.1.0","message":{"role":"user","content":"fix the login bug"}}
{"type":"assistant","sessionId":"s1","uuid":"a1","timestamp":"2026-01-01T10:00:05Z","message":{"id":"msg_1","model":"claude-sonnet-5-5","content":[{"type":"thinking","thinking":""}],"usage":{"input_tokens":10,"output_tokens":50,"cache_read_input_tokens":100,"cache_creation_input_tokens":20}}}
{"type":"assistant","sessionId":"s1","uuid":"a2","timestamp":"2026-01-01T10:00:06Z","message":{"id":"msg_1","model":"claude-sonnet-5-5","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"npm test"}}],"usage":{"input_tokens":10,"output_tokens":80,"cache_read_input_tokens":100,"cache_creation_input_tokens":20}}}
{"type":"user","sessionId":"s1","uuid":"u2","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"1 failing","is_error":true}]}}
{"type":"assistant","sessionId":"s1","uuid":"a3","message":{"id":"msg_2","model":"claude-sonnet-5-5","content":[{"type":"tool_use","id":"t2","name":"AskUserQuestion","input":{}}]}}
{"type":"user","sessionId":"s1","uuid":"u3","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":"User answered: use JWT"}]}}
{"type":"user","sessionId":"s1","uuid":"u4","isMeta":true,"message":{"role":"user","content":"Caveat: ignore"}}
{"type":"user","sessionId":"s1","uuid":"u5","message":{"role":"user","content":"<command-name>/clear</command-name>"}}
{"type":"user","sessionId":"s1","uuid":"u6","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}
{"type":"summary","summary":"Login bug fix","leafUuid":"a3"}
{"type":"user","sessionId":"s1","uuid":"u1","message":{"role":"user","content":"duplicate uuid ignored"}}
not json
"#;

    #[test]
    fn parses_claude_session() {
        let sessions = parse(SAMPLE);
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.external_id, "s1");
        assert_eq!(s.cwd.as_deref(), Some("C:\\Users\\me\\proj"));
        assert_eq!(s.git_branch.as_deref(), Some("main"));
        assert_eq!(s.title.as_deref(), Some("Login bug fix"));

        let kinds: Vec<_> = s.messages.iter().map(|m| (m.kind, m.text.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (MessageKind::Prompt, "fix the login bug"),
                (MessageKind::ToolCall, "npm test"),
                (MessageKind::ToolResult, "1 failing"),
                (MessageKind::ToolCall, "{}"),
                (MessageKind::Prompt, "User answered: use JWT"),
                (MessageKind::Command, "<command-name>/clear</command-name>"),
                (MessageKind::System, "[Request interrupted by user]"),
            ]
        );
        let tool_result = &s.messages[2];
        assert!(tool_result.is_error);
        assert_eq!(tool_result.tool_name.as_deref(), Some("Bash"));
        let call = &s.messages[1];
        assert_eq!(call.api_msg_id.as_deref(), Some("msg_1"));
        assert_eq!(call.usage.as_ref().unwrap().output_tokens, 80);
        assert_eq!(call.model.as_deref(), Some("claude-sonnet-5-5"));
    }
}
