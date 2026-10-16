//! Finding session log folders and importing changed files into the database.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::db::{normalize_path, Db};
use crate::model::SourceKind;
use crate::parse;

/// A folder that holds session logs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceConfig {
    pub id: String,
    pub kind: SourceKind,
    /// Where the logs come from: "windows", "wsl:Ubuntu", "linux", or a user label.
    pub location: String,
    pub path: String,
    pub enabled: bool,
    /// Found by auto-detection (as opposed to added by hand).
    pub auto: bool,
}

impl SourceConfig {
    pub fn is_wsl(&self) -> bool {
        self.location.starts_with("wsl:")
    }
    pub fn wsl_distro(&self) -> Option<&str> {
        self.location.strip_prefix("wsl:")
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IngestReport {
    pub files_scanned: usize,
    pub files_imported: usize,
    pub sessions_updated: usize,
    pub sources_skipped: Vec<String>,
    pub errors: Vec<String>,
    pub finished_at: String,
}

pub fn host_location() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        _ => "linux",
    }
}

fn make_source(kind: SourceKind, location: &str, path: PathBuf) -> SourceConfig {
    let path = path.to_string_lossy().to_string();
    SourceConfig {
        id: format!("{}:{}:{}", kind.as_str(), location, normalize_path(&path)),
        kind,
        location: location.to_string(),
        path,
        enabled: true,
        auto: true,
    }
}

/// Default log folders for a home directory (`~/.claude/projects`, `~/.codex/sessions`).
fn sources_in_home(home: &Path, location: &str, claude_dir: Option<PathBuf>, codex_dir: Option<PathBuf>) -> Vec<SourceConfig> {
    let mut out = Vec::new();
    let claude = claude_dir.unwrap_or_else(|| home.join(".claude")).join("projects");
    if claude.is_dir() {
        out.push(make_source(SourceKind::Claude, location, claude));
    }
    let codex = codex_dir.unwrap_or_else(|| home.join(".codex")).join("sessions");
    if codex.is_dir() {
        out.push(make_source(SourceKind::Codex, location, codex));
    }
    out
}

/// Detect every log folder on this machine, including WSL distributions on Windows.
pub fn detect_sources() -> Vec<SourceConfig> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let claude_env = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
        let codex_env = std::env::var_os("CODEX_HOME").map(PathBuf::from);
        out.extend(sources_in_home(&home, host_location(), claude_env, codex_env));
    }
    for distro in wsl::distros() {
        let Some(root) = wsl::root(&distro) else { continue };
        let location = format!("wsl:{distro}");
        let mut homes: Vec<PathBuf> = vec![root.join("root")];
        if let Ok(rd) = std::fs::read_dir(root.join("home")) {
            homes.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
        for h in homes {
            out.extend(sources_in_home(&h, &location, None, None));
        }
    }
    out
}

/// Merge freshly detected sources into the saved list, keeping user choices
/// (enabled flag, manual entries) intact.
pub fn merge_sources(saved: &[SourceConfig], detected: Vec<SourceConfig>) -> Vec<SourceConfig> {
    let mut out = saved.to_vec();
    for d in detected {
        if !out.iter().any(|s| s.id == d.id) {
            out.push(d);
        }
    }
    out
}

fn file_stamp(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some((meta.len() as i64, mtime))
}

pub struct IngestOptions<'a> {
    /// Sessions whose cwd starts with one of these are ignored (the app's own analysis runs).
    pub exclude_cwd_prefixes: &'a [String],
    /// Skip WSL sources whose distro is not running (avoids booting WSL from a background poll).
    pub only_running_wsl: bool,
    /// Re-import files even if size/mtime did not change.
    pub force: bool,
}

pub fn ingest_all(db: &Db, sources: &[SourceConfig], opts: &IngestOptions) -> IngestReport {
    let mut report = IngestReport::default();
    let running = if opts.only_running_wsl && sources.iter().any(|s| s.enabled && s.is_wsl()) {
        Some(wsl::running_distros())
    } else {
        None
    };
    for src in sources.iter().filter(|s| s.enabled) {
        if let (Some(running), Some(distro)) = (&running, src.wsl_distro()) {
            if !running.iter().any(|d| d.eq_ignore_ascii_case(distro)) {
                report.sources_skipped.push(format!("{} (WSL distro not running)", src.path));
                continue;
            }
        }
        if let Err(e) = ingest_source(db, src, opts, &mut report) {
            report.errors.push(format!("{}: {e:#}", src.path));
        }
    }
    report.finished_at = crate::db::now();
    report
}

fn ingest_source(db: &Db, src: &SourceConfig, opts: &IngestOptions, report: &mut IngestReport) -> Result<()> {
    let root = Path::new(&src.path);
    if !root.is_dir() {
        report.sources_skipped.push(format!("{} (folder not found)", src.path));
        return Ok(());
    }
    for entry in WalkDir::new(root).max_depth(6).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !entry.file_type().is_file() || path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        report.files_scanned += 1;
        let Some((size, mtime)) = file_stamp(path) else { continue };
        let key = path.to_string_lossy().to_string();
        if !opts.force && db.source_file_unchanged(&key, size, mtime)? {
            continue;
        }
        match ingest_file(db, src, path, opts) {
            Ok(n) => {
                report.files_imported += 1;
                report.sessions_updated += n;
                db.mark_source_file(&key, size, mtime)?;
            }
            Err(e) => report.errors.push(format!("{key}: {e:#}")),
        }
    }
    Ok(())
}

/// Import one log file. Returns the number of sessions written.
pub fn ingest_file(db: &Db, src: &SourceConfig, path: &Path, opts: &IngestOptions) -> Result<usize> {
    let bytes = std::fs::read(path)?;
    let content = String::from_utf8_lossy(&bytes);
    let sessions = match src.kind {
        SourceKind::Claude => parse::claude::parse(&content),
        SourceKind::Codex => {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("codex");
            parse::codex::parse(&content, stem).into_iter().collect()
        }
    };
    let file = path.to_string_lossy();
    let mut n = 0;
    for s in sessions {
        if s.messages.is_empty() {
            continue;
        }
        if let Some(cwd) = &s.cwd {
            let c = normalize_path(cwd);
            if opts.exclude_cwd_prefixes.iter().any(|p| c.starts_with(&normalize_path(p))) {
                continue;
            }
        }
        db.upsert_session(&s, &src.location, &file)?;
        n += 1;
    }
    Ok(n)
}

pub mod wsl {
    //! WSL helpers. All functions return empty results on non-Windows hosts.
    use std::path::PathBuf;

    #[cfg(windows)]
    fn run_wsl(args: &[&str]) -> Option<String> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("wsl.exe")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(decode_wsl_output(&out.stdout))
    }

    #[cfg(not(windows))]
    fn run_wsl(_args: &[&str]) -> Option<String> {
        None
    }

    /// `wsl.exe -l` prints UTF-16LE; be lenient in case a future version prints UTF-8.
    pub fn decode_wsl_output(bytes: &[u8]) -> String {
        let looks_utf16 = bytes.len() >= 2 && bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count() * 2 >= bytes.len() / 2;
        let s = if looks_utf16 {
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        };
        s.replace(['\u{feff}', '\0'], "")
    }

    fn parse_list(s: &str) -> Vec<String> {
        s.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.to_ascii_lowercase().starts_with("docker-desktop"))
            .collect()
    }

    pub fn distros() -> Vec<String> {
        run_wsl(&["-l", "-q"]).map(|s| parse_list(&s)).unwrap_or_default()
    }

    pub fn running_distros() -> Vec<String> {
        run_wsl(&["-l", "-q", "--running"]).map(|s| parse_list(&s)).unwrap_or_default()
    }

    /// UNC root of a distro's filesystem.
    pub fn root(distro: &str) -> Option<PathBuf> {
        if !cfg!(windows) {
            return None;
        }
        [format!(r"\\wsl.localhost\{distro}"), format!(r"\\wsl$\{distro}")]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_dir())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn decodes_utf16_distro_list() {
            let text = "\u{feff}Ubuntu-24.04\r\ndocker-desktop\r\nDebian\r\n";
            let bytes: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
            assert_eq!(parse_list(&decode_wsl_output(&bytes)), vec!["Ubuntu-24.04", "Debian"]);
            assert_eq!(parse_list(&decode_wsl_output(b"Ubuntu\n")), vec!["Ubuntu"]);
        }
    }
}
