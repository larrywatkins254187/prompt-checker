//! Silent auto-update.
//!
//! - On start (release builds) and every 6 hours: check GitHub Releases for a newer
//!   signed version and download it in the background.
//! - Once downloaded, the UI shows "Restart to update"; if the user never clicks it,
//!   the cached installer is applied quietly the next time the app starts.
//! - Settings has a manual "Check for updates" button.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    /// idle | checking | downloading | ready | up-to-date | error
    pub state: String,
    pub latest_version: Option<String>,
    pub notes: Option<String>,
    /// 0..1 while downloading, when the size is known.
    pub progress: Option<f64>,
    pub error: Option<String>,
    pub checked_at: Option<String>,
}

#[derive(Default)]
pub struct UpdaterState {
    status: Mutex<UpdateStatus>,
    pending: Mutex<Option<(Update, Vec<u8>)>>,
    busy: AtomicBool,
}

/// Clears the busy flag when a check ends, however it ends.
struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

fn cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("updates"))
}

fn cache_file(app: &AppHandle, version: &str) -> Option<PathBuf> {
    let safe: String = version.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').collect();
    cache_dir(app).map(|d| d.join(format!("prompt-checker-{safe}.bin")))
}

fn set_status(app: &AppHandle, f: impl FnOnce(&mut UpdateStatus)) {
    let state = app.state::<UpdaterState>();
    let snapshot = {
        let mut s = state.status.lock().unwrap();
        f(&mut s);
        s.clone()
    };
    let _ = app.emit("update-status", snapshot);
}

pub fn status(app: &AppHandle) -> UpdateStatus {
    let state = app.state::<UpdaterState>();
    let mut s = state.status.lock().unwrap().clone();
    s.current_version = app.package_info().version.to_string();
    if s.state.is_empty() {
        s.state = "idle".into();
    }
    s
}

/// Check for a newer version and download it. With `install_cached`, an installer that
/// was already downloaded in an earlier run is applied immediately (startup path).
pub async fn check(app: &AppHandle, install_cached: bool) -> Result<UpdateStatus, String> {
    let state = app.state::<UpdaterState>();
    if state.busy.swap(true, Ordering::SeqCst) {
        return Ok(status(app)); // a check is already running
    }
    let _guard = BusyGuard(&state.busy);
    if state.pending.lock().unwrap().is_some() {
        return Ok(status(app)); // already downloaded, waiting for a restart
    }
    let current = app.package_info().version.to_string();
    set_status(app, |s| {
        s.current_version = current.clone();
        s.state = "checking".into();
        s.error = None;
        s.progress = None;
    });

    let result = async {
        let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
        let Some(update) = update else {
            clear_cache(app, None);
            return Ok::<_, String>(None);
        };
        let version = update.version.clone();
        set_status(app, |s| {
            s.latest_version = Some(version.clone());
            s.notes = update.body.clone();
        });

        // Reuse an installer downloaded by a previous run.
        let cached = cache_file(app, &version).and_then(|p| std::fs::read(p).ok()).filter(|b| !b.is_empty());
        let from_cache = cached.is_some();
        let bytes = match cached {
            Some(b) => b,
            None => {
                clear_cache(app, None);
                set_status(app, |s| {
                    s.state = "downloading".into();
                    s.progress = Some(0.0);
                });
                let mut received: u64 = 0;
                let handle = app.clone();
                let bytes = update
                    .download(
                        move |chunk, total| {
                            received += chunk as u64;
                            if let Some(total) = total.filter(|t| *t > 0) {
                                let p = received as f64 / total as f64;
                                set_status(&handle, |s| s.progress = Some(p.min(1.0)));
                            }
                        },
                        || {},
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                if let Some(path) = cache_file(app, &version) {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let tmp = path.with_extension("part");
                    if std::fs::write(&tmp, &bytes).is_ok() {
                        let _ = std::fs::rename(&tmp, &path);
                    }
                }
                bytes
            }
        };
        Ok(Some((update, bytes, from_cache)))
    }
    .await;

    let now = pc_core::db::now();
    match result {
        Ok(None) => set_status(app, |s| {
            s.state = "up-to-date".into();
            s.latest_version = Some(current.clone());
            s.checked_at = Some(now);
            s.progress = None;
        }),
        Ok(Some((update, bytes, from_cache))) => {
            if install_cached && from_cache && !crate::analysis_running(app) {
                // Never clicked "Restart to update" last time: apply it quietly now.
                let _ = install_bytes(app, &update, &bytes);
            }
            *state.pending.lock().unwrap() = Some((update, bytes));
            set_status(app, |s| {
                s.state = "ready".into();
                s.checked_at = Some(now);
                s.progress = None;
            });
        }
        Err(e) => set_status(app, |s| {
            s.state = "error".into();
            s.error = Some(if e.contains("valid release JSON") {
                // GitHub answers 404 until the first signed release is published.
                "no published release found on GitHub yet".into()
            } else {
                e
            });
            s.checked_at = Some(now);
            s.progress = None;
        }),
    }
    Ok(status(app))
}

fn install_bytes(app: &AppHandle, update: &Update, bytes: &[u8]) -> Result<(), String> {
    // Remove the cache first so a broken installer cannot loop on every start.
    clear_cache(app, None);
    update.install(bytes).map_err(|e| e.to_string())?;
    // Windows exits inside install(); other platforms need an explicit restart.
    app.restart();
}

/// "Restart to update" button.
pub fn install_now(app: &AppHandle) -> Result<(), String> {
    if crate::analysis_running(app) {
        return Err("An analysis is still running. Wait for it to finish, or cancel it, then update.".into());
    }
    let state = app.state::<UpdaterState>();
    let pending = state.pending.lock().unwrap().take();
    let Some((update, bytes)) = pending else {
        return Err("No downloaded update. Check for updates first.".into());
    };
    install_bytes(app, &update, &bytes)
}

/// Delete cached installers (all, or all except `keep`).
fn clear_cache(app: &AppHandle, keep: Option<&PathBuf>) {
    let Some(dir) = cache_dir(app) else { return };
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if Some(&e.path()) != keep {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Background loop: check at start (applying a cached update) and every few hours.
pub fn start(app: AppHandle) {
    if cfg!(debug_assertions) {
        return; // dev builds: manual checks only
    }
    tauri::async_runtime::spawn(async move {
        tokio_sleep(Duration::from_secs(3)).await;
        let _ = check(&app, true).await;
        loop {
            tokio_sleep(CHECK_EVERY).await;
            let _ = check(&app, false).await;
        }
    });
}

async fn tokio_sleep(d: Duration) {
    // Tauri's async runtime is tokio; avoid a direct tokio dependency.
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}
