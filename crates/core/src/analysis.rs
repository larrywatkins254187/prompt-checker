//! Turning stored sessions into an analysis request for the model, and parsing the answer.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::db::{Criterion, Db, MessageRow};
use crate::redact::redact;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Prompt,
    Session,
    Project,
    Global,
    Draft,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::Prompt => "prompt",
            Scope::Session => "session",
            Scope::Project => "project",
            Scope::Global => "global",
            Scope::Draft => "draft",
        }
    }
}

/// How much of each session is sent to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextMode {
    /// Only the human's prompts.
    Prompts,
    /// Prompts and replies in full, tool calls/results shortened.
    Condensed,
    /// Everything (each item still capped so one huge tool output cannot crowd out the rest).
    Full,
}

impl ContextMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContextMode::Prompts => "prompts",
            ContextMode::Condensed => "condensed",
            ContextMode::Full => "full",
        }
    }
    fn lower(&self) -> Option<ContextMode> {
        match self {
            ContextMode::Full => Some(ContextMode::Condensed),
            ContextMode::Condensed => Some(ContextMode::Prompts),
            ContextMode::Prompts => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRequest {
    pub scope: Scope,
    pub target_id: Option<i64>,
    pub mode: ContextMode,
    pub model: String,
    /// Draft scope only: the prompt being written.
    pub draft_text: Option<String>,
    /// Draft scope only: optional notes about the project/situation.
    pub draft_context: Option<String>,
    /// Draft scope only: project folder the model may explore read-only.
    #[serde(default)]
    pub project_dir: Option<String>,
    /// Draft scope only: imported project whose folder should be used (resolved to `project_dir`).
    #[serde(default)]
    pub project_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltContext {
    #[serde(skip)]
    pub system_prompt: String,
    #[serde(skip)]
    pub user_prompt: String,
    #[serde(skip)]
    pub ref_map: Value,
    pub chars: usize,
    pub est_tokens: usize,
    pub prompt_count: usize,
    pub truncated: bool,
    pub notes: Vec<String>,
    /// Folder the CLI runs in with read-only tools (draft reviews with a project).
    pub project_dir: Option<String>,
}

/// Room kept free for the reviewer instructions and the model's reply.
const RESERVED_TOKENS: usize = 40_000;

/// Rough token count: ~4 characters per token for ASCII text, ~1.5 for other scripts
/// (Persian, CJK, …), which tokenise much more densely. Deliberately on the high side.
pub fn estimate_tokens(text: &str) -> usize {
    let (mut ascii, mut other) = (0usize, 0usize);
    for c in text.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            other += 1;
        }
    }
    ascii / 4 + other * 2 / 3 + 1
}

/// Longest prefix of `text` whose estimate fits in `budget` tokens.
fn cut_to_tokens(text: &str, budget: usize) -> &str {
    let mut used = 0f64;
    for (i, c) in text.char_indices() {
        used += if c.is_ascii() { 0.25 } else { 0.667 };
        if used > budget as f64 {
            return &text[..i];
        }
    }
    text
}

const ITEM_CAP_FULL: usize = 12_000;

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head} …[+{} chars]", s.chars().count() - max)
}

fn first_lines(s: &str, lines: usize, max: usize) -> String {
    let joined = s.lines().take(lines).collect::<Vec<_>>().join("\n");
    cut(&joined, max)
}

fn ts(m: &MessageRow) -> String {
    m.timestamp.as_deref().map(|t| t.chars().take(16).collect::<String>().replace('T', " ")).unwrap_or_default()
}

struct Labeler {
    next: usize,
    refs: Vec<Value>,
}

impl Labeler {
    fn label(&mut self, message_id: i64, session_id: i64) -> String {
        self.next += 1;
        let r = format!("P{}", self.next);
        self.refs.push(json!({"ref": r, "messageId": message_id, "sessionId": session_id}));
        r
    }
}

/// Render one session's messages as plain text for the model.
fn render_session(
    messages: &[MessageRow],
    session_id: i64,
    mode: ContextMode,
    labeler: &mut Labeler,
    target: Option<i64>,
) -> String {
    let mut out = String::new();
    for m in messages {
        if m.is_sidechain && mode != ContextMode::Full {
            continue;
        }
        let side = if m.is_sidechain { "  [subagent] " } else { "" };
        match m.kind.as_str() {
            "prompt" => {
                if m.is_sidechain {
                    out.push_str(&format!("{side}MAIN AGENT → SUBAGENT: {}\n\n", cut(&m.text, 2000)));
                    continue;
                }
                let r = labeler.label(m.id, session_id);
                let marker = if Some(m.id) == target { " ◀ TARGET PROMPT" } else { "" };
                let via = match m.tool_name.as_deref() {
                    Some(t) => format!(" (answer to {t})"),
                    None => String::new(),
                };
                out.push_str(&format!("[{r}] USER{via} {}{marker}:\n{}\n\n", ts(m), m.text.trim()));
            }
            "command" => {
                if mode != ContextMode::Prompts {
                    out.push_str(&format!("USER COMMAND: {}\n\n", cut(m.text.trim(), 500)));
                }
            }
            "response" => match mode {
                ContextMode::Prompts => {}
                ContextMode::Condensed => out.push_str(&format!("ASSISTANT: {}\n\n", cut(m.text.trim(), 1500))),
                ContextMode::Full => out.push_str(&format!("{side}ASSISTANT: {}\n\n", cut(m.text.trim(), ITEM_CAP_FULL))),
            },
            "tool_call" => {
                let name = m.tool_name.as_deref().unwrap_or("tool");
                match mode {
                    ContextMode::Prompts => {}
                    ContextMode::Condensed => out.push_str(&format!("  · {name}: {}\n", cut(m.text.trim(), 200))),
                    ContextMode::Full => {
                        let input = m.tool_input.as_deref().unwrap_or("");
                        out.push_str(&format!("{side}TOOL CALL {name}: {}\n  input: {}\n", cut(m.text.trim(), 500), cut(input, 3000)));
                    }
                }
            }
            "tool_result" => match mode {
                ContextMode::Prompts => {}
                ContextMode::Condensed => {
                    if m.is_error {
                        out.push_str(&format!("    ✗ error: {}\n", first_lines(m.text.trim(), 3, 300)));
                    }
                }
                ContextMode::Full => {
                    let flag = if m.is_error { " (ERROR)" } else { "" };
                    out.push_str(&format!("{side}TOOL RESULT{flag}: {}\n\n", cut(m.text.trim(), ITEM_CAP_FULL)));
                }
            },
            "system" => {
                if m.text.starts_with("[Request interrupted") {
                    out.push_str("(user interrupted the assistant)\n\n");
                }
            }
            _ => {}
        }
    }
    out
}

struct SessionBlock {
    header: String,
    session_id: i64,
    messages: Vec<MessageRow>,
}

fn render_blocks(blocks: &[SessionBlock], mode: ContextMode, target: Option<i64>) -> (String, Labeler) {
    let mut labeler = Labeler { next: 0, refs: vec![] };
    let mut text = String::new();
    for b in blocks {
        text.push_str(&b.header);
        text.push_str(&render_session(&b.messages, b.session_id, mode, &mut labeler, target));
        text.push('\n');
    }
    (text, labeler)
}

fn session_header(db: &Db, session_id: i64, index: Option<usize>) -> Result<String> {
    let s = db.get_session_summary(session_id)?;
    let n = index.map(|i| format!(" {i}")).unwrap_or_default();
    Ok(format!(
        "===== SESSION{n} · {} · {} · started {} · models: {} · {} tool calls, {} tool errors =====\n",
        s.source,
        s.title.as_deref().unwrap_or("(untitled)"),
        s.started_at.as_deref().unwrap_or("?"),
        s.models.as_deref().unwrap_or("?"),
        s.tool_call_count,
        s.error_count
    ))
}

/// Build the text sent to the model, fitting it into `max_input_tokens` (estimated).
pub fn build_context(db: &Db, req: &AnalysisRequest, criteria: &[Criterion], max_input_tokens: usize, do_redact: bool) -> Result<BuiltContext> {
    let budget = max_input_tokens.saturating_sub(RESERVED_TOKENS).max(1_000);
    let mut notes = Vec::new();
    let mut truncated = false;
    let target_prompt = (req.scope == Scope::Prompt).then_some(req.target_id).flatten();

    let (body, labeler) = match req.scope {
        Scope::Draft => {
            let draft = req.draft_text.as_deref().unwrap_or("").trim();
            if draft.is_empty() {
                bail!("The draft prompt is empty");
            }
            let mut body = String::new();
            if let Some(ctx) = req.draft_context.as_deref().filter(|c| !c.trim().is_empty()) {
                body.push_str(&format!("SITUATION / PROJECT NOTES FROM THE USER:\n{}\n\n", ctx.trim()));
            }
            if let Some(dir) = req.project_dir.as_deref().filter(|d| !d.trim().is_empty()) {
                body.push_str(&format!("PROJECT: the prompt is for the project in your current working directory ({dir}).\n\n"));
            }
            body.push_str(&format!("[DRAFT] PROMPT THE USER IS ABOUT TO SEND TO A CODING AGENT:\n{draft}\n"));
            let refs = Labeler { next: 0, refs: vec![json!({"ref": "DRAFT"})] };
            (body, refs)
        }
        Scope::Prompt | Scope::Session => {
            let (session_id, messages) = if req.scope == Scope::Prompt {
                let mid = req.target_id.ok_or_else(|| anyhow!("missing prompt id"))?;
                let sid = db.message_session(mid)?;
                let all = db.session_messages(sid, false)?;
                // Conversation up to the target prompt plus everything until the next human prompt.
                let pos = all.iter().position(|m| m.id == mid).ok_or_else(|| anyhow!("prompt not found"))?;
                let end = all[pos + 1..]
                    .iter()
                    .position(|m| m.kind == "prompt" && !m.is_sidechain && m.tool_name.is_none())
                    .map(|p| pos + 1 + p + 1) // include the next prompt: the user's reaction is evidence
                    .unwrap_or(all.len());
                (sid, all[..end].to_vec())
            } else {
                let sid = req.target_id.ok_or_else(|| anyhow!("missing session id"))?;
                (sid, db.session_messages(sid, false)?)
            };
            let blocks = vec![SessionBlock { header: session_header(db, session_id, None)?, session_id, messages }];
            fit(&blocks, req.mode, budget, target_prompt, &mut notes, &mut truncated)
        }
        Scope::Project | Scope::Global => {
            let session_ids: Vec<(i64, String)> = if req.scope == Scope::Project {
                let pid = req.target_id.ok_or_else(|| anyhow!("missing project id"))?;
                let name = db.project_name(pid)?;
                db.list_sessions(pid)?.into_iter().rev().map(|s| (s.id, name.clone())).collect()
            } else {
                let mut all = Vec::new();
                for p in db.list_projects()? {
                    for s in db.list_sessions(p.id)? {
                        all.push((s.id, p.name.clone(), s.started_at.clone().unwrap_or_default()));
                    }
                }
                all.sort_by(|a, b| a.2.cmp(&b.2));
                all.into_iter().map(|(id, name, _)| (id, name)).collect()
            };
            let mut blocks = Vec::new();
            for (i, (sid, pname)) in session_ids.iter().enumerate() {
                let mut header = session_header(db, *sid, Some(i + 1))?;
                if req.scope == Scope::Global {
                    header = format!("##### PROJECT: {pname}\n{header}");
                }
                let messages = db.session_messages(*sid, false)?;
                if messages.iter().any(|m| m.kind == "prompt") {
                    blocks.push(SessionBlock { header, session_id: *sid, messages });
                }
            }
            if blocks.is_empty() {
                bail!("No prompts found to analyse");
            }
            fit(&blocks, req.mode, budget, None, &mut notes, &mut truncated)
        }
    };

    let prompt_count = labeler.refs.len();
    if prompt_count == 0 {
        bail!("No prompts found to analyse");
    }
    let body = if do_redact { redact(&body) } else { body };
    let project_dir = if req.scope == Scope::Draft {
        req.project_dir.clone().filter(|d| !d.trim().is_empty())
    } else {
        None
    };
    let mut system_prompt = system_prompt(req.scope, criteria);
    if project_dir.is_some() {
        system_prompt.push_str(EXPLORE_INSTRUCTIONS);
    }
    let user_prompt = user_prompt(req.scope, &body, &notes);
    let chars = system_prompt.chars().count() + user_prompt.chars().count();
    let est_tokens = estimate_tokens(&system_prompt) + estimate_tokens(&user_prompt);
    Ok(BuiltContext {
        system_prompt,
        user_prompt,
        ref_map: Value::Array(labeler.refs),
        chars,
        est_tokens,
        prompt_count,
        truncated,
        notes,
        project_dir,
    })
}

/// Render blocks at the requested detail; if too large, lower the detail, then drop the
/// oldest sessions, then hard-cut.
fn fit(
    blocks: &[SessionBlock],
    mode: ContextMode,
    budget: usize,
    target: Option<i64>,
    notes: &mut Vec<String>,
    truncated: &mut bool,
) -> (String, Labeler) {
    let mut m = mode;
    loop {
        let (text, labeler) = render_blocks(blocks, m, target);
        if estimate_tokens(&text) <= budget {
            if m != mode {
                notes.push(format!("Context was too large for '{}' mode; sent in '{}' mode instead.", mode.as_str(), m.as_str()));
                *truncated = true;
            }
            return (text, labeler);
        }
        match m.lower() {
            Some(l) => m = l,
            None => break,
        }
    }
    if m != mode {
        notes.push(format!("Context was too large for '{}' mode; sent in '{}' mode instead.", mode.as_str(), m.as_str()));
    }
    *truncated = true;
    // Keep the most recent sessions that fit.
    let mut start = blocks.len();
    let mut size = 0usize;
    while start > 0 {
        let (t, _) = render_blocks(&blocks[start - 1..start], m, target);
        let n = estimate_tokens(&t);
        if size + n > budget && start < blocks.len() {
            break;
        }
        size += n;
        start -= 1;
    }
    if start > 0 {
        notes.push(format!("Only the {} most recent of {} sessions fit; older ones were left out.", blocks.len() - start, blocks.len()));
    }
    let (mut text, labeler) = render_blocks(&blocks[start..], m, target);
    if estimate_tokens(&text) > budget {
        let keep = cut_to_tokens(&text, budget).len();
        text.truncate(keep);
        text.push_str("\n…[cut to fit the context budget]\n");
        notes.push("The transcript was cut to fit the context budget.".into());
    }
    (text, labeler)
}

const EXPLORE_INSTRUCTIONS: &str = r#"

PROJECT ACCESS: your current working directory is the developer's project. Before answering, use the Read, Glob and Grep tools to learn what the prompt depends on: README, CLAUDE.md or AGENTS.md, the project layout, and the files, modules, tests and commands the prompt touches or should mention. Keep it focused (about 20 tool calls at most). You cannot and must not modify anything.
Use what you find to judge the prompt (missing context the agent would need, wrong assumptions, names that do not exist) and to make the rewrite concrete: real file paths, existing conventions, the actual test/build commands. Name the files you looked at in the summary. Your final message must still be ONLY the JSON object."#;

fn criteria_block(criteria: &[Criterion]) -> String {
    criteria
        .iter()
        .filter(|c| c.enabled)
        .map(|c| format!("- key \"{}\" — {} (weight {}): {}", c.key, c.name, c.weight, c.description))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn system_prompt(scope: Scope, criteria: &[Criterion]) -> String {
    let focus = match scope {
        Scope::Prompt => "You are reviewing ONE prompt (marked TARGET PROMPT) that a developer gave to an AI coding agent. Earlier conversation is included only as context; what happened after the prompt shows how well it worked. Score only the target prompt. Provide exactly one rewrite, for the target prompt.",
        Scope::Session => "You are reviewing every prompt a developer gave to an AI coding agent during one session. Judge how well the developer steered the agent across the whole session.",
        Scope::Project => "You are reviewing a developer's prompting across all sessions of one project. Look for recurring habits, both good and bad, and how they changed over time.",
        Scope::Global => "You are reviewing a developer's prompting across all of their projects. Identify their most important recurring habits and the few changes that would help them most.",
        Scope::Draft => "You are reviewing a prompt the developer has NOT sent yet. There is no outcome to look at; judge it on its own and help them improve it before they send it. Provide exactly one rewrite with ref \"DRAFT\".",
    };
    format!(
        r#"You are an expert reviewer of how developers prompt AI coding agents (Claude Code, Codex CLI).
{focus}

The transcript labels each human prompt as [P1], [P2], …. Lines starting with ASSISTANT, TOOL CALL, TOOL RESULT show what the agent did in response. Use them as evidence: misunderstandings, wrong files touched, tool errors, rework, and the user having to correct or repeat themselves often trace back to the prompt.

Score against these criteria, each from 0 to 10 (5 = typical, 8+ = excellent, below 4 = caused real problems):
{criteria}

Rules:
- Be specific and evidence-based. Every mistake must cite the prompt reference(s) it comes from and, where possible, what it led to.
- Judge prompts in context. A short follow-up such as "yes, do that" is fine when the context makes it clear; do not penalise brevity that worked.
- Do not invent facts that are not in the transcript. Text shown as [REDACTED] was hidden for privacy; ignore it.
- Rewrites must keep the developer's intent and show concretely what a better prompt would say.
- Write every field in English, including rewrites, even when the prompts are in another language. Quotes from prompts stay in their original language. Keep each field concise.

Reply with ONLY a JSON object (no markdown fences, no commentary) of this shape:
{{
  "overall_score": <number 0-10, one decimal>,
  "summary": "<2-4 sentence verdict>",
  "criteria": [{{"key": "<criterion key>", "score": <0-10>, "comment": "<one or two sentences>"}}],
  "strengths": ["<what the developer did well>"],
  "mistakes": [{{"title": "<short name>", "severity": "high|medium|low", "refs": ["P3"], "quote": "<short exact quote from the prompt>", "why": "<why it is a problem>", "impact": "<what it caused, citing evidence; empty if unknown>", "fix": "<what to do instead>"}}],
  "rewrites": [{{"ref": "P3", "original": "<original prompt, may be shortened>", "improved": "<the better prompt>", "explanation": "<what changed and why>"}}],
  "patterns": ["<recurring habit across prompts/sessions>"],
  "tips": ["<concrete habit to adopt next time>"]
}}
Include every enabled criterion in "criteria". Order mistakes from most to least severe. Give rewrites for the prompts that would benefit most (at most 5)."#,
        criteria = criteria_block(criteria)
    )
}

fn user_prompt(scope: Scope, body: &str, notes: &[String]) -> String {
    let what = match scope {
        Scope::Draft => "Here is the draft prompt to review.",
        _ => "Here is the transcript to review.",
    };
    let mut s = format!("{what}\n\n<transcript>\n{body}</transcript>\n");
    if !notes.is_empty() {
        s.push_str(&format!("\nNote: {}\n", notes.join(" ")));
    }
    s.push_str("\nRespond with the JSON object only.");
    s
}

/// Extract and normalise the JSON object from the model's reply.
/// Returns the result object and the overall score.
pub fn parse_result(text: &str, criteria: &[Criterion]) -> Result<(Value, Option<f64>)> {
    let mut v = extract_json(text).ok_or_else(|| anyhow!("The model did not return a JSON object"))?;
    if !v.is_object() {
        bail!("The model's reply was not a JSON object");
    }
    let clamp = |x: f64| (x.clamp(0.0, 10.0) * 10.0).round() / 10.0;

    // Normalise criterion scores.
    if let Some(Value::Array(items)) = v.get_mut("criteria") {
        for it in items.iter_mut() {
            if let Some(s) = it.get("score").and_then(as_f64) {
                it["score"] = json!(clamp(s));
            }
        }
    }
    let mut score = v.get("overall_score").and_then(as_f64).map(clamp);
    if score.is_none() {
        // Fall back to a weighted mean of the criteria.
        let mut sum = 0.0;
        let mut wsum = 0.0;
        if let Some(Value::Array(items)) = v.get("criteria") {
            for it in items {
                let key = it.get("key").and_then(Value::as_str).unwrap_or("");
                let w = criteria.iter().find(|c| c.key == key).map(|c| c.weight).unwrap_or(1.0);
                if let Some(s) = it.get("score").and_then(as_f64) {
                    sum += s * w;
                    wsum += w;
                }
            }
        }
        if wsum > 0.0 {
            score = Some(clamp(sum / wsum));
        }
    }
    if let Some(s) = score {
        v["overall_score"] = json!(s);
    }
    for key in ["strengths", "mistakes", "rewrites", "patterns", "tips", "criteria"] {
        if !v.get(key).map(Value::is_array).unwrap_or(false) {
            v[key] = json!([]);
        }
    }
    if !v.get("summary").map(Value::is_string).unwrap_or(false) {
        v["summary"] = json!("");
    }
    Ok((v, score))
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn extract_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    // ```json ... ``` fences
    if let Some(start) = t.find("```") {
        let after = &t[start + 3..];
        let after = after.strip_prefix("json").unwrap_or(after);
        if let Some(end) = after.find("```") {
            if let Ok(v) = serde_json::from_str::<Value>(after[..end].trim()) {
                return Some(v);
            }
        }
    }
    // Outermost braces, then each '{' in turn.
    let last = t.rfind('}')?;
    for (i, _) in t.match_indices('{') {
        if i >= last {
            break;
        }
        if let Ok(v) = serde_json::from_str::<Value>(&t[i..=last]) {
            if v.is_object() {
                return Some(v);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::default_criteria;
    use crate::model::{MessageKind, ParsedMessage, ParsedSession, SourceKind};

    fn seed(db: &Db) -> i64 {
        let mut s = ParsedSession::new(SourceKind::Claude, "s1".into());
        s.cwd = Some("/home/me/app".into());
        let mut push = |uuid: &str, kind: MessageKind, text: &str, ts: &str| {
            let mut m = ParsedMessage::new(uuid.into(), kind, text.into());
            m.timestamp = Some(ts.into());
            s.messages.push(m);
        };
        push("1", MessageKind::Prompt, "fix it", "2026-01-01T00:00:01Z");
        push("2", MessageKind::ToolCall, "npm test", "2026-01-01T00:00:02Z");
        push("3", MessageKind::ToolResult, &"x".repeat(5000), "2026-01-01T00:00:03Z");
        push("4", MessageKind::Response, "Which bug? my key is sk-ant-abcdefghijklmnopqrstuvwxyz", "2026-01-01T00:00:04Z");
        push("5", MessageKind::Prompt, "the login one", "2026-01-01T00:00:05Z");
        push("6", MessageKind::Response, "Fixed.", "2026-01-01T00:00:06Z");
        push("7", MessageKind::Prompt, "thanks", "2026-01-01T00:00:07Z");
        db.upsert_session(&s, "linux", "/tmp/x.jsonl").unwrap()
    }

    fn req(scope: Scope, target: i64, mode: ContextMode) -> AnalysisRequest {
        AnalysisRequest { scope, target_id: Some(target), mode, model: "sonnet".into(), draft_text: None, draft_context: None, project_dir: None, project_id: None }
    }

    #[test]
    fn builds_session_context_with_refs_and_redaction() {
        let db = Db::open_in_memory().unwrap();
        let sid = seed(&db);
        let c = build_context(&db, &req(Scope::Session, sid, ContextMode::Condensed), &default_criteria(), 100_000, true).unwrap();
        assert_eq!(c.prompt_count, 3);
        assert!(c.user_prompt.contains("[P1] USER"));
        assert!(c.user_prompt.contains("[P3] USER"));
        assert!(c.user_prompt.contains("[REDACTED]"));
        assert!(!c.user_prompt.contains("sk-ant-"));
        assert!(!c.user_prompt.contains(&"x".repeat(400)), "condensed mode must not include long tool output");
        assert!(c.system_prompt.contains("\"goal\""));

        let prompts = build_context(&db, &req(Scope::Session, sid, ContextMode::Prompts), &default_criteria(), 100_000, true).unwrap();
        assert!(!prompts.user_prompt.contains("ASSISTANT"));
    }

    #[test]
    fn prompt_scope_includes_outcome_and_marks_target() {
        let db = Db::open_in_memory().unwrap();
        let sid = seed(&db);
        let msgs = db.session_messages(sid, false).unwrap();
        let p1 = msgs.iter().find(|m| m.text == "fix it").unwrap().id;
        let c = build_context(&db, &req(Scope::Prompt, p1, ContextMode::Condensed), &default_criteria(), 100_000, true).unwrap();
        assert!(c.user_prompt.contains("fix it ◀ TARGET PROMPT") || c.user_prompt.contains("◀ TARGET PROMPT:\nfix it"));
        assert!(c.user_prompt.contains("the login one"), "the next prompt is evidence of the outcome");
        assert!(!c.user_prompt.contains("thanks"));
        assert_eq!(c.ref_map[0]["messageId"], json!(p1));
    }

    #[test]
    fn degrades_mode_when_over_budget() {
        let db = Db::open_in_memory().unwrap();
        let sid = seed(&db);
        let c = build_context(&db, &req(Scope::Session, sid, ContextMode::Full), &default_criteria(), 600, true).unwrap();
        assert!(c.truncated);
        assert!(!c.notes.is_empty());
    }

    #[test]
    fn parses_fenced_json_and_fills_defaults() {
        // goal weighs 1.5, efficiency 0.5 -> (10*1.5 + 4*0.5) / 2 = 8.5
        let reply = "Here you go:\n```json\n{\"criteria\":[{\"key\":\"goal\",\"score\":12},{\"key\":\"efficiency\",\"score\":\"4\"}],\"summary\":\"ok\"}\n```";
        let (v, score) = parse_result(reply, &default_criteria()).unwrap();
        assert_eq!(v["criteria"][0]["score"], json!(10.0));
        assert_eq!(score, Some(8.5));
        assert!(v["mistakes"].is_array());
        assert!(parse_result("no json here", &default_criteria()).is_err());
    }
}
