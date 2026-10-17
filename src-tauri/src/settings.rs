use pc_core::analysis::ContextMode;
use pc_core::db::Db;
use pc_core::ingest::SourceConfig;
use serde::{Deserialize, Serialize};

pub const DEFAULT_MODELS: &[&str] = &["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5", "claude-fable-5-1"];

/// User settings, stored as one JSON blob in the `settings` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub sources: Vec<SourceConfig>,
    pub sources_initialized: bool,
    /// Path to the `claude` executable; empty = find it on PATH.
    pub claude_path: Option<String>,
    /// Models offered in the analysis dialog.
    pub models: Vec<String>,
    pub default_model: String,
    pub default_mode: ContextMode,
    /// Upper bound on (estimated) input tokens sent to the model in one analysis.
    pub max_input_tokens: usize,
    /// Mask secrets before sending anything to the model.
    pub redact: bool,
    pub poll_seconds: u64,
    pub timeout_minutes: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            sources: vec![],
            sources_initialized: false,
            claude_path: None,
            models: DEFAULT_MODELS.iter().map(|s| s.to_string()).collect(),
            default_model: DEFAULT_MODELS[0].to_string(),
            default_mode: ContextMode::Condensed,
            max_input_tokens: 1_000_000,
            redact: true,
            poll_seconds: 30,
            timeout_minutes: 15,
        }
    }
}

const KEY: &str = "app_settings";

impl AppSettings {
    pub fn load(db: &Db) -> AppSettings {
        db.get_setting(KEY)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, db: &Db) -> anyhow::Result<()> {
        let mut s = self.clone();
        s.models.retain(|m| !m.trim().is_empty());
        if s.models.is_empty() {
            s.models = AppSettings::default().models;
        }
        s.max_input_tokens = s.max_input_tokens.clamp(50_000, 4_000_000);
        s.poll_seconds = s.poll_seconds.clamp(5, 3600);
        s.timeout_minutes = s.timeout_minutes.clamp(1, 120);
        db.set_setting(KEY, &serde_json::to_string(&s)?)
    }
}
