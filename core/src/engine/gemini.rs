//! Cloud inference via the Gemini API.
//!
//! Uses structured output (`responseMimeType: application/json` plus a
//! `responseSchema`) so the model returns only JSON matching the contract —
//! no prose can leak into the payload.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};

use crate::domain::{
    AnalyzeMode, AnalyzeSceneRequest, AnalyzeSceneResponse, BoundingBox, ChoreEntity,
    ChoreStatus,
};
use crate::prompt::{
    diff_system_prompt, diff_user_prompt, discover_system_prompt, discover_user_prompt,
    response_schema,
};

use super::{InferenceError, VisionInferenceEngine};

/// Default Gemini model identifier, overridable via `LARES_MODEL`.
///
/// Chore detection is a lightweight task; the full flash model is overkill.
/// Lite cuts cost and latency with no measurable accuracy loss for this use.
pub const DEFAULT_MODEL: &str = "gemini-2.5-flash-lite";

/// Longest-side pixel limit for images sent to the model.
const MAX_INPUT_DIM: u32 = 1280;

/// Base URL for the Gemini REST API.
const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Request timeout for a generateContent call.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Gemini-backed inference engine.
pub struct GeminiEngine {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl GeminiEngine {
    /// Build an engine with the given API key and model id.
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, InferenceError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| InferenceError::Config(format!("http client: {e}")))?;
        Ok(Self {
            client,
            api_key: api_key.into(),
            model: model.into(),
        })
    }
}

#[async_trait]
impl VisionInferenceEngine for GeminiEngine {
    /// Analyze a scene through the Gemini generateContent endpoint.
    async fn analyze_scene(
        &self,
        req: AnalyzeSceneRequest,
    ) -> Result<AnalyzeSceneResponse, InferenceError> {
        let start = Instant::now();
        let body = self.build_body(&req);
        let raw = self.post(&body).await?;
        let chores = parse_response(raw)?;
        Ok(AnalyzeSceneResponse {
            chores,
            model: self.name().to_string(),
            latency_ms: start.elapsed().as_millis() as u32,
        })
    }

    /// Identifier for the engine.
    fn name(&self) -> &str {
        "gemini"
    }
}

impl GeminiEngine {
    /// Build the generateContent request body for a scene request.
    fn build_body(&self, req: &AnalyzeSceneRequest) -> Value {
        let (system, user) = match req.mode == AnalyzeMode::Diff as i32 {
            true => (diff_system_prompt(), diff_user_prompt()),
            false => (discover_system_prompt(), discover_user_prompt()),
        };
        let frame = downscale_jpeg(&req.frame_jpeg, MAX_INPUT_DIM);
        let mut parts = Vec::new();
        if req.mode == AnalyzeMode::Diff as i32 {
            if let Some(reference) = req.reference_jpeg.as_deref() {
                parts.push(json!({ "text": "Image A (agreed target state):" }));
                parts.push(inline_image(&downscale_jpeg(reference, MAX_INPUT_DIM)));
            }
            parts.push(json!({ "text": "Image B (current frame):" }));
        } else {
            parts.push(json!({ "text": user }));
        }
        parts.push(inline_image(&frame));

        json!({
            "system_instruction": { "parts": [ { "text": system } ] },
            "contents": [ { "role": "user", "parts": parts } ],
            "generationConfig": generation_config(&self.model)
        })
    }

    /// POST a generateContent body and return the decoded response.
    async fn post(&self, body: &Value) -> Result<Value, InferenceError> {
        let url = format!("{BASE_URL}/models/{}:generateContent", self.model);
        let response = self
            .client
            .post(url)
            .query(&[("key", &self.api_key)])
            .json(body)
            .send()
            .await
            .map_err(|e| InferenceError::Network(e.to_string()))?;
        let status = response.status();
        let value = response
            .json::<Value>()
            .await
            .map_err(|e| InferenceError::Network(format!("decode: {e}")))?;
        if !status.is_success() {
            return Err(InferenceError::Network(format!(
                "gemini http {status}: {value}"
            )));
        }
        Ok(value)
    }
}

/// An inline JPEG part for the request contents.
fn inline_image(jpeg: &[u8]) -> Value {
    let data = base64::engine::general_purpose::STANDARD.encode(jpeg);
    json!({ "inline_data": { "mime_type": "image/jpeg", "data": data } })
}

/// Build the generation configuration, disabling model "thinking".
///
/// Thinking models (the 2.5 family) add seconds of hidden inference before the
/// JSON payload. Chores do not need reasoning, so it is turned off.
fn generation_config(model: &str) -> Value {
    let mut config = json!({
        "responseMimeType": "application/json",
        "responseSchema": response_schema(),
        "maxOutputTokens": 4096
    });
    if model.contains("2.5") {
        config["thinkingConfig"] = json!({ "thinkingBudget": 0 });
    }
    config
}

/// Downscale a JPEG to at most `max_dim` on its longest side.
///
/// Returns the original bytes if decoding or re-encoding fails, so a bad frame
/// degrades to the previous behavior instead of erroring out.
fn downscale_jpeg(jpeg: &[u8], max_dim: u32) -> Vec<u8> {
    let Ok(source) = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg) else {
        return jpeg.to_vec();
    };
    if source.width().max(source.height()) <= max_dim {
        return jpeg.to_vec();
    }
    let scaled = source.resize(
        max_dim,
        max_dim,
        image::imageops::FilterType::Triangle,
    );
    let mut out = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
    if scaled.write_with_encoder(encoder).is_err() {
        return jpeg.to_vec();
    }
    out
}

/// The envelope the model is expected to return.
#[derive(Debug, serde::Deserialize)]
struct RawResponse {
    chores: Vec<RawChore>,
}

/// A single chore as emitted by the model.
#[derive(Debug, serde::Deserialize)]
struct RawChore {
    box_2d: Vec<i32>,
    target: String,
    action: String,
    #[serde(default)]
    estimated_seconds: u32,
    #[serde(default = "default_confidence")]
    confidence: f32,
    #[serde(default)]
    subtasks: Vec<String>,
}

/// Confidence used when the model omits the field.
fn default_confidence() -> f32 {
    0.6
}

/// Extract chores from the raw generateContent response envelope.
fn parse_response(body: Value) -> Result<Vec<ChoreEntity>, InferenceError> {
    let text = body["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or_else(|| {
            InferenceError::InvalidResponse(
                "missing candidates[0].content.parts[0].text".to_string(),
            )
        })?;
    let parsed: RawResponse = serde_json::from_str(text)
        .map_err(|e| InferenceError::InvalidResponse(format!("json parse: {e}")))?;
    Ok(parsed.chores.into_iter().filter_map(raw_to_entity).collect())
}

/// Convert a raw model chore into a contract entity, skipping malformed rows.
fn raw_to_entity(raw: RawChore) -> Option<ChoreEntity> {
    if raw.box_2d.len() != 4 {
        return None;
    }
    Some(ChoreEntity {
        target: raw.target,
        action: raw.action,
        estimated_seconds: raw.estimated_seconds,
        status: ChoreStatus::Discovered as i32,
        r#box: Some(BoundingBox {
            ymin: raw.box_2d[0],
            xmin: raw.box_2d[1],
            ymax: raw.box_2d[2],
            xmax: raw.box_2d[3],
        }),
        confidence: raw.confidence,
        subtasks: raw.subtasks,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_response_envelope() {
        let body = json!({
            "candidates": [{
                "content": { "parts": [ { "text": r#"{"chores":[{"box_2d":[100,200,300,400],"target":"socks","action":"Put socks away","estimated_seconds":20}]}"# } ] }
            }]
        });
        let chores = parse_response(body).unwrap();
        assert_eq!(chores.len(), 1);
        assert_eq!(chores[0].target, "socks");
        assert_eq!(chores[0].estimated_seconds, 20);
        assert_eq!(chores[0].confidence, 0.6);
    }

    #[test]
    fn rejects_missing_text_part() {
        let body = json!({ "candidates": [ { "content": { "parts": [] } } ] });
        assert!(parse_response(body).is_err());
    }

    #[test]
    fn skips_rows_with_wrong_box_cardinality() {
        let body = json!({
            "candidates": [{
                "content": { "parts": [ { "text": r#"{"chores":[{"box_2d":[1,2,3],"target":"x","action":"y","estimated_seconds":1}]}"# } ] }
            }]
        });
        assert!(parse_response(body).unwrap().is_empty());
    }

    #[test]
    fn diff_body_includes_two_images() {
        let engine = GeminiEngine::new("key", "model").unwrap();
        let req = AnalyzeSceneRequest {
            room_id: "kitchen".to_string(),
            frame_jpeg: vec![1, 2, 3],
            reference_jpeg: Some(vec![4, 5, 6]),
            mode: AnalyzeMode::Diff.into(),
        };
        let body = engine.build_body(&req);
        let parts = body["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 4);
        assert_eq!(
            body["generationConfig"]["responseMimeType"],
            "application/json"
        );
    }

    #[test]
    fn thinking_disabled_for_2_5_models() {
        assert_eq!(
            generation_config("gemini-2.5-flash")["thinkingConfig"]["thinkingBudget"],
            0
        );
        assert_eq!(
            generation_config(DEFAULT_MODEL)["thinkingConfig"]["thinkingBudget"],
            0
        );
        assert!(generation_config("gemini-2.0-flash").get("thinkingConfig").is_none());
    }

    #[test]
    fn downscales_oversized_jpeg() {
        let source = image::RgbImage::from_pixel(2000, 1600, image::Rgb([128u8, 64, 32]));
        let mut jpeg = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90);
        source.write_with_encoder(encoder).unwrap();
        let out = downscale_jpeg(&jpeg, 1280);
        assert!(out.len() < jpeg.len());
        let decoded = image::load_from_memory_with_format(&out, image::ImageFormat::Jpeg).unwrap();
        assert!(decoded.width() <= 1280);
        assert!(decoded.height() <= 1280);
    }

    #[test]
    fn small_jpeg_passes_through_unchanged() {
        let source = image::RgbImage::from_pixel(640, 480, image::Rgb([10u8, 20, 30]));
        let mut jpeg = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90);
        source.write_with_encoder(encoder).unwrap();
        assert_eq!(downscale_jpeg(&jpeg, 1280), jpeg);
    }
}