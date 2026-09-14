//! HTTP routes exposing the Lares core over protojson.

use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};

use lares_core::diff;
use lares_core::domain::{
    now_unix, AnalyzeMode, AnalyzeSceneRequest, AnalyzeSceneResponse, ChoreEntity, ChoreStatus,
    FingerprintKind, LandmarkList, ListChoresResponse, ListFingerprintsResponse, NudgeRequest,
    NudgeResponse, ReferenceState, SetChoreStatusRequest, SetFingerprintRequest,
    SetReferenceRequest,
};
use lares_core::engine::InferenceError;
use lares_core::store::StoreError;

use crate::state::AppState;

/// Build the application router with the given shared state.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/analyze", post(analyze))
        .route(
            "/v1/rooms/{room_id}/reference",
            get(get_reference).post(set_reference),
        )
        .route("/v1/chores", get(list_chores))
        .route("/v1/chores/{id}/status", patch(set_status))
        .route("/v1/nudge", post(nudge))
        .route(
            "/v1/rooms/{room_id}/landmarks",
            get(get_landmarks),
        )
        .route(
            "/v1/rooms/{room_id}/fingerprint",
            get(get_fingerprints).post(set_fingerprint),
        )
        // Phone captures are multi-megabyte JPEGs; the default 2MB body limit
        // rejects them. 32MB headroom covers even large reference frames.
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state)
}

/// Query parameters for listing chores.
#[derive(Debug, serde::Deserialize)]
pub struct ChoresQuery {
    /// Optional room filter.
    pub room_id: Option<String>,
}

/// Query parameters for listing fingerprints.
#[derive(Debug, serde::Deserialize)]
pub struct FingerprintsQuery {
    /// Optional kind filter: latest (default), history, or clean.
    pub kind: Option<String>,
}

/// Liveness check reporting the active engine.
async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "engine": state.engine.name(),
        "policy": "noop"
    }))
}

/// Analyze a captured keyframe and persist the discovered chores.
async fn analyze(
    State(state): State<AppState>,
    Json(mut req): Json<AnalyzeSceneRequest>,
) -> Result<Json<AnalyzeSceneResponse>, ApiError> {
    if req.mode == AnalyzeMode::Diff as i32 && req.reference_jpeg.is_none() {
        req.reference_jpeg = load_reference(&state, &req.room_id).await?;
    }
    let mut response = state.engine.analyze_scene(req.clone()).await?;
    let chores = diff::postprocess(response.chores, &req.room_id);
    state.store.upsert_chores(&chores).await?;
    if req.mode == AnalyzeMode::Discover as i32 && !response.landmarks.is_empty() {
        state.store.upsert_landmarks(&req.room_id, &response.landmarks).await?;
    }
    response.chores = chores;
    Ok(Json(response))
}

/// Store a room's agreed target state and reference image.
async fn set_reference(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<SetReferenceRequest>,
) -> Result<StatusCode, ApiError> {
    let image_id = uuid::Uuid::new_v4().to_string();
    let refs_dir = state.refs_dir();
    tokio::fs::create_dir_all(&refs_dir).await?;
    let image_path = refs_dir.join(format!("{image_id}.jpg"));
    tokio::fs::write(image_path, &req.frame_jpeg).await?;
    let reference = ReferenceState {
        room_id,
        image_id,
        description: req.description,
        captured_at_unix: now_unix(),
    };
    state.store.save_reference(&reference).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Fetch the agreed target state for a room.
async fn get_reference(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> Result<Json<ReferenceState>, ApiError> {
    let reference = state
        .store
        .get_reference(&room_id)
        .await?
        .ok_or_else(|| ApiError::not_found("reference not found"))?;
    Ok(Json(reference))
}

/// List chores, optionally filtered by room.
async fn list_chores(
    State(state): State<AppState>,
    Query(query): Query<ChoresQuery>,
) -> Result<Json<ListChoresResponse>, ApiError> {
    let chores = state.store.list_chores(query.room_id.as_deref()).await?;
    Ok(Json(ListChoresResponse { chores }))
}

/// Transition a chore's lifecycle state.
async fn set_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SetChoreStatusRequest>,
) -> Result<Json<ChoreEntity>, ApiError> {
    let status = ChoreStatus::try_from(req.status)
        .map_err(|_| ApiError::bad_request("unknown chore status"))?;
    let chore = state
        .store
        .set_status(&id, status)
        .await?
        .ok_or_else(|| ApiError::not_found("chore not found"))?;
    Ok(Json(chore))
}

/// Evaluate the reminder policy against current chores and idle context.
async fn nudge(
    State(state): State<AppState>,
    Json(req): Json<NudgeRequest>,
) -> Result<Json<NudgeResponse>, ApiError> {
    let chores = state.store.list_chores(None).await?;
    let context = match req.context.as_ref() {
        Some(context) => context,
        None => return Ok(Json(NudgeResponse { nudges: Vec::new() })),
    };
    let nudge = state.policy.next_nudge(&chores, context);
    let nudges = nudge.into_iter().collect();
    Ok(Json(NudgeResponse { nudges }))
}

/// Fetch a room's current landmark set.
async fn get_landmarks(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> Result<Json<LandmarkList>, ApiError> {
    let landmarks = state.store.get_landmarks(&room_id).await?;
    Ok(Json(LandmarkList { landmarks }))
}

/// Store a room's scene fingerprint.
async fn set_fingerprint(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<SetFingerprintRequest>,
) -> Result<StatusCode, ApiError> {
    let kind = FingerprintKind::try_from(req.kind).unwrap_or(FingerprintKind::Latest);
    state
        .store
        .save_fingerprint(&room_id, kind, &req.grid_hash)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Fetch a room's stored fingerprints, newest first.
async fn get_fingerprints(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Query(query): Query<FingerprintsQuery>,
) -> Result<Json<ListFingerprintsResponse>, ApiError> {
    let kind = match query.kind.as_deref() {
        Some("clean") => FingerprintKind::Clean,
        Some("history") => FingerprintKind::History,
        _ => FingerprintKind::Latest,
    };
    let fingerprints = state.store.get_fingerprints(&room_id, kind).await?;
    Ok(Json(ListFingerprintsResponse { fingerprints }))
}

/// Load the stored reference image for a room, if any.
async fn load_reference(state: &AppState, room_id: &str) -> Result<Option<Vec<u8>>, ApiError> {
    let reference = match state.store.get_reference(room_id).await? {
        Some(reference) => reference,
        None => return Ok(None),
    };
    let path = state.refs_dir().join(format!("{}.jpg", reference.image_id));
    match tokio::fs::read(&path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) => Err(ApiError::internal(format!(
            "reference image for {room_id}: {err}"
        ))),
    }
}

/// An error serialized as `{ "error": "<message>" }` with an HTTP status.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    /// Build a 400 error.
    fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: message.into() }
    }

    /// Build a 404 error.
    fn not_found(message: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_FOUND, message: message.into() }
    }

    /// Build a 500 error.
    fn internal(message: impl Into<String>) -> Self {
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: message.into() }
    }
}

impl From<StoreError> for ApiError {
    /// Map persistence errors to 500s.
    fn from(err: StoreError) -> Self {
        Self::internal(err.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    /// Map filesystem errors to 500s.
    fn from(err: std::io::Error) -> Self {
        Self::internal(err.to_string())
    }
}

impl From<InferenceError> for ApiError {
    /// Map inference errors to 502s so transient model failures are visible.
    fn from(err: InferenceError) -> Self {
        Self { status: StatusCode::BAD_GATEWAY, message: err.to_string() }
    }
}

impl IntoResponse for ApiError {
    /// Serialize the error as a JSON body with its status.
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.message });
        (self.status, Json(body)).into_response()
    }
}