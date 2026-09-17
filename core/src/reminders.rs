//! Forgotten-task matching: pair overdue manual tasks with visual evidence.
//!
//! Pure heuristics, no network and no LLM calls. A match means "the room
//! shows the physical traces of a task that is past due", e.g. vision
//! reports "trash bags on the floor" while "take out trash" is overdue.

use crate::domain::{ChoreEntity, ChoreKind, ChoreStatus};
use std::collections::HashMap;

/// Token length below which a word is ignored during matching.
const MIN_TOKEN_LEN: usize = 3;

/// Common filler words that never count as evidence.
const STOPWORDS: [&str; 8] = ["the", "and", "out", "put", "get", "for", "with", "into"];

/// Whether a chore is an active (not done/dismissed) manual task.
fn is_active_task(chore: &ChoreEntity) -> bool {
    chore.kind == ChoreKind::Task as i32
        && chore.status != ChoreStatus::Done as i32
        && chore.status != ChoreStatus::Dismissed as i32
}

/// Whether an active task is overdue at `now_unix`.
fn is_overdue(chore: &ChoreEntity, now_unix: i64) -> bool {
    match chore.due_at_unix {
        Some(due) => due < now_unix,
        None => false,
    }
}

/// Lowercase a string and split it into meaningful match tokens.
fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.len() >= MIN_TOKEN_LEN && !STOPWORDS.contains(word))
        .map(str::to_string)
        .collect()
}

/// One overdue task matched against visual evidence.
#[derive(Debug, PartialEq)]
pub struct ForgottenMatch {
    /// The overdue task's id.
    pub task_id: String,
    /// Human-readable reason, e.g. `take out trash — saw "trash bags"`.
    pub reason: String,
}

/// Index vision targets by their meaningful tokens.
///
/// Maps each token to the first observed target containing it.
fn vision_token_index(vision: &[ChoreEntity]) -> HashMap<String, String> {
    let pairs = vision.iter().flat_map(|observed| {
        let target = observed.target.clone();
        tokens(&observed.target).into_iter().map(move |token| (token, target.clone()))
    });
    let mut index = HashMap::new();
    for (token, target) in pairs {
        index.entry(token).or_insert(target);
    }
    index
}

/// Match overdue manual tasks against vision-reported chores.
///
/// A task matches when one of its tokens appears in the vision token index.
/// Tasks without due dates never match. Linear in tasks + vision items.
pub fn match_forgotten(
    tasks: &[ChoreEntity],
    vision: &[ChoreEntity],
    now_unix: i64,
) -> Vec<ForgottenMatch> {
    if vision.is_empty() {
        return Vec::new();
    }
    let index = vision_token_index(vision);
    let mut matches = Vec::new();
    for task in tasks {
        let active = is_active_task(task) && is_overdue(task, now_unix);
        if !active {
            continue;
        }
        let task_tokens = tokens(&format!("{} {}", task.target, task.action));
        let hit = task_tokens.iter().find_map(|token| index.get(token));
        let Some(seen) = hit else { continue };
        matches.push(ForgottenMatch {
            task_id: task.id.clone(),
            reason: format!("{} — saw \"{seen}\"", task.action),
        });
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ChoreEntity;

    /// An overdue weekly "take out trash" task.
    fn trash_task() -> ChoreEntity {
        ChoreEntity {
            id: "t1".to_string(),
            target: "trash".to_string(),
            action: "take out trash".to_string(),
            kind: ChoreKind::Task as i32,
            status: ChoreStatus::Discovered as i32,
            due_at_unix: Some(1_000),
            ..Default::default()
        }
    }

    /// A vision-reported chore with the given target.
    fn vision(target: &str) -> ChoreEntity {
        ChoreEntity {
            id: format!("v-{target}"),
            target: target.to_string(),
            action: "clean up".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn matches_overdue_task_by_token_overlap() {
        let matches = match_forgotten(
            &[trash_task()],
            &[vision("trash bags on the floor")],
            2_000,
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].task_id, "t1");
        assert!(matches[0].reason.contains("trash"));
    }

    #[test]
    fn no_match_without_overlap() {
        let matches = match_forgotten(&[trash_task()], &[vision("messy counter")], 2_000);
        assert!(matches.is_empty());
    }

    #[test]
    fn future_due_never_matches() {
        let matches = match_forgotten(&[trash_task()], &[vision("trash bags")], 500);
        assert!(matches.is_empty());
    }

    #[test]
    fn done_tasks_never_match() {
        let mut task = trash_task();
        task.status = ChoreStatus::Done as i32;
        assert!(match_forgotten(&[task], &[vision("trash bags")], 2_000).is_empty());
    }

    #[test]
    fn vision_chores_and_undated_tasks_never_match_each_other() {
        let mut undated = trash_task();
        undated.due_at_unix = None;
        assert!(match_forgotten(&[undated], &[vision("trash bags")], 2_000).is_empty());
    }

    #[test]
    fn stopwords_do_not_create_matches() {
        let mut task = trash_task();
        task.target = "out".to_string();
        task.action = "get out".to_string();
        assert!(match_forgotten(&[task], &[vision("outdoor shoes")], 2_000).is_empty());
    }
}
