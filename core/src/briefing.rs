//! Briefing computation: merge occasions and dated tasks into a proactive
//! digest with lead-time flags.
//!
//! Pure date logic over [`chrono::NaiveDate`]; the caller supplies today's
//! date so the whole module is deterministically unit-testable.

use crate::domain::{
    Briefing, BriefingItem, BriefingItemKind, ChoreEntity, ChoreKind, LeadFlag, Occasion,
    OccasionKind, Preparation, PreparationKind, PreparationState,
};
use chrono::Datelike;
use std::collections::{HashMap, HashSet};

/// Default lead time (days before an occasion) to order a gift.
pub const GIFT_LEAD_DAYS: i64 = 14;
/// Default lead time (days before an occasion) to put up decorations.
pub const DECOR_LEAD_DAYS: i64 = 7;
/// Default lead time (days before an occasion) to order a cake.
pub const CAKE_LEAD_DAYS: i64 = 2;
/// Default lead time (days before an occasion) to get a card.
pub const CARD_LEAD_DAYS: i64 = 1;
/// Default lead time (days before an occasion) to clean.
pub const CLEANING_LEAD_DAYS: i64 = 1;

/// Compute the next occurrence of an occasion date from `today`.
///
/// Yearly "MM-DD" dates roll to next year once passed; one-time "YYYY-MM-DD"
/// dates return `None` once past. Returns `(date, is_yearly)`.
pub fn next_occurrence(date: &str, today: chrono::NaiveDate) -> Option<(chrono::NaiveDate, bool)> {
    if let Ok(day) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        if day >= today {
            return Some((day, false));
        }
        return None;
    }
    let md = chrono::NaiveDate::parse_from_str(&format!("2000-{date}"), "%Y-%m-%d").ok()?;
    let mut candidate = chrono::NaiveDate::from_ymd_opt(today.year(), md.month(), md.day())
        .or_else(|| chrono::NaiveDate::from_ymd_opt(today.year(), md.month(), md.day().saturating_sub(1)))?;
    if candidate < today {
        candidate = chrono::NaiveDate::from_ymd_opt(today.year() + 1, md.month(), md.day())
            .or_else(|| chrono::NaiveDate::from_ymd_opt(today.year() + 1, md.month(), 28))?;
    }
    Some((candidate, true))
}

/// Compute lead-time flags for a yearly occasion `days` days out.
fn lead_flags(days: i64, is_yearly: bool) -> Vec<i32> {
    let mut flags = Vec::new();
    if !is_yearly {
        return flags;
    }
    let candidates: [(i64, LeadFlag); 5] = [
        (GIFT_LEAD_DAYS, LeadFlag::Gift),
        (DECOR_LEAD_DAYS, LeadFlag::Decor),
        (CAKE_LEAD_DAYS, LeadFlag::Cake),
        (CARD_LEAD_DAYS, LeadFlag::Card),
        (CLEANING_LEAD_DAYS, LeadFlag::Cleaning),
    ];
    for (lead, flag) in candidates {
        if days <= lead {
            flags.push(flag as i32);
        }
    }
    flags
}

/// Map a preparation kind to its briefing lead flag, when one exists.
fn kind_flag(kind: i32) -> Option<LeadFlag> {
    match PreparationKind::try_from(kind) {
        Ok(PreparationKind::Gift) => Some(LeadFlag::Gift),
        Ok(PreparationKind::Cake) => Some(LeadFlag::Cake),
        Ok(PreparationKind::Card) => Some(LeadFlag::Card),
        Ok(PreparationKind::Decor) => Some(LeadFlag::Decor),
        Ok(PreparationKind::Cleaning) => Some(LeadFlag::Cleaning),
        _ => None,
    }
}

/// Occasion ids with kinds whose preparations are READY or DONE.
///
/// Such preparations silence the matching lead flag on that occasion.
fn suppression_map(preparations: &[Preparation]) -> HashMap<String, HashSet<i32>> {
    let mut map: HashMap<String, HashSet<i32>> = HashMap::new();
    for preparation in preparations {
        let resolved = preparation.state == PreparationState::Ready as i32
            || preparation.state == PreparationState::Done as i32;
        if !resolved || preparation.occasion_id.is_empty() {
            continue;
        }
        if let Some(flag) = kind_flag(preparation.kind) {
            map.entry(preparation.occasion_id.clone())
                .or_default()
                .insert(flag as i32);
        }
    }
    map
}

/// Convert a due/overdue TASK chore into a briefing item, if in horizon.
fn task_briefing_item(
    task: &ChoreEntity,
    today: chrono::NaiveDate,
    horizon: i64,
) -> Option<BriefingItem> {
    if task.kind != ChoreKind::Task as i32 {
        return None;
    }
    let due = task.due_at_unix?;
    let date = chrono::DateTime::from_timestamp(due, 0)?.date_naive();
    let days = (date - today).num_days();
    if days > horizon {
        return None;
    }
    Some(BriefingItem {
        title: task.action.clone(),
        days_until: days as i32,
        due_date: date.format("%Y-%m-%d").to_string(),
        kind: BriefingItemKind::Task as i32,
        flags: Vec::new(),
    })
}

/// Build the briefing: occasions and due tasks within `horizon_days`.
///
/// Tasks with overdue due dates are included with negative `days_until`.
/// Lead flags fire per kind (gift 14d, decor 7d, cake 2d, card/cleaning 1d)
/// and are suppressed per occasion when a matching preparation is resolved.
/// Items are sorted by `days_until` ascending.
pub fn build(
    occasions: &[Occasion],
    tasks: &[ChoreEntity],
    preparations: &[Preparation],
    today: chrono::NaiveDate,
    horizon_days: i32,
) -> Briefing {
    let horizon = i64::from(horizon_days);
    let suppressions = suppression_map(preparations);
    let empty = HashSet::new();
    let mut items = Vec::new();
    for occasion in occasions {
        let Some((date, yearly)) = next_occurrence(&occasion.date, today) else {
            continue;
        };
        let days = (date - today).num_days();
        if days > horizon {
            continue;
        }
        let suppressed = suppressions.get(&occasion.id).unwrap_or(&empty);
        let flags: Vec<i32> = lead_flags(days, yearly)
            .into_iter()
            .filter(|flag| !suppressed.contains(flag))
            .collect();
        items.push(BriefingItem {
            title: occasion.title.clone(),
            days_until: days as i32,
            due_date: date.format("%Y-%m-%d").to_string(),
            kind: BriefingItemKind::Occasion as i32,
            flags,
        });
    }
    for task in tasks {
        let brief = task_briefing_item(task, today, horizon);
        if let Some(item) = brief {
            items.push(item);
        }
    }
    items.sort_by_key(|item| item.days_until);
    Briefing { items }
}

/// Classify an occasion kind from its title.
pub fn occasion_kind(title: &str, yearly: bool) -> OccasionKind {
    let lower = title.to_lowercase();
    if lower.contains("birthday") {
        return OccasionKind::Birthday;
    }
    if lower.contains("anniversary") {
        return OccasionKind::Anniversary;
    }
    if yearly {
        return OccasionKind::Custom;
    }
    OccasionKind::Custom
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ChoreEntity, Occasion};

    /// A date safely in the middle of a non-leap year for stable tests.
    fn day(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// A yearly occasion on the given MM-DD.
    fn yearly_occasion(date: &str) -> Occasion {
        Occasion {
            id: "o1".into(),
            title: "Emma's Birthday".into(),
            date: date.into(),
            kind: OccasionKind::Birthday as i32,
            ..Default::default()
        }
    }

    /// A dated TASK chore due at the given unix timestamp.
    fn task_due(date: chrono::NaiveDate) -> ChoreEntity {
        let due = date
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        ChoreEntity {
            id: "t1".into(),
            action: "Order cake".into(),
            kind: ChoreKind::Task as i32,
            due_at_unix: Some(due),
            ..Default::default()
        }
    }

    #[test]
    fn yearly_date_rolls_to_next_year() {
        let today = day(2026, 9, 15);
        let (next, yearly) = next_occurrence("03-15", today).unwrap();
        assert_eq!(next, day(2027, 3, 15));
        assert!(yearly);
    }

    #[test]
    fn yearly_date_later_this_year_stays() {
        let today = day(2026, 9, 15);
        let (next, yearly) = next_occurrence("12-01", today).unwrap();
        assert_eq!(next, day(2026, 12, 1));
        assert!(yearly);
    }

    #[test]
    fn one_time_past_date_is_skipped() {
        let today = day(2026, 9, 15);
        assert!(next_occurrence("2026-01-01", today).is_none());
    }

    #[test]
    fn feb29_falls_back_to_feb28() {
        let today = day(2027, 1, 1);
        let (next, _) = next_occurrence("02-29", today).unwrap();
        assert_eq!(next, day(2027, 2, 28));
    }

    #[test]
    fn flags_fire_at_each_threshold() {
        assert!(lead_flags(21, true).is_empty());
        assert_eq!(lead_flags(14, true), vec![LeadFlag::Gift as i32]);
        assert_eq!(
            lead_flags(7, true),
            vec![LeadFlag::Gift as i32, LeadFlag::Decor as i32]
        );
        assert_eq!(
            lead_flags(2, true),
            vec![
                LeadFlag::Gift as i32,
                LeadFlag::Decor as i32,
                LeadFlag::Cake as i32,
            ]
        );
        assert_eq!(
            lead_flags(0, true),
            vec![
                LeadFlag::Gift as i32,
                LeadFlag::Decor as i32,
                LeadFlag::Cake as i32,
                LeadFlag::Card as i32,
                LeadFlag::Cleaning as i32,
            ]
        );
        assert!(lead_flags(0, false).is_empty());
    }

    #[test]
    fn briefing_merges_and_sorts() {
        let today = day(2026, 9, 15);
        let soon = yearly_occasion("09-17"); // 2 days out
        let later = yearly_occasion("10-01"); // 16 days out
        let task = task_due(day(2026, 9, 16));
        let briefing = build(&[later, soon], &[task], &[], today, 30);
        let days: Vec<i32> = briefing.items.iter().map(|i| i.days_until).collect();
        assert_eq!(days, vec![1, 2, 16]);
        assert_eq!(
            briefing.items[1].flags,
            vec![LeadFlag::Gift as i32, LeadFlag::Decor as i32, LeadFlag::Cake as i32]
        );
        assert_eq!(briefing.items[0].kind, BriefingItemKind::Task as i32);
    }

    #[test]
    fn overdue_task_included_with_negative_days() {
        let today = day(2026, 9, 15);
        let task = task_due(day(2026, 9, 10));
        let briefing = build(&[], &[task], &[], today, 7);
        assert_eq!(briefing.items[0].days_until, -5);
    }

    #[test]
    fn horizon_excludes_far_events() {
        let today = day(2026, 9, 15);
        let far = yearly_occasion("12-01");
        assert!(build(&[far], &[], &[], today, 7).items.is_empty());
    }

    #[test]
    fn vision_chores_never_become_briefing_tasks() {
        let today = day(2026, 9, 15);
        let mut chore = task_due(day(2026, 9, 16));
        chore.kind = ChoreKind::Vision as i32;
        assert!(build(&[], &[chore], &[], today, 30).items.is_empty());
    }

    #[test]
    fn resolved_preparation_suppresses_matching_flag() {
        let today = day(2026, 9, 15);
        let mut occasion = yearly_occasion("09-17"); // 2 days out
        occasion.id = "occ1".to_string();
        let ready_cake = Preparation {
            id: "pr1".to_string(),
            occasion_id: "occ1".to_string(),
            title: "chocolate cake".to_string(),
            kind: PreparationKind::Cake as i32,
            state: PreparationState::Ready as i32,
            ..Default::default()
        };
        let briefing = build(&[occasion.clone()], &[], &[ready_cake], today, 30);
        // CAKE suppressed; GIFT and DECOR still fire at 2 days out.
        assert_eq!(
            briefing.items[0].flags,
            vec![LeadFlag::Gift as i32, LeadFlag::Decor as i32]
        );
        // An IDEA preparation does not suppress.
        let idea = Preparation {
            id: "pr2".to_string(),
            occasion_id: "occ1".to_string(),
            title: "backup cake".to_string(),
            kind: PreparationKind::Cake as i32,
            state: PreparationState::Idea as i32,
            ..Default::default()
        };
        let briefing = build(&[occasion], &[], &[idea], today, 30);
        assert!(briefing.items[0].flags.contains(&(LeadFlag::Cake as i32)));
    }

    #[test]
    fn occasion_less_preparations_suppress_nothing() {
        let today = day(2026, 9, 15);
        let occasion = yearly_occasion("09-17");
        let floating = Preparation {
            id: "pr1".to_string(),
            person_id: "p1".to_string(),
            occasion_id: String::new(),
            title: "chocolate cake".to_string(),
            kind: PreparationKind::Cake as i32,
            state: PreparationState::Ready as i32,
        };
        let briefing = build(&[occasion], &[], &[floating], today, 30);
        assert!(briefing.items[0].flags.contains(&(LeadFlag::Cake as i32)));
    }
}
