//! Client for the lares-vision frozen-model microservice.
//!
//! OWLv2 open-vocabulary detection and CLIP room classification live in a
//! small Python sidecar (`vision-service/`, port 8877) so the Rust server
//! can enrich scans without holding torch. All calls are best-effort at
//! the call site: failures degrade to plain VLM behaviour.

use serde::Deserialize;

/// Timeout generous enough to cover a cold model load in the sidecar.
const REQUEST_TIMEOUT_SECS: u64 = 60;

/// Open-vocabulary queries for household landmarks and task spaces.
pub const LANDMARK_QUERIES: &[&str] = &[
    "a washing machine",
    "a dryer",
    "a dishwasher",
    "an oven",
    "a stove",
    "a fridge",
    "a kitchen sink",
    "a bed",
    "a nightstand",
    "a sofa",
    "a tv",
    "a desk",
    "a bookshelf",
    "a wardrobe",
    "a toilet",
    "a mirror",
];

/// Tick queries: messable objects plus the surfaces they land on.
///
/// Kept tight so a tick stays fast; storage destinations need no boxes —
/// a GOOD placement simply produces no condemning relation.
pub const TICK_QUERIES: &[&str] = &[
    "the floor",
    "a cup",
    "a mug",
    "a bottle",
    "a plate",
    "a bowl",
    "trash",
    "clothes",
    "socks",
    "a towel",
    "a toy",
    "a book",
    "a remote",
    "a phone",
    "a laptop",
    "a sofa",
    "a bed",
    "a chair",
    "a table",
    "a sink",
];

/// Predicate vocabulary offered to the relation model; aligned with the
/// directive grammar the action parser understands.
pub const RELATION_VOCABULARY: &[&str] = &["on", "in", "under", "next to", "in front of"];

/// Box synthesized for the floor when the detector returns none: the lower
/// band of the frame, where floors are in handheld scans.
const SYNTHETIC_FLOOR_BOX: [i32; 4] = [600, 0, 1000, 1000];

/// Reduce a detection to a bare noun ("a cup" -> "cup").
pub fn bare_label(query: &str) -> String {
    let trimmed = query.trim();
    let stripped = trimmed
        .strip_prefix("a ")
        .or_else(|| trimmed.strip_prefix("an "))
        .or_else(|| trimmed.strip_prefix("the "))
        .unwrap_or(trimmed);
    stripped.trim().to_lowercase()
}

/// Build the label/box inputs for a relation pass over tick detections.
///
/// Labels are normalized to bare nouns and deduplicated; when no floor was
/// detected, a synthetic floor region is appended so relations like
/// `cup --on--> floor` remain scorable in single-subject frames.
pub fn tick_inputs(detections: &[FrozenDetection]) -> (Vec<String>, Vec<[i32; 4]>) {
    let mut labels: Vec<String> = Vec::with_capacity(detections.len() + 1);
    let mut boxes: Vec<[i32; 4]> = Vec::with_capacity(detections.len() + 1);
    let mut has_floor = false;
    for detection in detections {
        let label = bare_label(&detection.label);
        if label.is_empty() {
            continue;
        }
        if label == "floor" {
            has_floor = true;
        }
        if labels.contains(&label) {
            continue;
        }
        labels.push(label);
        boxes.push(detection.r#box);
    }
    if !has_floor {
        labels.push("floor".into());
        boxes.push(SYNTHETIC_FLOOR_BOX);
    }
    (labels, boxes)
}

/// One observed relation between two detected regions.
#[derive(Debug, Clone, Deserialize)]
pub struct FrozenRelation {
    /// Object that was moved or is resting, bare noun.
    pub subject: String,
    /// Spatial predicate ("on", "in", "under", "next to").
    pub predicate: String,
    /// Place or container the subject relates to.
    pub object: String,
    /// Model confidence 0..1.
    pub score: f32,
}

/// One OWLv2 detection, box already normalized 0-1000 [ymin,xmin,ymax,xmax].
#[derive(Debug, Clone, Deserialize)]
pub struct FrozenDetection {
    /// Matched query text, e.g. "a dishwasher".
    pub label: String,
    /// Detection confidence 0..1.
    pub score: f32,
    /// Box in Lares convention [ymin, xmin, ymax, xmax].
    pub r#box: [i32; 4],
}

/// A CLIP room classification verdict.
#[derive(Debug, Clone, Deserialize)]
pub struct FrozenRoom {
    /// Winning room label from the requested candidates.
    pub room: String,
    /// Softmax confidence of the winner.
    pub confidence: f32,
}

/// HTTP client for the frozen-vision sidecar.
#[derive(Clone)]
pub struct FrozenVision {
    /// Base URL, e.g. `http://127.0.0.1:8877`.
    endpoint: String,
    /// Shared HTTP client.
    http: reqwest::Client,
}

impl FrozenVision {
    /// Create a client for the sidecar at `endpoint`.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .expect("static client config"),
        }
    }

    /// Detect household landmarks in a JPEG frame.
    pub async fn detect(
        &self,
        jpeg: &[u8],
        threshold: f32,
    ) -> Result<Vec<FrozenDetection>, String> {
        self.detect_queries(jpeg, LANDMARK_QUERIES, threshold).await
    }

    /// Detect with an explicit query list (the tick loop uses its own set).
    pub async fn detect_queries(
        &self,
        jpeg: &[u8],
        queries: &[&str],
        threshold: f32,
    ) -> Result<Vec<FrozenDetection>, String> {
        #[derive(Deserialize)]
        struct Response {
            detections: Vec<FrozenDetection>,
        }
        let body = serde_json::json!({
            "image_b64": base64_encode(jpeg),
            "queries": queries,
            "threshold": threshold,
        });
        let response = self
            .http
            .post(format!("{}/detect", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("detect: {e}"))?;
        let parsed: Response = response
            .error_for_status()
            .map_err(|e| format!("detect: {e}"))?
            .json()
            .await
            .map_err(|e| format!("detect parse: {e}"))?;
        Ok(parsed.detections)
    }

    /// Score relations between labeled boxes on a frame.
    pub async fn relate(
        &self,
        jpeg: &[u8],
        labels: &[String],
        boxes: &[[i32; 4]],
        vocabulary: &[&str],
    ) -> Result<Vec<FrozenRelation>, String> {
        #[derive(Deserialize)]
        struct Response {
            triplets: Vec<FrozenRelation>,
        }
        let body = serde_json::json!({
            "image_b64": base64_encode(jpeg),
            "labels": labels,
            "boxes": boxes,
            "vocabulary": vocabulary,
            "topk": 30,
        });
        let response = self
            .http
            .post(format!("{}/relate", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("relate: {e}"))?;
        let parsed: Response = response
            .error_for_status()
            .map_err(|e| format!("relate: {e}"))?
            .json()
            .await
            .map_err(|e| format!("relate parse: {e}"))?;
        Ok(parsed.triplets)
    }

    /// Classify the frame against candidate room labels.
    pub async fn classify_room(
        &self,
        jpeg: &[u8],
        rooms: &[&str],
    ) -> Result<FrozenRoom, String> {
        let body = serde_json::json!({
            "image_b64": base64_encode(jpeg),
            "rooms": rooms,
        });
        let response = self
            .http
            .post(format!("{}/room", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("room: {e}"))?;
        response
            .error_for_status()
            .map_err(|e| format!("room: {e}"))?
            .json()
            .await
            .map_err(|e| format!("room parse: {e}"))
    }
}

/// Base64-encode bytes for the sidecar's JSON bodies.
fn base64_encode(jpeg: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_label_strips_articles() {
        assert_eq!(bare_label("a cup"), "cup");
        assert_eq!(bare_label("an oven"), "oven");
        assert_eq!(bare_label("the floor"), "floor");
        assert_eq!(bare_label("trash"), "trash");
    }

    #[test]
    fn tick_inputs_dedupe_and_keep_floor() {
        let detections = vec![
            FrozenDetection { label: "a cup".into(), score: 0.7, r#box: [10, 10, 50, 50] },
            FrozenDetection { label: "the floor".into(), score: 0.6, r#box: [500, 0, 1000, 1000] },
            FrozenDetection { label: "a cup".into(), score: 0.5, r#box: [20, 20, 60, 60] },
        ];
        let (labels, boxes) = tick_inputs(&detections);
        assert_eq!(labels, vec!["cup".to_string(), "floor".to_string()]);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0], [10, 10, 50, 50]);
    }

    #[test]
    fn tick_inputs_synthesize_floor_when_absent() {
        let detections = vec![
            FrozenDetection { label: "socks".into(), score: 0.6, r#box: [5, 5, 40, 40] },
        ];
        let (labels, boxes) = tick_inputs(&detections);
        assert_eq!(labels.last().map(String::as_str), Some("floor"));
        assert_eq!(*boxes.last().unwrap(), SYNTHETIC_FLOOR_BOX);
    }
}
