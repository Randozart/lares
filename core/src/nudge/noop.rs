//! A reminder policy that never nudges. Default for the MVP.

use crate::domain::{ChoreEntity, IdleContext, Nudge};

use super::ReminderPolicy;

/// The default policy: surface nothing until Phase G ships a real policy.
pub struct NoopPolicy;

impl ReminderPolicy for NoopPolicy {
    /// Always decline to nudge.
    fn next_nudge(&self, _chores: &[ChoreEntity], _ctx: &IdleContext) -> Option<Nudge> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_never_nudges() {
        let policy = NoopPolicy;
        let ctx = IdleContext {
            screen_on: true,
            seconds_idle: 300,
            hour_local: 14,
            at_home: true,
        };
        assert!(policy.next_nudge(&[], &ctx).is_none());
    }
}