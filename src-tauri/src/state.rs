use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Idle,
    Working,
    Awaiting,
    Done,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: String,
    pub status: Status,
    pub label: String,
    pub original_prompt: Option<String>,
    pub source: String,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub updated: i64,
    pub state_entered_at: i64,
    pub working_accumulated_ms: u64,
}

#[derive(Clone, Debug)]
pub struct SetInput {
    pub id: String,
    pub status: Status,
    pub label: String,
    pub source: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
}

#[derive(Default)]
pub struct AppState {
    pub sessions: Mutex<Vec<AgentSession>>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<AgentSession> {
        self.sessions.lock().unwrap().clone()
    }

    pub fn apply_set(&self, input: SetInput, now_ms: i64) {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(existing) = sessions.iter_mut().find(|s| s.id == input.id) {
            let prior = existing.status;

            if prior == Status::Working && input.status != Status::Working {
                let delta = (now_ms - existing.state_entered_at).max(0) as u64;
                existing.working_accumulated_ms = existing.working_accumulated_ms.saturating_add(delta);
            }

            let task_boundary =
                matches!(prior, Status::Done | Status::Idle) && input.status == Status::Working;
            if task_boundary {
                existing.original_prompt = Some(input.label.clone());
                existing.working_accumulated_ms = 0;
            }

            if prior != input.status {
                existing.state_entered_at = now_ms;
            }

            existing.status = input.status;
            existing.label = input.label;
            if let Some(src) = input.source {
                existing.source = src;
            }
            if input.model.is_some() {
                existing.model = input.model;
            }
            if input.input_tokens.is_some() {
                existing.input_tokens = input.input_tokens;
            }
            existing.updated = now_ms;
        } else {
            let original_prompt = if input.status == Status::Working {
                Some(input.label.clone())
            } else {
                None
            };
            sessions.push(AgentSession {
                id: input.id,
                status: input.status,
                label: input.label,
                original_prompt,
                source: input.source.unwrap_or_else(|| "claude-code".to_string()),
                model: input.model,
                input_tokens: input.input_tokens,
                updated: now_ms,
                state_entered_at: now_ms,
                working_accumulated_ms: 0,
            });
        }
    }

    pub fn apply_clear(&self, id: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|s| s.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(id: &str, status: Status, label: &str) -> SetInput {
        SetInput {
            id: id.to_string(),
            status,
            label: label.to_string(),
            source: None,
            model: None,
            input_tokens: None,
        }
    }

    fn get<'a>(state: &'a AppState, id: &str) -> AgentSession {
        state
            .snapshot()
            .into_iter()
            .find(|s| s.id == id)
            .expect("session")
    }

    #[test]
    fn new_working_session_captures_original_prompt() {
        let state = AppState::new();
        state.apply_set(set("a", Status::Working, "fix foo.py"), 1000);

        let s = get(&state, "a");
        assert_eq!(s.status, Status::Working);
        assert_eq!(s.original_prompt.as_deref(), Some("fix foo.py"));
        assert_eq!(s.state_entered_at, 1000);
        assert_eq!(s.working_accumulated_ms, 0);
    }

    #[test]
    fn new_non_working_session_has_no_original_prompt() {
        let state = AppState::new();
        state.apply_set(set("a", Status::Idle, ""), 1000);
        assert_eq!(get(&state, "a").original_prompt, None);
    }

    #[test]
    fn approval_cycle_preserves_original_prompt_and_accumulator() {
        let state = AppState::new();
        // Initial working: task starts
        state.apply_set(set("a", Status::Working, "fix foo.py"), 0);
        // Claude asks for approval after 30s
        state.apply_set(set("a", Status::Awaiting, "run bash?"), 30_000);
        let mid = get(&state, "a");
        assert_eq!(mid.status, Status::Awaiting);
        assert_eq!(mid.original_prompt.as_deref(), Some("fix foo.py"));
        assert_eq!(mid.working_accumulated_ms, 30_000);
        assert_eq!(mid.state_entered_at, 30_000);

        // User approves after 5s; agent resumes working with noise label "yes"
        state.apply_set(set("a", Status::Working, "yes"), 35_000);
        let resumed = get(&state, "a");
        assert_eq!(resumed.status, Status::Working);
        assert_eq!(
            resumed.original_prompt.as_deref(),
            Some("fix foo.py"),
            "original prompt must survive approval cycle"
        );
        assert_eq!(
            resumed.working_accumulated_ms, 30_000,
            "accumulated time from before the approval must be preserved"
        );
        assert_eq!(resumed.state_entered_at, 35_000);
    }

    #[test]
    fn done_then_working_is_task_boundary_and_resets_state() {
        let state = AppState::new();
        state.apply_set(set("a", Status::Working, "fix foo.py"), 0);
        state.apply_set(set("a", Status::Done, "fixed!"), 60_000);
        let after_done = get(&state, "a");
        assert_eq!(
            after_done.working_accumulated_ms, 60_000,
            "working time accumulated on exit"
        );
        assert_eq!(after_done.original_prompt.as_deref(), Some("fix foo.py"));

        // New task on the same session
        state.apply_set(set("a", Status::Working, "add tests"), 120_000);
        let new_task = get(&state, "a");
        assert_eq!(new_task.original_prompt.as_deref(), Some("add tests"));
        assert_eq!(new_task.working_accumulated_ms, 0);
        assert_eq!(new_task.state_entered_at, 120_000);
    }

    #[test]
    fn idle_then_working_is_also_task_boundary() {
        let state = AppState::new();
        state.apply_set(set("a", Status::Idle, ""), 0);
        state.apply_set(set("a", Status::Working, "new task"), 10_000);
        let s = get(&state, "a");
        assert_eq!(s.original_prompt.as_deref(), Some("new task"));
        assert_eq!(s.working_accumulated_ms, 0);
    }

    #[test]
    fn working_to_error_accumulates_but_does_not_reset() {
        let state = AppState::new();
        state.apply_set(set("a", Status::Working, "do a thing"), 0);
        state.apply_set(set("a", Status::Error, "perm denied"), 5_000);
        let s = get(&state, "a");
        assert_eq!(s.status, Status::Error);
        assert_eq!(s.working_accumulated_ms, 5_000);
        assert_eq!(s.original_prompt.as_deref(), Some("do a thing"));
        assert_eq!(s.label, "perm denied");
    }

    #[test]
    fn same_status_update_keeps_state_entered_at() {
        let state = AppState::new();
        state.apply_set(set("a", Status::Working, "task"), 0);
        state.apply_set(set("a", Status::Working, "task"), 5_000);
        let s = get(&state, "a");
        assert_eq!(s.state_entered_at, 0, "state_entered_at must not reset within the same status");
    }

    #[test]
    fn clear_removes_session() {
        let state = AppState::new();
        state.apply_set(set("a", Status::Working, "task"), 0);
        state.apply_set(set("b", Status::Working, "other"), 0);
        state.apply_clear("a");
        let ids: Vec<String> = state.snapshot().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn model_and_tokens_are_updated_when_provided() {
        let state = AppState::new();
        state.apply_set(set("a", Status::Working, "task"), 0);
        state.apply_set(
            SetInput {
                id: "a".into(),
                status: Status::Working,
                label: "task".into(),
                source: None,
                model: Some("claude-opus-4-7".into()),
                input_tokens: Some(50_000),
            },
            1000,
        );
        let s = get(&state, "a");
        assert_eq!(s.model.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(s.input_tokens, Some(50_000));
    }
}
