//! A deterministic fake engine for tests, the dashboard, and development
//! without network access or API keys.

use async_trait::async_trait;

use crate::domain::{
    AnalyzeMode, AnalyzeSceneRequest, AnalyzeSceneResponse, BoundingBox, ChoreEntity,
    ChoreStatus,
};

use super::{InferenceError, VisionInferenceEngine};

/// Returns chore boxes that vary by room, mirroring a plausible model output.
pub struct MockEngine;

#[async_trait]
impl VisionInferenceEngine for MockEngine {
    /// Produce a deterministic, room-dependent chore list.
    async fn analyze_scene(
        &self,
        req: AnalyzeSceneRequest,
    ) -> Result<AnalyzeSceneResponse, InferenceError> {
        let start = std::time::Instant::now();
        let chores = mock_chores(req.room_id.as_str(), req.mode);
        Ok(AnalyzeSceneResponse {
            chores,
            landmarks: Vec::new(),
            model: self.name().to_string(),
            latency_ms: start.elapsed().as_millis() as u32,
            scene_class: String::new(),
        })
    }

    /// Deterministic inference: always the first candidate.
    async fn infer_room(
        &self,
        _frame_jpeg: Vec<u8>,
        candidates: &[super::RoomCandidate],
    ) -> Result<super::RoomInference, InferenceError> {
        let first = candidates.first().ok_or_else(|| {
            InferenceError::Config("no room references stored".to_string())
        })?;
        Ok(super::RoomInference {
            room_id: first.room_id.clone(),
            confidence: 0.9,
        })
    }

    /// Identifier for the mock engine.
    fn name(&self) -> &str {
        "mock"
    }
}

/// Build the deterministic chore set for a room and mode.
fn mock_chores(room_id: &str, mode: i32) -> Vec<ChoreEntity> {
    let mut chores = match room_id {
        "kitchen" => vec![
            boxed(
                "counter mess",
                "Move mugs into the dishwasher",
                BoundingBox { ymin: 200, xmin: 100, ymax: 600, xmax: 400 },
                0.96,
            ),
            boxed(
                "sink dishes",
                "Load 2 bowls onto the rack",
                BoundingBox { ymin: 100, xmin: 600, ymax: 700, xmax: 900 },
                0.91,
            ),
        ],
        _ => vec![boxed(
            "scattered socks",
            "Put socks in the laundry hamper",
            BoundingBox { ymin: 300, xmin: 200, ymax: 650, xmax: 550 },
            0.93,
        )],
    };
    if mode == AnalyzeMode::Diff as i32 {
        chores.truncate(1);
    }
    chores
}

/// Construct a boxed chore with the given bounding box and confidence.
fn boxed(target: &str, action: &str, bbox: BoundingBox, confidence: f32) -> ChoreEntity {
    ChoreEntity {
        room_id: String::new(),
        target: target.to_string(),
        action: action.to_string(),
        estimated_seconds: 30,
        status: ChoreStatus::Discovered as i32,
        r#box: Some(bbox),
        confidence,
        subtasks: vec![action.to_string()],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_room_specific_chores() {
        let engine = MockEngine;
        let req = AnalyzeSceneRequest {
            room_id: "kitchen".to_string(),
            frame_jpeg: vec![1, 2, 3],
            reference_jpeg: None,
            mode: AnalyzeMode::Discover.into(),
            sweep_jpegs: Vec::new(),
            source: 0,
            room_area: 0,
        };
        let resp = engine.analyze_scene(req).await.unwrap();
        assert_eq!(resp.chores.len(), 2);
        assert_eq!(resp.model, "mock");
    }

    #[tokio::test]
    async fn mock_diff_truncates_to_deltas() {
        let engine = MockEngine;
        let req = AnalyzeSceneRequest {
            room_id: "kitchen".to_string(),
            frame_jpeg: vec![1, 2, 3],
            reference_jpeg: Some(vec![4, 5, 6]),
            mode: AnalyzeMode::Diff.into(),
            sweep_jpegs: Vec::new(),
            source: 0,
            room_area: 0,
        };
        let resp = engine.analyze_scene(req).await.unwrap();
        assert_eq!(resp.chores.len(), 1);
    }
}