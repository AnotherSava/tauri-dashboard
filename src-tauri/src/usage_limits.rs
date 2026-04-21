use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::commands::{emit_usage_limits_updated, now_ms};
use crate::config::ConfigState;

#[derive(Clone, Debug, Serialize)]
pub struct LimitBucket {
    pub utilization: f32,
    pub resets_at: i64,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageStatus {
    Ok,
    Unavailable,
    AuthExpired,
    NetworkError,
}

#[derive(Clone, Debug, Serialize)]
pub struct UsageLimits {
    pub five_hour: Option<LimitBucket>,
    pub seven_day: Option<LimitBucket>,
    pub status: UsageStatus,
    pub updated: i64,
}

impl UsageLimits {
    fn empty() -> Self {
        Self {
            five_hour: None,
            seven_day: None,
            status: UsageStatus::Unavailable,
            updated: 0,
        }
    }
}

pub struct UsageLimitsState {
    inner: RwLock<UsageLimits>,
}

impl UsageLimitsState {
    pub fn new() -> Self {
        Self { inner: RwLock::new(UsageLimits::empty()) }
    }

    pub fn snapshot(&self) -> UsageLimits {
        self.inner.read().unwrap().clone()
    }

    fn replace(&self, next: UsageLimits) {
        *self.inner.write().unwrap() = next;
    }
}

#[derive(Deserialize)]
struct OauthWrapper {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<OauthCreds>,
}

#[derive(Deserialize)]
struct OauthCreds {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
}

#[derive(Deserialize)]
struct UsageBucketWire {
    utilization: f32,
    resets_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct UsageResponse {
    five_hour: Option<UsageBucketWire>,
    seven_day: Option<UsageBucketWire>,
}

#[derive(Debug)]
enum CredsError {
    Missing,
    Unreadable,
    Expired,
}

fn resolve_credentials_path() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE").ok()?
    } else {
        std::env::var("HOME").ok()?
    };
    Some(PathBuf::from(home).join(".claude").join(".credentials.json"))
}

fn read_credentials(path: &Path) -> Result<String, CredsError> {
    if !path.exists() {
        return Err(CredsError::Missing);
    }
    let bytes = std::fs::read(path).map_err(|_| CredsError::Unreadable)?;
    let wrapper: OauthWrapper =
        serde_json::from_slice(&bytes).map_err(|_| CredsError::Unreadable)?;
    let creds = wrapper.claude_ai_oauth.ok_or(CredsError::Unreadable)?;
    let token = creds
        .access_token
        .filter(|t| !t.trim().is_empty())
        .ok_or(CredsError::Unreadable)?;
    if let Some(exp) = creds.expires_at {
        if exp > 0 && exp <= now_ms() {
            return Err(CredsError::Expired);
        }
    }
    Ok(token)
}

// dead_code lint doesn't see Debug-derived reads; fields are consumed via `?err`.
#[allow(dead_code)]
#[derive(Debug)]
enum PollError {
    Auth(reqwest::StatusCode),
    HttpStatus(reqwest::StatusCode, String),
    Network(String),
    JsonParse(String),
}

async fn fetch_usage(
    client: &reqwest::Client,
    token: &str,
) -> Result<UsageResponse, PollError> {
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .map_err(|e| PollError::Network(e.to_string()))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
    {
        return Err(PollError::Auth(status));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let snippet = body.chars().take(200).collect::<String>();
        return Err(PollError::HttpStatus(status, snippet));
    }
    resp.json::<UsageResponse>()
        .await
        .map_err(|e| PollError::JsonParse(e.to_string()))
}

fn to_bucket(wire: UsageBucketWire) -> LimitBucket {
    // Anthropic's OAuth usage endpoint has historically returned either a
    // 0.0..1.0 fraction or a 0.0..100.0 percentage depending on deployment
    // state. Normalize to 0.0..1.0 before clamping so the UI is consistent.
    let raw = wire.utilization;
    let normalized = if raw > 1.5 { raw / 100.0 } else { raw };
    LimitBucket {
        utilization: normalized.clamp(0.0, 1.0),
        resets_at: wire.resets_at.timestamp_millis(),
    }
}

pub struct UsageLimitsPoller;

impl UsageLimitsPoller {
    pub fn spawn(app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            tracing::info!("usage limits poller started");
            loop {
                poll_once(&app, &client).await;
                let secs = poll_interval_seconds(&app);
                tokio::time::sleep(Duration::from_secs(secs)).await;
            }
        });
    }
}

const MIN_POLL_SECS: u64 = 60;

fn poll_interval_seconds(app: &AppHandle) -> u64 {
    app.try_state::<ConfigState>()
        .map(|cfg| cfg.snapshot().usage_limits_poll_interval_seconds)
        .unwrap_or(600)
        .max(MIN_POLL_SECS)
}

async fn poll_once(app: &AppHandle, client: &reqwest::Client) {
    let Some(state) = app.try_state::<UsageLimitsState>() else { return };
    let previous = state.snapshot();
    let now = now_ms();

    let Some(path) = resolve_credentials_path() else {
        state.replace(UsageLimits {
            five_hour: None,
            seven_day: None,
            status: UsageStatus::Unavailable,
            updated: now,
        });
        emit_usage_limits_updated(app);
        return;
    };

    let token = match read_credentials(&path) {
        Ok(t) => t,
        Err(CredsError::Missing | CredsError::Unreadable) => {
            state.replace(UsageLimits {
                five_hour: None,
                seven_day: None,
                status: UsageStatus::Unavailable,
                updated: now,
            });
            emit_usage_limits_updated(app);
            return;
        }
        Err(CredsError::Expired) => {
            state.replace(UsageLimits {
                five_hour: None,
                seven_day: None,
                status: UsageStatus::AuthExpired,
                updated: now,
            });
            emit_usage_limits_updated(app);
            return;
        }
    };

    match fetch_usage(client, &token).await {
        Ok(usage) => {
            tracing::debug!(
                five_hour_raw = ?usage.five_hour.as_ref().map(|b| b.utilization),
                seven_day_raw = ?usage.seven_day.as_ref().map(|b| b.utilization),
                "usage poll success"
            );
            state.replace(UsageLimits {
                five_hour: usage.five_hour.map(to_bucket),
                seven_day: usage.seven_day.map(to_bucket),
                status: UsageStatus::Ok,
                updated: now,
            });
            emit_usage_limits_updated(app);
        }
        Err(err) => {
            let status = match err {
                PollError::Auth(_) => UsageStatus::AuthExpired,
                _ => UsageStatus::NetworkError,
            };
            tracing::warn!(?err, "usage limits poll failed");
            // Keep last-known buckets visible on transient failures; the
            // tooltip's "updated Ns ago" carries the staleness signal.
            let keep_prev = matches!(status, UsageStatus::NetworkError)
                && previous.five_hour.is_some();
            state.replace(UsageLimits {
                five_hour: if keep_prev { previous.five_hour.clone() } else { None },
                seven_day: if keep_prev { previous.seven_day.clone() } else { None },
                status,
                updated: if keep_prev { previous.updated } else { now },
            });
            emit_usage_limits_updated(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_happy_credentials() {
        let bytes = br#"{
            "claudeAiOauth": {
                "accessToken": "sk-xxx",
                "refreshToken": "r",
                "expiresAt": 9999999999999
            }
        }"#;
        let w: OauthWrapper = serde_json::from_slice(bytes).unwrap();
        let creds = w.claude_ai_oauth.unwrap();
        assert_eq!(creds.access_token.as_deref(), Some("sk-xxx"));
        assert_eq!(creds.expires_at, Some(9999999999999));
    }

    #[test]
    fn tolerates_missing_oauth_block() {
        let bytes = br#"{"otherField": 1}"#;
        let w: OauthWrapper = serde_json::from_slice(bytes).unwrap();
        assert!(w.claude_ai_oauth.is_none());
    }

    #[test]
    fn deserializes_usage_response() {
        let bytes = br#"{
            "five_hour":  { "utilization": 0.42, "resets_at": "2026-04-20T22:00:00.000+00:00" },
            "seven_day":  { "utilization": 0.18, "resets_at": "2026-04-25T00:00:00Z" }
        }"#;
        let r: UsageResponse = serde_json::from_slice(bytes).unwrap();
        let fh = r.five_hour.unwrap();
        assert!((fh.utilization - 0.42).abs() < 1e-6);
        let expected = DateTime::parse_from_rfc3339("2026-04-20T22:00:00.000+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(fh.resets_at, expected);

        let sd = r.seven_day.unwrap();
        assert!((sd.utilization - 0.18).abs() < 1e-6);
    }

    #[test]
    fn to_bucket_keeps_fraction_scale() {
        let wire = UsageBucketWire {
            utilization: 0.42,
            resets_at: Utc::now(),
        };
        let b = to_bucket(wire);
        assert!((b.utilization - 0.42).abs() < 1e-6);
    }

    #[test]
    fn to_bucket_rescales_percentage_scale() {
        let wire = UsageBucketWire {
            utilization: 42.0,
            resets_at: Utc::now(),
        };
        let b = to_bucket(wire);
        assert!((b.utilization - 0.42).abs() < 1e-6);
    }

    #[test]
    fn to_bucket_clamps_out_of_range() {
        let wire = UsageBucketWire {
            utilization: 150.0,
            resets_at: Utc::now(),
        };
        let b = to_bucket(wire);
        assert!((b.utilization - 1.0).abs() < 1e-6);
    }

    #[test]
    fn missing_file_returns_missing() {
        let path = std::env::temp_dir().join("ai_agent_dashboard_missing_xyz.json");
        // Ensure it doesn't exist
        let _ = std::fs::remove_file(&path);
        assert!(matches!(read_credentials(&path), Err(CredsError::Missing)));
    }

    #[test]
    fn expired_token_returns_expired() {
        let dir = std::env::temp_dir().join(format!(
            "ai_agent_dashboard_usage_test_expired_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(
            f,
            r#"{{"claudeAiOauth":{{"accessToken":"x","expiresAt":100}}}}"#
        )
        .unwrap();
        drop(f);
        assert!(matches!(read_credentials(&path), Err(CredsError::Expired)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn valid_token_returned() {
        let dir = std::env::temp_dir().join(format!(
            "ai_agent_dashboard_usage_test_valid_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        let future = now_ms() + 60 * 60 * 1000;
        write!(
            f,
            r#"{{"claudeAiOauth":{{"accessToken":"tok","expiresAt":{future}}}}}"#
        )
        .unwrap();
        drop(f);
        assert_eq!(read_credentials(&path).unwrap(), "tok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn blank_token_treated_as_unreadable() {
        let dir = std::env::temp_dir().join(format!(
            "ai_agent_dashboard_usage_test_blank_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, r#"{{"claudeAiOauth":{{"accessToken":"  "}}}}"#).unwrap();
        drop(f);
        assert!(matches!(read_credentials(&path), Err(CredsError::Unreadable)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
