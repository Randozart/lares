//! Room inference from landmark labels.
//!
//! Pure matching over landmark profiles; the caller supplies the scan's
//! landmark labels and each room's accumulated profile, so the whole module
//! is deterministically unit-testable and costs no inference calls.

/// A winning room plus the landmarks that matched it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomMatch {
    /// The inferred room id.
    pub room_id: String,
    /// Scan landmarks present in the winning room's profile.
    pub evidence: Vec<String>,
}

/// Minimum matched landmarks for a confident room guess.
pub const MIN_LANDMARK_MATCHES: usize = 2;

/// Infer the room from scan landmark labels against per-room profiles.
///
/// A profile is `(room_id, landmark labels)`. A room's score is the count of
/// scan labels present in its profile (case-insensitive). Returns the
/// strictly best room only: ties and scores below
/// [`MIN_LANDMARK_MATCHES`] yield `None` — ambiguous scenes don't guess.
pub fn infer_from_landmarks(
    scan: &[String],
    profiles: &[(String, Vec<String>)],
) -> Option<RoomMatch> {
    if scan.is_empty() || profiles.is_empty() {
        return None;
    }
    let scan_lower: Vec<String> = scan.iter().map(|s| s.to_lowercase()).collect();
    let mut best: Option<(usize, &str, Vec<String>)> = None;
    let mut tie = false;
    for (room_id, labels) in profiles {
        let lower: Vec<String> = labels.iter().map(|s| s.to_lowercase()).collect();
        let evidence: Vec<String> = scan_lower
            .iter()
            .filter(|s| lower.contains(s))
            .cloned()
            .collect();
        let score = evidence.len();
        if score < MIN_LANDMARK_MATCHES {
            continue;
        }
        match &best {
            None => best = Some((score, room_id, evidence)),
            Some((best_score, _, _)) if score > *best_score => {
                best = Some((score, room_id, evidence));
                tie = false;
            }
            Some((best_score, _, _)) if score == *best_score => tie = true,
            _ => {}
        }
    }
    if tie {
        return None;
    }
    best.map(|(_, room_id, evidence)| RoomMatch {
        room_id: room_id.to_string(),
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profiles() -> Vec<(String, Vec<String>)> {
        vec![
            (
                "kitchen".to_string(),
                vec!["fridge".into(), "dishwasher".into(), "stove".into()],
            ),
            (
                "bedroom".to_string(),
                vec!["bed".into(), "nightstand".into()],
            ),
        ]
    }

    #[test]
    fn matching_landmarks_win() {
        let scan = vec!["fridge".to_string(), "dishwasher".to_string()];
        let m = infer_from_landmarks(&scan, &profiles()).unwrap();
        assert_eq!(m.room_id, "kitchen");
        assert_eq!(m.evidence, vec!["fridge".to_string(), "dishwasher".to_string()]);
    }

    #[test]
    fn case_insensitive_matching() {
        let scan = vec!["Fridge".to_string(), "DISHWASHER".to_string()];
        assert_eq!(infer_from_landmarks(&scan, &profiles()).unwrap().room_id, "kitchen");
    }

    #[test]
    fn single_match_is_not_enough() {
        let scan = vec!["fridge".to_string()];
        assert!(infer_from_landmarks(&scan, &profiles()).is_none());
    }

    #[test]
    fn tie_between_rooms_yields_none() {
        let mut profiles = profiles();
        profiles.push((
            "pantry".to_string(),
            vec!["fridge".into(), "dishwasher".into()],
        ));
        let scan = vec!["fridge".to_string(), "dishwasher".to_string()];
        assert!(infer_from_landmarks(&scan, &profiles).is_none());
    }

    #[test]
    fn strongest_profile_wins() {
        let scan = vec![
            "fridge".to_string(),
            "dishwasher".to_string(),
            "stove".to_string(),
            "bed".to_string(),
        ];
        assert_eq!(infer_from_landmarks(&scan, &profiles()).unwrap().room_id, "kitchen");
    }

    #[test]
    fn empty_inputs_yield_none() {
        assert!(infer_from_landmarks(&[], &profiles()).is_none());
        assert!(infer_from_landmarks(&["bed".to_string()], &[]).is_none());
    }

    #[test]
    fn unknown_landmarks_yield_none() {
        let scan = vec!["ufo".to_string(), "tractor".to_string()];
        assert!(infer_from_landmarks(&scan, &profiles()).is_none());
    }
}
