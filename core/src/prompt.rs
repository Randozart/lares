//! Prompt construction for the vision-language model.
//!
//! Strict grounding removes ambiguity: the model is told to ignore properly
//! stored items, prefer atomic actions, and return only JSON matching the
//! schema in [`response_schema`].

use serde_json::{json, Value};

/// System instruction for discovering actionable chores in a frame.
pub fn discover_system_prompt() -> String {
    concat!(
        "You are an assistive vision system that turns messy rooms into clear, actionable tasks. ",
        "Analyze the room image and identify actionable physical chores. ",
        "Ignore items that are properly stored. ",
        "Prefer atomic, single-step actions over broad tasks. ",
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
pub fn discover_user_prompt() -> String {
    "Analyze this frame and list every actionable chore.".to_string()
}

/// User-facing instruction for a DIFF pair.
pub fn diff_user_prompt() -> String {
    "Image A is the agreed target state. Image B is the current state. List the deltas."
        .to_string()
}

/// JSON schema constraining the model's structured output.
pub fn response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "chores": {
                "type": "array",
                "items": {
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
                        "subtasks": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["box_2d", "target", "action", "estimated_seconds"]
                }
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
        let p = discover_system_prompt();
        assert!(p.contains("actionable"));
        assert!(p.contains("atomic"));
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