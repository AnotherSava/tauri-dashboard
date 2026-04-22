# Move decision logic from Python hook into the Rust app

## Status

**Draft.** Not started. This refactor lands **before** the sticky-label policy work and deliberately creates an empty seat for it. Wire/module shape discussed and signed off 2026-04-21; three review notes folded in below.

## Context

The Python hook at `integrations/claude_hook.py` currently does about two-thirds of the pipeline's brain work:

| Hook responsibility today                             | Natural home |
|---                                                    |---           |
| Derive `chat_id` from `cwd` / `projects_root`         | App (already owns `ConfigState`) |
| Read `config.json` for `benign_closers`, `server_port`| App |
| Tail transcript to detect trailing `?` (`last_assistant_ends_with_question`) | App (`WatcherRegistry` already tails the same file) |
| Map argv + payload to `(status, label)` (`classify`)  | App (should live with `apply_set`) |
| Clean prompts (strip box-drawing glyphs)              | App |
| POST JSON to the widget                               | Hook — this is its only genuine job |

The hook can't give up:

- Being invoked by Claude Code (external contract)
- Reading the lifecycle-event payload from stdin
- Knowing how to locate the widget's HTTP server

See `docs/plans/draft/2026-04-21_21-00_field-flow-reference.md` for the current field-level flow.

## Goals

1. **Single source of truth for state-machine decisions.** All classification, ID derivation, prompt cleaning, and question-detection live in one Rust module.
2. **Hook shrinks to ~20 lines.** Read argv, read stdin, POST. No classification, no transcript reading, no config reading beyond locating the server.
3. **One HTTP endpoint, adapter-dispatched per client.** No speculative "generic" endpoints preserved for hypothetical future clients; instead, the endpoint shape itself is forward-compatible.
4. **Leave a seam for the sticky-label policy refactor.** Land `label_policy.rs` as a trivial one-function module whose body is today's inline logic. The upcoming sticky-label work then edits that module and only that module.
5. **Tests migrate with the logic.** `tests/test_claude_hook.py` becomes Rust unit tests on the adapter + label-policy modules.

Non-goals: adding new classification rules, changing the state machine's observable behavior, supporting new hook events (`PreToolUse`, `PostToolUse`) — those are follow-ups that become cheap after this refactor.

## Design

### Wire contract — one endpoint, client-discriminated

Drop `/api/status`. Add one endpoint:

```
POST /api/event
Content-Type: application/json

{
  "client": "claude",
  "event": "UserPromptSubmit",
  "payload": {
    "session_id": "...",
    "cwd": "...",
    "transcript_path": "...",
    "prompt": "...",
    "notification_type": "...",
    "message": "..."
  }
}
```

Server flow:

```
http_server::post_event
  └─ adapters::dispatch(client, event, payload) -> Option<SetInput | Clear>
      ├─ adapters::claude  (today — the only one)
      └─ adapters::<future>  (added as new clients arrive)
  └─ state.apply_set / state.apply_clear
```

Each adapter is a pure function — no state access, no HTTP, no I/O beyond parsing the payload. Adapters *may* request the server to attach the session's `transcript_path` to the `WatcherRegistry`; that side effect lives in `post_event` after the adapter returns, not inside the adapter.

**Forward-compatible by construction:** a new client is a new adapter module + a match arm in `dispatch`. The HTTP route never grows.

### Module layout

```
src-tauri/src/
  http_server.rs       — routes (only /api/event now), decode, dispatch, side-effects
  adapters/
    mod.rs             — dispatch(client, event, payload) -> AdapterOutput
    claude.rs          — Claude-Code-specific mapping (classify + derive_chat_id + ...)
  state.rs             — apply_set delegates to label_policy
  label_policy.rs      — NEW. Trivial today. The sticky-label seam.
```

`label_policy.rs` signature (today):

```rust
pub fn select(
    prev: Option<&AgentSession>,
    input: &SetInput,
    task_boundary: bool,
) -> (String /* label */, Option<String> /* original_prompt */);
```

Implementation is exactly the current inline logic from `state.rs:55-91`, extracted verbatim. Zero behavior change.

### Hook shape (target ~20 LOC)

```python
#!/usr/bin/env python3
import json, os, sys, urllib.request

DEFAULT_URL = "http://127.0.0.1:9077"
EVENT_MAP = {
    "working": "UserPromptSubmit",
    "done":    "Stop",
    "idle":    "Notification",
    "clear":   "SessionEnd",
}

def main() -> None:
    if len(sys.argv) < 2: return
    try: sys.stdin.reconfigure(encoding="utf-8", errors="replace")
    except Exception: pass
    try: payload = json.load(sys.stdin)
    except Exception: payload = {}
    url = os.environ.get("TAURI_DASHBOARD_URL", DEFAULT_URL).rstrip("/") + "/api/event"
    body = {"client": "claude", "event": EVENT_MAP.get(sys.argv[1], sys.argv[1]), "payload": payload}
    try:
        urllib.request.urlopen(
            urllib.request.Request(url, data=json.dumps(body).encode(),
                                   headers={"Content-Type": "application/json"},
                                   method="POST"),
            timeout=2)
    except Exception:
        pass
```

Key points:

- **Server URL resolution:** `$TAURI_DASHBOARD_URL` env var wins; default `http://127.0.0.1:9077`. No `config.json` read, no port lookup chain. If the user changes the port, they update the env var (widget can `setx` it on first run — see Open questions).
- **No classification, no transcript reading, no prompt cleaning.** The hook forwards Claude's payload verbatim; the server does everything else.
- **argv → event-name** is the one tiny translation the hook performs, so the wire is symmetrical with Claude Code's hook names rather than our `working`/`done`/`idle` slang.

### Adapter contract (Rust side)

```rust
// adapters/mod.rs
pub enum AdapterOutput {
    Set { input: SetInput, transcript_path: Option<PathBuf> },
    Clear { id: String },
    Ignore,
}

pub fn dispatch(client: &str, event: &str, payload: Value, cfg: &Config) -> AdapterOutput {
    match client {
        "claude" => claude::dispatch(event, payload, cfg),
        _ => AdapterOutput::Ignore,
    }
}
```

Adapter takes `cfg: &Config` so it can read `projects_root`, `benign_closers` without touching global state. Pure function — testable with literal JSON fixtures.

## Migration order

One commit per step, each individually shippable:

1. **Add `label_policy.rs`** with a single `select(...)` function. Call it from `apply_set` in `state.rs`. Zero behavior change. Move existing state-machine tests that target label behavior into `label_policy` tests. **Seam created.**
2. **Add `adapters/mod.rs` + `adapters/claude.rs`** with the classify / derive_chat_id / `last_assistant_ends_with_question` / `_clean_prompt` logic translated from Python. Add Rust unit tests covering the same cases as `tests/test_claude_hook.py`.
3. **Add `POST /api/event`** in `http_server.rs` calling `adapters::dispatch`. Keep `/api/status` temporarily.
4. **Rewrite `claude_hook.py`** to the ~20-line shape. Release note: "Hook updated — reinstall from repo. Your `config.json` no longer drives the port; set `$TAURI_DASHBOARD_URL` or rely on the default."
5. **Delete `/api/status`** after a few days of the new hook working. Clean up `SetPayload`, `ClearPayload`, and the Config-action branch in `post_status` if those aren't reachable by anything else.
6. **Delete `tests/test_claude_hook.py`** (or slim to covering just the wire-level POST). The logic is now in Rust tests.

Steps 1–2 can proceed in parallel if we're careful about test isolation. Step 4 is the only user-visible step.

## Open questions

1. **Env-var bootstrap.** Should the widget call `setx TAURI_DASHBOARD_URL=http://127.0.0.1:<port>` on first run so subsequent shells inherit it? Upside: hook works out of the box. Downside: mutating the user's persistent environment is heavier than it sounds; requires elevation on some systems and a shell restart to take effect. **Tentative:** don't `setx`; document the env var in `docs/pages/claude-code.md` setup section and let the user set it once if they prefer env over default.
2. **Transcript question-detection.** Today the hook reads the transcript synchronously per hook invocation. The app already tails the same file via `WatcherRegistry`. Options:
   - (a) Adapter opens and reads the transcript itself (mirror of current Python behavior; cheap for one call).
   - (b) `WatcherRegistry` caches the most recent assistant text per session so adapter can query without file I/O.
   - (b) is nicer but more state. **Tentative:** (a) for this refactor, (b) as a follow-up if perf matters.
3. **Which Claude event names to standardize on.** Pass through Claude Code's own names (`UserPromptSubmit`, `Stop`, `SessionStart`, `SessionEnd`, `Notification`) exactly? Or our own (`working`, `done`, `idle`, `clear`)? **Tentative:** pass through Claude's names. Adapter maps them to state transitions. The hook's argv → event mapping is then just a name lookup.
4. **Prompt logging.** Now unblocked — the adapter is the natural place for `tracing::debug!(chat_id, label = ?label, "classified")`. Do it in step 2.

## Files to touch

- `src-tauri/src/state.rs` — extract sticky-label logic; `apply_set` calls `label_policy::select`
- `src-tauri/src/label_policy.rs` — **new**, trivial, seam
- `src-tauri/src/adapters/mod.rs` — **new**
- `src-tauri/src/adapters/claude.rs` — **new**, ports the Python `classify` / `derive_chat_id` / `_clean_prompt` / `last_assistant_ends_with_question`
- `src-tauri/src/http_server.rs` — add `/api/event`, remove `/api/status` in step 5
- `src-tauri/src/lib.rs` — module declarations
- `integrations/claude_hook.py` — shrink to ~20 lines
- `tests/test_claude_hook.py` — remove (logic-side) or trim to wire-level only
- `docs/pages/claude-code.md` — document `$TAURI_DASHBOARD_URL`
- `docs/pages/http-api.md` — rewrite to describe `/api/event` + adapter discrimination

## Effort estimate

Half a day for the refactor itself, another half for porting the Python tests. Low risk — every step has a clean rollback and the hook/server interface has `#[serde(default)]`-tolerance for skew in either direction.
