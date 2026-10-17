//! Core logic for Prompt Checker: parsing Claude Code / Codex CLI session logs,
//! storing them in SQLite, building analysis prompts and running them through
//! the Claude Code CLI. Kept free of any Tauri dependency so it can be tested
//! on any platform.

pub mod analysis;
pub mod claude_settings;
pub mod cli;
pub mod db;
pub mod export;
pub mod ingest;
pub mod model;
pub mod parse;
pub mod pricing;
pub mod redact;
pub mod usage;

pub use db::Db;
