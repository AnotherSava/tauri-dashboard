---
name: dashboard-header-claude-code-limits
description: Add 5h + 7d Claude Code rate-limit pills to the widget header, powered by Anthropic's OAuth usage endpoint
---

# Dashboard header: Claude Code 5h + 7d limit pills

## Context

The widget header currently shows `AI AGENTS` on the left and a hide-to-tray `×` on the right (`src/App.svelte:54-57`). Claude Code enforces two rolling rate-limit windows — a **5-hour session bucket** and a **7-day weekly bucket** — and when you get near a cap it's nice to know without leaving the widget. The reference project `github.com/CodeZeno/Claude-Code-Usage-Monitor` does exactly this for a standalone tray app; we want the same two numbers rendered in our widget.

**Visual target:** inspired by the user-supplied sample (stacked segmented rows), tuned per review:
- Two stacked rows, one per window (5h on top, 7d below).
- No `5h`/`7d` label — the timer format disambiguates (a value with a leading `D:` digit is the 7d bucket).
- Overlay text *inside* each bar: `NN%` aligned left, `HH:MM` (5h row) or `D:HH:MM` (7d row) aligned right.
- Filled-segment color shifts by utilization threshold (green → amber → red, reusing the widget's existing session-state palette).

**Outcome:** a "limits strip" sits directly below the existing title/× header and above the session list. Values refresh every 60s in the background; the HH:MM countdown ticks live off the existing 1s clock.

## Data source

Anthropic exposes the exact numbers we want via an OAuth-gated endpoint. The upstream monitor confirms this works for Claude Max/Pro subscribers:

- **Endpoint:** `GET https://api.anthropic.com/api/oauth/usage`
- **Headers:** `Authorization: Bearer <accessToken>` + `anthropic-beta: oauth-2025-04-20`
- **Response shape:**
  ```json
  {
    "five_hour":  { "utilization": 0.0..1.0, "resets_at": "2026-04-20T22:00:00.000+00:00" },
    "seven_day":  { "utilization": 0.0..1.0, "resets_at": "..." }
  }
  ```
- **Token source:** `%USERPROFILE%\.claude\.credentials.json` → `claudeAiOauth.accessToken` + `claudeAiOauth.expiresAt` (ms epoch)

No rolling-window math, no pricing table, no JSONL parsing — Anthropic returns utilization pre-computed.

**v1 scope choices:**
- **Skip the `/v1/messages` ratelimit-header fallback.** The primary endpoint is reliable in the upstream's experience; a ping costs a request and adds code.
- **Skip auto token refresh (`claude -p .`)** from the upstream. If the token is expired we show a muted "—" state; the user refreshes by using Claude Code normally. Keeps scope tight and avoids invasive behaviour.

## Design

### Rust backend

**New module** `src-tauri/src/usage_limits.rs`:

```rust
pub struct LimitBucket { pub utilization: f32, pub resets_at_ms: i64 }
pub enum UsageStatus { Ok, Unavailable, AuthExpired, NetworkError }
pub struct UsageLimits {
    pub five_hour: Option<LimitBucket>,
    pub seven_day: Option<LimitBucket>,
    pub status: UsageStatus,
    pub updated_ms: i64,
}
pub struct UsageLimitsState(RwLock<UsageLimits>);
pub struct UsageLimitsPoller;   // spawn(app) -> 60s tokio interval loop
```

- **Poll cadence:** 60s. Values change slowly; 1s is overkill and costs requests.
- **Credential read:** `resolve_credentials_path()` returns `<user_home>/.claude/.credentials.json`. Use `std::env::var("USERPROFILE")` on Windows, `HOME` elsewhere — avoid pulling `dirs` crate for a one-liner.
- **ISO-8601 parsing:** add `chrono = { version = "0.4", default-features = false, features = ["serde", "clock"] }` to `Cargo.toml`. Deserialize `resets_at` as `DateTime<Utc>` and store the unix-ms. Cleaner than hand-rolling.
- **HTTP:** reuse the existing `reqwest` dep (already configured for rustls in `Cargo.toml:27`).
- **On each tick:** read credentials → check `expiresAt` → GET endpoint → update state → emit `usage_limits_updated`. Map failures onto `UsageStatus` variants and keep previous bucket values stale-but-visible when a single poll blips.
- **Event emit helper** `emit_usage_limits_updated(&AppHandle)` — mirrors `emit_sessions_updated` in `src-tauri/src/commands.rs:63`.

**New Tauri command** in `src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub fn get_usage_limits(state: State<UsageLimitsState>) -> UsageLimits { state.snapshot() }
```

**Wiring** in `src-tauri/src/lib.rs`:
1. `mod usage_limits;` after other mods (line 1-10).
2. `.manage(UsageLimitsState::new())` alongside `AppState::new()` (line 24).
3. Add `commands::get_usage_limits` to `generate_handler!` (line 26-35).
4. In `.setup(...)` after `notifications::NotificationManager::spawn(...)` on line 83: `usage_limits::UsageLimitsPoller::spawn(app.handle().clone());`.

### Frontend

**Types** in `src/lib/types.ts`:
```ts
export type UsageStatus = 'ok' | 'unavailable' | 'auth_expired' | 'network_error'
export type LimitBucket = { utilization: number; resets_at: number }   // epoch ms
export type UsageLimits = {
    five_hour: LimitBucket | null
    seven_day: LimitBucket | null
    status: UsageStatus
    updated: number
}
```

Add a formatter `formatCompactRemaining(ms: number, mode: 'hm' | 'dhm'): string` alongside the existing `formatTime` / `formatTokens` helpers in `types.ts`:
- `'hm'` → zero-padded `HH:MM` (e.g. `04:32`, `00:47`). Used by the 5h row; never needs days since the window cap is <5h.
- `'dhm'` → `D:HH:MM` (e.g. `6:23:45`). Used by the 7d row.
- `null` / negative → `--:--` / `-:--:--`.

**API** in `src/lib/api.ts`: add `getUsageLimits()` and `onUsageLimitsUpdated()` following the `getSessions` / `onSessionsUpdated` pattern at lines 5-7 and 33-37.

**New component** `src/lib/components/LimitBar.svelte`:
- Props: `bucket: LimitBucket | null`, `status: UsageStatus`, `now: number`, `format: 'hm' | 'dhm'`
- Layout — single row, `position: relative`:
  - A flex track of 20 segments fills the row (the bar).
  - Two absolutely-positioned overlays on top of the bar:
    - Left overlay: `NN%` — `left: 6px`
    - Right overlay: `HH:MM` or `D:HH:MM` — `right: 6px`
  - Overlay text: 10px monospace, 700 weight, color `#f0f0f2`, `text-shadow: 0 0 2px rgba(0,0,0,0.8)` so it reads cleanly over both filled and empty segments.
- **Segment count: 20** — clean math (each segment = 5%), more visual granularity than the sample without looking busy.
- **Filled count:** `round(utilization * 20)` clamped to `[0, 20]`; if `utilization > 0` and filled would be `0`, bump to `1` so a small non-zero value still shows.
- **Fill color by utilization threshold** — reuses the widget's existing state palette (`SessionItem.svelte:104-123`):
  - `< 0.50` → `#047857` (done green)
  - `0.50 ≤ u < 0.85` → `#b45309` (awaiting amber)
  - `≥ 0.85` → `#b91c1c` (error red)
- **Empty segment:** `#2a2a2d` (existing border tone).
- **Segment visuals:** flex-grow to fill row width, height 14px, 2px gap, `border-radius: 1px`. Row total height ~16px.
- **Non-ok states:** every segment empty; overlays show `--%` and `--:--` / `-:--:--`. Tooltip (via `title` attr) carries the reason:
  - `unavailable` → "Sign in via Claude Code to enable"
  - `auth_expired` → "Token expired — run Claude Code to refresh"
  - `network_error` → "Anthropic API unreachable — last try Ns ago"
- **Ok-state tooltip:** `"Resets Apr 20, 22:00 UTC · updated Ns ago"` derived from `bucket.resets_at` + `usage.updated`.

**New component** `src/lib/components/LimitsStrip.svelte`:
- Props: `usage: UsageLimits | null`, `now: number`
- Two stacked `LimitBar` rows — top = 5h with `format='hm'`, bottom = 7d with `format='dhm'`.
- Padding `4px 12px 6px`, 4px row-gap. Background `#17171a` (same as header) with `border-bottom: 1px solid #2a2a2d`.
- Not a drag region — hover needs to land on the bar for the tooltip.

**App changes** in `src/App.svelte`:
- Add `let usage = $state<UsageLimits | null>(null)` next to `sessions` / `config` (lines 14-16).
- In `onMount`, `usage = await getUsageLimits()` and `unlistenUsage = await onUsageLimitsUpdated((u) => (usage = u))` alongside lines 22-27.
- Unlisten on teardown (lines 42-45).
- Markup: insert `<LimitsStrip {usage} {now} />` between the existing `<header>…</header>` and the `{#if config}<SessionList …/>{/if}` block. The title/× header stays untouched — the strip sits immediately below it.
- The existing 1s `now` ticker (line 40) already drives live countdown refresh — no new interval needed.

### Graceful failure

| Condition | Rust `status` | Strip rendered |
|---|---|---|
| `.credentials.json` missing or unreadable | `Unavailable` | empty bars, `--%` + `--:--` / `-:--:--` overlays, tooltip "Sign in via Claude Code" |
| `expiresAt` < now, or API returns 401/403 | `AuthExpired` | same, tooltip "Token expired — run Claude Code to refresh" |
| Network timeout / 5xx / JSON shape mismatch | `NetworkError` | keep last-good filled segments if within ~5 min, else empty, tooltip "Anthropic API unreachable" |
| All good | `Ok` | green/amber/red-filled bars per threshold, `NN%` + `HH:MM` / `D:HH:MM` overlays, reset-time tooltip |

### Files to create / modify

- **Create:** `src-tauri/src/usage_limits.rs`
- **Create:** `src/lib/components/LimitBar.svelte`
- **Create:** `src/lib/components/LimitsStrip.svelte`
- **Modify:** `src-tauri/Cargo.toml` — add `chrono`
- **Modify:** `src-tauri/src/lib.rs` — register module, state, command, spawn poller
- **Modify:** `src-tauri/src/commands.rs` — add `get_usage_limits`, `emit_usage_limits_updated`
- **Modify:** `src/lib/types.ts` — add `UsageLimits`, `LimitBucket`, `UsageStatus`, `formatCompactRemaining`
- **Modify:** `src/lib/api.ts` — add `getUsageLimits` + `onUsageLimitsUpdated`
- **Modify:** `src/App.svelte` — state, subscription, insert `<LimitsStrip />` below header

### Tests

- `usage_limits.rs` `#[cfg(test)]`:
  - Parses credentials JSON (happy + missing `claudeAiOauth` + missing file).
  - Deserializes the API response including ISO-8601 `resets_at`.
  - Classifies expired token based on `expiresAt`.
- Frontend: no automated tests in this codebase — smoke-test manually.

## Verification

1. Start the widget in dev (`npm run tauri dev` — confirm exact script from `package.json`).
2. Widget opens; chrome now shows: thin title/× header on top, then a two-row limits strip (20-segment bars with `NN%` / `HH:MM` or `D:HH:MM` text overlays).
3. Hover each bar — tooltip shows the exact reset timestamp and "updated Ns ago".
4. The HH:MM / D:HH:MM countdown ticks down once per second; percentage stays stable between the 60s polls.
5. Values refresh within 60s of each poll landing against Anthropic (tail `widget.log` for the poll entries).
6. Rename `~/.claude/.credentials.json` temporarily → within 60s the bars empty out and overlays show `--%` / `--:--` / `-:--:--` with the "sign in" tooltip; rename back → values return on the next poll.
7. Verify threshold colors by temporarily overriding `utilization` values at each boundary (unit test covers the ramp).
8. `cd src-tauri && cargo test` — new unit tests pass.
9. `cd src-tauri && cargo clippy --all-targets -- -D warnings` stays clean.

## Out of scope for v1 (mentioned if we want follow-ups)

- `/v1/messages` ratelimit-header fallback (upstream uses it when primary endpoint is flaky).
- Spawning `claude -p .` to auto-refresh expired tokens.
- WSL multi-distro credential hunt.
- Config toggle to hide the pills (`show_usage_limits: bool`).
- Tray menu badge mirroring the 7d bucket.
