//! Storage norms and relation-based task judgement.
//!
//! The frozen loop (detect + relate) proposes `(object, place)` relations
//! every tick; this module decides which of them are tasks. The primary
//! judge is a lookup into the learned norm table — no language model in
//! the per-tick path. An LLM only teaches norms for unseen combos, and
//! those verdicts are cached.
//!
//! Also hosts the expected-relation parser: our own directives follow a
//! fixed grammar ("PUT THE CUP IN THE SINK"), so each open chore implies
//! an expected relation that the frozen loop can observe — completion
//! detection without a VLM call.

use std::collections::{HashMap, HashSet};

use crate::domain::{ChoreEntity, ChoreStatus, Norm};

/// Norm verdict: the object belongs at the place.
pub const VERDICT_GOOD: &str = "GOOD";
/// Norm verdict: the object is misplaced — a task candidate.
pub const VERDICT_BAD: &str = "BAD";

/// Minimum relation score for a judgement.
pub const MIN_RELATION_SCORE: f32 = 0.2;

/// One observed relation between detected regions.
#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    /// Detected object, bare noun ("cup").
    pub subject: String,
    /// Spatial predicate ("on", "in", "under", "next to").
    pub predicate: String,
    /// Place or container the subject relates to ("floor", "sink").
    pub object: String,
    /// Model confidence 0..1.
    pub score: f32,
}

/// A relation the judge marked as a misplaced-object task.
#[derive(Debug, Clone)]
pub struct JudgedRelation {
    /// The observed relation.
    pub relation: Relation,
    /// The norm that condemned it (supplies the action hint).
    pub norm: Norm,
}

/// The relation an open chore expects to observe once it is done.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedRelation {
    /// Object the directive moves ("cup", "dirty dishes").
    pub object: String,
    /// Target predicate ("in", "on", "under", "next to").
    pub predicate: String,
    /// Destination ("sink", "dishwasher", "hamper").
    pub place: String,
}

/// An observed relation satisfying an open chore's expectation.
#[derive(Debug, Clone)]
pub struct CompletionMatch {
    /// The chore this observation completes.
    pub chore_id: String,
    /// The chore's action text (for chip display).
    pub action: String,
    /// The observed relation that matched.
    pub observed: Relation,
}

/// Judge observed relations against the norm table.
///
/// A relation becomes a task candidate when its `(subject, place)` norm is
/// BAD. Combos with no norm are ignored unless the subject is a known
/// messable class (present anywhere in the table) — unknown-benign pairs
/// like `sofa next to table` must not chip, while `cup on ottoman` should.
pub fn judge(norms: &[Norm], relations: &[Relation]) -> Vec<JudgedRelation> {
    let mut index: HashMap<(&str, &str), &Norm> = HashMap::new();
    for norm in norms {
        index.insert((norm.object_class.as_str(), norm.place.as_str()), norm);
    }
    let mut judged = Vec::new();
    for relation in relations {
        if relation.score < MIN_RELATION_SCORE {
            continue;
        }
        let subject = relation.subject.trim().to_lowercase();
        let place = relation.object.trim().to_lowercase();
        let Some(norm) = index.get(&(subject.as_str(), place.as_str())) else {
            continue;
        };
        if norm.verdict != VERDICT_BAD {
            continue;
        }
        judged.push(JudgedRelation {
            relation: relation.clone(),
            norm: (*norm).clone(),
        });
    }
    judged
}

/// Flag relations whose combo has no norm yet but involves a known
/// messable object resting somewhere highly suspicious — these surface
/// as chips and feed the norm teacher. Furniture placements are left
/// alone: `cup on ottoman` is not worth a chip on day one.
pub fn unknown_combos(norms: &[Norm], relations: &[Relation]) -> Vec<Relation> {
    let index: HashSet<(&str, &str)> = norms
        .iter()
        .map(|n| (n.object_class.as_str(), n.place.as_str()))
        .collect();
    let known_objects: HashSet<&str> =
        norms.iter().map(|n| n.object_class.as_str()).collect();
    relations
        .iter()
        .filter(|r| r.score >= MIN_RELATION_SCORE)
        .filter(|r| {
            let subject = r.subject.trim().to_lowercase();
            let place = r.object.trim().to_lowercase();
            known_objects.contains(subject.as_str())
                && !index.contains(&(subject.as_str(), place.as_str()))
        })
        .filter(|r| {
            let place = r.object.trim().to_lowercase();
            matches!(place.as_str(), "floor" | "bed" | "stairs" | "ground")
        })
        .cloned()
        .collect()
}

/// Parse a directive into the relation that would observe its completion.
///
/// Our prompt-shaped grammar: `PUT|PLACE|RETURN|STOW|DROP ... THE <object>
/// IN|INTO|ON|ONTO|UNDER|BESIDE|NEXT TO ... THE <place>`. Returns None for
/// directives that do not move an object to a destination.
pub fn parse_expected_relation(action: &str) -> Option<ExpectedRelation> {
    let lowered = action.to_lowercase();
    let tokens: Vec<&str> = lowered
        .split(|c: char| c.is_whitespace() || c == ',' || c == '.')
        .filter(|t| !t.is_empty())
        .collect();
    const VERBS: [&str; 5] = ["put", "place", "return", "stow", "drop"];
    const PREPOSITIONS: [&str; 7] = [
        "in", "into", "on", "onto", "under", "beside", "next",
    ];
    let verb_pos = tokens.iter().position(|t| VERBS.contains(t))?;
    let prep_pos = tokens
        .iter()
        .enumerate()
        .skip(verb_pos + 1)
        .find(|(_, t)| PREPOSITIONS.contains(t))
        .map(|(i, _)| i)?;
    let object = strip_article(&tokens[verb_pos + 1..prep_pos].join(" "));
    let mut rest: &[&str] = &tokens[prep_pos..];
    let predicate = canonical_predicate(rest[0]);
    if rest[0] == "next" {
        rest = &rest[1..]; // consume "to" below via place extraction
    }
    let place = strip_article(&rest[1..].join(" "));
    if object.is_empty() || place.is_empty() {
        return None;
    }
    Some(ExpectedRelation { object, predicate, place })
}

/// Map a directive preposition to its observed-relation equivalent.
fn canonical_predicate(token: &str) -> String {
    match token {
        "into" => "in".to_string(),
        "onto" => "on".to_string(),
        "beside" | "next" => "next to".to_string(),
        other => other.to_string(),
    }
}

/// Strip leading articles and fillers from a noun phrase.
fn strip_article(phrase: &str) -> String {
    let trimmed = phrase.trim();
    for prefix in ["the ", "a ", "an ", "dirty ", "scattered "] {
        if let Some(stripped) = trimmed.strip_prefix(prefix) {
            return stripped.trim().to_string();
        }
    }
    trimmed.to_string()
}

/// Find open chores whose expected relation is satisfied by an observation.
pub fn find_completions(open: &[ChoreEntity], relations: &[Relation]) -> Vec<CompletionMatch> {
    let mut matches = Vec::new();
    for chore in open.iter().filter(|c| is_open(c)) {
        let Some(expected) = parse_expected_relation(&chore.action) else {
            continue;
        };
        for relation in relations {
            if relation.score < MIN_RELATION_SCORE {
                continue;
            }
            if phrase_matches(&expected.object, &relation.subject)
                && predicate_matches(&expected.predicate, &relation.predicate)
                && phrase_matches(&expected.place, &relation.object)
            {
                matches.push(CompletionMatch {
                    chore_id: chore.id.clone(),
                    action: chore.action.clone(),
                    observed: relation.clone(),
                });
                break;
            }
        }
    }
    matches
}

/// Two noun phrases match when either contains the other ("dishes" ⊂
/// "dirty dishes"); empty strings never match.
fn phrase_matches(expected: &str, observed: &str) -> bool {
    let e = expected.trim().to_lowercase();
    let o = observed.trim().to_lowercase();
    !e.is_empty() && !o.is_empty() && (o.contains(&e) || e.contains(&o))
}

/// Predicate equivalence across synonym spellings.
fn predicate_matches(expected: &str, observed: &str) -> bool {
    canonical_predicate(expected) == canonical_predicate(observed)
}

/// Whether a chore is still open (eligible for completion detection).
pub fn is_open(chore: &ChoreEntity) -> bool {
    chore.status == ChoreStatus::Discovered as i32
        || chore.status == ChoreStatus::InProgress as i32
}

/// Ask a chat-completions endpoint whether an object belongs at a place.
///
/// One-shot norm teacher for combos the seed table has never seen; the
/// verdict is cached into the norms table by the caller so the language
/// model runs once per combo, ever. Returns (verdict, action hint) or
/// None when the endpoint or the answer is unusable.
pub async fn judge_norm_via_llm(
    http: &reqwest::Client,
    endpoint: &str,
    model: &str,
    object: &str,
    place: &str,
) -> Option<(String, String)> {
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You are a household tidiness judge. \
                Answer ONLY minified JSON: {\"verdict\":\"GOOD\"|\"BAD\",\"action\":\"<directive>\"}. \
                GOOD means the object belongs at that place in a tidy home; BAD means misplaced, \
                and action is one imperative sentence saying where it should go."},
            {"role": "user", "content": format!("Object: {object}. Place: {place}.")},
        ],
        "temperature": 0,
        "max_tokens": 80,
    });
    let response = http
        .post(format!("{endpoint}/v1/chat/completions"))
        .json(&body)
        .timeout(std::time::Duration::from_secs(45))
        .send()
        .await
        .ok()?;
    let value: serde_json::Value = response.json().await.ok()?;
    let text = value["choices"][0]["message"]["content"].as_str()?;
    let text = text.trim().trim_matches('`');
    let text = text.strip_prefix("json").unwrap_or(text).trim();
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    let verdict = parsed["verdict"].as_str()?.to_uppercase();
    if verdict != VERDICT_GOOD && verdict != VERDICT_BAD {
        return None;
    }
    let action = parsed["action"].as_str().unwrap_or("").to_string();
    Some((verdict, action))
}

/// One seed category: objects sharing storage destinations.
struct SeedCategory {
    objects: &'static [&'static str],
    bad_places: &'static [&'static str],
    good_places: &'static [&'static str],
    hint: &'static str,
}

const WARE: SeedCategory = SeedCategory {
    objects: &["cup", "mug", "glass", "bottle", "plate", "bowl", "fork", "spoon", "knife"],
    bad_places: &["floor", "bed", "sofa", "couch", "stairs", "dresser"],
    good_places: &["sink", "dishwasher", "counter", "cupboard", "table"],
    hint: "in the sink",
};

const TRASH: SeedCategory = SeedCategory {
    objects: &["trash", "wrapper", "bag", "paper", "cardboard"],
    bad_places: &["floor", "bed", "sofa", "couch", "table", "desk", "stairs"],
    good_places: &["bin", "trash can", "garbage bin"],
    hint: "in the bin",
};

const CLOTHES: SeedCategory = SeedCategory {
    objects: &["clothes", "socks", "shirt", "towel", "jacket", "pants"],
    bad_places: &["floor", "bed", "sofa", "couch", "chair", "stairs"],
    good_places: &["hamper", "closet", "drawer", "wardrobe"],
    hint: "in the hamper",
};

const TOYS: SeedCategory = SeedCategory {
    objects: &["toy"],
    bad_places: &["floor", "stairs"],
    good_places: &["toy box", "shelf", "box", "bed"],
    hint: "in the toy box",
};

const LOOSE_ITEMS: SeedCategory = SeedCategory {
    objects: &["book", "remote", "controller", "phone", "laptop", "charger", "cable", "pen", "keys"],
    bad_places: &["floor", "stairs"],
    good_places: &["table", "desk", "shelf", "tv stand", "nightstand"],
    hint: "on the table",
};

/// Populate the norm table with household defaults on first boot.
///
/// No-op when the table already has rows — user corrections and cached
/// teacher verdicts must never be overwritten by a re-seed.
pub async fn seed_defaults(store: &crate::store::Store) -> Result<(), crate::store::StoreError> {
    let existing = store.list_norms().await?;
    if !existing.is_empty() {
        return Ok(());
    }
    let now = crate::domain::now_unix();
    for category in [&WARE, &TRASH, &CLOTHES, &TOYS, &LOOSE_ITEMS] {
        for object in category.objects {
            for place in category.bad_places {
                store.upsert_norm(&Norm {
                    object_class: (*object).into(),
                    place: (*place).into(),
                    verdict: VERDICT_BAD.into(),
                    action_hint: format!("Put the {object} {}", category.hint),
                    source: "seed".into(),
                    updated_at: now,
                })
                .await?;
            }
            for place in category.good_places {
                store.upsert_norm(&Norm {
                    object_class: (*object).into(),
                    place: (*place).into(),
                    verdict: VERDICT_GOOD.into(),
                    action_hint: String::new(),
                    source: "seed".into(),
                    updated_at: now,
                })
                .await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::BoundingBox;

    /// Build a norm row for tests.
    fn norm(object: &str, place: &str, verdict: &str) -> Norm {
        Norm {
            object_class: object.into(),
            place: place.into(),
            verdict: verdict.into(),
            action_hint: format!("Put the {object} away"),
            source: "seed".into(),
            updated_at: 0,
        }
    }

    /// Build a relation for tests.
    fn relation(subject: &str, predicate: &str, object: &str, score: f32) -> Relation {
        Relation {
            subject: subject.into(),
            predicate: predicate.into(),
            object: object.into(),
            score,
        }
    }

    #[test]
    fn bad_norm_creates_candidate() {
        let norms = vec![norm("cup", "floor", VERDICT_BAD)];
        let judged = judge(&norms, &[relation("cup", "on", "floor", 0.7)]);
        assert_eq!(judged.len(), 1);
        assert_eq!(judged[0].relation.subject, "cup");
    }

    #[test]
    fn good_norm_suppresses() {
        let norms = vec![norm("cup", "sink", VERDICT_GOOD)];
        let judged = judge(&norms, &[relation("cup", "in", "sink", 0.8)]);
        assert!(judged.is_empty());
    }

    #[test]
    fn unknown_combo_is_ignored_by_judge() {
        let norms = vec![norm("cup", "floor", VERDICT_BAD)];
        let judged = judge(&norms, &[relation("decoy", "on", "moon", 0.9)]);
        assert!(judged.is_empty());
    }

    #[test]
    fn low_score_relations_are_skipped() {
        let norms = vec![norm("cup", "floor", VERDICT_BAD)];
        let judged = judge(&norms, &[relation("cup", "on", "floor", 0.1)]);
        assert!(judged.is_empty());
    }

    #[test]
    fn parses_the_canonical_directive() {
        let e = parse_expected_relation("Put the cup in the sink").unwrap();
        assert_eq!(e.object, "cup");
        assert_eq!(e.predicate, "in");
        assert_eq!(e.place, "sink");
    }

    #[test]
    fn parses_dishes_into_dishwasher() {
        let e = parse_expected_relation("Put the dirty dishes in the dishwasher").unwrap();
        assert_eq!(e.object, "dirty dishes");
        assert_eq!(e.predicate, "in");
        assert_eq!(e.place, "dishwasher");
    }

    #[test]
    fn non_moving_directive_yields_none() {
        assert!(parse_expected_relation("Wipe the table").is_none());
    }

    #[test]
    fn completion_matches_across_adjectives() {
        let chore = ChoreEntity {
            id: "c1".into(),
            action: "Put the dirty dishes in the dishwasher".into(),
            status: ChoreStatus::Discovered as i32,
            ..Default::default()
        };
        let observed = vec![relation("dishes", "in", "dishwasher", 0.6)];
        let matches = find_completions(&[chore], &observed);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].chore_id, "c1");
    }

    #[test]
    fn done_chores_are_not_matched() {
        let chore = ChoreEntity {
            id: "c2".into(),
            action: "Put the cup in the sink".into(),
            status: ChoreStatus::Done as i32,
            ..Default::default()
        };
        let observed = vec![relation("cup", "in", "sink", 0.6)];
        assert!(find_completions(&[chore], &observed).is_empty());
    }

    #[test]
    fn boxes_do_not_confuse_matching() {
        let chore = ChoreEntity {
            id: "c3".into(),
            action: "Put the socks in the hamper".into(),
            status: ChoreStatus::Discovered as i32,
            r#box: Some(BoundingBox { ymin: 0, xmin: 0, ymax: 1, xmax: 1 }),
            ..Default::default()
        };
        let observed = vec![relation("socks", "in", "hamper", 0.55)];
        assert_eq!(find_completions(&[chore], &observed).len(), 1);
    }

    #[tokio::test]
    async fn seed_populates_once_and_never_overwrites() {
        let store = crate::store::Store::connect_in_memory().await.unwrap();
        seed_defaults(&store).await.unwrap();
        let first = store.list_norms().await.unwrap();
        assert!(first.len() > 50, "seed should populate a real matrix");
        // A user correction must survive a re-seed.
        store
            .upsert_norm(&norm("cup", "floor", VERDICT_GOOD))
            .await
            .unwrap();
        seed_defaults(&store).await.unwrap();
        let corrected = store
            .list_norms()
            .await
            .unwrap()
            .into_iter()
            .find(|n| n.object_class == "cup" && n.place == "floor")
            .unwrap();
        assert_eq!(corrected.verdict, VERDICT_GOOD, "re-seed must be a no-op");
    }
}
