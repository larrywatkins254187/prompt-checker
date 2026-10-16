# Prompt Checker

A Windows desktop app that collects every prompt you have given to **Claude Code** and **Codex CLI**, keeps them in a local SQLite database, and uses Claude (through your own `claude` CLI login) to review how well you prompt: scores, concrete mistakes with evidence, and rewritten versions of your prompts.

## Features

- **Automatic import** from
  - `%USERPROFILE%\.claude\projects` (Claude Code, including subagent transcripts)
  - `%USERPROFILE%\.codex\sessions` (Codex CLI)
  - the same folders inside every **WSL** distribution (`\\wsl.localhost\<distro>\home\<user>\…`)
  - any extra folder you add in Settings

  New and changed files are imported every 30 s. Stopped WSL distributions are skipped by the automatic import, so the app never boots WSL on its own; **Rescan** scans everything. Data already imported is kept even after Claude Code deletes old transcripts.
- **Projects** are grouped by working folder (rename them, or merge folders that are really the same project, e.g. Windows + WSL checkouts).
- **Session viewer**: your prompts (numbered P1, P2, …), the assistant's replies, collapsible tool calls with failures highlighted, and your answers to Claude's clarifying questions.
- **Analysis** at four levels: one prompt, one session, one project, or all projects. Pick the model (Sonnet 5.5, Opus 5.5, Haiku 5.5, Fable 5.1, or any id/alias) and how much context to send:
  - *Prompts only*: cheapest;
  - *Condensed*: prompts + replies, tool calls as one-liners, only failed tool output;
  - *Full*: everything (each item capped).

  Each report has an overall score, per-criterion scores, mistakes (with links back to the exact prompt), suggested rewrites, strengths, recurring patterns and tips. Every analysis is kept, so you can compare models or track progress, and export it to Markdown/JSON.
- **Prompt rewriter**: review a prompt *before* you send it.
- **Editable rubric**: change, reorder, weight or add scoring criteria.
- **Full-text search** across prompts, replies and tool output.
- **Usage**: sessions/prompts per day, tokens and estimated cost per project and model.
- **Privacy**: API keys, tokens, passwords, private keys and credentials in URLs are masked as `[REDACTED]` before anything is sent to the model ("View exact input" shows precisely what is sent). The local database keeps the original text.

## Install

Download the installer (`.msi` or `-setup.exe`) from the latest **Build** workflow run (artifact *prompt-checker-windows*) or from a GitHub release.

Requirements:
- Windows 10/11 (WebView2 is installed automatically if missing).
- [Claude Code](https://docs.claude.com/en/docs/claude-code) installed and logged in, for analyses. The app finds `claude` on `PATH`, in `%USERPROFILE%\.local\bin` or the npm global folder; you can also set the path in Settings.

### Keep your history

Claude Code deletes transcripts older than 30 days by default. The app shows a banner offering to set `"cleanupPeriodDays": 99999` in `%USERPROFILE%\.claude\settings.json` (a `.bak` copy is made first). You can also set it by hand.

## How analyses run

Each analysis calls

```
claude -p --model <model> --output-format json --system-prompt-file <reviewer instructions> \
       --tools "" --no-session-persistence --strict-mcp-config
```

with the transcript on stdin, in an empty working folder. No tools, no MCP servers, and nothing is saved as a Claude Code session, so analysis runs never show up as projects. It uses whatever account `claude` is logged into (your subscription). Analyses only run when you click **Analyse**.

## Development

Stack: [Tauri 2](https://tauri.app) (Rust) + React + TypeScript + SQLite (`rusqlite`, bundled, with FTS5).

```
crates/core      parsing, database, ingestion, analysis prompts, CLI runner (no Tauri dependency, unit-tested)
src-tauri        Tauri commands, background importer, settings
src              React UI
```

```bash
npm install
npm run tauri dev          # run the app
cargo test -p pc-core      # core tests
npm run typecheck
npx tauri build            # installer (on Windows)

# Import this machine's logs into a scratch DB and print what was found:
cargo run -p pc-core --example scan -- scan.sqlite
```

On Linux, building the desktop shell needs the WebKitGTK packages listed in the [Tauri prerequisites](https://tauri.app/start/prerequisites/). Windows installers are built by `.github/workflows/build.yml`; pushing a `v*` tag attaches them to a draft release.

The app's data lives in `%APPDATA%\com.promptchecker.app\prompt-checker.sqlite`.
