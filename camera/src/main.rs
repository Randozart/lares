//! Motion-triggered camera node for Lares.
//!
//! Pulls frames from an MJPEG-over-HTTP stream (e.g. an old Android phone
//! running an IP-webcam app), detects scene change via downscaled mean
//! absolute difference, and posts changed frames to `/v1/analyze` tagged
//! with `ANALYZE_SOURCE_CAMERA_NODE`. Fingerprints and reminder matching
//! stay server-side.
//!
//! Configuration via environment:
//! - `LARES_CAMERA_URL` — MJPEG stream URL (required)
//! - `LARES_SERVER` — Lares server base URL (default `http://localhost:8787`)
//! - `LARES_CAMERA_ROOM` — room id (default `cam-room`)
//! - `LARES_CAMERA_THRESHOLD` — mean abs diff (0-255) to count as motion (default 8)
//! - `LARES_CAMERA_COOLDOWN_SECS` — minimum seconds between posts (default 60)

use std::time::{Duration, Instant};

use base64::Engine as _;
use image::DynamicImage;

/// A single JPEG frame extracted from an MJPEG byte stream.
#[derive(Debug, PartialEq)]
pub struct MjpegFrame {
    /// The extracted JPEG bytes, from SOI (FFD8) to EOI (FFD9).
    pub jpeg: Vec<u8>,
    /// Number of bytes consumed from the buffer (frame + delimiter tail).
    pub consumed: usize,
}

/// Scan a buffer for the first complete JPEG frame (SOI..EOI).
///
/// MJPEG multipart bodies are mostly delimiters and headers around raw
/// JPEG bytes; scanning for the markers is simpler and more robust than
/// parsing the multipart envelope. Returns `None` when the buffer holds
/// no complete frame yet.
pub fn next_jpeg(buffer: &[u8]) -> Option<MjpegFrame> {
    let soi = find(buffer, &[0xFF, 0xD8], 0)?;
    let eoi = find(buffer, &[0xFF, 0xD9], soi)?;
    let end = eoi + 2;
    Some(MjpegFrame {
        jpeg: buffer[soi..end].to_vec(),
        consumed: end,
    })
}

/// Find the first index of a byte pattern at or after `from`.
fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Downscaled grayscale mean-absolute-difference between two frames.
///
/// Both frames are resized to 64x36 (16:9 letterbox-insensitive enough for
/// motion gating) before comparison, keeping the cost negligible.
pub fn motion_score(previous: &DynamicImage, current: &DynamicImage) -> f32 {
    let a = previous
        .resize_exact(64, 36, image::imageops::FilterType::CatmullRom)
        .to_luma8();
    let b = current
        .resize_exact(64, 36, image::imageops::FilterType::CatmullRom)
        .to_luma8();
    let (pa, pb) = (a.as_raw(), b.as_raw());
    let total: u64 = pa
        .iter()
        .zip(pb.iter())
        .map(|(x, y)| (*x as i16 - *y as i16).unsigned_abs() as u64)
        .sum();
    total as f32 / pa.len() as f32
}

/// Decode JPEG bytes into an image, rejecting undecodable frames.
fn decode(jpeg: &[u8]) -> Option<DynamicImage> {
    image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()
}

/// Node configuration resolved from the environment.
struct Config {
    stream_url: String,
    server_url: String,
    room: String,
    threshold: f32,
    cooldown: Duration,
}

/// Load configuration, exiting when the stream URL is missing.
fn load_config() -> Config {
    let stream_url = std::env::var("LARES_CAMERA_URL").unwrap_or_default();
    if stream_url.is_empty() {
        eprintln!("LARES_CAMERA_URL is required (an MJPEG stream URL)");
        std::process::exit(1);
    }
    let threshold = std::env::var("LARES_CAMERA_THRESHOLD")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(8.0);
    let cooldown_secs = std::env::var("LARES_CAMERA_COOLDOWN_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60);
    Config {
        stream_url,
        server_url: std::env::var("LARES_SERVER").unwrap_or_else(|_| "http://localhost:8787".into()),
        room: std::env::var("LARES_CAMERA_ROOM").unwrap_or_else(|_| "cam-room".into()),
        threshold,
        cooldown: Duration::from_secs(cooldown_secs),
    }
}

/// Post one frame to the Lares server for analysis.
async fn post_frame(http: &reqwest::Client, config: &Config, jpeg: &[u8]) {
    let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg);
    let body = serde_json::json!({
        "roomId": config.room,
        "mode": "ANALYZE_MODE_DISCOVER",
        "source": "ANALYZE_SOURCE_CAMERA_NODE",
        "frameJpeg": b64,
    });
    let url = format!("{}/v1/analyze", config.server_url.trim_end_matches('/'));
    match http.post(&url).json(&body).send().await {
        Ok(response) => {
            tracing::info!("posted frame → {}", response.status());
        }
        Err(err) => {
            tracing::warn!("post failed: {err}");
        }
    }
}

/// Mutable per-run node state.
struct NodeState {
    /// Previous decoded frame for motion comparison.
    previous: Option<DynamicImage>,
    /// When the last frame was posted, for cooldown gating.
    last_posted: Option<Instant>,
}

/// Drain complete frames from the buffer, tracking motion.
///
/// Returns the first motion frame worth posting, or `None` when the scene
/// is unchanged or the node is still cooling down.
fn absorb_motion_frame(
    buffer: &mut Vec<u8>,
    state: &mut NodeState,
    threshold: f32,
    cooled_down: bool,
) -> Option<Vec<u8>> {
    while let Some(frame) = next_jpeg(buffer) {
        buffer.drain(..frame.consumed);
        let Some(current) = decode(&frame.jpeg) else { continue };
        let score = state
            .previous
            .as_ref()
            .map(|prev| motion_score(prev, &current))
            .unwrap_or(f32::INFINITY);
        state.previous = Some(current);
        if score >= threshold && cooled_down {
            return Some(frame.jpeg);
        }
    }
    None
}

/// Serve one stream connection, posting motion frames until it ends.
async fn stream_once(
    http: &reqwest::Client,
    config: &Config,
    state: &mut NodeState,
    response: reqwest::Response,
) {
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::with_capacity(256 * 1024);
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let Ok(bytes) = chunk else { break };
        buffer.extend_from_slice(&bytes);
        if buffer.len() > 8 * 1024 * 1024 {
            buffer.clear();
        }
        let cooled_down = state
            .last_posted
            .map(|at| at.elapsed() >= config.cooldown)
            .unwrap_or(true);
        let Some(jpeg) =
            absorb_motion_frame(&mut buffer, state, config.threshold, cooled_down)
        else {
            continue;
        };
        tracing::info!("motion detected; posting frame");
        post_frame(http, config, &jpeg).await;
        state.last_posted = Some(Instant::now());
    }
}

/// Stream the MJPEG source, posting frames on motion after each cooldown.
async fn run(config: Config) {
    let http = reqwest::Client::new();
    let mut state = NodeState { previous: None, last_posted: None };
    loop {
        let response = match http.get(&config.stream_url).send().await {
            Ok(response) if response.status().is_success() => response,
            Ok(response) => {
                tracing::warn!("stream http {}; retrying in 10s", response.status());
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
            Err(err) => {
                tracing::warn!("stream error {err}; retrying in 10s");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        stream_once(&http, &config, &mut state, response).await;
        tracing::warn!("stream ended; reconnecting in 5s");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let config = load_config();
    tracing::info!(
        "lares-camera starting: stream={} server={} room={}",
        config.stream_url,
        config.server_url,
        config.room
    );
    run(config).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal JPEG-shaped byte frame around arbitrary content.
    fn jpeg_shape(content: &[u8]) -> Vec<u8> {
        let mut frame = vec![0xFF, 0xD8];
        frame.extend_from_slice(content);
        frame.extend_from_slice(&[0xFF, 0xD9]);
        frame
    }

    #[test]
    fn extracts_frame_from_multipart_stream() {
        let mut stream = b"--BoundaryString\r\nContent-Type: image/jpeg\r\n\r\n".to_vec();
        stream.extend_from_slice(&jpeg_shape(b"frame-one"));
        stream.extend_from_slice(b"\r\n--BoundaryString\r\n");
        let frame = next_jpeg(&stream).unwrap();
        assert_eq!(frame.jpeg, jpeg_shape(b"frame-one"));
        // Consumed exactly up to the EOI marker.
        assert_eq!(&stream[frame.consumed - 2..frame.consumed], &[0xFF, 0xD9]);
    }

    #[test]
    fn returns_none_without_complete_frame() {
        let partial = vec![0xFF, 0xD8, 1, 2, 3];
        assert!(next_jpeg(&partial).is_none());
        let empty = vec![];
        assert!(next_jpeg(&empty).is_none());
    }

    #[test]
    fn identical_frames_have_zero_motion() {
        let image = DynamicImage::new_luma8(320, 240);
        assert_eq!(motion_score(&image, &image), 0.0);
    }

    #[test]
    fn inverted_frames_have_high_motion() {
        let a = DynamicImage::new_luma8(320, 240);
        let mut inverted = image::GrayImage::new(320, 240);
        for pixel in inverted.iter_mut() {
            *pixel = 255;
        }
        let inverted = DynamicImage::ImageLuma8(inverted);
        assert!(motion_score(&a, &inverted) > 100.0);
    }
}
