//! HTTP routes exposing the Lares core over protojson.

use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
    Json, Router,
};

use lares_core::diff;
use lares_core::domain::{
    now_unix, AnalyzeMode, AnalyzeSceneRequest, AnalyzeSceneResponse, BoundingBox, ChoreEntity,
    ChoreKind, ChoreStatus, CompletionCandidate, FingerprintKind, HudResponse, HudTask,
    InferRoomRequest, InferRoomResponse, LandmarkList, ListChoresResponse,
    ListFingerprintsResponse, NudgeRequest, NudgeResponse, RecurrenceFreq, ReferenceState,
    Reminder, SetChoreStatusRequest, SetFingerprintRequest, SetReferenceRequest, TickRequest,
    TickResponse,
};
use lares_core::engine::RoomCandidate;
use lares_core::engine::InferenceError;
use lares_core::store::StoreError;

use crate::state::AppState;

/// Build the application router with the given shared state.
pub fn router(state: AppState) -> Router {
    let core = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/analyze", post(analyze))
        .route("/v1/rooms/infer", post(infer_room))
        .route(
            "/v1/rooms/{room_id}/reference",
            get(get_reference).post(set_reference),
        )
        .route("/v1/chores", get(list_chores).delete(wipe_chores))
        .route("/v1/chores/{id}/status", patch(set_status))
        .route("/v1/nudge", post(nudge))
        .route("/v1/hud", get(hud_get).post(hud_post_state))
        .route("/v1/tick", post(tick))
        .route(
            "/v1/rooms/{room_id}/landmarks",
            get(get_landmarks),
        )
        .route(
            "/v1/rooms/{room_id}/fingerprint",
            get(get_fingerprints).post(set_fingerprint),
        )
        .route(
            "/v1/rooms/{room_id}/expected",
            get(list_expected).post(add_expected),
        )
        .route(
            "/v1/rooms/{room_id}/expected/{label}",
            delete(remove_expected),
        )
        .with_state(state.clone());
    let proactive = crate::people::router(state);
    core.merge(proactive)
        // Phone captures are multi-megabyte JPEGs; the default 2MB body limit
        // rejects them. 32MB headroom covers even large reference frames.
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
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
        "policy": "noop",
        "frozen": state.frozen.is_some()
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
    enrich_with_frozen(&state, &req, &mut response).await;
    let chores = diff::postprocess(response.chores, &req.room_id, req.room_area);
    check_forgotten_tasks(&state, &chores).await?;
    if req.mode == AnalyzeMode::Discover as i32 && !response.landmarks.is_empty() {
        state.store.upsert_landmarks(&req.room_id, &response.landmarks).await?;
    }
    response.chores = chores;
    Ok(Json(response))
}

/// Enrich a scan with the frozen-vision sidecar when configured.
///
/// Two additions: OWLv2 landmark detections merged into the response's
/// landmark list (so room profiles stay populated even when the VLM returns
/// none, e.g. dark frames), and a CLIP scene class for diagnostics. All
/// sidecar failures degrade silently to plain VLM behaviour.
async fn enrich_with_frozen(
    state: &AppState,
    req: &AnalyzeSceneRequest,
    response: &mut AnalyzeSceneResponse,
) {
    let Some(frozen) = state.frozen.as_ref() else {
        return;
    };
    if req.frame_jpeg.is_empty() || req.mode != AnalyzeMode::Discover as i32 {
        return;
    }
    match frozen.detect(&req.frame_jpeg, FROZEN_DETECT_THRESHOLD).await {
        Ok(detections) => {
            tracing::info!(count = detections.len(), "frozen detections");
            response.landmarks =
                diff::merge_landmarks(&response.landmarks, &detections, FROZEN_DETECT_THRESHOLD);
        }
        Err(e) => tracing::warn!(error = %e, "frozen detect failed"),
    }
    match frozen.classify_room(&req.frame_jpeg, SCENE_CLASSES).await {
        Ok(room) => {
            tracing::info!(room = %room.room, confidence = room.confidence, "frozen scene class");
            response.scene_class = room.room;
        }
        Err(e) => tracing::warn!(error = %e, "frozen room classify failed"),
    }
}

/// Minimum OWLv2 score for a detection to count as a landmark.
const FROZEN_DETECT_THRESHOLD: f32 = 0.3;

/// Candidate captions for the frozen CLIP room classification.
const SCENE_CLASSES: &[&str] = &[
    "kitchen",
    "bedroom",
    "living room",
    "office",
    "bathroom",
    "hallway",
    "dining room",
    "garage",
    "attic",
    "basement",
    "balcony",
    "garden",
];

/// Match a scan's vision output against overdue tasks and store reminders.
///
/// Any analyze source can trigger this; camera nodes are simply the
/// hands-free case. Dedupe and delivery live in the store/client.
async fn check_forgotten_tasks(state: &AppState, vision: &[ChoreEntity]) -> Result<(), ApiError> {
    if vision.is_empty() {
        return Ok(());
    }
    let tasks = state.store.list_chores(None).await?;
    let now = lares_core::domain::now_unix();
    for forgotten in lares_core::reminders::match_forgotten(&tasks, vision, now) {
        let reminder = Reminder {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: forgotten.task_id,
            reason: forgotten.reason,
            created_at_unix: now,
            delivered: false,
        };
        state.store.add_reminder(&reminder).await?;
    }
    Ok(())
}

/// Infer which room a frame or landmark set matches best.
///
/// Landmark path (free, instant): with >= 2 landmark labels the scan is
/// matched against per-room landmark profiles accumulated from past scans.
/// Vision path (fallback): the frame is compared against stored room
/// reference images by the engine.
async fn infer_room(
    State(state): State<AppState>,
    Json(req): Json<InferRoomRequest>,
) -> Result<Json<InferRoomResponse>, ApiError> {
    if req.landmarks.len() >= 2 {
        let profiles = state.store.landmark_profiles().await?;
        if let Some(m) = lares_core::rooms::infer_from_landmarks(&req.landmarks, &profiles) {
            let hits = m.evidence.len() as f32;
            return Ok(Json(InferRoomResponse {
                room_id: m.room_id,
                confidence: hits / req.landmarks.len() as f32,
                evidence: m.evidence,
            }));
        }
    }
    if req.frame_jpeg.is_empty() {
        return Err(ApiError::bad_request(
            "frameJpeg is required when landmark inference is unavailable",
        ));
    }
    let references = state.store.list_references().await?;
    if references.is_empty() {
        return Err(ApiError::bad_request(
            "no room references stored — capture one with REF first",
        ));
    }
    let refs_dir = state.refs_dir();
    let mut candidates = Vec::with_capacity(references.len());
    for reference in references {
        let path = refs_dir.join(format!("{}.jpg", reference.image_id));
        let jpeg = tokio::fs::read(path)
            .await
            .map_err(|e| ApiError::internal(format!("reference read: {e}")))?;
        candidates.push(RoomCandidate {
            room_id: reference.room_id,
            description: reference.description,
            jpeg,
        });
    }
    let inference = state.engine.infer_room(req.frame_jpeg, &candidates).await?;
    Ok(Json(InferRoomResponse {
        room_id: inference.room_id,
        confidence: inference.confidence,
        evidence: Vec::new(),
    }))
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


/// Query parameters for wiping chores.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WipeQuery {
    /// Optional room filter; omit to wipe every room.
    pub room_id: Option<String>,
}

/// Delete chores (optionally one room). Returns the number removed.
async fn wipe_chores(
    State(state): State<AppState>,
    Query(query): Query<WipeQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let removed = state.store.wipe_chores(query.room_id.as_deref()).await?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

/// List chores, optionally filtered by room.
async fn list_chores(
    State(state): State<AppState>,
    Query(query): Query<ChoresQuery>,
) -> Result<Json<ListChoresResponse>, ApiError> {
    let chores = state.store.list_chores(query.room_id.as_deref()).await?;
    Ok(Json(ListChoresResponse { chores }))
}

/// Transition a chore's lifecycle state, respawning recurring tasks.
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
    spawn_next_recurrence(&state, &chore).await?;
    Ok(Json(chore))
}

/// Spawn the next instance of a completed recurring task.
///
/// The completed instance stays as DONE history; the next one is a fresh
/// entity with a new id and a due date one period after the previous due.
async fn spawn_next_recurrence(state: &AppState, chore: &ChoreEntity) -> Result<(), ApiError> {
    let is_done_task = chore.status == ChoreStatus::Done as i32
        && chore.kind == ChoreKind::Task as i32;
    if !is_done_task {
        return Ok(());
    }
    let Some(recurrence) = chore.recurrence.as_ref() else {
        return Ok(());
    };
    let freq = RecurrenceFreq::try_from(recurrence.freq);
    let step_days = match freq {
        Ok(RecurrenceFreq::Weekly) => 7,
        Ok(RecurrenceFreq::Biweekly) => 14,
        _ => return Ok(()),
    };
    let base_due = chore.due_at_unix.unwrap_or_else(lares_core::domain::now_unix);
    let mut next = ChoreEntity {
        id: uuid::Uuid::new_v4().to_string(),
        room_id: chore.room_id.clone(),
        target: chore.target.clone(),
        action: chore.action.clone(),
        estimated_seconds: chore.estimated_seconds,
        status: ChoreStatus::Discovered as i32,
        kind: ChoreKind::Task as i32,
        due_at_unix: Some(base_due + i64::from(step_days) * 86_400),
        recurrence: chore.recurrence,
        context_tags: chore.context_tags.clone(),
        ..Default::default()
    };
    crate::people::apply_task_defaults(&mut next);
    state.store.upsert_chores(std::slice::from_ref(&next)).await?;
    Ok(())
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

/// A list of expected object labels.
#[derive(Debug, serde::Serialize)]
struct ExpectedResponse {
    labels: Vec<String>,
}

/// Request body for adding an expected object.
#[derive(Debug, serde::Deserialize)]
struct AddExpectedRequest {
    label: String,
}

/// List expected object labels for a room.
async fn list_expected(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> Result<Json<ExpectedResponse>, ApiError> {
    let labels = state.store.get_expected(&room_id).await?;
    Ok(Json(ExpectedResponse { labels }))
}

/// Add an expected object label to a room.
async fn add_expected(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(req): Json<AddExpectedRequest>,
) -> Result<StatusCode, ApiError> {
    let label = req.label.trim().to_string();
    if label.is_empty() {
        return Err(ApiError::bad_request("label is required"));
    }
    state.store.add_expected(&room_id, &label).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove an expected object label from a room.
async fn remove_expected(
    State(state): State<AppState>,
    Path((room_id, label)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    state.store.remove_expected(&room_id, &label).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Request body for the phone to post ephemeral scan-target state.
#[derive(Debug, serde::Deserialize)]
pub struct HudStateRequest {
    /// The room the phone inferred for the current location.
    pub room_id: String,
    /// Number of scan-targets the client is currently tracking.
    pub target_count: u32,
}

/// Phone posts live scan-target count after each scan (fire-and-forget).
async fn hud_post_state(
    State(state): State<AppState>,
    Json(req): Json<HudStateRequest>,
) -> Result<StatusCode, ApiError> {
    let mut hud = state.hud_state.lock().await;
    hud.room_id = req.room_id;
    hud.target_count = req.target_count;
    hud.updated_at_unix = now_unix();
    Ok(StatusCode::NO_CONTENT)
}

/// Minimal HUD payload for the ESP32 firmware to poll every few seconds.
async fn hud_get(State(state): State<AppState>) -> Result<Json<HudResponse>, ApiError> {
    let hud = state.hud_state.lock().await.clone();
    let chores = state.store.list_chores(None).await?;
    let today = chrono::Local::now().date_naive();
    let done = ChoreStatus::Done as i32;
    let dismissed = ChoreStatus::Dismissed as i32;
    let task = ChoreKind::Task as i32;
    let horizon = 30_i64;
    let mut tasks: Vec<HudTask> = chores
        .iter()
        .filter(|c| c.kind == task && c.status != done && c.status != dismissed)
        .filter_map(|c| {
            let due = c.due_at_unix?;
            let date = chrono::DateTime::from_timestamp(due, 0)?.date_naive();
            let days = (date - today).num_days();
            if days > horizon {
                return None;
            }
            let mut title = c.action.clone();
            title.truncate(40);
            Some(HudTask { title, days: days as i32 })
        })
        .collect();
    tasks.sort_by_key(|t| t.days);
    let next_task = tasks.first().cloned();
    tasks.truncate(3);
    Ok(Json(HudResponse {
        room: hud.room_id,
        targets: hud.target_count,
        next_task,
        tasks,
        updated_at: hud.updated_at_unix,
    }))
}

/// Fast ambient tick: frozen models only, no VLM.
///
/// Detects messable objects and surfaces, scores relations between them,
/// judges each against the learned norm table, and checks open chores'
/// expected relations for completion. Candidates ride ChoreEntity shape so
/// the client's ephemeral HUD pipeline renders them unchanged. Requires
/// LARES_VISION_ENDPOINT; the client falls back to VLM scans otherwise.
async fn tick(
    State(state): State<AppState>,
    Json(req): Json<TickRequest>,
) -> Result<Json<TickResponse>, ApiError> {
    let Some(frozen) = state.frozen.as_ref() else {
        return Err(ApiError::bad_request("frozen vision not configured"));
    };
    if req.frame_jpeg.is_empty() {
        return Err(ApiError::bad_request("frameJpeg is required"));
    }
    let detections = frozen
        .detect_queries(&req.frame_jpeg, lares_core::engine::frozen::TICK_QUERIES, 0.3)
        .await
        .map_err(ApiError::internal)?;
    let (labels, boxes) = lares_core::engine::frozen::tick_inputs(&detections);
    let relations = if labels.len() >= 2 {
        frozen
            .relate(
                &req.frame_jpeg,
                &labels,
                &boxes,
                lares_core::engine::frozen::RELATION_VOCABULARY,
            )
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .map(|t| lares_core::norms::Relation {
                subject: t.subject,
                predicate: t.predicate,
                object: t.object,
                score: t.score,
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let norm_rows = state.store.list_norms().await?;
    let unknowns = lares_core::norms::unknown_combos(&norm_rows, &relations);
    let mut judged = lares_core::norms::judge(&norm_rows, &relations);
    judged.sort_by(|a, b| b.relation.score.total_cmp(&a.relation.score));
    judged.truncate(MAX_TICK_CANDIDATES);

    // Teach the norm table asynchronously for unseen suspicious combos;
    // capped per tick so a novel room cannot stampede the LLM.
    if let Ok(endpoint) = std::env::var("LARES_LOCAL_ENDPOINT") {
        let model = std::env::var("LARES_MODEL").unwrap_or_else(|_| "qwen2.5-vl".into());
        for combo in unknowns.into_iter().take(2) {
            let store = state.store.clone();
            let http = reqwest::Client::new();
            let endpoint = endpoint.clone();
            let model = model.clone();
            tokio::spawn(async move {
                if let Some((verdict, action)) = lares_core::norms::judge_norm_via_llm(
                    &http, &endpoint, &model, &combo.subject, &combo.object,
                )
                .await
                {
                    let norm = lares_core::domain::Norm {
                        object_class: combo.subject,
                        place: combo.object,
                        verdict,
                        action_hint: action,
                        source: "llm".into(),
                        updated_at: now_unix(),
                    };
                    let _ = store.upsert_norm(&norm).await;
                }
            });
        }
    }

    let mut candidates = Vec::with_capacity(judged.len());
    for entry in &judged {
        let subject = entry.relation.subject.clone();
        let subject_box = labels
            .iter()
            .position(|l| *l == subject)
            .map(|i| boxes[i]);
        let action = if entry.norm.action_hint.is_empty() {
            format!("Put the {subject} away")
        } else {
            entry.norm.action_hint.clone()
        };
        candidates.push(ChoreEntity {
            id: format!("tick:{}", uuid::Uuid::new_v4()),
            room_id: req.room_id.clone(),
            target: subject,
            action,
            kind: ChoreKind::Task as i32,
            status: ChoreStatus::Discovered as i32,
            confidence: entry.relation.score,
            r#box: subject_box.map(|b| BoundingBox {
                ymin: b[0],
                xmin: b[1],
                ymax: b[2],
                xmax: b[3],
            }),
            estimated_seconds: 20,
            ..Default::default()
        });
    }

    let chores = state.store.list_chores(None).await?;
    let completions = lares_core::norms::find_completions(&chores, &relations)
        .into_iter()
        .map(|m| CompletionCandidate { chore_id: m.chore_id, action: m.action })
        .collect();

    // Novelty audit: a confident detection whose bare label was never
    // stored in this room's landmark profile.
    let stored = state.store.get_landmarks(&req.room_id).await?;
    let known: std::collections::HashSet<String> =
        stored.iter().map(|l| l.label.to_lowercase()).collect();
    let audit_recommended = detections.iter().any(|d| {
        let label = lares_core::engine::frozen::bare_label(&d.label);
        d.score >= 0.5 && label != "floor" && !known.contains(&label)
    });

    let scene_class = frozen
        .classify_room(&req.frame_jpeg, SCENE_CLASSES)
        .await
        .map(|r| r.room)
        .unwrap_or_default();

    Ok(Json(TickResponse {
        room_id: req.room_id,
        scene_class,
        candidates,
        completions,
        audit_recommended,
    }))
}

/// Maximum misplaced-object candidates a single tick may surface.
const MAX_TICK_CANDIDATES: usize = 4;

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
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: message.into() }
    }

    /// Build a 404 error.
    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_FOUND, message: message.into() }
    }

    /// Build a 500 error.
    pub(crate) fn internal(message: impl Into<String>) -> Self {
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