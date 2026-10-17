//! Dev tool: import this machine's logs into a scratch database and print a summary.
//! `cargo run -p pc-core --example scan -- [db-path]`

use pc_core::analysis::{build_context, AnalysisRequest, ContextMode, Scope};
use pc_core::db::Db;
use pc_core::ingest::{detect_sources, ingest_all, IngestOptions};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "scan.sqlite".into());
    let db = Db::open(std::path::Path::new(&path))?;
    let sources = detect_sources();
    println!("sources: {sources:#?}");
    let report = ingest_all(&db, &sources, &IngestOptions { exclude_cwd_prefixes: &[], only_running_wsl: false, force: true });
    println!("{report:#?}");
    for p in db.list_projects()? {
        println!("\n== {} ({}) sessions={} prompts={} cost={:?}", p.name, p.location, p.session_count, p.prompt_count, p.est_cost);
        for s in db.list_sessions(p.id)? {
            println!("  - {} {} prompts={} tools={} errors={} tokens={:?} models={:?}", s.external_id, s.started_at.as_deref().unwrap_or(""), s.prompt_count, s.tool_call_count, s.error_count, s.tokens, s.models);
            let req = AnalysisRequest { scope: Scope::Session, target_id: Some(s.id), mode: ContextMode::Condensed, model: "x".into(), draft_text: None, draft_context: None, project_dir: None, project_id: None };
            let criteria = db.list_criteria()?;
            match build_context(&db, &req, &criteria, 1_000_000, true) {
                Ok(c) => println!("    context: {} chars (~{} tokens), {} prompts\n{}", c.chars, c.est_tokens, c.prompt_count, c.user_prompt.chars().take(1500).collect::<String>()),
                Err(e) => println!("    context error: {e}"),
            }
        }
    }
    Ok(())
}
