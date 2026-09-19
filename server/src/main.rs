//! Lares server entry point.

mod people;
mod routes;
mod state;

use std::sync::Arc;

use lares_core::engine::{
    gemini::{GeminiEngine, DEFAULT_MODEL},
    local::LocalEngine, mock::MockEngine, VisionInferenceEngine,
};
use lares_core::nudge::noop::NoopPolicy;
use lares_core::store::Store;
use tracing_subscriber::EnvFilter;

use state::AppState;

/// Configure, connect, and serve.
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let data_dir = std::env::var("LARES_DATA_DIR").unwrap_or_else(|_| "./data".to_string());
    let bind = std::env::var("LARES_BIND").unwrap_or_else(|_| "0.0.0.0:8787".to_string());

    let engine = build_engine();
    let store = match Store::connect(&data_dir).await {
        Ok(store) => store,
        Err(err) => {
            eprintln!("failed to connect store: {err}");
            std::process::exit(1);
        }
    };

    let app_state = AppState {
        engine: Arc::from(engine),
        policy: Arc::new(NoopPolicy),
        store,
        data_dir: std::path::PathBuf::from(data_dir),
        hud_state: Arc::new(tokio::sync::Mutex::new(lares_core::domain::HudState::default())),
        frozen: std::env::var("LARES_VISION_ENDPOINT").ok().map(|endpoint| {
            std::sync::Arc::new(lares_core::engine::frozen::FrozenVision::new(endpoint))
        }),
    };

    let mut app = routes::router(app_state);
    if let Ok(apk_path) = std::env::var("LARES_APK_PATH") {
        if std::path::Path::new(&apk_path).is_file() {
            app = app.route_service("/apk", tower_http::services::ServeFile::new(apk_path.clone()));
            tracing::info!("serving APK at /apk: {apk_path}");
        }
    }
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("failed to bind {bind}: {err}");
            std::process::exit(1);
        }
    };
    tracing::info!("lares-server listening on {bind}");
    axum::serve(listener, app).await.expect("server failed");
}

/// Build the inference engine selected by `LARES_ENGINE`.
fn build_engine() -> Box<dyn VisionInferenceEngine> {
    let kind = std::env::var("LARES_ENGINE").unwrap_or_else(|_| "mock".to_string());
    match kind.as_str() {
        "gemini" => {
            let api_key = resolve_gemini_key();
            if api_key.is_empty() {
                eprintln!(
                    "GEMINI_API_KEY is required for LARES_ENGINE=gemini (or set GEMINI_API_KEY_FILE)"
                );
                std::process::exit(1);
            }
            let model = std::env::var("LARES_MODEL")
                .unwrap_or_else(|_| DEFAULT_MODEL.to_string());
            Box::new(GeminiEngine::new(api_key, model).expect("failed to build gemini engine"))
        }
        "local" => {
            let endpoint = std::env::var("LARES_LOCAL_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:8866".to_string());
            Box::new(LocalEngine::new(endpoint))
        }
        _ => Box::new(MockEngine),
    }
}

/// Resolve the Gemini API key from the environment or a key file.
///
/// Prefers `GEMINI_API_KEY`; falls back to `GEMINI_API_KEY_FILE` (a file whose
/// first non-empty line is the key), which is convenient for service managers
/// and non-POSIX shells.
fn resolve_gemini_key() -> String {
    let from_env = std::env::var("GEMINI_API_KEY").unwrap_or_default();
    if !from_env.is_empty() {
        return from_env;
    }
    let path = std::env::var("GEMINI_API_KEY_FILE").unwrap_or_default();
    if path.is_empty() {
        return String::new();
    }
    std::fs::read_to_string(&path)
        .map(|contents| contents.trim().to_string())
        .unwrap_or_default()
}