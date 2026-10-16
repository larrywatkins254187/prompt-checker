//! End-to-end: log folders on disk -> database -> queries.

use std::fs;

use pc_core::db::Db;
use pc_core::ingest::{ingest_all, IngestOptions, SourceConfig};
use pc_core::model::SourceKind;

fn source(kind: SourceKind, path: &std::path::Path) -> SourceConfig {
    SourceConfig {
        id: format!("{}:test", kind.as_str()),
        kind,
        location: "windows".into(),
        path: path.to_string_lossy().into(),
        enabled: true,
        auto: false,
    }
}

#[test]
fn imports_incrementally_and_survives_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    let claude_dir = dir.path().join("claude/projects/C--work-app");
    fs::create_dir_all(&claude_dir).unwrap();
    let file = claude_dir.join("s1.jsonl");
    let line1 = r#"{"type":"user","sessionId":"s1","uuid":"u1","timestamp":"2026-01-01T10:00:00Z","cwd":"C:\\Work\\App","message":{"role":"user","content":"add login page"}}"#;
    let line2 = r#"{"type":"assistant","sessionId":"s1","uuid":"a1","timestamp":"2026-01-01T10:00:05Z","message":{"id":"m1","model":"claude-sonnet-5-5","content":[{"type":"text","text":"Done, added login page."}],"usage":{"input_tokens":1000,"output_tokens":1000}}}"#;
    fs::write(&file, format!("{line1}\n")).unwrap();

    let codex_dir = dir.path().join("codex/sessions/2026/01/02");
    fs::create_dir_all(&codex_dir).unwrap();
    fs::write(
        codex_dir.join("rollout-x.jsonl"),
        r#"{"type":"session_meta","payload":{"id":"cx1","cwd":"c:\\work\\app\\"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"write tests"}]}}
"#,
    )
    .unwrap();

    let db = Db::open(&dir.path().join("db.sqlite")).unwrap();
    let sources = vec![
        source(SourceKind::Claude, &dir.path().join("claude/projects")),
        source(SourceKind::Codex, &dir.path().join("codex/sessions")),
    ];
    let opts = IngestOptions { exclude_cwd_prefixes: &[], only_running_wsl: false, force: false };

    let r = ingest_all(&db, &sources, &opts);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.files_imported, 2);

    // Same folder (case/trailing slash differ) from both tools -> one project.
    let projects = db.list_projects().unwrap();
    assert_eq!(projects.len(), 1, "{projects:?}");
    assert_eq!(projects[0].name, "App");
    assert_eq!(projects[0].session_count, 2);
    assert_eq!(projects[0].prompt_count, 2);

    // Unchanged files are skipped.
    let r = ingest_all(&db, &sources, &opts);
    assert_eq!(r.files_imported, 0);

    // Appended lines are picked up.
    fs::write(&file, format!("{line1}\n{line2}\n")).unwrap();
    let r = ingest_all(&db, &sources, &opts);
    assert_eq!(r.files_imported, 1);
    let sessions = db.list_sessions(projects[0].id).unwrap();
    let s1 = sessions.iter().find(|s| s.external_id == "s1").unwrap();
    assert_eq!(s1.message_count, 2);
    assert_eq!(s1.tokens.output, 1000);
    assert!((s1.est_cost.unwrap() - 0.012).abs() < 1e-9);

    // Search works across sources.
    assert_eq!(db.search("login", &[], None, 10).unwrap().len(), 2);
    assert_eq!(db.search("login", &["prompt".into()], None, 10).unwrap().len(), 1);

    // Claude Code deleting old transcripts must not delete our copy.
    fs::remove_file(&file).unwrap();
    ingest_all(&db, &sources, &opts);
    assert_eq!(db.list_sessions(projects[0].id).unwrap().len(), 2);

    let stats = db.stats(36500).unwrap();
    assert_eq!(stats.sessions, 2);
    assert_eq!(stats.prompts, 2);
}

#[test]
fn excludes_analysis_workdir_and_merges_projects() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("projects/x");
    fs::create_dir_all(&p).unwrap();
    fs::write(
        p.join("a.jsonl"),
        r#"{"type":"user","sessionId":"a","uuid":"1","cwd":"/data/analysis-workdir","message":{"content":"internal"}}
{"type":"user","sessionId":"b","uuid":"1","cwd":"/home/me/one","message":{"content":"one"}}
{"type":"user","sessionId":"c","uuid":"1","cwd":"/home/me/two","message":{"content":"two"}}
"#,
    )
    .unwrap();
    let db = Db::open_in_memory().unwrap();
    let exclude = vec!["/data/analysis-workdir".to_string()];
    let opts = IngestOptions { exclude_cwd_prefixes: &exclude, only_running_wsl: false, force: false };
    ingest_all(&db, &[source(SourceKind::Claude, &dir.path().join("projects"))], &opts);
    let projects = db.list_projects().unwrap();
    assert_eq!(projects.len(), 2);
    db.merge_projects(projects[0].id, projects[1].id).unwrap();
    let projects = db.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].session_count, 2);
}
