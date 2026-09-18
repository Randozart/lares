//! Local inference via an OpenAI-compatible vision server.
//!
//! Speaks the `llama-server` chat-completions dialect with base64
//! `image_url` content parts and a strict `json_schema` response format.
//! Point `LARES_LOCAL_ENDPOINT` at a running llama.cpp server hosting a
//! grounded vision model (Qwen2.5-VL with an mmproj); see
//! `scripts/alpha-setup.sh` for the one-time setup on the inference box.

use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::domain::{area_display_name, AnalyzeSceneRequest, AnalyzeSceneResponse, RoomArea};
use crate::prompt::{
    diff_system_prompt, diff_user_prompt, discover_system_prompt, discover_user_prompt,
    response_schema, sweep_system_prompt, sweep_user_prompt,
};

use super::gemini::parse_scene_text;
use super::gemini::MAX_INPUT_DIM;
use super::{InferenceError, VisionInferenceEngine};

/// Timeout for a local completion; small models can still be slow on CPU.
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// A local inference engine speaking the OpenAI chat-completions protocol.
pub struct LocalEngine {
    /// Base URL, e.g. `http://alpha:8866`.
    endpoint: String,
    /// Model id as served by the local runtime.
    model: String,
    /// Shared HTTP client.
    http: reqwest::Client,
}

impl LocalEngine {
    /// Create a local engine pointed at an inference endpoint.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: "qwen2.5-vl".to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .expect("local engine http client"),
        }
    }

    /// The configured endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Build the chat-completions request body for a scene request.
    fn build_body(&self, req: &AnalyzeSceneRequest) -> Value {
        let area = area_display_name(RoomArea::try_from(req.room_area).unwrap_or_default());
        let mut image_jpegs: Vec<&[u8]> = Vec::new();
        let (system, user) = if !req.sweep_jpegs.is_empty() {
            image_jpegs = req.sweep_jpegs.iter().map(|jpeg| jpeg.as_slice()).collect();
            (sweep_system_prompt(), sweep_user_prompt(area))
        } else {
            image_jpegs.push(&req.frame_jpeg);
            if let Some(reference) = req.reference_jpeg.as_ref() {
                image_jpegs.push(reference);
            }
            match req.mode == crate::domain::AnalyzeMode::Diff as i32 {
                true => (diff_system_prompt(), diff_user_prompt(area)),
                false => (discover_system_prompt(area), discover_user_prompt(area)),
            }
        };
        let mut content = vec![json!({ "type": "text", "text": user })];
        for jpeg in &image_jpegs {
            content.push(json!({
                "type": "image_url",
                "image_url": { "url": data_url(jpeg) }
            }));
        }
        json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": content }
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": "scene", "schema": response_schema() }
            },
            "temperature": 0
        })
    }

    /// Send the request and parse the scene from the reply.
    async fn analyze_impl(
        &self,
        req: AnalyzeSceneRequest,
    ) -> Result<AnalyzeSceneResponse, InferenceError> {
        let start = std::time::Instant::now();
        let body = self.build_body(&req);
        let url = format!("{}/v1/chat/completions", self.endpoint.trim_end_matches('/'));
        let response = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| InferenceError::Network(format!("{e}")))?;
        if !response.status().is_success() {
            return Err(InferenceError::Unavailable(format!(
                "local server http {}",
                response.status()
            )));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| InferenceError::InvalidResponse(format!("body: {e}")))?;
        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                InferenceError::InvalidResponse("missing choices[0].message.content".to_string())
            })?;
        let scene = parse_scene_text(text)?;
        Ok(AnalyzeSceneResponse {
            chores: scene.chores,
            landmarks: scene.landmarks,
            model: self.model.clone(),
            latency_ms: start.elapsed().as_millis() as u32,
        })
    }
}

/// Encode JPEG bytes as a base64 data URL.
fn data_url(jpeg: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg);
    format!("data:image/jpeg;base64,{b64}")
}

#[async_trait]
impl VisionInferenceEngine for LocalEngine {
    /// Analyze a scene frame against the local vision server.
    async fn analyze_scene(
        &self,
        req: AnalyzeSceneRequest,
    ) -> Result<AnalyzeSceneResponse, InferenceError> {
        let mut req = req;
        for jpeg in req.sweep_jpegs.iter_mut() {
            *jpeg = super::gemini::downscale_jpeg(jpeg, MAX_INPUT_DIM);
        }
        req.frame_jpeg = super::gemini::downscale_jpeg(&req.frame_jpeg, MAX_INPUT_DIM);
        if let Some(reference) = req.reference_jpeg.as_mut() {
            *reference = super::gemini::downscale_jpeg(reference, MAX_INPUT_DIM);
        }
        self.analyze_impl(req).await
    }

    /// Identifier for the engine.
    fn name(&self) -> &str {
        "local"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AnalyzeMode, ChoreEntity};

    #[test]
    fn body_carries_images_and_schema() {
        let engine = LocalEngine::new("http://localhost:8866");
        let req = AnalyzeSceneRequest {
            room_id: "kitchen".to_string(),
            frame_jpeg: vec![1, 2, 3],
            reference_jpeg: None,
            mode: AnalyzeMode::Discover.into(),
            sweep_jpegs: Vec::new(),
            room_area: 0,
            source: 0,
        };
        let body = engine.build_body(&req);
        let content = body["messages"][1]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[1]["type"], "image_url");
        assert!(content[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,"));
        assert_eq!(
            body["response_format"]["type"],
            "json_schema"
        );
    }

    #[test]
    fn parses_openai_shaped_reply() {
        let reply = json!({
            "choices": [{
                "message": {
                    "content": "{\"chores\":[{\"box_2d\":[100,200,300,400],\"target\":\"socks\",\"action\":\"Put away\",\"estimated_seconds\":20}],\"landmarks\":[]}"
                }
            }]
        });
        let text = reply["choices"][0]["message"]["content"].as_str().unwrap();
        let scene = parse_scene_text(text).unwrap();
        assert_eq!(scene.chores.len(), 1);
        let chore: &ChoreEntity = &scene.chores[0];
        assert_eq!(chore.target, "socks");
    }
}
