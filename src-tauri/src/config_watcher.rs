use crate::config::{Config, ConfigState};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// Spawn a notify watcher on `config.json`. When the file is modified
/// externally (or by us), re-read it, update the managed state, apply the
/// changes to the window, and emit `config_updated` for the frontend.
pub fn spawn(app: AppHandle, path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let parent = match path.parent() {
            Some(p) => p.to_path_buf(),
            None => {
                eprintln!("[config_watcher] config path has no parent: {path:?}");
                return;
            }
        };
        if let Err(e) = std::fs::create_dir_all(&parent) {
            eprintln!("[config_watcher] create_dir_all({parent:?}) failed: {e}");
            return;
        }

        let watched = path.clone();
        let mut watcher: RecommendedWatcher = match notify::recommended_watcher(
            move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else { return };
                if !matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_)
                ) {
                    return;
                }
                if event.paths.iter().any(|p| p == &watched) {
                    let _ = tx.send(());
                }
            },
        ) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[config_watcher] failed to create watcher: {e}");
                return;
            }
        };

        // Watch the parent directory; the config file itself may not exist yet.
        if let Err(e) = watcher.watch(&parent, RecursiveMode::NonRecursive) {
            eprintln!("[config_watcher] watch({parent:?}) failed: {e}");
            return;
        }

        // Debounce: many editors rewrite via temp file + rename, producing a
        // burst of events; collapse those to one reload.
        let debounce = Duration::from_millis(150);
        let mut last_fire: Option<Instant> = None;

        while let Some(()) = rx.recv().await {
            let now = Instant::now();
            if let Some(prev) = last_fire {
                if now.duration_since(prev) < debounce {
                    continue;
                }
            }
            last_fire = Some(now);

            let new_cfg = Config::load_or_default(&path);
            let Some(state) = app.try_state::<ConfigState>() else {
                continue;
            };
            let prior = state.snapshot();
            if serde_json::to_string(&new_cfg).ok() == serde_json::to_string(&prior).ok() {
                continue; // no effective change (likely our own write)
            }
            state.with_mut(|c| *c = new_cfg.clone());
            apply_config_to_window(&app, &new_cfg, Some(&prior));
            let _ = app.emit("config_updated", &new_cfg);
        }
    });
}

/// Apply the settings that can change at runtime (always_on_top, window
/// position). Port changes require a restart and are intentionally ignored.
pub fn apply_config_to_window(app: &AppHandle, cfg: &Config, prior: Option<&Config>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let prior_aot = prior.map(|p| p.always_on_top);
    if prior_aot != Some(cfg.always_on_top) {
        let _ = window.set_always_on_top(cfg.always_on_top);
    }
    if cfg.save_window_position {
        if let Some(pos) = cfg.window_position {
            let _ = window.set_position(tauri::PhysicalPosition::new(pos.x, pos.y));
        }
    }
}

/// Position the window in the bottom-right of the primary monitor with a
/// small margin. Called at startup when `save_window_position` is off or when
/// no saved position is available.
pub fn apply_default_position(window: &tauri::WebviewWindow) {
    let monitor = match window.primary_monitor() {
        Ok(Some(m)) => m,
        _ => return,
    };
    let screen = monitor.size();
    let size = window.outer_size().unwrap_or_default();
    let margin: i32 = 16;
    let taskbar_allowance: i32 = 60;
    let x = screen.width as i32 - size.width as i32 - margin;
    let y = screen.height as i32 - size.height as i32 - margin - taskbar_allowance;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}
