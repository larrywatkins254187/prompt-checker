//! Markdown / JSON export of sessions and analyses.

use serde_json::{json, Value};

use crate::db::{AnalysisRow, SessionDetail};

fn fence(text: &str) -> String {
    // Use a fence longer than any backtick run inside the text.
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(longest.max(2) + 1)
}

pub fn session_markdown(d: &SessionDetail) -> String {
    let s = &d.session;
    let mut out = format!(
        "# {}\n\n- Project: {}\n- Source: {} ({})\n- Folder: {}\n- Branch: {}\n- Started: {}\n- Ended: {}\n- Models: {}\n- Prompts: {} · Tool calls: {} · Tool errors: {}\n\n---\n\n",
        s.title.clone().or(s.first_prompt.clone()).unwrap_or_else(|| s.external_id.clone()).lines().next().unwrap_or(""),
        d.project_name,
        s.source,
        s.location,
        s.cwd.as_deref().unwrap_or("-"),
        s.git_branch.as_deref().unwrap_or("-"),
        s.started_at.as_deref().unwrap_or("-"),
        s.ended_at.as_deref().unwrap_or("-"),
        s.models.as_deref().unwrap_or("-"),
        s.prompt_count,
        s.tool_call_count,
        s.error_count
    );
    for m in &d.messages {
        let ts = m.timestamp.as_deref().unwrap_or("");
        match m.kind.as_str() {
            "prompt" => out.push_str(&format!("## 🧑 Prompt · {ts}\n\n{}\n\n", m.text.trim())),
            "command" => out.push_str(&format!("**Command:** `{}`\n\n", m.text.trim().replace('`', "'"))),
            "response" => out.push_str(&format!("### 🤖 Assistant\n\n{}\n\n", m.text.trim())),
            "tool_call" => {
                let f = fence(&m.text);
                out.push_str(&format!("**Tool call · {}**\n\n{f}\n{}\n{f}\n\n", m.tool_name.as_deref().unwrap_or("tool"), m.text.trim()));
            }
            "tool_result" => {
                let f = fence(&m.text);
                let label = if m.is_error { "Tool error" } else { "Tool result" };
                out.push_str(&format!("<details><summary>{label}</summary>\n\n{f}\n{}\n{f}\n\n</details>\n\n", m.text.trim()));
            }
            _ => {}
        }
    }
    out
}

pub fn session_json(d: &SessionDetail) -> Value {
    json!(d)
}

pub fn analysis_markdown(a: &AnalysisRow) -> String {
    let r = a.result.clone().unwrap_or(Value::Null);
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let mut out = format!(
        "# Prompt analysis — {}\n\n- Scope: {}\n- Model: {}\n- Context: {}\n- Date: {}\n- Overall score: {}\n\n",
        a.target_label.as_deref().unwrap_or(&a.scope),
        a.scope,
        a.model,
        a.context_mode,
        a.created_at,
        a.overall_score.map(|x| format!("{x:.1} / 10")).unwrap_or_else(|| "-".into())
    );
    out.push_str(&format!("## Summary\n\n{}\n\n", s(&r["summary"])));
    if let Some(items) = r["criteria"].as_array().filter(|a| !a.is_empty()) {
        out.push_str("## Scores\n\n| Criterion | Score | Comment |\n|---|---|---|\n");
        for c in items {
            out.push_str(&format!("| {} | {} | {} |\n", s(&c["key"]), c["score"], s(&c["comment"]).replace('|', "\\|")));
        }
        out.push('\n');
    }
    let list = |out: &mut String, title: &str, key: &str| {
        if let Some(items) = r[key].as_array().filter(|a| !a.is_empty()) {
            out.push_str(&format!("## {title}\n\n"));
            for i in items {
                out.push_str(&format!("- {}\n", i.as_str().unwrap_or(&i.to_string())));
            }
            out.push('\n');
        }
    };
    list(&mut out, "Strengths", "strengths");
    if let Some(items) = r["mistakes"].as_array().filter(|a| !a.is_empty()) {
        out.push_str("## Mistakes\n\n");
        for m in items {
            let refs = m["refs"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            out.push_str(&format!("### {} ({}) {}\n\n", s(&m["title"]), s(&m["severity"]), refs));
            if !s(&m["quote"]).is_empty() {
                out.push_str(&format!("> {}\n\n", s(&m["quote"])));
            }
            out.push_str(&format!("- **Why:** {}\n- **Impact:** {}\n- **Fix:** {}\n\n", s(&m["why"]), s(&m["impact"]), s(&m["fix"])));
        }
    }
    if let Some(items) = r["rewrites"].as_array().filter(|a| !a.is_empty()) {
        out.push_str("## Rewrites\n\n");
        for w in items {
            out.push_str(&format!(
                "### {}\n\n**Original**\n\n> {}\n\n**Improved**\n\n{}\n\n_{}_\n\n",
                s(&w["ref"]),
                s(&w["original"]).replace('\n', "\n> "),
                s(&w["improved"]),
                s(&w["explanation"])
            ));
        }
    }
    list(&mut out, "Recurring patterns", "patterns");
    list(&mut out, "Tips", "tips");
    out
}
