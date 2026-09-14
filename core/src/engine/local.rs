//! Placeholder for local / on-device inference.
//!
//! Phase F backs this with Florence-2 or Qwen2.5-VL via `candle` or `ort`,
//! running either as a LAN server or compiled into the app via UniFFI.

use async_trait::async_trait;

use crate::domain::{AnalyzeSceneRequest, AnalyzeSceneResponse};

use super::{InferenceError, VisionInferenceEngine};

/// A not-yet-implemented local inference engine.
pub struct LocalEngine {
    endpoint: String,
}

impl LocalEngine {
    /// Create a local engine pointed at an inference endpoint.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }

    /// The configured endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

#[async_trait]
impl VisionInferenceEngine for LocalEngine {
    /// Local inference is not implemented until Phase F.
    async fn analyze_scene(
        &self,
        _req: AnalyzeSceneRequest,
    ) -> Result<AnalyzeSceneResponse, InferenceError> {
        Err(InferenceError::Unavailable(format!(
            "local engine at {} is not implemented (Phase F)",
            self.endpoint
        )))
    }

    /// Identifier for the engine.
    fn name(&self) -> &str {
        "local"
    }
}