//! Claude Code deletes transcripts older than `cleanupPeriodDays` (default 30) from
//! `~/.claude/settings.json`. These helpers check and, on request, raise that limit so
//! old sessions are not lost before the app imports them.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

pub const CLAUDE_DEFAULT_CLEANUP_DAYS: i64 = 30;
pub const RECOMMENDED_DAYS: i64 = 99_999;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionStatus {
    pub settings_path: String,
    pub exists: bool,
    /// Value in the file, if set.
    pub cleanup_period_days: Option<i64>,
    /// Effective retention in days.
    pub effective_days: i64,
    /// True when sessions will be deleted within a year.
    pub at_risk: bool,
}

pub fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))?;
    Some(base.join("settings.json"))
}

pub fn check() -> Result<RetentionStatus> {
    let path = settings_path().context("no home directory")?;
    let exists = path.is_file();
    let value = if exists {
        let text = std::fs::read_to_string(&path)?;
        serde_json::from_str::<Value>(&text).ok()
    } else {
        None
    };
    let days = value.as_ref().and_then(|v| v.get("cleanupPeriodDays")).and_then(Value::as_i64);
    let effective = days.unwrap_or(CLAUDE_DEFAULT_CLEANUP_DAYS);
    Ok(RetentionStatus {
        settings_path: path.to_string_lossy().into_owned(),
        exists,
        cleanup_period_days: days,
        effective_days: effective,
        at_risk: effective < 365,
    })
}

/// Set `cleanupPeriodDays`, keeping every other setting. A backup is written next to the file.
pub fn set_cleanup_days(days: i64) -> Result<RetentionStatus> {
    let path = settings_path().context("no home directory")?;
    let mut value = if path.is_file() {
        let text = std::fs::read_to_string(&path)?;
        std::fs::write(path.with_extension("json.bak"), &text).context("writing backup")?;
        serde_json::from_str::<Value>(&text).context("settings.json is not valid JSON; not touching it")?
    } else {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        json!({})
    };
    let obj = value.as_object_mut().context("settings.json is not a JSON object")?;
    obj.insert("cleanupPeriodDays".into(), json!(days));
    std::fs::write(&path, serde_json::to_string_pretty(&value)? + "\n")?;
    check()
}
