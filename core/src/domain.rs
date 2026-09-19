//! Domain helpers and re-exports over the generated protobuf contract.

pub use crate::lares::v1::{
    AnalyzeMode, AnalyzeSceneRequest, AnalyzeSceneResponse, AnalyzeSource, Briefing, BriefingItem,
    BriefingItemKind, BoundingBox, ChoreEntity, ChoreKind, ChoreStatus, CompletionCandidate,
    FingerprintKind, FingerprintRecord, IdleContext, ImportCalendarRequest, ImportCalendarResponse,
    InferRoomRequest, InferRoomResponse, Landmark, LandmarkList, LeadFlag, ListChoresResponse,
    ListFingerprintsResponse, Nudge, NudgeRequest, NudgeResponse, Occasion, OccasionKind,
    OccasionList, Person, PersonList, Preparation, PreparationKind, PreparationList,
    PreparationState, Recurrence, RecurrenceFreq, ReferenceState, Reminder, ReminderList,
    RoomArea, SetChoreStatusRequest, SetFingerprintRequest, SetReferenceRequest, TickRequest,
    TickResponse,
};

/// Lower bound of normalized box coordinates.
pub const BOX_MIN: i32 = 0;
/// Upper bound of normalized box coordinates.
pub const BOX_MAX: i32 = 1000;
/// Minimum confidence for a chore to survive post-processing.
pub const DEFAULT_MIN_CONFIDENCE: f32 = 0.45;

impl FingerprintKind {
    /// Persisted string form of the fingerprint kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            FingerprintKind::Clean => "clean",
            FingerprintKind::History => "history",
            _ => "latest",
        }
    }

    /// Parse a persisted string form back into a kind.
    pub fn parse(value: &str) -> Self {
        match value {
            "clean" => FingerprintKind::Clean,
            "history" => FingerprintKind::History,
            _ => FingerprintKind::Latest,
        }
    }
}

/// Current unix time in whole seconds, saturating at zero on clock error.
pub fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() as i64,
        Err(_) => 0,
    }
}

/// Clamp a box's coordinates into the normalized 0..=1000 range.
pub fn clamp_box(b: &mut BoundingBox) {
    b.ymin = b.ymin.clamp(BOX_MIN, BOX_MAX);
    b.xmin = b.xmin.clamp(BOX_MIN, BOX_MAX);
    b.ymax = b.ymax.clamp(BOX_MIN, BOX_MAX);
    b.xmax = b.xmax.clamp(BOX_MIN, BOX_MAX);
}

/// Whether a box has zero or negative area (invalid after clamping).
pub fn box_is_degenerate(b: &BoundingBox) -> bool {
    b.ymax <= b.ymin || b.xmax <= b.xmin
}

/// Generate a fresh chore identifier.
pub fn new_chore_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Live HUD state posted by the phone after each scan.
#[derive(Debug, Clone, Default)]
pub struct HudState {
    /// Last room the phone inferred for the current location.
    pub room_id: String,
    /// Number of ephemeral scan-targets currently tracked by the client.
    pub target_count: u32,
    /// Titles of the current ephemeral targets (max ~4), for VISION tags.
    pub targets: Vec<String>,
    /// Unix timestamp of the last post (to detect stale data).
    pub updated_at_unix: i64,
}

/// One tag rendered on head-mounted/external HUD clients.
///
/// Tags are the only HUD content: live observations and (later) overheard
/// requests. Stored registry chores never appear on a HUD.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HudTag {
    /// Stable id so clients can animate fades instead of redrawing.
    pub id: String,
    /// Source of the tag: "VISION" today; "REQUEST"/"MEETING" later.
    pub kind: String,
    /// Short display title.
    pub title: String,
    /// Optional detail line (empty for vision tags).
    pub snippet: String,
    /// Seconds since the tag was produced.
    pub age: i64,
}

/// Synthesize VISION tags from the phone's posted target titles.
///
/// Pure helper over the ephemeral HUD state; tags inherit their age from
/// the last state post so clients can dim stale data. At most `max` tags,
/// titles truncated for the narrow waveguide.
pub fn synthesize_vision_tags(
    targets: &[String],
    updated_at_unix: i64,
    now_unix: i64,
    max: usize,
) -> Vec<HudTag> {
    targets
        .iter()
        .take(max)
        .filter(|t| !t.trim().is_empty())
        .enumerate()
        .map(|(i, title)| {
            let mut trimmed = title.trim().to_string();
            if trimmed.len() > 24 {
                trimmed.truncate(24);
            }
            HudTag {
                id: format!("vision:{}:{}", i, trimmed.to_lowercase()),
                kind: "VISION".into(),
                title: trimmed,
                snippet: String::new(),
                age: (now_unix - updated_at_unix).max(0),
            }
        })
        .collect()
}

/// A storage norm: whether an object class belongs at a place.
///
/// Learned from seeds, user corrections, and cached LLM verdicts; the
/// frozen-loop judge consults it instead of running a language model
/// every tick.
#[derive(Debug, Clone)]
pub struct Norm {
    /// Object class, lowercased bare noun ("cup", "dirty dishes").
    pub object_class: String,
    /// Place the object was observed at ("floor", "sink", "sofa").
    pub place: String,
    /// "GOOD" (belongs there) or "BAD" (misplaced — task candidate).
    pub verdict: String,
    /// Directive template for BAD verdicts, e.g. "Put the cup in the sink".
    pub action_hint: String,
    /// Provenance: "seed", "user", or "llm".
    pub source: String,
    /// Unix timestamp of the last write.
    pub updated_at: i64,
}

/// The lightweight JSON payload the HUD firmware polls every few seconds.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HudResponse {
    /// Last posted room (empty if no scan has fired yet).
    pub room: String,
    /// Live scan-target count from the phone.
    pub targets: u32,
    /// The next due task (title + days until), if any.
    pub next_task: Option<HudTask>,
    /// Up to 3 top-priority tasks for the mini ticker.
    pub tasks: Vec<HudTask>,
    /// Unix timestamp of the last state update.
    pub updated_at: i64,
    /// Live tags for head-mounted clients (VISION now; REQUEST later).
    pub tags: Vec<HudTag>,
}

/// A single task line in the HUD payload.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HudTask {
    /// Short title (truncated to 40 chars).
    pub title: String,
    /// Days until due (negative = overdue).
    pub days: i32,
}

/// Coarse grid cell for a box center, used for near-duplicate collapsing.
///
/// Returns `(row, col)` in units of `cell`. Guards against a zero cell size.
pub fn center_cell(b: &BoundingBox, cell: i32) -> (i32, i32) {
    let cy = (b.ymin + b.ymax) / 2;
    let cx = (b.xmin + b.xmax) / 2;
    let size = cell.max(1);
    (cy / size, cx / size)
}

/// Human-readable display name for a room area.
pub fn area_display_name(area: RoomArea) -> &'static str {
    match area {
        RoomArea::Kitchen => "kitchen",
        RoomArea::Bathroom => "bathroom",
        RoomArea::Bedroom => "bedroom",
        RoomArea::LivingRoom => "living room",
        RoomArea::DiningRoom => "dining room",
        RoomArea::Office => "office",
        RoomArea::Garage => "garage",
        RoomArea::Laundry => "laundry",
        RoomArea::Hallway => "hallway",
        RoomArea::KidsRoom => "kids room",
        RoomArea::Patio => "patio",
        RoomArea::Other => "room",
        _ => "room",
    }
}
#[cfg(test)]
mod domain_tests {
    use super::*;

    #[test]
    fn vision_tags_synthesize_with_stable_ids() {
        let tags = synthesize_vision_tags(
            &["socks".into(), "cup".into()],
            1000,
            1060,
            4,
        );
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].id, "vision:0:socks");
        assert_eq!(tags[0].kind, "VISION");
        assert_eq!(tags[0].age, 60);
    }

    #[test]
    fn vision_tags_truncate_and_cap() {
        let targets = vec![
            "a very long target title exceeding the waveguide width".into(),
            "socks".into(),
            "cup".into(),
            "book".into(),
            "plate".into(),
            "extra".into(),
        ];
        let tags = synthesize_vision_tags(&targets, 100, 100, 4);
        assert_eq!(tags.len(), 4);
        assert_eq!(tags[0].title.len(), 24);
    }

    #[test]
    fn empty_targets_yield_no_tags() {
        assert!(synthesize_vision_tags(&[], 0, 100, 4).is_empty());
        assert!(synthesize_vision_tags(&["  ".into()], 0, 100, 4).is_empty());
    }
}
