mod commands;
mod config;
mod config_watcher;
mod http_server;
mod state;
mod tray;

use config::ConfigState;
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::get_sessions,
            commands::get_config,
            commands::hide_window,
            commands::show_window,
            commands::toggle_window,
            commands::quit_app,
        ])
        .setup(|app| {
            use tauri::Manager;

            let app_data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data).ok();
            let config_path = app_data.join("config.json");

            let config_state = ConfigState::new(config_path.clone());
            // Ensure a config.json exists on first run so tray "Open config
            // file" and external editing both work without further steps.
            if !config_path.exists() {
                let _ = config_state.save_to_disk();
            }
            let current_config = config_state.snapshot();
            let server_port = current_config.server_port;
            app.manage(config_state);

            // Apply config to the window
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_always_on_top(current_config.always_on_top);
                match (current_config.save_window_position, current_config.window_position) {
                    (true, Some(pos)) => {
                        let _ = window.set_position(tauri::PhysicalPosition::new(pos.x, pos.y));
                    }
                    _ => {
                        config_watcher::apply_default_position(&window);
                    }
                }
            }

            tray::setup(app.handle())?;
            config_watcher::spawn(app.handle().clone(), config_path);

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                http_server::run(handle, server_port).await;
            });

            #[cfg(debug_assertions)]
            seed_dev_sessions(&app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                save_window_position_if_enabled(window);
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn save_window_position_if_enabled(window: &tauri::Window) {
    use tauri::Manager;
    let Some(state) = window.try_state::<ConfigState>() else {
        return;
    };
    let should_save = state.snapshot().save_window_position;
    if !should_save {
        return;
    }
    let Ok(pos) = window.outer_position() else {
        return;
    };
    state.with_mut(|c| {
        c.window_position = Some(config::WindowPosition { x: pos.x, y: pos.y });
    });
    let _ = state.save_to_disk();
}

#[cfg(debug_assertions)]
fn seed_dev_sessions(app: &tauri::AppHandle) {
    use crate::commands::{emit_sessions_updated, now_ms};
    use crate::state::{SetInput, Status};
    use tauri::Manager;

    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let now = now_ms();
    let s = 1000;
    let min = 60 * s;

    state.apply_set(
        SetInput {
            id: "tauri-dashboard".into(),
            status: Status::Working,
            label: Some("I want to migrate an existing electron project to tauri framework".into()),
            source: Some("claude-code".into()),
            model: Some("claude-opus-4-7".into()),
            input_tokens: Some(75_000),
        },
        now - 3 * min,
    );

    state.apply_set(
        SetInput {
            id: "auth-service".into(),
            status: Status::Working,
            label: Some("Add pytest coverage for auth module".into()),
            source: Some("claude-code".into()),
            model: Some("claude-sonnet-4-6".into()),
            input_tokens: Some(152_000),
        },
        now - 4 * min - 12 * s,
    );
    state.apply_set(
        SetInput {
            id: "auth-service".into(),
            status: Status::Awaiting,
            label: Some("Can I run bash: pytest -xvs tests/test_auth.py?".into()),
            source: None,
            model: None,
            input_tokens: Some(152_000),
        },
        now - 45 * s,
    );

    emit_sessions_updated(app);
}
