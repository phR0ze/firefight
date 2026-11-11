//! HTTP routes: admin REST + WebSocket for the control deck, WebSocket for agents.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::Utc;
use firefight_core::model::{Action, ClientId, ClientView, TimerDef, TimerId, TimerKind, default_focus_on_fire};
use firefight_core::protocol::{AgentMsg, ServerMsg};
use firefight_core::run::TimerRun;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::hub::{Bulk, Control, HelloOutcome, Hub};
use crate::store::token_matches_plain;

#[derive(Clone)]
pub struct AppState {
    pub hub: Arc<Mutex<Hub>>,
    pub admin_token: Arc<String>,
}

impl AppState {
    fn hub(&self) -> MutexGuard<'_, Hub> {
        self.hub.lock().expect("hub mutex poisoned")
    }
}

pub fn router(state: AppState) -> Router {
    let admin = Router::new()
        .route("/timers", get(list_timers).post(create_timer))
        .route("/timers/{id}", put(update_timer).delete(delete_timer))
        .route("/timers/{id}/{op}", post(control_timer))
        .route("/clients", get(list_clients))
        .route("/clients/start", post(|s| bulk_all(s, Bulk::Start)))
        .route("/clients/pause", post(|s| bulk_all(s, Bulk::Pause)))
        .route("/clients/reset", post(|s| bulk_all(s, Bulk::Reset)))
        .route("/clients/{id}", axum::routing::patch(rename_client).delete(delete_client))
        .route("/clients/{id}/approve", post(approve_client))
        .route("/clients/{id}/assignments", put(set_assignments))
        .route("/clients/{id}/test", post(test_run))
        .route("/clients/{id}/{op}", post(bulk_client))
        .route("/clients/{id}/timers/{timer_id}/{op}", post(control_client_timer))
        .route("/ws", get(admin_ws))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_admin));

    Router::new()
        .route("/api/health", get(|| async { Json(json!({ "ok": true })) }))
        .route("/api/agent/ws", get(agent_ws))
        .nest("/api/admin", admin)
        .with_state(state)
}

// ---- errors & auth ----------------------------------------------------------

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        let msg = e.to_string();
        let code = if msg.starts_with("no such") { StatusCode::NOT_FOUND } else { StatusCode::BAD_REQUEST };
        ApiError(code, msg)
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// Accepts `Authorization: Bearer <token>` or `?token=<token>`.
async fn require_admin(
    State(state): State<AppState>,
    Query(q): Query<TokenQuery>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);
    match bearer.or(q.token) {
        Some(t) if token_matches_plain(&t, &state.admin_token) => Ok(next.run(req).await),
        _ => Err(ApiError(StatusCode::UNAUTHORIZED, "invalid admin token".into())),
    }
}

// ---- timers -----------------------------------------------------------------

/// Timer fields editable from the deck; the id comes from the path or is generated.
#[derive(Deserialize)]
struct TimerDraft {
    name: String,
    kind: TimerKind,
    #[serde(default)]
    actions: Vec<Action>,
    #[serde(default = "default_focus_on_fire")]
    focus_on_fire: bool,
}

impl TimerDraft {
    fn into_def(self, id: TimerId) -> Result<TimerDef, ApiError> {
        let def = TimerDef {
            id,
            name: self.name.trim().to_string(),
            kind: self.kind,
            actions: self.actions,
            focus_on_fire: self.focus_on_fire,
        };
        def.validate().map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
        Ok(def)
    }
}

async fn list_timers(State(s): State<AppState>) -> Json<Vec<TimerDef>> {
    Json(s.hub().timers().to_vec())
}

async fn create_timer(State(s): State<AppState>, Json(draft): Json<TimerDraft>) -> ApiResult<TimerDef> {
    let def = draft.into_def(Uuid::new_v4())?;
    Ok(Json(s.hub().upsert_timer(def, Utc::now())))
}

async fn update_timer(
    State(s): State<AppState>,
    Path(id): Path<TimerId>,
    Json(draft): Json<TimerDraft>,
) -> ApiResult<TimerDef> {
    let def = draft.into_def(id)?;
    let mut hub = s.hub();
    if !hub.timers().iter().any(|t| t.id == id) {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such timer".into()));
    }
    Ok(Json(hub.upsert_timer(def, Utc::now())))
}

async fn delete_timer(State(s): State<AppState>, Path(id): Path<TimerId>) -> Result<StatusCode, ApiError> {
    s.hub().delete_timer(id, Utc::now())?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AddQuery {
    secs: Option<u64>,
}

async fn control_timer(
    State(s): State<AppState>,
    Path((id, op)): Path<(TimerId, String)>,
    Query(q): Query<AddQuery>,
) -> ApiResult<TimerRun> {
    let op = match op.as_str() {
        "start" | "resume" => Control::Start,
        "pause" => Control::Pause,
        "reset" => Control::Reset,
        "dismiss" => Control::Dismiss,
        "add" => Control::Add { secs: q.secs.unwrap_or(60) },
        _ => return Err(ApiError(StatusCode::NOT_FOUND, format!("unknown operation {op}"))),
    };
    Ok(Json(s.hub().control(id, op, Utc::now())?))
}

// ---- clients ----------------------------------------------------------------

async fn list_clients(State(s): State<AppState>) -> Json<Vec<ClientView>> {
    Json(s.hub().clients())
}

async fn approve_client(State(s): State<AppState>, Path(id): Path<ClientId>) -> ApiResult<ClientView> {
    Ok(Json(s.hub().approve_client(id, Utc::now())?))
}

async fn delete_client(State(s): State<AppState>, Path(id): Path<ClientId>) -> Result<StatusCode, ApiError> {
    s.hub().remove_client(id, "removed by administrator")?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Rename {
    name: String,
}

async fn rename_client(
    State(s): State<AppState>,
    Path(id): Path<ClientId>,
    Json(body): Json<Rename>,
) -> ApiResult<ClientView> {
    Ok(Json(s.hub().rename_client(id, body.name)?))
}

#[derive(Serialize, Deserialize)]
struct Assignments {
    timers: Vec<TimerId>,
}

async fn set_assignments(
    State(s): State<AppState>,
    Path(id): Path<ClientId>,
    Json(body): Json<Assignments>,
) -> ApiResult<ClientView> {
    Ok(Json(s.hub().set_assignments(id, body.timers, Utc::now())?))
}

fn parse_bulk(op: &str) -> Result<Bulk, ApiError> {
    match op {
        "start" => Ok(Bulk::Start),
        "pause" => Ok(Bulk::Pause),
        "reset" => Ok(Bulk::Reset),
        _ => Err(ApiError(StatusCode::NOT_FOUND, format!("unknown operation {op}"))),
    }
}

/// Start, pause or reset every timer assigned to a client, for that client only.
async fn bulk_client(State(s): State<AppState>, Path((id, op)): Path<(ClientId, String)>) -> ApiResult<Vec<TimerRun>> {
    let op = parse_bulk(&op)?;
    Ok(Json(s.hub().bulk_client(id, op, Utc::now())?))
}

/// Reset, dismiss or add time to one of a client's timers, for that client only.
async fn control_client_timer(
    State(s): State<AppState>,
    Path((id, timer_id, op)): Path<(ClientId, TimerId, String)>,
    Query(q): Query<AddQuery>,
) -> ApiResult<TimerRun> {
    let op = match op.as_str() {
        "reset" => Control::Reset,
        "dismiss" => Control::Dismiss,
        "add" => Control::Add { secs: q.secs.unwrap_or(60) },
        _ => return Err(ApiError(StatusCode::NOT_FOUND, format!("unknown operation {op}"))),
    };
    Ok(Json(s.hub().control_client(id, timer_id, op, Utc::now())?))
}

/// Start, pause or reset every timer assigned to any approved client, for every client with it.
/// Clients following their own copy of a run are started and paused where they are; resetting
/// brings them back in step on each timer's shared run.
async fn bulk_all(State(s): State<AppState>, op: Bulk) -> ApiResult<Vec<TimerRun>> {
    Ok(Json(s.hub().bulk_all(op, Utc::now())?))
}

#[derive(Deserialize)]
struct TestRun {
    actions: Vec<Action>,
}

async fn test_run(
    State(s): State<AppState>,
    Path(id): Path<ClientId>,
    Json(body): Json<TestRun>,
) -> Result<StatusCode, ApiError> {
    s.hub().test_run(id, body.actions)?;
    Ok(StatusCode::ACCEPTED)
}

// ---- websockets -------------------------------------------------------------

fn to_msg<T: Serialize>(v: &T) -> Message {
    Message::Text(serde_json::to_string(v).expect("serializable").into())
}

async fn admin_ws(State(s): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| admin_session(s, socket))
}

async fn admin_session(s: AppState, mut socket: WebSocket) {
    // Subscribe and snapshot under one lock so no event is missed in between.
    let (mut rx, snapshot) = {
        let hub = s.hub();
        (hub.subscribe_admin(), hub.snapshot(Utc::now()))
    };
    if socket.send(to_msg(&snapshot)).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            ev = rx.recv() => {
                let msg = match ev {
                    Ok(ev) => to_msg(&ev),
                    Err(RecvError::Lagged(_)) => to_msg(&s.hub().snapshot(Utc::now())),
                    Err(RecvError::Closed) => break,
                };
                if socket.send(msg).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
        }
    }
}

async fn agent_ws(State(s): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| agent_session(s, socket))
}

async fn agent_session(s: AppState, mut socket: WebSocket) {
    let hello = match tokio::time::timeout(Duration::from_secs(10), socket.recv()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<AgentMsg>(&t).ok(),
        _ => None,
    };
    let Some(AgentMsg::Hello { client_id, token, hostname, name, enroll_key }) = hello else {
        let _ = socket.send(to_msg(&ServerMsg::Rejected { reason: "expected hello".into() })).await;
        return;
    };

    let outcome = s.hub().agent_hello(client_id, token, hostname, name, enroll_key, Utc::now());
    let (id, conn_id, mut rx) = match outcome {
        HelloOutcome::Accepted { client_id, conn_id, rx } => (client_id, conn_id, rx),
        HelloOutcome::Rejected(reason) => {
            tracing::warn!("agent rejected: {reason}");
            let _ = socket.send(to_msg(&ServerMsg::Rejected { reason })).await;
            return;
        }
    };

    let mut ping = tokio::time::interval(Duration::from_secs(20));
    loop {
        tokio::select! {
            out = rx.recv() => {
                let Some(msg) = out else { break };
                let closing = matches!(msg, ServerMsg::Rejected { .. });
                if socket.send(to_msg(&msg)).await.is_err() || closing {
                    break;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => match serde_json::from_str::<AgentMsg>(&t) {
                    Ok(msg) => handle_agent_msg(&s, id, msg),
                    Err(e) => tracing::debug!(client = %id, "bad agent message: {e}"),
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => {
                if socket.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
            }
        }
    }
    s.hub().agent_disconnected(id, conn_id, Utc::now());
}

fn handle_agent_msg(s: &AppState, id: ClientId, msg: AgentMsg) {
    let mut hub = s.hub();
    if !hub.is_approved(id) {
        return;
    }
    match msg {
        AgentMsg::ActionResult { result } => hub.agent_result(id, result),
        AgentMsg::RunComplete { timer_id, run_id } => hub.agent_run_complete(id, timer_id, run_id, Utc::now()),
        AgentMsg::Hello { .. } => {}
    }
}
