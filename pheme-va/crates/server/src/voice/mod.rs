//! HTTP adaptation only; the shared Rust runtime owns inference and conversations.
mod stream;
#[cfg(test)]
mod tests;
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
pub use va_runtime::voice::Limits;
use va_runtime::{
    conversations, voice as agent, ErrorKind, RequestOptions, RuntimeError, TurnInput,
};

pub struct HttpError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}
impl From<RuntimeError> for HttpError {
    fn from(error: RuntimeError) -> Self {
        let status = match error.0 {
            ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
            ErrorKind::Conflict => StatusCode::CONFLICT,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::Expired => StatusCode::GONE,
            ErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorKind::Capacity => StatusCode::TOO_MANY_REQUESTS,
            ErrorKind::Timeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            code: error.1.code,
            message: error.1.message,
        }
    }
}
impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"error":{"code":self.code,"message":self.message}})),
        )
            .into_response()
    }
}
fn http_error(status: StatusCode, code: &'static str, message: &'static str) -> HttpError {
    HttpError {
        status,
        code,
        message,
    }
}
fn options(headers: &HeaderMap) -> Result<RequestOptions, HttpError> {
    let idempotency_key = headers
        .get("idempotency-key")
        .map(|v| v.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| {
            http_error(
                StatusCode::BAD_REQUEST,
                "invalid_idempotency_key",
                "Idempotency-Key must be printable ASCII of 1 to 128 bytes.",
            )
        })?;
    Ok(RequestOptions {
        run_id: crate::header_string(headers, "x-run-id"),
        experiment_id: crate::header_string(headers, "x-experiment-id"),
        incident_active: crate::header_bool(headers, "x-incident-active"),
        idempotency_key,
        audio_source: crate::header_string(headers, "x-audio-source")
            .unwrap_or_else(|| "audio".into()),
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextRequest {
    text: String,
}
fn media_type(headers: &HeaderMap) -> String {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}
fn text_body(headers: &HeaderMap, body: &[u8]) -> Result<String, HttpError> {
    if media_type(headers) != "application/json" {
        return Err(http_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Content-Type must be application/json.",
        ));
    }
    if body.len() > 64 * 1024 {
        return Err(http_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "body_too_large",
            "JSON body exceeds the configured byte limit.",
        ));
    }
    serde_json::from_slice::<TextRequest>(body)
        .map(|r| r.text)
        .map_err(|_| {
            http_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "Expected a JSON object containing only text.",
            )
        })
}
fn input(headers: &HeaderMap, body: Bytes) -> Result<TurnInput, HttpError> {
    match media_type(headers).as_str() {
        "application/json" => Ok(TurnInput::Text(text_body(headers, &body)?)),
        "audio/wav" | "audio/x-wav" => Ok(TurnInput::Audio(body.to_vec())),
        _ => Err(http_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Content-Type must be audio/wav or application/json.",
        )),
    }
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/voice/turns", post(create))
        .route("/v1/voice/turns/{turn_id}", get(recover))
        .route("/v1/voice/turns/{turn_id}/submit", post(submit))
        .route("/v1/voice/turns/{turn_id}/cancel", post(cancel))
        .route("/v1/voice/reset", post(reset))
        .route("/v1/voice/inspect", get(inspect))
        .route("/v1/voice/test/reply", post(test_reply))
        .route("/v1/voice/conversations", post(new_conversation))
        .route("/v1/voice/conversations/{id}", get(snapshot))
        .route("/v1/voice/conversations/{id}/metrics", get(metrics))
        .route("/v1/voice/conversations/{id}/runs/{run}", get(run))
        .route("/v1/voice/conversations/{id}/finish", post(finish))
        .route("/v1/voice/conversations/{id}/turns", post(start_turn))
        .route("/v1/voice/conversations/{id}/turns/{turn}", get(turn))
        .route(
            "/v1/voice/conversations/{id}/turns/{turn}/submit",
            post(approve),
        )
        .route(
            "/v1/voice/conversations/{id}/turns/{turn}/cancel",
            post(stop_turn),
        )
}
async fn create(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let (input, options) = match input(&headers, body).and_then(|i| Ok((i, options(&headers)?))) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    match agent::create(state.runtime, input, options, false).await {
        Ok(s) => stream::response(s),
        Err(e) => HttpError::from(e).into_response(),
    }
}
async fn submit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let text = match text_body(&headers, &body) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    match agent::submit(&state, &id, text) {
        Ok(t) => Json(t).into_response(),
        Err(e) => HttpError::from(e).into_response(),
    }
}
async fn recover(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<agent::Turn>, HttpError> {
    Ok(Json(agent::recover(&state, &id)?))
}
async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<agent::Turn>, HttpError> {
    Ok(Json(agent::cancel(&state, &id)?))
}
async fn reset(State(state): State<AppState>) -> Result<Json<serde_json::Value>, HttpError> {
    agent::reset(&state).await?;
    Ok(Json(json!({"status":"reset"})))
}
async fn inspect(State(state): State<AppState>) -> Json<agent::Inspection> {
    Json(agent::inspect(&state))
}
async fn test_reply(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let (text, options) = match text_body(&headers, &body).and_then(|t| Ok((t, options(&headers)?)))
    {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    match agent::test_reply(state.runtime, text, options) {
        Ok(s) => stream::response(s),
        Err(e) => HttpError::from(e).into_response(),
    }
}
async fn new_conversation(
    State(state): State<AppState>,
) -> Result<Json<va_core::chat::ChatSnapshot>, HttpError> {
    Ok(Json(conversations::new_conversation(&state)?))
}
async fn snapshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<va_core::chat::ChatSnapshot>, HttpError> {
    Ok(Json(conversations::snapshot(&state, &id, false)?))
}
#[derive(Default, Deserialize)]
struct Cursor {
    #[serde(default)]
    after: u64,
}
async fn metrics(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(cursor): Query<Cursor>,
) -> Result<Json<conversations::MetricPage>, HttpError> {
    Ok(Json(conversations::metrics(&state, &id, cursor.after)?))
}
async fn run(
    State(state): State<AppState>,
    Path((id, run)): Path<(String, String)>,
) -> Result<Json<va_core::chat::ChatRun>, HttpError> {
    Ok(Json(conversations::run(&state, &id, &run)?))
}
async fn start_turn(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (input, options) = match input(&headers, body).and_then(|i| Ok((i, options(&headers)?))) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    match conversations::start_turn(&state, &id, input, options, false).await {
        Ok(s) => stream::response(s),
        Err(e) => HttpError::from(e).into_response(),
    }
}
async fn turn(
    State(state): State<AppState>,
    Path((id, turn)): Path<(String, String)>,
) -> Result<Json<agent::Turn>, HttpError> {
    Ok(Json(conversations::turn(&state, &id, &turn)?))
}
async fn approve(
    State(state): State<AppState>,
    Path((id, turn)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let text = match text_body(&headers, &body) {
        Ok(t) => t,
        Err(e) => return e.into_response(),
    };
    match conversations::approve(&state, &id, &turn, text) {
        Ok(t) => Json(t).into_response(),
        Err(e) => HttpError::from(e).into_response(),
    }
}
async fn stop_turn(
    State(state): State<AppState>,
    Path((id, turn)): Path<(String, String)>,
) -> Result<Json<agent::Turn>, HttpError> {
    Ok(Json(conversations::stop_turn(&state, &id, &turn)?))
}
async fn finish(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<va_core::chat::ChatSnapshot>, HttpError> {
    Ok(Json(conversations::finish(&state, &id)?))
}
