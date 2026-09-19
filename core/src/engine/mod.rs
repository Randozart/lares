//! Pluggable vision inference.
//!
//! All model access goes through [`VisionInferenceEngine`]. Backends are
//! swappable at initialization: cloud Gemini ([`gemini::GeminiEngine`]), a
//! deterministic fake ([`mock::MockEngine`]), or a local stub
//! ([`local::LocalEngine`]) for Phase F.

pub mod frozen;
pub mod gemini;
pub mod local;
pub mod mock;

use async_trait::async_trait;

use crate::domain::{AnalyzeSceneRequest, AnalyzeSceneResponse};

/// One labeled room reference offered to room inference.
#[derive(Debug, Clone)]
pub struct RoomCandidate {
    /// The candidate room id.
    pub room_id: String,
    /// Free-text description captured with the reference.
    pub description: String,
    /// The reference frame, JPEG.
    pub jpeg: Vec<u8>,
}

/// The engine's best guess of which room a frame shows.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomInference {
    /// Best-matching room id.
    pub room_id: String,
    /// Match confidence, 0.0..1.0.
    pub confidence: f32,
}

/// Errors produced by an inference engine.
#[derive(Debug, thiserror::Error)]
pub enum InferenceError {
    /// A network or transport failure.
    #[error("network error: {0}")]
    Network(String),
    /// The engine returned payload that could not be parsed or validated.
    #[error("invalid model response: {0}")]
    InvalidResponse(String),
    /// The engine is missing required configuration.
    #[error("missing configuration: {0}")]
    Config(String),
    /// The engine exists but cannot serve requests yet.
    #[error("engine unavailable: {0}")]
    Unavailable(String),
}

/// Backend-agnostic scene analysis.
#[async_trait]
pub trait VisionInferenceEngine: Send + Sync {
    /// Analyze a scene frame and return grounded chore entities.
    async fn analyze_scene(
        &self,
        req: AnalyzeSceneRequest,
    ) -> Result<AnalyzeSceneResponse, InferenceError>;

    /// Match a frame against labeled room references to infer the room.
    ///
    /// Engines without multimodal matching may leave this unimplemented.
    async fn infer_room(
        &self,
        _frame_jpeg: Vec<u8>,
        _candidates: &[RoomCandidate],
    ) -> Result<RoomInference, InferenceError> {
        Err(InferenceError::Unavailable(
            "room inference not implemented for this engine".to_string(),
        ))
    }

    /// Human-readable engine identifier for logs and responses.
    fn name(&self) -> &str;
}