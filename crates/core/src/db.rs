use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{ParsedSession, Usage};
use crate::pricing;

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id          INTEGER PRIMARY KEY,
    key         TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    path        TEXT,
    location    TEXT NOT NULL,
    merged_into INTEGER REFERENCES projects(id),
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id                    INTEGER PRIMARY KEY,
    source                TEXT NOT NULL,
    external_id           TEXT NOT NULL,
    project_id            INTEGER NOT NULL REFERENCES projects(id),
    location              TEXT NOT NULL,
    file_path             TEXT,
    cwd                   TEXT,
    git_branch            TEXT,
    cli_version           TEXT,
    title                 TEXT,
    started_at            TEXT,
    ended_at              TEXT,
    models                TEXT,
    prompt_count          INTEGER NOT NULL DEFAULT 0,
    message_count         INTEGER NOT NULL DEFAULT 0,
    tool_call_count       INTEGER NOT NULL DEFAULT 0,
    error_count           INTEGER NOT NULL DEFAULT 0,
    input_tokens          INTEGER NOT NULL DEFAULT 0,
    output_tokens         INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    est_cost              REAL,
    UNIQUE (source, external_id)
);
CREATE INDEX IF NOT EXISTS sessions_project ON sessions(project_id);

CREATE TABLE IF NOT EXISTS messages (
    id                    INTEGER PRIMARY KEY,
    session_id            INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    uuid                  TEXT NOT NULL,
    seq                   INTEGER NOT NULL,
    kind                  TEXT NOT NULL,
    timestamp             TEXT,
    model                 TEXT,
    text                  TEXT NOT NULL,
    tool_name             TEXT,
    tool_input            TEXT,
    is_error              INTEGER NOT NULL DEFAULT 0,
    is_sidechain          INTEGER NOT NULL DEFAULT 0,
    api_msg_id            TEXT,
    input_tokens          INTEGER,
    output_tokens         INTEGER,
    cache_read_tokens     INTEGER,
    cache_creation_tokens INTEGER,
    UNIQUE (session_id, uuid)
);
CREATE INDEX IF NOT EXISTS messages_session ON messages(session_id, timestamp, seq);
CREATE INDEX IF NOT EXISTS messages_kind ON messages(kind);

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    text, content='messages', content_rowid='id', tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE OF text ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO messages_fts(rowid, text) VALUES (new.id, new.text);
END;

CREATE TABLE IF NOT EXISTS source_files (
    path        TEXT PRIMARY KEY,
    size        INTEGER NOT NULL,
    mtime       INTEGER NOT NULL,
    ingested_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS criteria (
    id          INTEGER PRIMARY KEY,
    key         TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL,
    weight      REAL NOT NULL DEFAULT 1,
    enabled     INTEGER NOT NULL DEFAULT 1,
    sort        INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS analyses (
    id            INTEGER PRIMARY KEY,
    scope         TEXT NOT NULL,
    target_id     INTEGER,
    model         TEXT NOT NULL,
    context_mode  TEXT NOT NULL,
    status        TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    finished_at   TEXT,
    overall_score REAL,
    result_json   TEXT,
    ref_map_json  TEXT,
    input_text    TEXT,
    raw_output    TEXT,
    error         TEXT,
    cost_usd      REAL,
    duration_ms   INTEGER,
    truncated     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS analyses_target ON analyses(scope, target_id);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

pub fn default_criteria() -> Vec<Criterion> {
    let c = |key: &str, name: &str, description: &str| Criterion {
        id: None,
        key: key.into(),
        name: name.into(),
        description: description.into(),
        weight: 1.0,
        enabled: true,
    };
    vec![
        c("clarity", "Clarity & specificity", "Is the request unambiguous? Does it say exactly what to build, change or answer, instead of leaving the model to guess?"),
        c("context", "Context", "Does it give the background the model needs: relevant files, current behaviour, error messages, why the change is wanted?"),
        c("success", "Success criteria", "Does it define what 'done' looks like and how to verify it (tests to pass, expected output, acceptance checks)?"),
        c("scope", "Scope & decomposition", "Is the task sized sensibly? Large work split into steps, unrelated asks not mixed into one prompt?"),
        c("constraints", "Constraints & preferences", "Are technology choices, style, things to avoid and limits stated up front rather than discovered after the model gets them wrong?"),
        c("feedback", "Feedback quality", "When correcting the model, are follow-ups precise (what was wrong, what to do instead) rather than vague ('still broken', 'try again')?"),
        c("efficiency", "Efficiency", "Does the prompting avoid wasted turns and tokens: repeated instructions, needless back-and-forth, re-explaining things already said?"),
    ]
}

// ---------------------------------------------------------------------------
// Row types sent to the UI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_creation: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: i64,
    pub name: String,
    pub path: Option<String>,
    pub location: String,
    pub session_count: i64,
    pub prompt_count: i64,
    pub first_activity: Option<String>,
    pub last_activity: Option<String>,
    pub tokens: Tokens,
    pub est_cost: Option<f64>,
    pub sources: Vec<String>,
    pub latest_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: i64,
    pub project_id: i64,
    pub source: String,
    pub location: String,
    pub external_id: String,
    pub title: Option<String>,
    pub first_prompt: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cli_version: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub models: Option<String>,
    pub prompt_count: i64,
    pub message_count: i64,
    pub tool_call_count: i64,
    pub error_count: i64,
    pub tokens: Tokens,
    pub est_cost: Option<f64>,
    pub file_path: Option<String>,
    pub latest_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    pub id: i64,
    pub uuid: String,
    pub kind: String,
    pub timestamp: Option<String>,
    pub model: Option<String>,
    pub text: String,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub is_error: bool,
    pub is_sidechain: bool,
    /// True when `text` was cut for display; fetch the full row with `get_message`.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session: SessionSummary,
    pub project_name: String,
    pub messages: Vec<MessageRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub message_id: i64,
    pub session_id: i64,
    pub project_id: i64,
    pub project_name: String,
    pub session_title: Option<String>,
    pub kind: String,
    pub snippet: String,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Criterion {
    pub id: Option<i64>,
    pub key: String,
    pub name: String,
    pub description: String,
    pub weight: f64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRow {
    pub id: i64,
    pub scope: String,
    pub target_id: Option<i64>,
    pub target_label: Option<String>,
    pub model: String,
    pub context_mode: String,
    pub status: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub overall_score: Option<f64>,
    pub result: Option<Value>,
    pub ref_map: Option<Value>,
    pub input_text: Option<String>,
    pub raw_output: Option<String>,
    pub error: Option<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptScore {
    pub message_id: i64,
    pub score: f64,
    pub analysis_id: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub projects: i64,
    pub sessions: i64,
    pub prompts: i64,
    pub tool_calls: i64,
    pub tool_errors: i64,
    pub tokens: Tokens,
    pub est_cost: f64,
    pub analyses: i64,
    pub by_day: Vec<DayStat>,
    pub by_model: Vec<ModelStat>,
    pub by_project: Vec<ProjectStat>,
    pub by_source: Vec<SourceStat>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayStat {
    pub day: String,
    pub sessions: i64,
    pub prompts: i64,
    pub est_cost: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStat {
    pub model: String,
    pub responses: i64,
    pub output_tokens: i64,
    pub est_cost: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStat {
    pub project_id: i64,
    pub name: String,
    pub sessions: i64,
    pub prompts: i64,
    pub output_tokens: i64,
    pub est_cost: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStat {
    pub source: String,
    pub location: String,
    pub sessions: i64,
    pub prompts: i64,
}

/// New analysis to be recorded before the CLI runs.
pub struct NewAnalysis<'a> {
    pub scope: &'a str,
    pub target_id: Option<i64>,
    pub model: &'a str,
    pub context_mode: &'a str,
    pub input_text: Option<&'a str>,
    pub ref_map: Option<&'a Value>,
    pub truncated: bool,
}

// ---------------------------------------------------------------------------

pub struct Db {
    pub conn: Connection,
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Project grouping key: same folder on the same machine/distro -> same project.
pub fn project_key(location: &str, cwd: &str) -> String {
    format!("{location}|{}", normalize_path(cwd))
}

pub fn normalize_path(p: &str) -> String {
    let mut s = p.trim().replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    // Windows paths are case-insensitive.
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        s = s.to_lowercase();
    }
    s
}

pub fn project_name_from_path(p: &str) -> String {
    let n = normalize_path(p);
    let original = p.trim().replace('\\', "/");
    let original = original.trim_end_matches('/');
    original
        .rsplit('/')
        .find(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or(n)
}

fn tokens_from(row: &Row, start: usize) -> rusqlite::Result<Tokens> {
    Ok(Tokens {
        input: row.get::<_, Option<i64>>(start)?.unwrap_or(0),
        output: row.get::<_, Option<i64>>(start + 1)?.unwrap_or(0),
        cache_read: row.get::<_, Option<i64>>(start + 2)?.unwrap_or(0),
        cache_creation: row.get::<_, Option<i64>>(start + 3)?.unwrap_or(0),
    })
}

const DISPLAY_LIMIT: usize = 20_000;

fn truncate_for_display(text: String, kind: &str) -> (String, bool) {
    if (kind == "tool_result" || kind == "system") && text.len() > DISPLAY_LIMIT {
        let mut cut = DISPLAY_LIMIT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        (text[..cut].to_string(), true)
    } else {
        (text, false)
    }
}

/// Turn free text into a safe FTS5 query: every word becomes a quoted prefix term.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

const SESSION_COLS: &str = "s.id, s.project_id, s.source, s.location, s.external_id, s.title, \
    (SELECT substr(m.text, 1, 300) FROM messages m WHERE m.session_id = s.id AND m.kind = 'prompt' AND m.is_sidechain = 0 ORDER BY m.timestamp, m.seq LIMIT 1), \
    s.cwd, s.git_branch, s.cli_version, s.started_at, s.ended_at, s.models, s.prompt_count, s.message_count, \
    s.tool_call_count, s.error_count, s.input_tokens, s.output_tokens, s.cache_read_tokens, s.cache_creation_tokens, \
    s.est_cost, s.file_path, \
    (SELECT a.overall_score FROM analyses a WHERE a.scope = 'session' AND a.target_id = s.id AND a.status = 'done' ORDER BY a.id DESC LIMIT 1)";

fn session_from_row(r: &Row) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary {
        id: r.get(0)?,
        project_id: r.get(1)?,
        source: r.get(2)?,
        location: r.get(3)?,
        external_id: r.get(4)?,
        title: r.get(5)?,
        first_prompt: r.get(6)?,
        cwd: r.get(7)?,
        git_branch: r.get(8)?,
        cli_version: r.get(9)?,
        started_at: r.get(10)?,
        ended_at: r.get(11)?,
        models: r.get(12)?,
        prompt_count: r.get(13)?,
        message_count: r.get(14)?,
        tool_call_count: r.get(15)?,
        error_count: r.get(16)?,
        tokens: tokens_from(r, 17)?,
        est_cost: r.get(21)?,
        file_path: r.get(22)?,
        latest_score: r.get(23)?,
    })
}

const ANALYSIS_COLS: &str = "id, scope, target_id, model, context_mode, status, created_at, finished_at, overall_score, \
    result_json, ref_map_json, input_text, raw_output, error, cost_usd, duration_ms, truncated";

fn analysis_from_row(r: &Row) -> rusqlite::Result<AnalysisRow> {
    let parse = |s: Option<String>| s.and_then(|s| serde_json::from_str::<Value>(&s).ok());
    Ok(AnalysisRow {
        id: r.get(0)?,
        scope: r.get(1)?,
        target_id: r.get(2)?,
        target_label: None,
        model: r.get(3)?,
        context_mode: r.get(4)?,
        status: r.get(5)?,
        created_at: r.get(6)?,
        finished_at: r.get(7)?,
        overall_score: r.get(8)?,
        result: parse(r.get(9)?),
        ref_map: parse(r.get(10)?),
        input_text: r.get(11)?,
        raw_output: r.get(12)?,
        error: r.get(13)?,
        cost_usd: r.get(14)?,
        duration_ms: r.get(15)?,
        truncated: r.get::<_, i64>(16)? != 0,
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < SCHEMA_VERSION {
            conn.execute_batch(SCHEMA)?;
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        }
        let db = Db { conn };
        let count: i64 = db.conn.query_row("SELECT COUNT(*) FROM criteria", [], |r| r.get(0))?;
        if count == 0 {
            db.save_criteria(&default_criteria())?;
        }
        Ok(db)
    }

    // ---------------------------------------------------------------- settings

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------------- ingest

    pub fn source_file_unchanged(&self, path: &str, size: i64, mtime: i64) -> Result<bool> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row("SELECT size, mtime FROM source_files WHERE path = ?1", [path], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        Ok(row == Some((size, mtime)))
    }

    pub fn mark_source_file(&self, path: &str, size: i64, mtime: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO source_files(path, size, mtime, ingested_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, ingested_at = excluded.ingested_at",
            params![path, size, mtime, now()],
        )?;
        Ok(())
    }

    fn ensure_project(&self, location: &str, cwd: Option<&str>) -> Result<i64> {
        let cwd = cwd.unwrap_or("(unknown folder)");
        let key = project_key(location, cwd);
        let existing: Option<(i64, Option<i64>)> = self
            .conn
            .query_row("SELECT id, merged_into FROM projects WHERE key = ?1", [&key], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if let Some((id, merged)) = existing {
            return Ok(merged.unwrap_or(id));
        }
        self.conn.execute(
            "INSERT INTO projects(key, name, path, location, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![key, project_name_from_path(cwd), cwd, location, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Run `f` inside a transaction.
    pub fn in_tx<T>(&self, f: impl FnOnce(&Db) -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match f(self) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Insert or update a parsed session and its messages. Returns the session id.
    pub fn upsert_session(&self, s: &ParsedSession, location: &str, file_path: &str) -> Result<i64> {
        self.in_tx(|db| db.upsert_session_inner(s, location, file_path))
    }

    fn upsert_session_inner(&self, s: &ParsedSession, location: &str, file_path: &str) -> Result<i64> {
        let source = s.source.as_str();
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM sessions WHERE source = ?1 AND external_id = ?2",
                params![source, s.external_id],
                |r| r.get(0),
            )
            .optional()?;
        let session_id = match existing {
            Some(id) => {
                self.conn.execute(
                    "UPDATE sessions SET cwd = COALESCE(cwd, ?2), git_branch = COALESCE(?3, git_branch),
                     cli_version = COALESCE(?4, cli_version), title = COALESCE(?5, title),
                     file_path = COALESCE(file_path, ?6) WHERE id = ?1",
                    params![id, s.cwd, s.git_branch, s.cli_version, s.title, file_path],
                )?;
                id
            }
            None => {
                let project_id = self.ensure_project(location, s.cwd.as_deref())?;
                self.conn.execute(
                    "INSERT INTO sessions(source, external_id, project_id, location, file_path, cwd, git_branch, cli_version, title)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![source, s.external_id, project_id, location, file_path, s.cwd, s.git_branch, s.cli_version, s.title],
                )?;
                self.conn.last_insert_rowid()
            }
        };

        {
            let mut stmt = self.conn.prepare_cached(
                "INSERT INTO messages(session_id, uuid, seq, kind, timestamp, model, text, tool_name, tool_input,
                    is_error, is_sidechain, api_msg_id, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                 ON CONFLICT(session_id, uuid) DO UPDATE SET
                    kind = excluded.kind, text = excluded.text, model = excluded.model,
                    tool_name = excluded.tool_name, is_error = excluded.is_error,
                    input_tokens = excluded.input_tokens, output_tokens = excluded.output_tokens,
                    cache_read_tokens = excluded.cache_read_tokens, cache_creation_tokens = excluded.cache_creation_tokens
                 WHERE messages.text IS NOT excluded.text OR messages.output_tokens IS NOT excluded.output_tokens
                    OR messages.kind IS NOT excluded.kind",
            )?;
            for (seq, m) in s.messages.iter().enumerate() {
                let u = m.usage.as_ref();
                stmt.execute(params![
                    session_id,
                    m.uuid,
                    seq as i64,
                    m.kind.as_str(),
                    m.timestamp,
                    m.model,
                    m.text,
                    m.tool_name,
                    m.tool_input,
                    m.is_error as i64,
                    m.is_sidechain as i64,
                    m.api_msg_id,
                    u.map(|u| u.input_tokens),
                    u.map(|u| u.output_tokens),
                    u.map(|u| u.cache_read_tokens),
                    u.map(|u| u.cache_creation_tokens),
                ])?;
            }
        }
        self.recompute_session(session_id, s.usage_total.as_ref())?;
        Ok(session_id)
    }

    /// Refresh the denormalised counters on a session row from its messages.
    pub fn recompute_session(&self, session_id: i64, usage_total: Option<&Usage>) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET
                prompt_count    = (SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND kind = 'prompt' AND is_sidechain = 0),
                message_count   = (SELECT COUNT(*) FROM messages WHERE session_id = ?1),
                tool_call_count = (SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND kind = 'tool_call'),
                error_count     = (SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND kind = 'tool_result' AND is_error = 1),
                started_at      = (SELECT MIN(timestamp) FROM messages WHERE session_id = ?1),
                ended_at        = (SELECT MAX(timestamp) FROM messages WHERE session_id = ?1),
                models          = (SELECT group_concat(model, ', ') FROM (SELECT DISTINCT model FROM messages WHERE session_id = ?1 AND model IS NOT NULL))
             WHERE id = ?1",
            [session_id],
        )?;

        let (usage, cost) = match usage_total {
            Some(u) => {
                let model: Option<String> = self.conn.query_row(
                    "SELECT model FROM messages WHERE session_id = ?1 AND model IS NOT NULL ORDER BY seq DESC LIMIT 1",
                    [session_id],
                    |r| r.get(0),
                ).optional()?;
                let cost = model.and_then(|m| pricing::estimate(&m, u));
                (u.clone(), cost)
            }
            None => {
                let mut stmt = self.conn.prepare_cached(
                    "SELECT MAX(model), MAX(input_tokens), MAX(output_tokens), MAX(cache_read_tokens), MAX(cache_creation_tokens)
                     FROM messages WHERE session_id = ?1 AND api_msg_id IS NOT NULL GROUP BY api_msg_id",
                )?;
                let rows = stmt.query_map([session_id], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        Usage {
                            input_tokens: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            output_tokens: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            cache_read_tokens: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                            cache_creation_tokens: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        },
                    ))
                })?;
                let mut total = Usage::default();
                let mut cost: Option<f64> = None;
                for row in rows {
                    let (model, u) = row?;
                    total.input_tokens += u.input_tokens;
                    total.output_tokens += u.output_tokens;
                    total.cache_read_tokens += u.cache_read_tokens;
                    total.cache_creation_tokens += u.cache_creation_tokens;
                    if let Some(c) = model.and_then(|m| pricing::estimate(&m, &u)) {
                        cost = Some(cost.unwrap_or(0.0) + c);
                    }
                }
                (total, cost)
            }
        };
        self.conn.execute(
            "UPDATE sessions SET input_tokens = ?2, output_tokens = ?3, cache_read_tokens = ?4,
                cache_creation_tokens = ?5, est_cost = ?6 WHERE id = ?1",
            params![
                session_id,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_creation_tokens,
                cost
            ],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------------- projects

    pub fn list_projects(&self) -> Result<Vec<ProjectSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.name, p.path, p.location,
                COUNT(s.id), COALESCE(SUM(s.prompt_count), 0), MIN(s.started_at), MAX(s.ended_at),
                SUM(s.input_tokens), SUM(s.output_tokens), SUM(s.cache_read_tokens), SUM(s.cache_creation_tokens),
                SUM(s.est_cost), group_concat(DISTINCT s.source),
                (SELECT a.overall_score FROM analyses a WHERE a.scope = 'project' AND a.target_id = p.id AND a.status = 'done' ORDER BY a.id DESC LIMIT 1)
             FROM projects p LEFT JOIN sessions s ON s.project_id = p.id
             WHERE p.merged_into IS NULL
             GROUP BY p.id
             HAVING COUNT(s.id) > 0
             ORDER BY MAX(s.ended_at) DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ProjectSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                path: r.get(2)?,
                location: r.get(3)?,
                session_count: r.get(4)?,
                prompt_count: r.get(5)?,
                first_activity: r.get(6)?,
                last_activity: r.get(7)?,
                tokens: tokens_from(r, 8)?,
                est_cost: r.get(12)?,
                sources: r
                    .get::<_, Option<String>>(13)?
                    .map(|s| s.split(',').map(String::from).collect())
                    .unwrap_or_default(),
                latest_score: r.get(14)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_project(&self, id: i64) -> Result<Option<ProjectSummary>> {
        Ok(self.list_projects()?.into_iter().find(|p| p.id == id))
    }

    pub fn project_name(&self, id: i64) -> Result<String> {
        Ok(self.conn.query_row("SELECT name FROM projects WHERE id = ?1", [id], |r| r.get(0))?)
    }

    pub fn rename_project(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute("UPDATE projects SET name = ?2 WHERE id = ?1", params![id, name.trim()])?;
        Ok(())
    }

    /// Move every session of `source` into `target`; future sessions from that folder follow.
    pub fn merge_projects(&self, source: i64, target: i64) -> Result<()> {
        anyhow::ensure!(source != target, "cannot merge a project into itself");
        self.in_tx(|db| {
        let tx = &db.conn;
        tx.execute("UPDATE sessions SET project_id = ?2 WHERE project_id = ?1", params![source, target])?;
        tx.execute(
            "UPDATE projects SET merged_into = ?2 WHERE id = ?1 OR merged_into = ?1",
            params![source, target],
        )?;
        tx.execute(
            "UPDATE analyses SET target_id = ?2 WHERE scope = 'project' AND target_id = ?1",
            params![source, target],
        )?;
        Ok(())
        })
    }

    // ---------------------------------------------------------------- sessions

    pub fn list_sessions(&self, project_id: i64) -> Result<Vec<SessionSummary>> {
        let sql = format!(
            "SELECT {SESSION_COLS} FROM sessions s WHERE s.project_id = ?1 ORDER BY s.started_at DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([project_id], session_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_session_summary(&self, id: i64) -> Result<SessionSummary> {
        let sql = format!("SELECT {SESSION_COLS} FROM sessions s WHERE s.id = ?1");
        Ok(self.conn.query_row(&sql, [id], session_from_row)?)
    }

    pub fn session_messages(&self, session_id: i64, truncate: bool) -> Result<Vec<MessageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, uuid, kind, timestamp, model, text, tool_name, tool_input, is_error, is_sidechain
             FROM messages WHERE session_id = ?1 ORDER BY COALESCE(timestamp, ''), seq",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            let kind: String = r.get(2)?;
            let text: String = r.get(5)?;
            let (text, truncated) = if truncate { truncate_for_display(text, &kind) } else { (text, false) };
            Ok(MessageRow {
                id: r.get(0)?,
                uuid: r.get(1)?,
                kind,
                timestamp: r.get(3)?,
                model: r.get(4)?,
                text,
                tool_name: r.get(6)?,
                tool_input: r.get(7)?,
                is_error: r.get::<_, i64>(8)? != 0,
                is_sidechain: r.get::<_, i64>(9)? != 0,
                truncated,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_session(&self, id: i64) -> Result<SessionDetail> {
        let session = self.get_session_summary(id)?;
        let project_name = self.project_name(session.project_id)?;
        let messages = self.session_messages(id, true)?;
        Ok(SessionDetail { session, project_name, messages })
    }

    pub fn get_message(&self, id: i64) -> Result<MessageRow> {
        Ok(self.conn.query_row(
            "SELECT id, uuid, kind, timestamp, model, text, tool_name, tool_input, is_error, is_sidechain FROM messages WHERE id = ?1",
            [id],
            |r| {
                Ok(MessageRow {
                    id: r.get(0)?,
                    uuid: r.get(1)?,
                    kind: r.get(2)?,
                    timestamp: r.get(3)?,
                    model: r.get(4)?,
                    text: r.get(5)?,
                    tool_name: r.get(6)?,
                    tool_input: r.get(7)?,
                    is_error: r.get::<_, i64>(8)? != 0,
                    is_sidechain: r.get::<_, i64>(9)? != 0,
                    truncated: false,
                })
            },
        )?)
    }

    /// (message id, session id) of a message - used to navigate from analysis references.
    pub fn message_session(&self, message_id: i64) -> Result<i64> {
        Ok(self.conn.query_row("SELECT session_id FROM messages WHERE id = ?1", [message_id], |r| r.get(0))?)
    }

    // ---------------------------------------------------------------- search

    pub fn search(&self, query: &str, kinds: &[String], project_id: Option<i64>, limit: i64) -> Result<Vec<SearchHit>> {
        let Some(q) = fts_query(query) else { return Ok(vec![]) };
        let kinds_filter = if kinds.is_empty() {
            String::new()
        } else {
            let list = kinds
                .iter()
                .filter(|k| k.chars().all(|c| c.is_ascii_alphabetic() || c == '_'))
                .map(|k| format!("'{k}'"))
                .collect::<Vec<_>>()
                .join(",");
            format!("AND m.kind IN ({list})")
        };
        let sql = format!(
            "SELECT m.id, m.session_id, s.project_id, p.name, COALESCE(s.title, ''), m.kind,
                snippet(messages_fts, 0, char(1), char(2), '…', 24), m.timestamp
             FROM messages_fts
             JOIN messages m ON m.id = messages_fts.rowid
             JOIN sessions s ON s.id = m.session_id
             JOIN projects p ON p.id = s.project_id
             WHERE messages_fts MATCH ?1 {kinds_filter} AND (?2 IS NULL OR s.project_id = ?2)
             ORDER BY rank LIMIT ?3"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![q, project_id, limit], |r| {
            let title: String = r.get(4)?;
            Ok(SearchHit {
                message_id: r.get(0)?,
                session_id: r.get(1)?,
                project_id: r.get(2)?,
                project_name: r.get(3)?,
                session_title: (!title.is_empty()).then_some(title),
                kind: r.get(5)?,
                snippet: r.get(6)?,
                timestamp: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---------------------------------------------------------------- stats

    pub fn stats(&self, days: i64) -> Result<Stats> {
        let mut st = Stats::default();
        self.conn.query_row(
            "SELECT COUNT(DISTINCT project_id), COUNT(*), COALESCE(SUM(prompt_count),0), COALESCE(SUM(tool_call_count),0),
                COALESCE(SUM(error_count),0), SUM(input_tokens), SUM(output_tokens), SUM(cache_read_tokens),
                SUM(cache_creation_tokens), COALESCE(SUM(est_cost), 0)
             FROM sessions",
            [],
            |r| {
                st.projects = r.get(0)?;
                st.sessions = r.get(1)?;
                st.prompts = r.get(2)?;
                st.tool_calls = r.get(3)?;
                st.tool_errors = r.get(4)?;
                st.tokens = tokens_from(r, 5)?;
                st.est_cost = r.get(9)?;
                Ok(())
            },
        )?;
        st.analyses = self.conn.query_row("SELECT COUNT(*) FROM analyses WHERE status = 'done'", [], |r| r.get(0))?;

        let mut stmt = self.conn.prepare(
            "SELECT substr(started_at, 1, 10) AS day, COUNT(*), SUM(prompt_count), COALESCE(SUM(est_cost), 0)
             FROM sessions WHERE started_at >= date('now', ?1)
             GROUP BY day ORDER BY day",
        )?;
        st.by_day = stmt
            .query_map([format!("-{days} days")], |r| {
                Ok(DayStat { day: r.get(0)?, sessions: r.get(1)?, prompts: r.get(2)?, est_cost: r.get(3)? })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT model, COUNT(*), SUM(i), SUM(o), SUM(cr), SUM(cc) FROM (
                SELECT MAX(model) AS model, MAX(input_tokens) AS i, MAX(output_tokens) AS o,
                       MAX(cache_read_tokens) AS cr, MAX(cache_creation_tokens) AS cc
                FROM messages WHERE api_msg_id IS NOT NULL AND model IS NOT NULL GROUP BY session_id, api_msg_id
             ) GROUP BY model ORDER BY SUM(o) DESC",
        )?;
        st.by_model = stmt
            .query_map([], |r| {
                let model: String = r.get(0)?;
                let u = Usage {
                    input_tokens: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    output_tokens: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    cache_read_tokens: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    cache_creation_tokens: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                };
                Ok(ModelStat {
                    est_cost: pricing::estimate(&model, &u),
                    model,
                    responses: r.get(1)?,
                    output_tokens: u.output_tokens,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        // Codex sessions report usage per session, not per message.
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(models, 'codex'), COUNT(*), SUM(output_tokens) FROM sessions WHERE source = 'codex' GROUP BY models",
        )?;
        let codex: Vec<ModelStat> = stmt
            .query_map([], |r| {
                Ok(ModelStat { model: r.get(0)?, responses: r.get(1)?, output_tokens: r.get::<_, Option<i64>>(2)?.unwrap_or(0), est_cost: None })
            })?
            .collect::<rusqlite::Result<_>>()?;
        st.by_model.extend(codex);

        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.name, COUNT(s.id), SUM(s.prompt_count), SUM(s.output_tokens), COALESCE(SUM(s.est_cost), 0)
             FROM projects p JOIN sessions s ON s.project_id = p.id
             GROUP BY p.id ORDER BY SUM(s.prompt_count) DESC",
        )?;
        st.by_project = stmt
            .query_map([], |r| {
                Ok(ProjectStat {
                    project_id: r.get(0)?,
                    name: r.get(1)?,
                    sessions: r.get(2)?,
                    prompts: r.get(3)?,
                    output_tokens: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    est_cost: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT source, location, COUNT(*), SUM(prompt_count) FROM sessions GROUP BY source, location ORDER BY COUNT(*) DESC",
        )?;
        st.by_source = stmt
            .query_map([], |r| Ok(SourceStat { source: r.get(0)?, location: r.get(1)?, sessions: r.get(2)?, prompts: r.get(3)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(st)
    }

    // ---------------------------------------------------------------- criteria

    pub fn list_criteria(&self) -> Result<Vec<Criterion>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, key, name, description, weight, enabled FROM criteria ORDER BY sort, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Criterion {
                id: r.get(0)?,
                key: r.get(1)?,
                name: r.get(2)?,
                description: r.get(3)?,
                weight: r.get(4)?,
                enabled: r.get::<_, i64>(5)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replace the whole rubric with `list` (in order).
    pub fn save_criteria(&self, list: &[Criterion]) -> Result<()> {
        self.conn.execute("DELETE FROM criteria", [])?;
        for (i, c) in list.iter().enumerate() {
            let key = if c.key.trim().is_empty() { slug(&c.name) } else { c.key.trim().to_string() };
            self.conn.execute(
                "INSERT INTO criteria(key, name, description, weight, enabled, sort) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(key) DO NOTHING",
                params![key, c.name.trim(), c.description.trim(), c.weight, c.enabled as i64, i as i64],
            )?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- analyses

    pub fn create_analysis(&self, a: &NewAnalysis) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO analyses(scope, target_id, model, context_mode, status, created_at, input_text, ref_map_json, truncated)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7, ?8)",
            params![
                a.scope,
                a.target_id,
                a.model,
                a.context_mode,
                now(),
                a.input_text,
                a.ref_map.map(|v| v.to_string()),
                a.truncated as i64
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_analysis(
        &self,
        id: i64,
        score: Option<f64>,
        result: &Value,
        raw: &str,
        cost: Option<f64>,
        duration_ms: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE analyses SET status = 'done', finished_at = ?2, overall_score = ?3, result_json = ?4,
                raw_output = ?5, cost_usd = ?6, duration_ms = ?7 WHERE id = ?1",
            params![id, now(), score, result.to_string(), raw, cost, duration_ms],
        )?;
        Ok(())
    }

    pub fn fail_analysis(&self, id: i64, error: &str, raw: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE analyses SET status = 'error', finished_at = ?2, error = ?3, raw_output = ?4 WHERE id = ?1",
            params![id, now(), error, raw],
        )?;
        Ok(())
    }

    /// Analyses left 'running' by a previous app run can never finish.
    pub fn fail_stale_analyses(&self) -> Result<()> {
        self.conn.execute(
            "UPDATE analyses SET status = 'error', error = 'Interrupted (app was closed while analysing)', finished_at = ?1
             WHERE status = 'running'",
            [now()],
        )?;
        Ok(())
    }

    pub fn get_analysis(&self, id: i64) -> Result<AnalysisRow> {
        let sql = format!("SELECT {ANALYSIS_COLS} FROM analyses WHERE id = ?1");
        let mut a = self.conn.query_row(&sql, [id], analysis_from_row)?;
        a.target_label = self.target_label(&a.scope, a.target_id)?;
        Ok(a)
    }

    pub fn list_analyses(&self, scope: Option<&str>, target_id: Option<i64>) -> Result<Vec<AnalysisRow>> {
        let sql = format!(
            "SELECT {ANALYSIS_COLS} FROM analyses
             WHERE (?1 IS NULL OR scope = ?1) AND (?2 IS NULL OR target_id = ?2)
             ORDER BY id DESC LIMIT 500"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows: Vec<AnalysisRow> = stmt
            .query_map(params![scope, target_id], analysis_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for mut a in rows {
            a.target_label = self.target_label(&a.scope, a.target_id)?;
            // The list view does not need the bulky raw output.
            a.raw_output = None;
            out.push(a);
        }
        Ok(out)
    }

    /// Prompt-scope analyses whose target prompt belongs to `session_id`.
    pub fn session_prompt_analyses(&self, session_id: i64) -> Result<Vec<AnalysisRow>> {
        let sql = format!(
            "SELECT {ANALYSIS_COLS} FROM analyses WHERE scope = 'prompt'
             AND target_id IN (SELECT id FROM messages WHERE session_id = ?1) ORDER BY id DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([session_id], analysis_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_analysis(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM analyses WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Latest finished score per prompt in a session.
    pub fn prompt_scores(&self, session_id: i64) -> Result<Vec<PromptScore>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.target_id, a.overall_score, a.id FROM analyses a
             JOIN messages m ON m.id = a.target_id
             WHERE a.scope = 'prompt' AND a.status = 'done' AND m.session_id = ?1 AND a.overall_score IS NOT NULL
             ORDER BY a.id",
        )?;
        let mut latest: HashMap<i64, PromptScore> = HashMap::new();
        for row in stmt.query_map([session_id], |r| {
            Ok(PromptScore { message_id: r.get(0)?, score: r.get(1)?, analysis_id: r.get(2)? })
        })? {
            let row = row?;
            latest.insert(row.message_id, row);
        }
        let mut v: Vec<_> = latest.into_values().collect();
        v.sort_by_key(|p| p.message_id);
        Ok(v)
    }

    fn target_label(&self, scope: &str, target: Option<i64>) -> Result<Option<String>> {
        let Some(t) = target else {
            return Ok(match scope {
                "global" => Some("All projects".into()),
                "draft" => Some("Draft prompt".into()),
                _ => None,
            });
        };
        let label: Option<String> = match scope {
            "project" => self.conn.query_row("SELECT name FROM projects WHERE id = ?1", [t], |r| r.get(0)).optional()?,
            "session" => self
                .conn
                .query_row(
                    "SELECT p.name || ' · ' || COALESCE(s.title, substr(s.started_at, 1, 16), s.external_id)
                     FROM sessions s JOIN projects p ON p.id = s.project_id WHERE s.id = ?1",
                    [t],
                    |r| r.get(0),
                )
                .optional()?,
            "prompt" => self
                .conn
                .query_row("SELECT substr(text, 1, 80) FROM messages WHERE id = ?1", [t], |r| r.get(0))
                .optional()?,
            _ => None,
        };
        Ok(label)
    }
}

fn slug(s: &str) -> String {
    let mut out: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    out = out.trim_matches('_').to_string();
    if out.is_empty() {
        out = format!("c{}", chrono::Utc::now().timestamp_millis());
    }
    out
}
