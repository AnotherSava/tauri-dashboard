# Telegram Notifications — Move Under Dashboard Control

## Context

Telegram notifications currently live outside this repo at `D:/projects/claude/claude/hooks/notifications/` as two Python scripts (`telegram.py`, `record-prompt.py`) registered in `~/.claude/settings.json` against Claude Code's `Notification` and `UserPromptSubmit` hooks. They keep their own state (message IDs in `%TEMP%`), own credentials (`.env`), own activity heuristic (transcript mtime), and a single hard-coded 60s debounce.

The dashboard already owns the canonical session state machine in Rust (`Status { Idle, Working, Awaiting, Done, Error }`, with `state_entered_at` per session). A second copy of "is this agent stuck?" in Python duplicates logic and drifts from what the widget shows.

Requirements for the dashboard-side implementation:

1. Config holds `bot_token`, `chat_id`, and a `state -> minimum_duration_ms` map. A message fires only after the agent has been continuously in that state for at least that duration.
2. When the agent leaves a state that had a message sent, that message is deleted from Telegram.
3. When the user dismisses a session via the header "×" (calling `remove_session` → `apply_clear`), any already-sent Telegram message for that session is deleted, and any pending-but-not-yet-fired notification for that session is cancelled. Removing a row must not leave orphaned messages in Telegram.
4. The Python implementation outside this repo is retired (hook entries removed from `~/.claude/settings.json`).
5. Design must extend cleanly to desktop notifications later, driven by a similar per-state threshold map.

Language: **Rust, in-process.**

## Approach: 1-second polling loop (no event plumbing)

The frontend already ticks once per second (`App.svelte:40`) and uses `now - session.state_entered_at` to render elapsed time in the UI (`types.ts:45`). The backend notification trigger can use the exact same math on the same 1-second cadence. That eliminates the need to plumb transition events through `apply_set` / `apply_clear` / `log_watcher` / `config_watcher`.

A single background task ticks every 1s and runs a pure reconciler per notifier:

```
tick():
  cfg = config_state.snapshot()
  sessions = app_state.snapshot()  // already existing method
  for notifier in [telegram, ... (future: desktop)]:
    reconcile(notifier, cfg, sessions, outstanding[notifier.name()], now_ms)

reconcile(notifier, cfg, sessions, outstanding, now):
  # 1) dismiss stale: outstanding message whose session vanished or left the state it was fired for
  for (session_id, o) in outstanding.iter():
    if sessions.get(session_id).map(|s| s.status != o.for_status).unwrap_or(true):
      notifier.dismiss(o.handle)  # fire-and-forget errors
      outstanding.remove(session_id)
  # 2) fire new: session in state >= threshold, no outstanding
  for s in sessions:
    if outstanding.contains(&s.id): continue
    let Some(threshold) = notifier.thresholds().get(status_key(s.status)) else continue
    if *threshold == 0: continue
    if now - s.state_entered_at >= *threshold as i64:
      match notifier.send(s).await:
        Ok(handle) => outstanding.insert(s.id.clone(), Outstanding{handle, for_status: s.status, sent_at: now}),
        Err(e) => warn!(?e),
```

**Why polling is better than transition events here:**
- Worst-case 1s latency — negligible for 60s+ thresholds.
- No `JoinHandle::abort()` cooperative-cancel worries, no `state_entered_at`-race guard inside a spawned task.
- No refactor of `apply_set` / `apply_clear` signatures, no touching `log_watcher`, `http_server`, `commands`, or `config_watcher` to fire events.
- Reconcile is a pure function of `(sessions, config, outstanding, now)` — trivially unit-testable.
- Uses identical time math to the frontend, so "5m elapsed" on the card and "threshold 5m" always agree.

**Config hot-reload handled implicitly.** Because each tick reads `config_state.snapshot()`, threshold changes apply on the next tick. No `ConfigReloaded` event needed.

**Session dismissal via "×" handled implicitly.** When the user clicks the × on a session row, the frontend calls the existing `remove_session` command (`commands.rs:44`), which invokes `state.apply_clear(&id)`. The session disappears from `app_state.snapshot()` on the next tick. The reconciler's dismissal pass treats "session not in snapshot" identically to "session transitioned to a different state": it calls `notifier.dismiss(handle)` and removes the entry from `outstanding`. Pending-but-not-yet-fired notifications need no special handling — there are no spawned tasks to cancel in the polling design; the reconciler simply skips the now-missing id on subsequent ticks. **No changes to `remove_session` or `apply_clear` are required.**

**Credential change detection.** Each tick, compare `(cfg.telegram.bot_token, cfg.telegram.chat_id)` against what's cached in the notifier. If they changed, drop the outstanding map for that notifier silently (the old bot can't delete messages with new credentials) and rebuild the `reqwest::Client`.

## Extensibility: `Notifier` trait

Both Telegram and (future) desktop notifications follow the identical lifecycle — send when threshold elapses, dismiss on state change. Abstract:

```rust
// src-tauri/src/notifications.rs

#[derive(Clone, Debug)]
pub struct Outstanding {
    pub handle: String,         // channel-specific opaque id (Telegram message_id, desktop notification tag, ...)
    pub for_status: Status,
    pub sent_at_ms: i64,
}

#[async_trait::async_trait]
pub trait Notifier: Send + Sync {
    /// Stable identifier used as the outer key in the outstanding map and in logs.
    fn channel_name(&self) -> &'static str;
    /// Whether this notifier is currently usable (credentials present, etc.).
    fn is_enabled(&self) -> bool;
    /// Per-state duration thresholds (ms). Missing key or 0 = silent for that state.
    fn thresholds(&self) -> &HashMap<String, u64>;
    async fn send(&self, session: &AgentSession) -> Result<String, anyhow::Error>;
    async fn dismiss(&self, handle: &str) -> Result<(), anyhow::Error>;
}
```

The manager holds a `Vec<Box<dyn Notifier>>`. v1 registers only the `TelegramNotifier`; a later `DesktopNotifier` plugs in the same way.

## Config Schema

### `src-tauri/src/config.rs`

Nest under a `notifications` object so future channels sit beside `telegram`:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NotificationsConfig {
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    // future: pub desktop: Option<DesktopConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    #[serde(default)]
    pub bot_token: Option<String>,
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default)]
    pub state_thresholds_ms: HashMap<String, u64>,  // keys: "idle" | "working" | "awaiting" | "done" | "error"
}

pub struct Config {
    ...existing fields...
    #[serde(default)]
    pub notifications: Option<NotificationsConfig>,
}
```

String keys (not `HashMap<Status, u64>`) because JSON object keys are strings anyway and `Status` isn't `Hash`. Unknown keys ignored. Missing state or threshold 0 ⇒ silent.

**Default in `Config::default()`:**

```rust
notifications: Some(NotificationsConfig {
    telegram: Some(TelegramConfig {
        bot_token: None,
        chat_id: None,
        state_thresholds_ms: [("awaiting".into(), 60_000), ("error".into(), 60_000)].into_iter().collect(),
    }),
}),
```

Notifications are effectively disabled until the user populates `bot_token` and `chat_id`. The threshold defaults ship sensible values so no extra JSON editing is needed beyond credentials.

## New files

### `src-tauri/src/telegram.rs` (~100 lines)

```rust
pub struct TelegramNotifier {
    client: reqwest::Client,
    credentials: Mutex<Option<TelegramCreds>>,      // (bot_token, chat_id)
    thresholds: Mutex<HashMap<String, u64>>,
}

impl TelegramNotifier {
    pub fn new() -> Self { ... }
    /// Called once per tick before reconciliation; detects credential change and signals caller.
    pub fn sync_config(&self, cfg: Option<&TelegramConfig>) -> SyncOutcome {
        // returns: Unchanged | CredsChanged | Disabled
    }
}

#[async_trait::async_trait]
impl Notifier for TelegramNotifier {
    fn channel_name(&self) -> &'static str { "telegram" }
    fn is_enabled(&self) -> bool { self.credentials.lock().unwrap().is_some() }
    fn thresholds(&self) -> &HashMap<String, u64> { ... }   // will actually return a snapshot via a RwLock
    async fn send(&self, session: &AgentSession) -> anyhow::Result<String> {
        // POST https://api.telegram.org/bot{token}/sendMessage {chat_id, text}
        // parse result.message_id -> stringify
    }
    async fn dismiss(&self, handle: &str) -> anyhow::Result<()> {
        // POST .../deleteMessage {chat_id, message_id}
        // Swallow 400 ("message to delete not found"/">48h old") and 403 ("bot blocked") at debug level.
    }
}
```

Message text (per user's minimal choice):

```
[{session_id}] {status}
{label}         // line omitted if label is empty
```

### `src-tauri/src/notifications.rs` (~200 lines)

Contains the `Notifier` trait, `Outstanding`, the reconciler, and the spawn entry point.

```rust
pub struct NotificationManager;

impl NotificationManager {
    pub fn spawn(app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            let telegram = Arc::new(TelegramNotifier::new());
            let notifiers: Vec<Arc<dyn Notifier>> = vec![telegram.clone()];
            let mut outstanding: HashMap<&'static str, HashMap<String, Outstanding>> =
                notifiers.iter().map(|n| (n.channel_name(), HashMap::new())).collect();

            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            loop {
                ticker.tick().await;
                let Some(cfg_state) = app.try_state::<ConfigState>() else { continue };
                let Some(app_state) = app.try_state::<AppState>() else { continue };
                let cfg = cfg_state.snapshot();
                let sessions = app_state.snapshot();

                // Telegram: sync config; drop outstanding on cred change
                let outcome = telegram.sync_config(cfg.notifications.as_ref().and_then(|n| n.telegram.as_ref()));
                if matches!(outcome, SyncOutcome::CredsChanged | SyncOutcome::Disabled) {
                    outstanding.get_mut("telegram").unwrap().clear();
                }

                for n in &notifiers {
                    if !n.is_enabled() { continue; }
                    reconcile(n.as_ref(), &sessions, outstanding.get_mut(n.channel_name()).unwrap(), now_ms()).await;
                }
            }
        });
    }
}

// Pure (except for the async calls inside):
async fn reconcile(
    notifier: &dyn Notifier,
    sessions: &[AgentSession],
    outstanding: &mut HashMap<String, Outstanding>,
    now_ms: i64,
) {
    // 1) dismissals
    let to_dismiss: Vec<(String, Outstanding)> = outstanding.iter()
        .filter(|(id, o)| sessions.iter().find(|s| &s.id == *id).map_or(true, |s| s.status != o.for_status))
        .map(|(k,v)| (k.clone(), v.clone()))
        .collect();
    for (id, o) in to_dismiss {
        let _ = notifier.dismiss(&o.handle).await.inspect_err(|e| tracing::debug!(channel=notifier.channel_name(), ?e, "dismiss failed"));
        outstanding.remove(&id);
    }
    // 2) sends
    for s in sessions {
        if outstanding.contains_key(&s.id) { continue; }
        let key = status_key(s.status);
        let Some(threshold) = notifier.thresholds().get(key).copied().filter(|t| *t > 0) else { continue };
        if (now_ms - s.state_entered_at) < threshold as i64 { continue; }
        match notifier.send(s).await {
            Ok(handle) => { outstanding.insert(s.id.clone(), Outstanding { handle, for_status: s.status, sent_at_ms: now_ms }); }
            Err(e) => tracing::warn!(channel=notifier.channel_name(), id=%s.id, ?e, "send failed"),
        }
    }
}

fn status_key(s: Status) -> &'static str { /* "idle" | "working" | ... */ }
```

## Changes to existing files

### `src-tauri/src/config.rs`

Add `TelegramConfig`, `NotificationsConfig`, and `notifications: Option<NotificationsConfig>` to `Config`. Populate the default as described above.

### `src-tauri/src/lib.rs`

After `config_watcher::spawn(...)` at line 79 (inside the setup closure), add:

```rust
notifications::NotificationManager::spawn(app.handle().clone());
```

Register a new Tauri command in the `invoke_handler!` at line 24:

```rust
commands::test_telegram_notification,
```

Add `mod notifications;` and `mod telegram;` at the top.

### `src-tauri/src/commands.rs`

Add one command:

```rust
#[tauri::command]
pub async fn test_telegram_notification(app: tauri::AppHandle) -> Result<(), String> {
    // Build an ephemeral TelegramNotifier from current config; call send() with a synthetic session;
    // schedule a dismiss 5s later. Return Err on any failure so the frontend/CLI can show it.
}
```

### `src-tauri/Cargo.toml`

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
async-trait = "0.1"
anyhow = "1"
```

`rustls-tls` avoids native-tls / OpenSSL build pain on Windows. `api.telegram.org` uses a standard CA chain that rustls' webpki roots accept.

### Files NOT modified

- `src-tauri/src/state.rs` — untouched.
- `src-tauri/src/log_watcher.rs` — untouched.
- `src-tauri/src/http_server.rs` — untouched.
- `src-tauri/src/config_watcher.rs` — untouched; the 1s ticker picks up config changes via the already-hot-reloaded `ConfigState`.
- Frontend — untouched. No new UI; users edit `config.json` directly.

## Extending to Desktop Notifications (future, not this PR)

The Notifier trait is designed so desktop notifications slot in without further refactoring:

1. Add `desktop: Option<DesktopConfig>` to `NotificationsConfig` with its own `state_thresholds_ms`.
2. Add `tauri-plugin-notification` to `Cargo.toml` and register it in `lib.rs`.
3. Create `src-tauri/src/desktop_notifier.rs` with `impl Notifier for DesktopNotifier`. Send uses the plugin's notification builder; dismiss uses `.remove()` with a stored tag/id.
4. Register it in the `notifiers` vec in `NotificationManager::spawn`. The reconciler runs unchanged.

**Dismissal caveat.** Windows Action Center notifications can be removed programmatically via `ToastNotificationManager` (which the plugin wraps on Windows) using the notification tag/group. macOS is more restrictive and "dismiss" may no-op — acceptable; the notification center already dedupes. Document this in the future PR.

## Retiring the Python Implementation

Out-of-repo change — no file edits in this repo. After the Rust side ships and is verified:

1. Edit `~/.claude/settings.json`: remove the `Notification` and `UserPromptSubmit` hook entries that invoke `D:/projects/claude/claude/hooks/notifications/telegram.py` and `record-prompt.py`.
2. Leave the scripts on disk for reference; the user can delete them manually later.

## Testing

**Unit tests** (cargo test, no network):

1. `reconcile()` — construct a mock `Notifier` that records calls into a `Mutex<Vec<Event>>`. Cover:
   - Session in state with threshold elapsed + no outstanding ⇒ `send` called once; outstanding populated.
   - Session in state but threshold not yet reached ⇒ no send.
   - Session with outstanding, state unchanged ⇒ nothing happens.
   - Session transitioned to different state ⇒ `dismiss` called with stored handle; outstanding removed.
   - Session vanished from snapshot (covers both the `apply_clear` path from the "×" button and `SessionEnd`-driven clears) ⇒ `dismiss` called; outstanding removed.
   - Session with no outstanding is vanished mid-threshold (e.g., user clicks × while session is 30s into a 60s threshold) ⇒ no send, no dismiss, next tick simply skips.
   - Threshold 0 or missing ⇒ state is silent, no send regardless of elapsed time.
2. `TelegramNotifier::sync_config` — returns `Unchanged` / `CredsChanged` / `Disabled` correctly on various cfg transitions.
3. `build_message_text` — `"[{id}] {status}"` alone, `"[{id}] {status}\n{label}"` with non-empty label.
4. `status_key` — exhaustive mapping.

**Manual smoke test:**

1. Create a throwaway Telegram bot via BotFather; get `chat_id` via `https://api.telegram.org/bot<token>/getUpdates` after sending `/start` in your own chat.
2. Populate `%APPDATA%\com.anothersava.ai-agent-dashboard\config.json` under `notifications.telegram.bot_token` and `.chat_id`.
3. With the widget running, trigger a fake awaiting:
   ```
   curl -X POST http://127.0.0.1:9077/api/status -H "Content-Type: application/json" \
        -d '{"action":"set","id":"smoke","status":"awaiting","label":"test message"}'
   ```
4. After ~60s observe a Telegram message `[smoke] awaiting\ntest message`.
5. Transition:
   ```
   curl -X POST http://127.0.0.1:9077/api/status -H "Content-Type: application/json" \
        -d '{"action":"set","id":"smoke","status":"working","label":"test"}'
   ```
6. Within ~1s the prior message is deleted.
7. **Dismissal via × smoke test.** Re-trigger the 60s awaiting, wait for the message to appear in Telegram, then click the × on the `smoke` row in the widget header. Within ~1s the Telegram message should be deleted.
8. **Dismissal before threshold smoke test.** Trigger a fresh awaiting, dismiss via × after 10s (well before the 60s threshold). Confirm no Telegram message ever fires.
9. From devtools call `await __TAURI__.core.invoke('test_telegram_notification')` and confirm a test message appears and self-deletes 5s later.

## Risks & Notes

- **1s reconcile cadence** means up to 1s latency on send/delete. Acceptable for a human-visible notification at 60s+ thresholds.
- **Reconcile runs one notifier at a time per tick** (awaits between them). If a Telegram request stalls 10s, the desktop reconciler waits. For v1 (Telegram only) this is fine; if/when desktop ships, parallelize with `join_all` or give each notifier its own ticker task. Easy to refactor when needed.
- **Widget restart.** Outstanding map is in memory only; any pre-restart pending Telegram message stays until the user deletes it or Telegram's 48h delete window expires. Acceptable.
- **Credential change mid-run.** Dropping the outstanding map is the only safe choice — the new bot can't delete the old bot's messages. Logged at warn.
- **429 rate limits.** Not handled in v1. Personal dashboard with one chat shouldn't hit them.

## Critical Files

New:
- `src-tauri/src/notifications.rs` — `Notifier` trait, reconciler, `NotificationManager::spawn`.
- `src-tauri/src/telegram.rs` — `TelegramNotifier` (impl `Notifier`).

Modified:
- `src-tauri/Cargo.toml` — add `reqwest`, `async-trait`, `anyhow`.
- `src-tauri/src/config.rs` — add `TelegramConfig`, `NotificationsConfig`, `notifications` field + defaults.
- `src-tauri/src/lib.rs` — register modules; spawn `NotificationManager`; register `test_telegram_notification` command.
- `src-tauri/src/commands.rs` — add `test_telegram_notification` command.

Untouched: `state.rs`, `log_watcher.rs`, `http_server.rs`, `config_watcher.rs`, all frontend code.

## Verification

1. `cargo test` from `src-tauri/` — existing tests still pass; new reconciler unit tests pass.
2. `cargo build` from `src-tauri/` — no reqwest build errors on Windows with rustls.
3. Launch the widget (`npm run tauri dev`). Watch `%APPDATA%\com.anothersava.ai-agent-dashboard\widget.log` for the manager's startup log and per-tick debug lines.
4. Run the full manual smoke test sequence above.
5. Inspect `sessions_updated` event payload in devtools — confirm no new fields on `AgentSession`.
