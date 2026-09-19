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
        #[derive(Deserialize)]
        struct Response {
            detections: Vec<FrozenDetection>,
        }
        let body = serde_json::json!({
            "image_b64": base64_encode(jpeg),
            "queries": LANDMARK_QUERIES,
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
