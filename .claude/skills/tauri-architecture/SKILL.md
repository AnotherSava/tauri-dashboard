---
name: understanding-tauri-architecture
description: "Build and debug Tauri desktop applications using the Rust backend, webview frontend, IPC commands/events, and capability-based security model. Use when scaffolding a Tauri project, defining Rust commands, configuring tauri.conf.json, setting up capabilities and permissions, or debugging cross-platform desktop app issues."
---

# Tauri Architecture

## Workflow: Building a Tauri App

1. **Set up project** — scaffold with `npm create tauri-app@latest`, configure `tauri.conf.json`
2. **Define commands** — write `#[tauri::command]` functions in Rust, register with `generate_handler!`
3. **Configure capabilities** — create capability files in `src-tauri/capabilities/` granting permissions per window
4. **Connect frontend** — call commands via `invoke()` from `@tauri-apps/api/core`
5. **Test IPC** — verify commands return expected data, events propagate, permissions enforce correctly

> **Checkpoint:** After step 3, run `cargo build` in `src-tauri/` to catch permission and configuration errors at compile time.

## Core-Shell Architecture

Tauri splits into two layers communicating via IPC:

- **Core (Rust backend):** System access, file operations, native features, plugin management, security enforcement. Never exposes direct system access to the frontend.
- **Shell (Frontend):** HTML/CSS/JS rendered in the platform's native webview. Framework-agnostic (React, Vue, Svelte, Solid, vanilla). Sandboxed — all system calls go through `@tauri-apps/api`.

### Platform WebView Engines

Tauri uses the OS-native webview rather than bundling a browser engine, which keeps binaries small (~600KB vs ~150MB for Electron) and lets the OS vendor handle security patches.

| Platform | WebView Engine |
|----------|----------------|
| Windows  | WebView2 (Edge/Chromium) |
| macOS    | WKWebView (Safari/WebKit) |
| Linux    | WebKitGTK |
| iOS      | WKWebView |
| Android  | Android WebView |

Test on all target platforms — rendering and feature availability differ slightly between webview versions.

## Key Ecosystem Crates

- **tauri** — central orchestrator; reads `tauri.conf.json` at compile time, manages webview injection, hosts system API
- **tauri-runtime** — abstracts platform-specific webview interactions
- **tauri-macros / tauri-codegen** — compile-time code generation for commands, context, asset embedding
- **TAO** — cross-platform window creation (windows, menus, system trays)
- **WRY** — cross-platform webview rendering, JS evaluation, event bridging

## IPC: Commands and Events

Tauri uses asynchronous message passing over JSON-RPC. The Core validates and permission-checks every request before execution.

### Commands (Request-Response)

```rust
// Rust backend — define a command
#[tauri::command]
fn greet(name: String) -> String {
    format!("Hello, {}!", name)
}

// Register in builder
tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![greet])
```

```javascript
// JavaScript frontend — invoke the command
import { invoke } from '@tauri-apps/api/core';
const greeting = await invoke('greet', { name: 'World' });
```

- All arguments must be JSON-serializable
- Returns a Promise; supports async Rust functions
- JS uses camelCase args, Rust receives snake_case

### Events (Fire-and-Forget)

```javascript
// Frontend: emit and listen
import { emit, listen } from '@tauri-apps/api/event';
emit('user-action', { action: 'clicked' });
const unlisten = await listen('download-progress', (event) => {
    console.log(event.payload);
});
```

```rust
// Backend: listen and emit
app.listen("user-action", |event| {
    println!("User action: {}", event.payload());
});
app.emit("download-progress", 50)?;
```

- Bidirectional — both frontend and backend can emit/listen
- No return value; best for lifecycle events and state changes

> **Checkpoint:** Test IPC by invoking a simple command from the frontend console and confirming the Rust handler logs execution.

## Security Model

Tauri enforces a deny-by-default security posture with multiple layers:

1. **WebView sandboxing** — frontend code runs inside the webview sandbox
2. **IPC validation** — all messages crossing the trust boundary are validated
3. **Capabilities** — JSON/TOML files in `src-tauri/capabilities/` defining which permissions each window gets
4. **Permissions** — fine-grained control over allowed operations per plugin
5. **Scopes** — restrict command behavior (e.g., limit file access to specific directories)
6. **CSP** — Content Security Policy restricts frontend network and script behavior

### Capability Configuration

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main-window-capability",
  "description": "Permissions for the main application window",
  "windows": ["main"],
  "permissions": [
    "core:path:default",
    "core:window:allow-set-title",
    "fs:read-files",
    "fs:scope-app-data"
  ]
}
```

Key rules:
- Each window has its own capability set — window isolation by default
- Only bundled code can access Tauri APIs (no remote access by default)
- Permission errors surface at compile time

> **Checkpoint:** After adding a new capability, rebuild and verify the window can only access explicitly permitted APIs.

## Rust Backend Structure

```
src-tauri/
+-- Cargo.toml              # Rust dependencies
+-- tauri.conf.json         # Tauri configuration
+-- capabilities/           # Permission definitions
|   +-- main.json
+-- src/
    +-- main.rs             # Entry point (desktop)
    +-- lib.rs              # Core app logic
    +-- commands/           # Command modules
    |   +-- mod.rs
    |   +-- file_ops.rs
    +-- state.rs            # App state management
```

### Entry Point Pattern

```rust
// src-tauri/src/lib.rs
mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::read_file,
        ])
        .manage(AppState::default())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### Command Patterns

```rust
// Basic command
#[tauri::command]
fn simple_command() -> String {
    "Hello".into()
}

// With arguments (camelCase from JS, snake_case in Rust)
#[tauri::command]
fn with_args(user_name: String, age: u32) -> String {
    format!("{} is {} years old", user_name, age)
}

// With error handling
#[tauri::command]
fn fallible() -> Result<String, String> {
    Ok("Success".into())
}

// Async command
#[tauri::command]
async fn async_command() -> Result<String, String> {
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok("Done".into())
}

// Accessing app state
#[tauri::command]
fn with_state(state: tauri::State<'_, AppState>) -> String {
    state.get_value()
}

// Accessing window
#[tauri::command]
fn with_window(window: tauri::WebviewWindow) -> String {
    window.label().to_string()
}
```

> **Checkpoint:** After adding new commands, verify they appear in `generate_handler![]` and that matching capability permissions are granted.

## No Runtime Bundled

Tauri does not ship a runtime. Rust compiles to native machine code, frontend assets are embedded in the binary, and the OS webview handles rendering. Result: small, fast executables that are harder to reverse-engineer than Electron apps with bundled JavaScript.
