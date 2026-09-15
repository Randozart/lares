//! Deterministic post-processing of raw engine output.
//!
//! Normalizes identity, clamps boxes, drops degenerate or low-confidence
//! entries, collapses near-duplicates, and guarantees every chore carries at
//! least one atomic subtask. Pure and unit-tested without any network.

use std::collections::HashMap;

use crate::domain::{
    box_is_degenerate, center_cell, clamp_box, new_chore_id, now_unix, ChoreEntity, ChoreStatus,
    DEFAULT_MIN_CONFIDENCE,
};

/// Grid cell size (in normalized units) for near-duplicate collapsing.
const DEDUPE_CELL: i32 = 120;

/// Maximum number of chores returned per scan.
const MAX_CHORES: usize = 6;

/// Post-process raw engine chores into final, stable entities.
///
/// Applies [`normalize_chore`], retention filters, near-duplicate collapsing,
/// and caps the result at [`MAX_CHORES`] by descending confidence.
pub fn postprocess(
    mut chores: Vec<ChoreEntity>,
    room_id: &str,
    room_area: i32,
) -> Vec<ChoreEntity> {
    let now = now_unix();
    for chore in &mut chores {
        normalize_chore(chore, room_id, room_area, now);
    }
    chores.retain(keep_chore);
    let mut collapsed = collapse_near_duplicates(chores);
    collapsed.truncate(MAX_CHORES);
    collapsed
}

/// Normalize one chore in place: identity, room, status, box, subtasks, time.
fn normalize_chore(chore: &mut ChoreEntity, room_id: &str, room_area: i32, now: i64) {
    if chore.id.is_empty() {
        chore.id = new_chore_id();
    }
    if chore.room_id.is_empty() {
        chore.room_id = room_id.to_string();
    }
    if chore.room_area == 0 {
        chore.room_area = room_area;
    }
    if chore.status == ChoreStatus::Unspecified as i32 {
        chore.status = ChoreStatus::Discovered as i32;
    }
    if let Some(bbox) = chore.r#box.as_mut() {
        clamp_box(bbox);
    }
    if chore.subtasks.is_empty() && !chore.action.is_empty() {
        chore.subtasks.push(chore.action.clone());
    }
    if chore.how_to.is_empty() {
        chore.how_to = vec![format!("Do the chore: {}", chore.action)];
    }
    chore.last_seen_unix = Some(now);
}

/// Whether a chore survives post-processing retention filters.
fn keep_chore(chore: &ChoreEntity) -> bool {
    if chore.action.trim().is_empty() {
        return false;
    }
    if chore.confidence < DEFAULT_MIN_CONFIDENCE {
        return false;
    }
    match chore.r#box.as_ref() {
        Some(bbox) => !box_is_degenerate(bbox),
        None => true,
    }
}

/// Collapse near-duplicate chores sharing a target and coarse center cell.
///
/// Keeps the highest-confidence representative of each key. Single pass over
/// the input, so it is O(n) and deterministic.
fn collapse_near_duplicates(chores: Vec<ChoreEntity>) -> Vec<ChoreEntity> {
    let mut best: HashMap<(String, i32, i32), ChoreEntity> = HashMap::new();
    for chore in chores {
        let key = dedupe_key(&chore);
        let superseded = match best.get(&key) {
            Some(existing) => existing.confidence >= chore.confidence,
            None => false,
        };
        if !superseded {
            best.insert(key, chore);
        }
    }
    let mut out: Vec<ChoreEntity> = best.into_values().collect();
    out.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    out
}

/// Compute the near-duplicate key for a chore.
///
/// Boxed chores key on `(target, center cell)`; unboxed chores key on target
/// alone with a sentinel cell.
fn dedupe_key(chore: &ChoreEntity) -> (String, i32, i32) {
    let target = chore.target.trim().to_lowercase();
    match chore.r#box.as_ref() {
        Some(bbox) => {
            let (row, col) = center_cell(bbox, DEDUPE_CELL);
            (target, row, col)
        }
        None => (target, i32::MIN, i32::MIN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::BoundingBox;

    /// Build a chore for testing.
    fn chore(target: &str, confidence: f32, box_: Option<BoundingBox>) -> ChoreEntity {
        ChoreEntity {
            target: target.to_string(),
            action: format!("do {target}"),
            confidence,
            r#box: box_,
            ..Default::default()
        }
    }

    /// A valid, non-degenerate box centered in the middle of the frame.
    fn mid_box() -> BoundingBox {
        BoundingBox { ymin: 200, xmin: 200, ymax: 500, xmax: 500 }
    }

    #[test]
    fn assigns_identity_and_room() {
        let out = postprocess(vec![chore("socks", 0.9, Some(mid_box()))], "bedroom", 0);
        assert_eq!(out.len(), 1);
        assert!(!out[0].id.is_empty());
        assert_eq!(out[0].room_id, "bedroom");
        assert_eq!(out[0].status, ChoreStatus::Discovered as i32);
        assert_eq!(out[0].subtasks, vec!["do socks".to_string()]);
        assert!(out[0].last_seen_unix.is_some());
    }

    #[test]
    fn drops_low_confidence_and_degenerate_boxes() {
        let low_conf = chore("low", 0.1, Some(mid_box()));
        let degenerate = chore("bad", 0.9, Some(BoundingBox { ymin: 500, xmin: 500, ymax: 500, xmax: 500 }));
        let empty_action = {
            let mut c = chore("none", 0.9, Some(mid_box()));
            c.action.clear();
            c
        };
        let out = postprocess(vec![low_conf, degenerate, empty_action], "kitchen", 0);
        assert!(out.is_empty());
    }

    #[test]
    fn clamps_out_of_range_boxes() {
        let mut c = chore("mess", 0.9, Some(BoundingBox { ymin: -50, xmin: 10, ymax: 2000, xmax: 500 }));
        normalize_chore(&mut c, "room", 0, 0);
        let box_ = c.r#box.unwrap();
        assert!(box_.ymin >= 0 && box_.ymax <= 1000);
    }

    #[test]
    fn collapses_near_duplicates_keeping_highest_confidence() {
        let a = chore("socks", 0.5, Some(mid_box()));
        let b = chore("socks", 0.95, Some(mid_box()));
        let out = postprocess(vec![a, b], "bedroom", 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].confidence, 0.95);
    }

    #[test]
    fn keeps_distinct_targets_in_same_cell() {
        let socks = chore("socks", 0.9, Some(mid_box()));
        let mugs = chore("mugs", 0.9, Some(mid_box()));
        let out = postprocess(vec![socks, mugs], "kitchen", 0);
        assert_eq!(out.len(), 2);
    }
}