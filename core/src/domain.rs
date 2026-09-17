//! Domain helpers and re-exports over the generated protobuf contract.

pub use crate::lares::v1::{
    AnalyzeMode, AnalyzeSceneRequest, AnalyzeSceneResponse, AnalyzeSource, Briefing, BriefingItem,
    BriefingItemKind, BoundingBox, ChoreEntity, ChoreKind, ChoreStatus, FingerprintKind,
    FingerprintRecord, IdleContext, ImportCalendarRequest, ImportCalendarResponse, Landmark,
    LandmarkList, LeadFlag, ListChoresResponse, ListFingerprintsResponse, Nudge, NudgeRequest,
    NudgeResponse, Occasion, OccasionKind, OccasionList, Person, PersonList, Preparation,
    PreparationKind, PreparationList, PreparationState, Recurrence, RecurrenceFreq,
    ReferenceState, Reminder, ReminderList, RoomArea, SetChoreStatusRequest,
    SetFingerprintRequest, SetReferenceRequest,
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