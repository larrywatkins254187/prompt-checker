//! Tauri shell: exposes `pc-core` to the React UI as commands and runs the
//! background importer.

mod settings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pc_core::analysis::{self, AnalysisRequest, BuiltContext};
use pc_core::db::{
    AnalysisRow, Criterion, Db, MessageRow, NewAnalysis, ProjectSummary, PromptScore, SearchHit, SessionDetail,
    SessionSummary, Stats,
};
use pc_core::ingest::{self, IngestOptions, IngestReport, SourceConfig};
use pc_core::{claude_settings, cli, export};
use serde::Serialize;
use settings::AppSettings;
use tauri::{AppHandle, Emitter, Manager, State};

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

pub struct AppState {
    db_path: PathBuf,
    /// Folder the Claude CLI runs in for analyses; sessions from here are never imported.
    workdir: PathBuf,
    db: Mutex<Db>,
    ingest_lock: Mutex<()>,
    ingest_running: AtomicBool,
    last_report: Mutex<Option<IngestReport>>,
    cancels: Mutex<HashMap<i64, Arc<AtomicBool>>>,
}

impl AppState {
    fn open_db(&self) -> anyhow::Result<Db> {
        Db::open(&self.db_path)
    }
    fn settings(&self) -> AppSettings {
        let db = self.db.lock().unwrap();
        AppSettings::load(&db)
    }
}

// ------------------------------------------------------------------ ingest

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IngestStatus {
    running: bool,
    last_report: Option<IngestReport>,
}

fn run_ingest(app: &AppHandle, manual: bool, force: bool) -> anyhow::Result<IngestReport> {
    let state = app.state::<AppState>();
    let _guard = state.ingest_lock.lock().unwrap();
    state.ingest_running.store(true, Ordering::Relaxed);
    let _ = app.emit("ingest-status", IngestStatus { running: true, last_report: None });

    let result = (|| {
        let settings = state.settings();
        let db = state.open_db()?;
        let exclude = vec![state.workdir.to_string_lossy().to_string()];
        let opts = IngestOptions { exclude_cwd_prefixes: &exclude, only_running_wsl: !manual, force };
        Ok(ingest::ingest_all(&db, &settings.sources, &opts))
    })();

    state.ingest_running.store(false, Ordering::Relaxed);
    if let Ok(report) = &result {
        *state.last_report.lock().unwrap() = Some(report.clone());
    }
    let _ = app.emit(
        "ingest-status",
        IngestStatus { running: false, last_report: state.last_report.lock().unwrap().clone() },
    );
    result
}

fn start_poller(app: AppHandle) {
    std::thread::spawn(move || {
        // First run: detect log folders.
        {
            let state = app.state::<AppState>();
            let db = state.db.lock().unwrap();
            let mut s = AppSettings::load(&db);
            if !s.sources_initialized {
                s.sources = ingest::detect_sources();
                s.sources_initialized = true;
                let _ = s.save(&db);
            }
        }
        loop {
            // Automatic scans skip stopped WSL distros so the app never boots WSL by itself.
            let _ = run_ingest(&app, false, false);
            let secs = app.state::<AppState>().settings().poll_seconds.max(5);
            std::thread::sleep(Duration::from_secs(secs));
        }
    });
}

#[tauri::command]
async fn rescan(app: AppHandle, force: Option<bool>) -> CmdResult<IngestReport> {
    tauri::async_runtime::spawn_blocking(move || run_ingest(&app, true, force.unwrap_or(false)))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
fn ingest_status(state: State<AppState>) -> IngestStatus {
    IngestStatus {
        running: state.ingest_running.load(Ordering::Relaxed),
        last_report: state.last_report.lock().unwrap().clone(),
    }
}

#[tauri::command]
async fn detect_sources(app: AppHandle) -> CmdResult<Vec<SourceConfig>> {
    let detected = tauri::async_runtime::spawn_blocking(ingest::detect_sources).await.map_err(err)?;
    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let mut s = AppSettings::load(&db);
    s.sources = ingest::merge_sources(&s.sources, detected);
    s.sources_initialized = true;
    s.save(&db).map_err(err)?;
    Ok(s.sources)
}

// ------------------------------------------------------------------ settings

#[tauri::command]
fn get_settings(state: State<AppState>) -> AppSettings {
    state.settings()
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: AppSettings) -> CmdResult<AppSettings> {
    let db = state.db.lock().unwrap();
    settings.save(&db).map_err(err)?;
    Ok(AppSettings::load(&db))
}

#[tauri::command]
async fn claude_cli_status(app: AppHandle) -> CmdResult<String> {
    let state = app.state::<AppState>();
    let cfg = cli_config(&state);
    tauri::async_runtime::spawn_blocking(move || cli::version(&cfg)).await.map_err(err)?.map_err(err)
}

#[tauri::command]
fn retention_status() -> CmdResult<claude_settings::RetentionStatus> {
    claude_settings::check().map_err(err)
}

#[tauri::command]
fn fix_retention() -> CmdResult<claude_settings::RetentionStatus> {
    claude_settings::set_cleanup_days(claude_settings::RECOMMENDED_DAYS).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    db_path: String,
    workdir: String,
}

#[tauri::command]
fn app_info(app: AppHandle, state: State<AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        db_path: state.db_path.to_string_lossy().into_owned(),
        workdir: state.workdir.to_string_lossy().into_owned(),
    }
}

// ------------------------------------------------------------------ browsing

#[tauri::command]
fn list_projects(state: State<AppState>) -> CmdResult<Vec<ProjectSummary>> {
    state.db.lock().unwrap().list_projects().map_err(err)
}

#[tauri::command]
fn get_project(state: State<AppState>, id: i64) -> CmdResult<Option<ProjectSummary>> {
    state.db.lock().unwrap().get_project(id).map_err(err)
}

#[tauri::command]
fn rename_project(state: State<AppState>, id: i64, name: String) -> CmdResult<()> {
    if name.trim().is_empty() {
        return Err("Name cannot be empty".into());
    }
    state.db.lock().unwrap().rename_project(id, &name).map_err(err)
}

#[tauri::command]
fn merge_projects(state: State<AppState>, source: i64, target: i64) -> CmdResult<()> {
    state.db.lock().unwrap().merge_projects(source, target).map_err(err)
}

#[tauri::command]
fn list_sessions(state: State<AppState>, project_id: i64) -> CmdResult<Vec<SessionSummary>> {
    state.db.lock().unwrap().list_sessions(project_id).map_err(err)
}

#[tauri::command]
fn get_session(state: State<AppState>, id: i64) -> CmdResult<SessionDetail> {
    state.db.lock().unwrap().get_session(id).map_err(err)
}

#[tauri::command]
fn get_message(state: State<AppState>, id: i64) -> CmdResult<MessageRow> {
    state.db.lock().unwrap().get_message(id).map_err(err)
}

#[tauri::command]
fn message_session(state: State<AppState>, id: i64) -> CmdResult<i64> {
    state.db.lock().unwrap().message_session(id).map_err(err)
}

#[tauri::command]
fn prompt_scores(state: State<AppState>, session_id: i64) -> CmdResult<Vec<PromptScore>> {
    state.db.lock().unwrap().prompt_scores(session_id).map_err(err)
}

#[tauri::command]
fn search(state: State<AppState>, query: String, kinds: Vec<String>, project_id: Option<i64>) -> CmdResult<Vec<SearchHit>> {
    state.db.lock().unwrap().search(&query, &kinds, project_id, 200).map_err(err)
}

#[tauri::command]
fn stats(state: State<AppState>, days: Option<i64>) -> CmdResult<Stats> {
    state.db.lock().unwrap().stats(days.unwrap_or(90)).map_err(err)
}

// ------------------------------------------------------------------ rubric

#[tauri::command]
fn list_criteria(state: State<AppState>) -> CmdResult<Vec<Criterion>> {
    state.db.lock().unwrap().list_criteria().map_err(err)
}

#[tauri::command]
fn save_criteria(state: State<AppState>, criteria: Vec<Criterion>) -> CmdResult<Vec<Criterion>> {
    if !criteria.iter().any(|c| c.enabled) {
        return Err("Enable at least one criterion".into());
    }
    let db = state.db.lock().unwrap();
    db.in_tx(|db| db.save_criteria(&criteria)).map_err(err)?;
    db.list_criteria().map_err(err)
}

#[tauri::command]
fn reset_criteria(state: State<AppState>) -> CmdResult<Vec<Criterion>> {
    let db = state.db.lock().unwrap();
    db.in_tx(|db| db.save_criteria(&pc_core::db::default_criteria())).map_err(err)?;
    db.list_criteria().map_err(err)
}

// ------------------------------------------------------------------ analyses

fn cli_config(state: &AppState) -> cli::CliConfig {
    let s = state.settings();
    cli::CliConfig {
        path: s.claude_path.clone().filter(|p| !p.trim().is_empty()),
        workdir: state.workdir.clone(),
        timeout: Duration::from_secs(s.timeout_minutes.max(1) * 60),
    }
}

fn build(state: &AppState, req: &AnalysisRequest) -> anyhow::Result<BuiltContext> {
    let s = state.settings();
    let db = state.db.lock().unwrap();
    let criteria: Vec<Criterion> = db.list_criteria()?.into_iter().filter(|c| c.enabled).collect();
    analysis::build_context(&db, req, &criteria, s.max_context_chars, s.redact)
}

#[tauri::command]
fn preview_analysis(state: State<AppState>, request: AnalysisRequest) -> CmdResult<BuiltContext> {
    build(&state, &request).map_err(err)
}

/// The exact text that would be sent, for transparency.
#[tauri::command]
fn preview_analysis_text(state: State<AppState>, request: AnalysisRequest) -> CmdResult<String> {
    let c = build(&state, &request).map_err(err)?;
    Ok(format!("===== SYSTEM PROMPT =====\n{}\n\n===== INPUT =====\n{}", c.system_prompt, c.user_prompt))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisEvent {
    id: i64,
    status: String,
    scope: String,
    target_id: Option<i64>,
}

#[tauri::command]
fn start_analysis(app: AppHandle, state: State<AppState>, request: AnalysisRequest) -> CmdResult<i64> {
    let model = request.model.trim().to_string();
    if model.is_empty() {
        return Err("Choose a model".into());
    }
    let built = build(&state, &request).map_err(err)?;
    let id = {
        let db = state.db.lock().unwrap();
        db.create_analysis(&NewAnalysis {
            scope: request.scope.as_str(),
            target_id: request.target_id,
            model: &model,
            context_mode: request.mode.as_str(),
            input_text: request.draft_text.as_deref(),
            ref_map: Some(&built.ref_map),
            truncated: built.truncated,
        })
        .map_err(err)?
    };
    let cancel = Arc::new(AtomicBool::new(false));
    state.cancels.lock().unwrap().insert(id, cancel.clone());
    let cfg = cli_config(&state);
    let event = AnalysisEvent { id, status: "running".into(), scope: request.scope.as_str().into(), target_id: request.target_id };
    let _ = app.emit("analysis-updated", event.clone());

    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let outcome = cli::run(&cfg, &model, &built.system_prompt, &built.user_prompt, cancel);
        let status = match state.open_db() {
            Ok(db) => {
                let criteria = db.list_criteria().unwrap_or_default();
                match outcome {
                    Ok(res) => match analysis::parse_result(&res.text, &criteria) {
                        Ok((value, score)) => {
                            let _ = db.finish_analysis(id, score, &value, &res.raw, res.cost_usd, res.duration_ms);
                            "done"
                        }
                        Err(e) => {
                            let _ = db.fail_analysis(id, &err(e), Some(&res.raw));
                            "error"
                        }
                    },
                    Err(e) => {
                        let _ = db.fail_analysis(id, &err(e), None);
                        "error"
                    }
                }
            }
            Err(_) => "error",
        };
        state.cancels.lock().unwrap().remove(&id);
        let _ = app.emit("analysis-updated", AnalysisEvent { status: status.into(), ..event });
    });
    Ok(id)
}

#[tauri::command]
fn cancel_analysis(state: State<AppState>, id: i64) {
    if let Some(flag) = state.cancels.lock().unwrap().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn list_analyses(state: State<AppState>, scope: Option<String>, target_id: Option<i64>) -> CmdResult<Vec<AnalysisRow>> {
    state.db.lock().unwrap().list_analyses(scope.as_deref(), target_id).map_err(err)
}

#[tauri::command]
fn session_prompt_analyses(state: State<AppState>, session_id: i64) -> CmdResult<Vec<AnalysisRow>> {
    state.db.lock().unwrap().session_prompt_analyses(session_id).map_err(err)
}

#[tauri::command]
fn get_analysis(state: State<AppState>, id: i64) -> CmdResult<AnalysisRow> {
    state.db.lock().unwrap().get_analysis(id).map_err(err)
}

#[tauri::command]
fn delete_analysis(state: State<AppState>, id: i64) -> CmdResult<()> {
    state.db.lock().unwrap().delete_analysis(id).map_err(err)
}

// ------------------------------------------------------------------ export

#[tauri::command]
fn export_session(state: State<AppState>, id: i64, format: String, path: String) -> CmdResult<()> {
    let db = state.db.lock().unwrap();
    let mut detail = db.get_session(id).map_err(err)?;
    detail.messages = db.session_messages(id, false).map_err(err)?;
    let text = match format.as_str() {
        "json" => serde_json::to_string_pretty(&export::session_json(&detail)).map_err(err)?,
        _ => export::session_markdown(&detail),
    };
    std::fs::write(&path, text).map_err(err)
}

#[tauri::command]
fn export_analysis(state: State<AppState>, id: i64, format: String, path: String) -> CmdResult<()> {
    let a = state.db.lock().unwrap().get_analysis(id).map_err(err)?;
    let text = match format.as_str() {
        "json" => serde_json::to_string_pretty(&a).map_err(err)?,
        _ => export::analysis_markdown(&a),
    };
    std::fs::write(&path, text).map_err(err)
}

// ------------------------------------------------------------------ app

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("prompt-checker.sqlite");
            let workdir = data_dir.join("analysis-workdir");
            std::fs::create_dir_all(&workdir)?;
            let db = Db::open(&db_path)?;
            db.fail_stale_analyses()?;
            app.manage(AppState {
                db_path,
                workdir,
                db: Mutex::new(db),
                ingest_lock: Mutex::new(()),
                ingest_running: AtomicBool::new(false),
                last_report: Mutex::new(None),
                cancels: Mutex::new(HashMap::new()),
            });
            start_poller(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            rescan,
            ingest_status,
            detect_sources,
            get_settings,
            save_settings,
            claude_cli_status,
            retention_status,
            fix_retention,
            app_info,
            list_projects,
            get_project,
            rename_project,
            merge_projects,
            list_sessions,
            get_session,
            get_message,
            message_session,
            prompt_scores,
            search,
            stats,
            list_criteria,
            save_criteria,
            reset_criteria,
            preview_analysis,
            preview_analysis_text,
            start_analysis,
            cancel_analysis,
            list_analyses,
            session_prompt_analyses,
            get_analysis,
            delete_analysis,
            export_session,
            export_analysis,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Prompt Checker");
}
