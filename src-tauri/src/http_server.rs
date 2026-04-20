use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use std::net::SocketAddr;
use tauri::{AppHandle, Manager};

use crate::commands::{emit_sessions_updated, now_ms};
use crate::state::{AppState, SetInput, Status};

pub const DEFAULT_PORT: u16 = 9077;

pub async fn run(app: AppHandle, port: u16) {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[http_server] failed to bind on {addr}: {e}");
            return;
        }
    };
    eprintln!("[http_server] listening on http://{addr}");

    let router = Router::new()
        .route("/api/status", post(post_status))
        .with_state(app);

    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("[http_server] serve error: {e}");
    }
}

#[derive(Deserialize, Debug)]
#[serde(tag = "action", rename_all = "lowercase")]
#[allow(dead_code)]
enum StatusRequest {
    Set(SetPayload),
    Clear(ClearPayload),
    Config(serde_json::Value),
}

#[derive(Deserialize, Debug)]
struct SetPayload {
    id: String,
    status: Status,
    label: Option<String>,
    source: Option<String>,
    model: Option<String>,
    #[serde(rename = "inputTokens")]
    input_tokens: Option<u64>,
    // Tolerated-but-ignored fields so the existing Python hook's body shape
    // doesn't trigger deserialization errors.
    #[serde(default, rename = "transcript_path")]
    _transcript_path: Option<String>,
    #[serde(default)]
    _updated: Option<i64>,
}

#[derive(Deserialize, Debug)]
struct ClearPayload {
    id: String,
}

async fn post_status(
    State(app): State<AppHandle>,
    headers: HeaderMap,
    Json(payload): Json<StatusRequest>,
) -> StatusCode {
    // CSRF guard: block browser-originated requests. Tools using urllib / curl
    // don't send Origin; browser XHRs do. "null" is allowed (file:// / data:).
    if let Some(origin) = headers.get("origin") {
        match origin.to_str() {
            Ok("null") => {}
            _ => return StatusCode::FORBIDDEN,
        }
    }

    let Some(state) = app.try_state::<AppState>() else {
        return StatusCode::INTERNAL_SERVER_ERROR;
    };

    match payload {
        StatusRequest::Set(p) => {
            let input = SetInput {
                id: p.id,
                status: p.status,
                label: p.label,
                source: p.source,
                model: p.model,
                input_tokens: p.input_tokens,
            };
            state.apply_set(input, now_ms());
        }
        StatusRequest::Clear(p) => {
            state.apply_clear(&p.id);
        }
        StatusRequest::Config(_) => {
            // Stage 5 wires real config handling; accept and ignore for now.
            return StatusCode::NO_CONTENT;
        }
    }
    emit_sessions_updated(&app);
    StatusCode::NO_CONTENT
}
