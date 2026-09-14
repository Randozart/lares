//! Shared application state for all request handlers.

use std::path::PathBuf;
use std::sync::Arc;

use lares_core::engine::VisionInferenceEngine;
use lares_core::nudge::ReminderPolicy;
use lares_core::store::Store;

/// State shared by every request handler.
#[derive(Clone)]
pub struct AppState {
    /// The active inference engine (mock, gemini, or local).
    pub engine: Arc<dyn VisionInferenceEngine>,
    /// The reminder policy (NoopPolicy in the MVP).
    pub policy: Arc<dyn ReminderPolicy>,
    /// SQLite persistence.
    pub store: Store,
    /// Directory for stored frame/reference images.
    pub data_dir: PathBuf,
}

impl AppState {
    /// Build the reference-image directory path.
    pub fn refs_dir(&self) -> PathBuf {
        self.data_dir.join("refs")
    }
}