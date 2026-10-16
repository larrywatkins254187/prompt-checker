use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Claude,
    Codex,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Claude => "claude",
            SourceKind::Codex => "codex",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(SourceKind::Claude),
            "codex" => Some(SourceKind::Codex),
            _ => None,
        }
    }
}

/// What a stored message row represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// Something the human typed (or answered) - the thing we analyse.
    Prompt,
    /// Slash command / bash-mode input typed by the human.
    Command,
    /// Assistant text.
    Response,
    ToolCall,
    ToolResult,
    /// Interruptions, compaction summaries, injected context, etc.
    System,
}

impl MessageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageKind::Prompt => "prompt",
            MessageKind::Command => "command",
            MessageKind::Response => "response",
            MessageKind::ToolCall => "tool_call",
            MessageKind::ToolResult => "tool_result",
            MessageKind::System => "system",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
}

#[derive(Debug, Clone)]
pub struct ParsedMessage {
    /// Stable identifier inside the session; re-ingesting the same file yields the same uuids.
    pub uuid: String,
    pub kind: MessageKind,
    pub timestamp: Option<String>,
    pub model: Option<String>,
    pub text: String,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub is_error: bool,
    pub is_sidechain: bool,
    /// API message id; usage is reported once per id (Claude streams one id over several lines).
    pub api_msg_id: Option<String>,
    pub usage: Option<Usage>,
}

impl ParsedMessage {
    pub fn new(uuid: String, kind: MessageKind, text: String) -> Self {
        ParsedMessage {
            uuid,
            kind,
            timestamp: None,
            model: None,
            text,
            tool_name: None,
            tool_input: None,
            is_error: false,
            is_sidechain: false,
            api_msg_id: None,
            usage: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedSession {
    pub source: SourceKind,
    pub external_id: String,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cli_version: Option<String>,
    pub title: Option<String>,
    /// Session-level token totals when the log reports them cumulatively (Codex).
    pub usage_total: Option<Usage>,
    pub messages: Vec<ParsedMessage>,
}

impl ParsedSession {
    pub fn new(source: SourceKind, external_id: String) -> Self {
        ParsedSession {
            source,
            external_id,
            cwd: None,
            git_branch: None,
            cli_version: None,
            title: None,
            usage_total: None,
            messages: Vec::new(),
        }
    }
}
