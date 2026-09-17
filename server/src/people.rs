//! People, occasions, calendar import, briefing, and manual task chores.
//!
//! The proactive household layer: birthdays and dated tasks drive the daily
//! briefing; a shared Google Calendar ICS URL is the primary data source.

use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};

use lares_core::briefing;
use lares_core::domain::{
    ChoreEntity, ChoreKind, ChoreStatus, Briefing, ImportCalendarRequest, ImportCalendarResponse,
    Occasion, OccasionList, OccasionKind, Person, PersonList,
};
use lares_core::ics;

use crate::routes::ApiError;
use crate::state::AppState;

/// Default briefing horizon when the client does not specify one.
const DEFAULT_HORIZON_DAYS: i32 = 7;

/// Namespace for deterministic occasion ids, so calendar re-imports update
/// instead of duplicating.
const OCCASION_NAMESPACE: [u8; 16] = *b"LaresOccasionV1!";

/// Build the people/occasions/briefing router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/people", get(list_people).post(add_person))
        .route("/v1/people/{id}", delete(delete_person))
        .route("/v1/occasions", get(list_occasions).post(add_occasion))
        .route("/v1/occasions/{id}", delete(delete_occasion))
        .route("/v1/chores", post(create_chore))
        .route("/v1/calendar/import", post(import_calendar))
        .route("/v1/briefing", get(briefing_handler))
        .with_state(state)
}

/// Query parameters for the briefing.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefingQuery {
    /// How many days ahead to include (default 7).
    pub horizon_days: Option<u32>,
}

/// List all known people.
async fn list_people(State(state): State<AppState>) -> Result<Json<PersonList>, ApiError> {
    let people = state.store.list_people().await?;
    Ok(Json(PersonList { people }))
}

/// Add a person; the server assigns the id.
async fn add_person(
    State(state): State<AppState>,
    Json(mut person): Json<Person>,
) -> Result<Json<Person>, ApiError> {
    if person.name.trim().is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    if person.id.is_empty() {
        person.id = uuid::Uuid::new_v4().to_string();
    }
    let stored = state.store.upsert_person(&person).await?;
    Ok(Json(stored))
}

/// Delete a person by id.
async fn delete_person(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.store.delete_person(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List all occasions.
async fn list_occasions(State(state): State<AppState>) -> Result<Json<OccasionList>, ApiError> {
    let occasions = state.store.list_occasions().await?;
    Ok(Json(OccasionList { occasions }))
}

/// Add an occasion; the server assigns the id.
async fn add_occasion(
    State(state): State<AppState>,
    Json(mut occasion): Json<Occasion>,
) -> Result<Json<Occasion>, ApiError> {
    if occasion.title.trim().is_empty() || occasion.date.trim().is_empty() {
        return Err(ApiError::bad_request("title and date are required"));
    }
    if occasion.id.is_empty() {
        occasion.id = uuid::Uuid::new_v4().to_string();
    }
    if occasion.kind == OccasionKind::Unspecified as i32 {
        occasion.kind = OccasionKind::Custom as i32;
    }
    let stored = state.store.upsert_occasion(&occasion).await?;
    Ok(Json(stored))
}

/// Delete an occasion by id.
async fn delete_occasion(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.store.delete_occasion(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Create a manual TASK or REMINDER chore (no vision box required).
async fn create_chore(
    State(state): State<AppState>,
    Json(mut chore): Json<ChoreEntity>,
) -> Result<Json<ChoreEntity>, ApiError> {
    if chore.action.trim().is_empty() {
        return Err(ApiError::bad_request("action is required"));
    }
    if chore.id.is_empty() {
        chore.id = uuid::Uuid::new_v4().to_string();
    }
    if chore.status == ChoreStatus::Unspecified as i32 {
        chore.status = ChoreStatus::Discovered as i32;
    }
    if chore.kind == ChoreKind::Unspecified as i32 {
        chore.kind = ChoreKind::Task as i32;
    }
    if chore.subtasks.is_empty() {
        chore.subtasks.push(chore.action.clone());
    }
    if chore.how_to.is_empty() {
        chore.how_to.push(chore.action.clone());
    }
    chore.last_seen_unix = Some(lares_core::domain::now_unix());
    state.store.upsert_chores(std::slice::from_ref(&chore)).await?;
    Ok(Json(chore))
}

/// Fetch an ICS calendar and merge its events into people and occasions.
async fn import_calendar(
    State(state): State<AppState>,
    Json(req): Json<ImportCalendarRequest>,
) -> Result<Json<ImportCalendarResponse>, ApiError> {
    if req.url.trim().is_empty() {
        return Err(ApiError::bad_request("url is required"));
    }
    let body = fetch_ics(&req.url).await?;
    let events = ics::parse_ics(&body);
    let mut imported: u32 = 0;
    for event in events {
        import_event(&state, &event).await?;
        imported += 1;
    }
    state.store.record_calendar_sync(req.url.trim()).await?;
    let people = state.store.list_people().await?;
    Ok(Json(ImportCalendarResponse {
        imported,
        people: people.len() as u32,
    }))
}

/// Fetch an ICS document body with a bounded timeout.
async fn fetch_ics(url: &str) -> Result<String, ApiError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| ApiError::internal(format!("http client: {e}")))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("calendar fetch: {e}")))?;
    if !response.status().is_success() {
        return Err(ApiError::internal(format!(
            "calendar fetch http {}",
            response.status()
        )));
    }
    response
        .text()
        .await
        .map_err(|e| ApiError::internal(format!("calendar body: {e}")))
}

/// Import one parsed event: derive person, classify, and upsert.
async fn import_event(state: &AppState, event: &ics::IcsEvent) -> Result<(), ApiError> {
    let kind = briefing::occasion_kind(&event.summary, event.yearly);
    let person_id = match ics::person_name_from_title(&event.summary) {
        Some(name) => ensure_person(state, &name).await?,
        None => String::new(),
    };
    let occasion = Occasion {
        id: occasion_identity(&event.summary, &event.date),
        person_id,
        title: event.summary.clone(),
        date: event.date.clone(),
        kind: kind as i32,
        notes: String::new(),
    };
    state.store.upsert_occasion(&occasion).await?;
    Ok(())
}

/// Find a person by name (case-insensitive) or create them.
///
/// Family-scale lists make the per-event linear scan appropriate.
async fn ensure_person(state: &AppState, name: &str) -> Result<String, ApiError> {
    let people = state.store.list_people().await?;
    if let Some(existing) = people
        .iter()
        .find(|person| person.name.eq_ignore_ascii_case(name))
    {
        return Ok(existing.id.clone());
    }
    let person = Person {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        notes: String::new(),
    };
    state.store.upsert_person(&person).await?;
    Ok(person.id)
}

/// Deterministic occasion id from title and date, so re-imports update
/// existing rows instead of duplicating them.
fn occasion_identity(title: &str, date: &str) -> String {
    let namespace = uuid::Uuid::from_bytes(OCCASION_NAMESPACE);
    uuid::Uuid::new_v5(&namespace, format!("{title}|{date}").as_bytes()).to_string()
}

/// Compute the proactive briefing: occasions and due tasks in the horizon.
async fn briefing_handler(
    State(state): State<AppState>,
    Query(query): Query<BriefingQuery>,
) -> Result<Json<Briefing>, ApiError> {
    let today = chrono::Local::now().date_naive();
    let occasions = state.store.list_occasions().await?;
    let chores = state.store.list_chores(None).await?;
    let horizon = query.horizon_days.unwrap_or(DEFAULT_HORIZON_DAYS as u32) as i32;
    let done = ChoreStatus::Done as i32;
    let dismissed = ChoreStatus::Dismissed as i32;
    let task = ChoreKind::Task as i32;
    let active: Vec<ChoreEntity> = chores
        .into_iter()
        .filter(|chore| chore.kind == task && chore.status != done && chore.status != dismissed)
        .collect();
    Ok(Json(briefing::build(
        &occasions, &active, today, horizon,
    )))
}
