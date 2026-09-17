//! Prompt construction for the vision-language model.
//!
//! Strict grounding removes ambiguity: the model is told to ignore properly
//! stored items, prefer atomic actions, and return only JSON matching the
//! schema in [`response_schema`].

use serde_json::{json, Value};

/// System instruction for discovering actionable chores in a frame.
pub fn discover_system_prompt(room_area: &str) -> String {
    format!(
        "You are an assistive vision system that turns messy rooms into clear, actionable tasks. \
         Analyze the room image and identify actionable physical chores. \
         Be conservative: only report items you can see clearly. \
         If you are not sure of the specific type of an object, use a generic label \
         (e.g. \"drink container\") rather than guessing a specific one. \
         Ignore items that are properly stored, and omit anything you are not sure \
         needs attention. \
         Prefer atomic, single-step actions over broad tasks. \
         Directives must restore items to sensible storage for a tidy {room_area}: \
         remotes and controllers go to the TV stand or coffee table, dishes to the \
         sink or dishwasher, clothes to the hamper or closet, toys to a shelf or \
         box, trash to the bin. Never suggest the floor as a destination. \
         Only report items that are genuinely out of place in a tidy {room_area}. \
         For every chore provide 2-4 concrete physical steps in `how_to` \
         that a person can follow without needing judgment: name the specific \
         object, where it goes, and what to do with it. \
         Return ONLY JSON matching the provided schema."
    )
}

/// System instruction for a multi-frame sweep of a single room.
pub fn sweep_system_prompt() -> String {
    concat!(
        "You are an assistive vision system that turns messy rooms into clear, actionable tasks. ",
        "The user has panned across a room and provided several consecutive frames. ",
        "Identify actionable physical chores across the whole room. ",
        "Report each chore ONCE, in the frame where it is clearest, and set `image` ",
        "to that frame's 0-based index. ",
        "Be conservative: only report items you can see clearly; use a generic label ",
        "when unsure of the specific type (e.g. \"drink container\") rather than guessing. ",
        "Ignore properly stored items and omit anything that does not clearly need attention. ",
        "For every chore provide 2-4 concrete physical steps in `how_to`. ",
        "Return ONLY JSON matching the provided schema."
    )
    .to_string()
}

/// System instruction for diffing the current frame against a reference state.
pub fn diff_system_prompt() -> String {
    concat!(
        "You are an assistive vision system that compares a room against its agreed target state. ",
        "Image A is the agreed target state for this room. Image B is the current state. ",
        "Output bounding boxes ONLY for deltas: items present in B that belong elsewhere, ",
        "or missing arrangements expected by A. ",
        "Return ONLY JSON matching the provided schema."
    )
    .to_string()
}

/// User-facing instruction for a DISCOVER frame.
pub fn discover_user_prompt(room_area: &str) -> String {
    format!(
        "Analyze this {room_area} and list every actionable chore. \
         Assign each distinct physical object an object_index (1-based, \
         stable across the scene)."
    )
}

/// User-facing instruction for a DIFF pair.
pub fn diff_user_prompt(room_area: &str) -> String {
    format!(
        "Image A is the agreed target state. Image B is the current {room_area}. \
         List the deltas. Assign each distinct physical object an object_index \
         (1-based, stable across the scene)."
    )
}

/// User-facing instruction for a sweep.
pub fn sweep_user_prompt(room_area: &str) -> String {
    format!(
        "These frames are consecutive views of one {room_area} during a pan. \
         List every actionable chore once, in the frame where it is clearest. \
         Assign each distinct physical object an object_index (1-based, stable \
         across the scene)."
    )
}

/// JSON schema for a single chore entry.
fn chore_item_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "box_2d": {
                "type": "array",
                "items": { "type": "integer" },
                "description": "[ymin, xmin, ymax, xmax] normalized 0-1000"
            },
            "target": { "type": "string" },
            "action": { "type": "string" },
            "estimated_seconds": { "type": "integer" },
            "confidence": { "type": "number" },
            "subtasks": { "type": "array", "items": { "type": "string" } },
            "how_to": {
                "type": "array",
                "items": { "type": "string" },
                "description": "2-4 concrete physical steps, naming specific objects by their object_index"
            },
            "object_index": {
                "type": "integer",
                "description": "1-based index identifying this specific physical object in the scene"
            },
            "image": {
                "type": "integer",
                "description": "Frame index (0-based) this chore was seen in; only set for sweeps"
            }
        },
        "required": ["box_2d", "target", "action", "estimated_seconds"]
    })
}

/// JSON schema constraining the model's structured output.
pub fn response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "chores": {
                "type": "array",
                "items": chore_item_schema()
            },
            "landmarks": {
                "type": "array",
                "description": "Stable named anchors in the room (sink, hamper, sofa, table)",
                "items": {
                    "type": "object",
                    "properties": {
                        "label": { "type": "string" },
                        "box_2d": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "[ymin, xmin, ymax, xmax] normalized 0-1000"
                        }
                    },
                    "required": ["label", "box_2d"]
                }
            }
        },
        "required": ["chores"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_prompt_mentions_actionable_and_atomic() {
        let p = discover_system_prompt("kitchen");
        assert!(p.contains("actionable"));
        assert!(p.contains("atomic"));
        assert!(p.contains("kitchen"));
        assert!(p.contains("Never suggest the floor"));
    }

    #[test]
    fn diff_prompt_references_two_images() {
        let p = diff_system_prompt();
        assert!(p.contains("Image A"));
        assert!(p.contains("Image B"));
    }

    #[test]
    fn schema_requires_chores_array() {
        let s = response_schema();
        assert_eq!(s["required"][0], "chores");
        assert!(s["properties"]["chores"]["type"].is_string());
    }
}