//! Usage statistics over a time range: activity and token use per hour/day, per
//! project and per model, plus what the app's own analyses cost.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{bail, Result};
use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime, TimeZone, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::db::{Db, Tokens};
use crate::model::Usage;
use crate::pricing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BucketSize {
    Hour,
    Day,
}

/// Half-open range `[from, to)` in UTC, bucketed in the viewer's local time.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRange {
    pub from: String,
    pub to: String,
    /// Minutes east of UTC (e.g. 210 for Tehran).
    pub tz_offset_minutes: i32,
    pub bucket: BucketSize,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub sessions: i64,
    pub active_projects: i64,
    pub prompts: i64,
    pub tool_calls: i64,
    pub tool_errors: i64,
    pub tokens: Tokens,
    pub est_cost: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    /// Local start of the bucket: `YYYY-MM-DD` or `YYYY-MM-DDTHH`.
    pub key: String,
    pub sessions: i64,
    pub prompts: i64,
    pub tokens: Tokens,
    pub est_cost: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUsage {
    pub project_id: i64,
    pub name: String,
    pub sessions: i64,
    pub prompts: i64,
    pub tokens: Tokens,
    pub est_cost: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub responses: i64,
    pub tokens: Tokens,
    pub est_cost: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceUsage {
    pub source: String,
    pub location: String,
    pub sessions: i64,
    pub prompts: i64,
}

/// Tokens spent by this app's own analyses (through the Claude CLI).
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisUsage {
    pub runs: i64,
    pub failed: i64,
    pub tokens: Tokens,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    pub totals: UsageTotals,
    pub buckets: Vec<UsageBucket>,
    pub by_project: Vec<ProjectUsage>,
    pub by_model: Vec<ModelUsage>,
    pub by_source: Vec<SourceUsage>,
    pub analyses: AnalysisUsage,
}

fn add(t: &mut Tokens, u: &Usage) {
    t.input += u.input_tokens;
    t.output += u.output_tokens;
    t.cache_read += u.cache_read_tokens;
    t.cache_creation += u.cache_creation_tokens;
}

pub fn total(t: &Tokens) -> i64 {
    t.input + t.output + t.cache_read + t.cache_creation
}

struct Bucketer {
    offset: FixedOffset,
    size: BucketSize,
}

impl Bucketer {
    fn key_of(&self, ts: &str) -> Option<String> {
        let dt = DateTime::parse_from_rfc3339(ts).ok()?.with_timezone(&self.offset);
        Some(match self.size {
            BucketSize::Day => dt.format("%Y-%m-%d").to_string(),
            BucketSize::Hour => dt.format("%Y-%m-%dT%H").to_string(),
        })
    }

    /// Every bucket key in `[from, to)`, so empty hours/days still show.
    fn all_keys(&self, from: &DateTime<Utc>, to: &DateTime<Utc>) -> Vec<String> {
        let step = match self.size {
            BucketSize::Day => Duration::days(1),
            BucketSize::Hour => Duration::hours(1),
        };
        let mut keys = Vec::new();
        let mut t = from.with_timezone(&self.offset);
        let end = to.with_timezone(&self.offset);
        // Cap to keep a bad range from allocating forever.
        while t < end && keys.len() < 5000 {
            if let Some(k) = self.key_of(&t.to_rfc3339()) {
                if keys.last() != Some(&k) {
                    keys.push(k);
                }
            }
            t += step;
        }
        keys
    }
}

fn parse_utc(s: &str) -> Result<DateTime<Utc>> {
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Ok(d.with_timezone(&Utc));
    }
    let n = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")?;
    Ok(Utc.from_utc_datetime(&n))
}

impl Db {
    pub fn usage(&self, range: &UsageRange) -> Result<UsageReport> {
        let from = parse_utc(&range.from)?;
        let to = parse_utc(&range.to)?;
        if to <= from {
            bail!("The end of the range must be after its start");
        }
        let Some(offset) = FixedOffset::east_opt(range.tz_offset_minutes * 60) else {
            bail!("invalid time zone offset");
        };
        let b = Bucketer { offset, size: range.bucket };
        // Same textual format as stored timestamps so string comparison works in SQL.
        let f = from.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let t = to.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        let mut rep = UsageReport::default();
        let mut buckets: BTreeMap<String, UsageBucket> = b
            .all_keys(&from, &to)
            .into_iter()
            .map(|k| (k.clone(), UsageBucket { key: k, ..Default::default() }))
            .collect();
        let mut projects: HashMap<i64, ProjectUsage> = HashMap::new();
        let mut models: HashMap<String, ModelUsage> = HashMap::new();
        let mut sources: HashMap<(String, String), SourceUsage> = HashMap::new();
        let mut active_projects: HashSet<i64> = HashSet::new();

        let mut record_tokens = |project: i64, model: Option<String>, ts: &str, u: &Usage, rep: &mut UsageReport,
                                 buckets: &mut BTreeMap<String, UsageBucket>, projects: &mut HashMap<i64, ProjectUsage>,
                                 count_response: bool| {
            let cost = model.as_deref().and_then(|m| pricing::estimate(m, u));
            add(&mut rep.totals.tokens, u);
            rep.totals.est_cost += cost.unwrap_or(0.0);
            if let Some(bk) = b.key_of(ts).and_then(|k| buckets.get_mut(&k)) {
                add(&mut bk.tokens, u);
                bk.est_cost += cost.unwrap_or(0.0);
            }
            let p = projects.entry(project).or_insert_with(|| ProjectUsage { project_id: project, ..Default::default() });
            add(&mut p.tokens, u);
            p.est_cost += cost.unwrap_or(0.0);
            let name = model.unwrap_or_else(|| "unknown".into());
            let m = models.entry(name.clone()).or_insert_with(|| ModelUsage { model: name, ..Default::default() });
            if count_response {
                m.responses += 1;
            }
            add(&mut m.tokens, u);
            if let Some(c) = cost {
                m.est_cost = Some(m.est_cost.unwrap_or(0.0) + c);
            }
        };

        // Claude Code: usage is per API message (streamed over several rows -> MAX per id).
        {
            let mut stmt = self.conn.prepare(
                "SELECT s.project_id, MAX(m.model), MIN(m.timestamp), MAX(m.input_tokens), MAX(m.output_tokens),
                        MAX(m.cache_read_tokens), MAX(m.cache_creation_tokens)
                 FROM messages m JOIN sessions s ON s.id = m.session_id
                 WHERE m.api_msg_id IS NOT NULL AND m.timestamp >= ?1 AND m.timestamp < ?2
                 GROUP BY m.session_id, m.api_msg_id",
            )?;
            let rows = stmt.query_map(params![f, t], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    Usage {
                        input_tokens: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        output_tokens: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        cache_read_tokens: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                        cache_creation_tokens: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    },
                ))
            })?;
            for row in rows {
                let (project, model, ts, u) = row?;
                record_tokens(project, model, &ts, &u, &mut rep, &mut buckets, &mut projects, true);
            }
        }
        // Codex: usage is only known per session; attribute it to the session start.
        {
            let mut stmt = self.conn.prepare(
                "SELECT project_id, models, started_at, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens
                 FROM sessions WHERE source = 'codex' AND started_at >= ?1 AND started_at < ?2",
            )?;
            let rows = stmt.query_map(params![f, t], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    Usage {
                        input_tokens: r.get(3)?,
                        output_tokens: r.get(4)?,
                        cache_read_tokens: r.get(5)?,
                        cache_creation_tokens: r.get(6)?,
                    },
                ))
            })?;
            for row in rows {
                let (project, model, ts, u) = row?;
                let model = model.map(|m| m.split(", ").last().unwrap_or("").to_string()).filter(|m| !m.is_empty());
                record_tokens(project, model, &ts, &u, &mut rep, &mut buckets, &mut projects, false);
            }
        }
        // Prompts.
        {
            let mut stmt = self.conn.prepare(
                "SELECT s.project_id, s.source, s.location, m.timestamp FROM messages m JOIN sessions s ON s.id = m.session_id
                 WHERE m.kind = 'prompt' AND m.is_sidechain = 0 AND m.timestamp >= ?1 AND m.timestamp < ?2",
            )?;
            let rows = stmt.query_map(params![f, t], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
            })?;
            for row in rows {
                let (project, source, location, ts) = row?;
                rep.totals.prompts += 1;
                if let Some(bk) = b.key_of(&ts).and_then(|k| buckets.get_mut(&k)) {
                    bk.prompts += 1;
                }
                projects.entry(project).or_insert_with(|| ProjectUsage { project_id: project, ..Default::default() }).prompts += 1;
                sources
                    .entry((source.clone(), location.clone()))
                    .or_insert_with(|| SourceUsage { source, location, ..Default::default() })
                    .prompts += 1;
            }
        }
        // Sessions with any activity in the range, counted in the bucket of their first message there.
        {
            let mut stmt = self.conn.prepare(
                "SELECT s.project_id, s.source, s.location, MIN(m.timestamp) FROM messages m JOIN sessions s ON s.id = m.session_id
                 WHERE m.timestamp >= ?1 AND m.timestamp < ?2 GROUP BY m.session_id",
            )?;
            let rows = stmt.query_map(params![f, t], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
            })?;
            for row in rows {
                let (project, source, location, ts) = row?;
                rep.totals.sessions += 1;
                active_projects.insert(project);
                if let Some(bk) = b.key_of(&ts).and_then(|k| buckets.get_mut(&k)) {
                    bk.sessions += 1;
                }
                projects.entry(project).or_insert_with(|| ProjectUsage { project_id: project, ..Default::default() }).sessions += 1;
                sources
                    .entry((source.clone(), location.clone()))
                    .or_insert_with(|| SourceUsage { source, location, ..Default::default() })
                    .sessions += 1;
            }
        }
        self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(m.is_error), 0) FROM messages m
             WHERE m.kind IN ('tool_call', 'tool_result') AND m.timestamp >= ?1 AND m.timestamp < ?2
               AND (m.kind = 'tool_call' OR m.is_error = 1)",
            params![f, t],
            |r| {
                let n: i64 = r.get(0)?;
                let errors: i64 = r.get(1)?;
                rep.totals.tool_calls = n - errors;
                rep.totals.tool_errors = errors;
                Ok(())
            },
        )?;
        rep.totals.active_projects = active_projects.len() as i64;

        // App's own analyses.
        self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(status = 'error'), 0), SUM(input_tokens), SUM(output_tokens),
                    SUM(cache_read_tokens), SUM(cache_creation_tokens), COALESCE(SUM(cost_usd), 0)
             FROM analyses WHERE created_at >= ?1 AND created_at < ?2 AND status != 'running'",
            params![from.to_rfc3339_opts(chrono::SecondsFormat::Secs, true), to.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)],
            |r| {
                rep.analyses = AnalysisUsage {
                    runs: r.get(0)?,
                    failed: r.get(1)?,
                    tokens: Tokens {
                        input: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        output: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        cache_read: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        cache_creation: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    },
                    cost_usd: r.get(6)?,
                };
                Ok(())
            },
        )?;

        // Names, ordering.
        let mut names: HashMap<i64, String> = HashMap::new();
        {
            let mut stmt = self.conn.prepare("SELECT id, name FROM projects")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                let (id, name) = row?;
                names.insert(id, name);
            }
        }
        rep.by_project = projects
            .into_values()
            .map(|mut p| {
                p.name = names.get(&p.project_id).cloned().unwrap_or_else(|| format!("#{}", p.project_id));
                p
            })
            .collect();
        rep.by_project.sort_by(|a, b| total(&b.tokens).cmp(&total(&a.tokens)).then(b.prompts.cmp(&a.prompts)));
        rep.by_model = models.into_values().collect();
        rep.by_model.sort_by(|a, b| b.tokens.output.cmp(&a.tokens.output));
        rep.by_source = sources.into_values().collect();
        rep.by_source.sort_by(|a, b| b.sessions.cmp(&a.sessions));
        rep.buckets = buckets.into_values().collect();
        Ok(rep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MessageKind, ParsedMessage, ParsedSession, SourceKind};

    #[test]
    fn buckets_by_local_day_and_counts_streamed_usage_once() {
        let db = Db::open_in_memory().unwrap();
        let mut s = ParsedSession::new(SourceKind::Claude, "s".into());
        s.cwd = Some("/p".into());
        let mut prompt = ParsedMessage::new("1".into(), MessageKind::Prompt, "hi".into());
        // 22:00 UTC = 01:30 next day in Tehran (+03:30).
        prompt.timestamp = Some("2026-10-15T22:00:00.000Z".into());
        s.messages.push(prompt);
        for (i, out) in [(2, 10), (3, 40)] {
            let mut m = ParsedMessage::new(i.to_string(), MessageKind::Response, "ok".into());
            m.timestamp = Some("2026-10-15T22:00:05.000Z".into());
            m.api_msg_id = Some("msg".into());
            m.model = Some("claude-sonnet-5-5".into());
            m.usage = Some(Usage { input_tokens: 100, output_tokens: out, cache_read_tokens: 1000, cache_creation_tokens: 0 });
            s.messages.push(m);
        }
        db.upsert_session(&s, "linux", "/x").unwrap();

        let range = UsageRange {
            from: "2026-10-14T20:30:00.000Z".into(), // 2026-10-15 00:00 Tehran
            to: "2026-10-16T20:30:00.000Z".into(),   // 2026-10-17 00:00 Tehran
            tz_offset_minutes: 210,
            bucket: BucketSize::Day,
        };
        let r = db.usage(&range).unwrap();
        assert_eq!(r.buckets.iter().map(|b| b.key.as_str()).collect::<Vec<_>>(), vec!["2026-10-15", "2026-10-16"]);
        assert_eq!(r.buckets[1].prompts, 1);
        assert_eq!(r.buckets[1].tokens.output, 40, "streamed usage counted once (max)");
        assert_eq!(r.totals.tokens.cache_read, 1000);
        assert_eq!(r.totals.sessions, 1);
        assert_eq!(r.by_project[0].name, "p");
        assert_eq!(r.by_model[0].responses, 1);

        let hourly = db.usage(&UsageRange { bucket: BucketSize::Hour, ..range }).unwrap();
        assert_eq!(hourly.buckets.len(), 48);
        assert_eq!(hourly.buckets.iter().find(|b| b.prompts == 1).unwrap().key, "2026-10-16T01");
    }
}
