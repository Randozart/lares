//! Proactive reminder seam (Phase G).
//!
//! The MVP wires [`noop::NoopPolicy`] and never nudges. The trait and contract
//! types exist now so the reminder feature can land without touching the chore
//! contract or the persistence layer.

pub mod noop;

use crate::domain::{ChoreEntity, IdleContext, Nudge};

/// Decides whether to surface a chore to an idle user.
pub trait ReminderPolicy: Send + Sync {
    /// Return the next nudge to surface, if any.
    fn next_nudge(&self, chores: &[ChoreEntity], ctx: &IdleContext) -> Option<Nudge>;
}