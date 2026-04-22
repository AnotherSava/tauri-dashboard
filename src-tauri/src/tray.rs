use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

use crate::commands::emit_config_updated;
use crate::config::ConfigState;

const MENU_SHOW_HIDE: &str = "show_hide";
const MENU_ALWAYS_ON_TOP: &str = "always_on_top";
const MENU_SAVE_POSITION: &str = "save_position";
const MENU_AUTOSTART: &str = "autostart";
const MENU_OPEN_DATA_DIR: &str = "open_data_dir";
const MENU_ABOUT: &str = "about";
const MENU_QUIT: &str = "quit";

/// Tray menu item handles kept in managed state so menu handlers can update
/// check-marks after toggling the underlying setting.
pub struct TrayHandles {
    pub always_on_top: CheckMenuItem<Wry>,
    pub save_position: CheckMenuItem<Wry>,
    pub autostart: CheckMenuItem<Wry>,
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let show_hide = MenuItem::with_id(app, MENU_SHOW_HIDE, "Show / Hide", true, None::<&str>)?;

    let (aot_initial, save_pos_initial) = app
        .try_state::<ConfigState>()
        .map(|s| {
            let c = s.snapshot();
            (c.always_on_top, c.save_window_position)
        })
        .unwrap_or((true, false));
    let always_on_top = CheckMenuItem::with_id(
        app, MENU_ALWAYS_ON_TOP, "Always on top", true, aot_initial, None::<&str>,
    )?;

    let save_position = CheckMenuItem::with_id(
        app, MENU_SAVE_POSITION, "Save position on exit", true, save_pos_initial, None::<&str>,
    )?;

    let autostart_initial = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app, MENU_AUTOSTART, "Open on system start", true, autostart_initial, None::<&str>,
    )?;

    let open_data_dir = MenuItem::with_id(app, MENU_OPEN_DATA_DIR, "Open config/logs location", true, None::<&str>)?;
    let about = MenuItem::with_id(app, MENU_ABOUT, "About", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show_hide,
            &PredefinedMenuItem::separator(app)?,
            &always_on_top,
            &save_position,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &open_data_dir,
            &PredefinedMenuItem::separator(app)?,
            &about,
            &quit,
        ],
    )?;

    app.manage(TrayHandles {
        always_on_top: always_on_top.clone(),
        save_position: save_position.clone(),
        autostart: autostart.clone(),
    });

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("window icon".into()))?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("AI Agent Dashboard")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu_event(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        MENU_SHOW_HIDE => toggle_window(app),
        MENU_ALWAYS_ON_TOP => toggle_always_on_top(app),
        MENU_SAVE_POSITION => toggle_save_position(app),
        MENU_AUTOSTART => toggle_autostart(app),
        MENU_OPEN_DATA_DIR => open_data_dir(app),
        MENU_ABOUT => show_about(app),
        MENU_QUIT => app.exit(0),
        _ => {}
    }
}

fn toggle_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(true) {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_always_on_top(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let new_state = !window.is_always_on_top().unwrap_or(false);
    let _ = window.set_always_on_top(new_state);
    if let Some(state) = app.try_state::<ConfigState>() {
        state.with_mut(|c| c.always_on_top = new_state);
        let _ = state.save_to_disk();
    }
    if let Some(handles) = app.try_state::<TrayHandles>() {
        let _ = handles.always_on_top.set_checked(new_state);
    }
    emit_config_updated(app);
}

fn toggle_save_position(app: &AppHandle) {
    let Some(state) = app.try_state::<ConfigState>() else {
        return;
    };
    let new_state = !state.snapshot().save_window_position;
    state.with_mut(|c| c.save_window_position = new_state);
    let _ = state.save_to_disk();
    if let Some(handles) = app.try_state::<TrayHandles>() {
        let _ = handles.save_position.set_checked(new_state);
    }
    emit_config_updated(app);
}

fn toggle_autostart(app: &AppHandle) {
    let manager = app.autolaunch();
    let enabled = manager.is_enabled().unwrap_or(false);
    let new_state = if enabled {
        manager.disable().is_ok() && false
    } else {
        manager.enable().is_ok()
    };
    if let Some(handles) = app.try_state::<TrayHandles>() {
        let _ = handles.autostart.set_checked(new_state);
    }
}

fn open_data_dir(app: &AppHandle) {
    let Ok(dir) = app.path().app_data_dir() else {
        tracing::warn!("open_data_dir: app_data_dir unavailable");
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    if let Err(e) = open::that(&dir) {
        tracing::warn!(?e, path = %dir.display(), "open_data_dir failed");
    }
}

fn show_about(app: &AppHandle) {
    let version = app.package_info().version.to_string();
    let body = format!(
        "AI Agent Dashboard\nv{version}\n\nAlways-on-top widget for tracking AI coding agents."
    );
    let handle = app.clone();
    app.dialog()
        .message(body)
        .title("About")
        .buttons(MessageDialogButtons::Ok)
        .show(move |_| {
            drop(handle);
        });
}
