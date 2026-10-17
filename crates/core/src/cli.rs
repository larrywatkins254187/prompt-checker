//! Running analyses through the Claude Code CLI (`claude -p`), which uses the
//! user's existing Claude subscription/login instead of a separate API key.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::model::Usage;

#[derive(Debug, Clone)]
pub struct CliConfig {
    /// Explicit path to the `claude` executable; auto-detected when `None`.
    pub path: Option<String>,
    /// Folder the CLI runs in (kept empty so the analysis has nothing to explore).
    pub workdir: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct CliResult {
    pub text: String,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
    /// Tokens used by the whole run (all turns and models).
    pub usage: Option<Usage>,
    pub raw: String,
}

/// Read-only tools the reviewer may use when it is given a project folder.
pub const READ_ONLY_TOOLS: &str = "Read,Glob,Grep";

#[cfg(windows)]
const EXTENSIONS: &[&str] = &[".exe", ".cmd", ".bat", ""];
#[cfg(not(windows))]
const EXTENSIONS: &[&str] = &[""];

/// Locate the `claude` executable: explicit setting, then PATH, then common install folders.
pub fn resolve_claude(explicit: Option<&str>) -> Result<PathBuf> {
    if let Some(p) = explicit.map(str::trim).filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
        bail!("Claude CLI not found at {}", p.display());
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".claude").join("local"));
        dirs.push(home.join(".npm-global").join("bin"));
        dirs.push(home.join("AppData").join("Roaming").join("npm"));
    }
    for dir in dirs {
        for ext in EXTENSIONS {
            let candidate = dir.join(format!("claude{ext}"));
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    bail!("Could not find the Claude Code CLI ('claude'). Install it, or set its path in Settings.")
}

fn command(program: &Path) -> Command {
    // Rust escapes arguments correctly for .cmd/.bat targets on Windows.
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `claude --version`, to show in Settings and to verify the setup.
pub fn version(cfg: &CliConfig) -> Result<String> {
    let program = resolve_claude(cfg.path.as_deref())?;
    let out = command(&program).arg("--version").output().with_context(|| format!("running {}", program.display()))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || text.is_empty() {
        bail!("'{} --version' failed: {}", program.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(format!("{text} ({})", program.display()))
}

struct Output {
    status_ok: bool,
    stdout: String,
    stderr: String,
}

fn run_process(program: &Path, args: &[String], stdin_text: &str, cwd: &Path, cfg: &CliConfig, cancel: &AtomicBool) -> Result<Output> {
    std::fs::create_dir_all(&cfg.workdir).ok();
    let mut child = command(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {}", program.display()))?;

    let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
    let input = stdin_text.to_string();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
        // dropping stdin closes it so the CLI starts working
    });
    let mut out_pipe = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let mut err_pipe = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;
    let out_reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out_pipe.read_to_end(&mut s);
        s
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err_pipe.read_to_end(&mut s);
        s
    });

    let deadline = Instant::now() + cfg.timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Cancelled");
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Timed out after {} seconds", cfg.timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let _ = writer.join();
    let stdout = String::from_utf8_lossy(&out_reader.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned();
    Ok(Output { status_ok: status.success(), stdout, stderr })
}

/// Run one analysis. The system prompt is passed as a file (falling back to an inline
/// argument for older CLI versions); the transcript is sent on stdin.
///
/// Without `project_dir` the model gets no tools at all and runs in the empty app work
/// folder. With it, the CLI runs inside that folder with read-only tools only.
pub fn run(
    cfg: &CliConfig,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    project_dir: Option<&Path>,
    cancel: Arc<AtomicBool>,
) -> Result<CliResult> {
    if let Some(dir) = project_dir {
        if !dir.is_dir() {
            bail!("Project folder not found: {}", dir.display());
        }
    }
    let cwd = project_dir.unwrap_or(&cfg.workdir);
    let program = resolve_claude(cfg.path.as_deref())?;
    std::fs::create_dir_all(&cfg.workdir).ok();
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let sys_file = cfg.workdir.join(format!("system-prompt-{}-{n}.txt", std::process::id()));
    std::fs::write(&sys_file, system_prompt).context("writing system prompt file")?;

    let base = |extra: &[&str]| -> Vec<String> {
        let mut a: Vec<String> = vec!["-p".into(), "--model".into(), model.into(), "--output-format".into(), "json".into()];
        a.extend(extra.iter().map(|s| s.to_string()));
        a
    };
    let sys_path = sys_file.to_string_lossy().to_string();
    let attempts: Vec<Vec<String>> = if project_dir.is_some() {
        let tools = ["--tools", READ_ONLY_TOOLS, "--allowedTools", READ_ONLY_TOOLS, "--permission-mode", "dontAsk"];
        let mut first = vec!["--system-prompt-file", sys_path.as_str(), "--no-session-persistence", "--strict-mcp-config"];
        first.extend(tools);
        let mut fallback = vec!["--system-prompt", system_prompt];
        fallback.extend(["--allowedTools", READ_ONLY_TOOLS]);
        vec![base(&first), base(&fallback)]
    } else {
        vec![
            base(&["--system-prompt-file", &sys_path, "--tools", "", "--no-session-persistence", "--strict-mcp-config"]),
            // Older CLI versions: fewer flags, inline system prompt.
            base(&["--system-prompt", system_prompt]),
        ]
    };

    let mut last_err = None;
    for args in attempts {
        let out = run_process(&program, &args, user_prompt, cwd, cfg, &cancel);
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                let _ = std::fs::remove_file(&sys_file);
                return Err(e);
            }
        };
        let unknown_flag = !out.status_ok
            && (out.stderr.contains("unknown option") || out.stderr.contains("Unknown option") || out.stderr.contains("unexpected argument"));
        if unknown_flag {
            last_err = Some(anyhow!("{}", out.stderr.trim()));
            continue;
        }
        let _ = std::fs::remove_file(&sys_file);
        return parse_cli_output(&out.stdout, &out.stderr, out.status_ok);
    }
    let _ = std::fs::remove_file(&sys_file);
    Err(last_err.unwrap_or_else(|| anyhow!("Claude CLI failed")))
}

/// Parse `claude -p --output-format json` output.
pub fn parse_cli_output(stdout: &str, stderr: &str, ok: bool) -> Result<CliResult> {
    // The JSON result is the last JSON line (some versions print warnings before it).
    let parsed = stdout
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str::<Value>(l.trim()).ok().filter(Value::is_object))
        .or_else(|| serde_json::from_str::<Value>(stdout.trim()).ok());
    let Some(v) = parsed else {
        let msg = if !stderr.trim().is_empty() { stderr.trim() } else { stdout.trim() };
        bail!("Claude CLI returned no result{}: {}", if ok { "" } else { " (failed)" }, cut(msg, 2000));
    };
    let text = v.get("result").and_then(Value::as_str).unwrap_or("").to_string();
    let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false)
        || v.get("subtype").and_then(Value::as_str).map(|s| s != "success").unwrap_or(false);
    if is_error || (!ok && text.is_empty()) {
        let detail = if !text.is_empty() { text.clone() } else { stderr.trim().to_string() };
        bail!("Claude CLI reported an error: {}", cut(&detail, 2000));
    }
    Ok(CliResult {
        text,
        cost_usd: v.get("total_cost_usd").or_else(|| v.get("cost_usd")).and_then(Value::as_f64),
        duration_ms: v.get("duration_ms").and_then(Value::as_i64),
        usage: usage_of(&v),
        raw: stdout.to_string(),
    })
}

/// Total tokens of a run: `modelUsage` sums every model involved (e.g. after a fallback);
/// older CLI versions only report `usage`.
fn usage_of(v: &Value) -> Option<Usage> {
    let int = |o: &Value, k: &str| o.get(k).and_then(Value::as_i64).unwrap_or(0);
    if let Some(models) = v.get("modelUsage").and_then(Value::as_object).filter(|m| !m.is_empty()) {
        let mut u = Usage::default();
        for m in models.values() {
            u.input_tokens += int(m, "inputTokens");
            u.output_tokens += int(m, "outputTokens");
            u.cache_read_tokens += int(m, "cacheReadInputTokens");
            u.cache_creation_tokens += int(m, "cacheCreationInputTokens");
        }
        return Some(u);
    }
    v.get("usage").filter(|u| u.is_object()).map(|u| Usage {
        input_tokens: int(u, "input_tokens"),
        output_tokens: int(u, "output_tokens"),
        cache_read_tokens: int(u, "cache_read_input_tokens"),
        cache_creation_tokens: int(u, "cache_creation_input_tokens"),
    })
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success_output() {
        let out = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":1234,"result":"{\"overall_score\":7}","total_cost_usd":0.05}"#;
        let r = parse_cli_output(out, "", true).unwrap();
        assert_eq!(r.text, "{\"overall_score\":7}");
        assert_eq!(r.cost_usd, Some(0.05));
        assert_eq!(r.duration_ms, Some(1234));
        assert_eq!(r.usage, None);
    }

    #[test]
    fn reads_token_usage() {
        let out = r#"{"type":"result","subtype":"success","is_error":false,"result":"{}","usage":{"input_tokens":6,"output_tokens":322,"cache_read_input_tokens":8173,"cache_creation_input_tokens":4374},"modelUsage":{"claude-haiku-5-5":{"inputTokens":6,"outputTokens":322,"cacheReadInputTokens":8173,"cacheCreationInputTokens":4374},"claude-opus-5-5":{"inputTokens":1,"outputTokens":1,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}}}"#;
        let u = parse_cli_output(out, "", true).unwrap().usage.unwrap();
        assert_eq!((u.input_tokens, u.output_tokens, u.cache_read_tokens, u.cache_creation_tokens), (7, 323, 8173, 4374));
        let only_usage = r#"{"type":"result","subtype":"success","result":"{}","usage":{"input_tokens":2,"output_tokens":3}}"#;
        assert_eq!(parse_cli_output(only_usage, "", true).unwrap().usage.unwrap().output_tokens, 3);
    }

    #[test]
    fn reports_errors() {
        let out = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Invalid model"}"#;
        assert!(parse_cli_output(out, "", false).unwrap_err().to_string().contains("Invalid model"));
        assert!(parse_cli_output("", "not logged in", false).unwrap_err().to_string().contains("not logged in"));
    }
}
