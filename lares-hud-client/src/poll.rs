//! Server polling: GET /v1/hud every couple of seconds, no drama.
//!
//! Plain-thread blocking HTTP/1.1 over std::net — the server speaks
//! plain HTTP inside the tailnet, so there is no TLS stack here, no
//! async runtime, nothing but sockets and serde. Failures keep the last
//! state and flip `connected`, which the renderer shows as NO LINK.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::AtomicBool;
#[cfg(target_os = "android")]
use std::sync::atomic::Ordering;
#[cfg(target_os = "android")]
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;

use crate::model::HudModel;

/// One tag as the server serializes it.
#[derive(Debug, Deserialize)]
pub struct Tag {
    pub id: String,
    #[allow(dead_code)]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub snippet: String,
    #[serde(default)]
    pub age: i64,
}

/// The /v1/hud payload.
#[derive(Debug, Deserialize)]
pub struct HudResponse {
    #[serde(default)]
    pub room: String,
    #[serde(default)]
    pub targets: u32,
    #[serde(default)]
    pub tags: Vec<Tag>,
}

/// One misplaced-object candidate from a head-mounted tick.
#[derive(Debug, Deserialize)]
pub struct TickCandidate {
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub action: String,
}

/// The /v1/tick payload.
#[derive(Debug, Deserialize)]
pub struct TickResponse {
    #[serde(default)]
    pub room_id: String,
    #[serde(default)]
    pub scene_class: String,
    #[serde(default)]
    pub candidates: Vec<TickCandidate>,
}

/// Minimal HTTP/1.1 POST with a JSON body.
pub fn http_post(url: &str, body: &str) -> std::io::Result<String> {
    let (hostport, path) = split_url(url);
    let mut stream = TcpStream::connect(hostport)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let body_start = match text.find("\r\n\r\n") {
        Some(i) => i + 4,
        None => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "no header")),
    };
    Ok(text[body_start..].to_string())
}

/// Adopt tick candidates into the render model as fresh tags.
pub fn apply_tick(shared: &Shared, response: &TickResponse, now: u64) {
    let mut model = shared.model.lock().unwrap();
    if !response.room_id.is_empty() {
        model.room = response.room_id.clone();
    }
    model.tag_titles = response
        .candidates
        .iter()
        .map(|c| {
            if c.action.is_empty() {
                c.target.clone()
            } else {
                c.action.clone()
            }
        })
        .collect();
    model.tag_ages = response.candidates.iter().map(|_| 0).collect();
    model.connected = true;
    model.tick = now;
}

/// State shared between the poll thread and the render loop.
#[derive(Default)]
pub struct Shared {
    pub model: Mutex<HudModel>,
    pub blanked: AtomicBool,
}

/// Server URL, compiled in via LARES_HUD_SERVER at build time.
pub const SERVER_URL: &str = match option_env!("LARES_HUD_SERVER") {
    Some(url) => url,
    None => "http://100.111.244.0:8787",
};

/// Poll interval.
pub const POLL_SECS: u64 = 2;

/// Extract host:port and path from an http:// URL (the only form used).
pub fn split_url(url: &str) -> (&str, &str) {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    }
}

/// Minimal HTTP/1.1 GET returning the response body.
///
/// Handles Content-Length and connection-close framing; that is the
/// entire protocol surface this client needs.
pub fn http_get(url: &str) -> std::io::Result<String> {
    let (hostport, path) = split_url(url);
    let mut stream = TcpStream::connect(hostport)?;
    stream.set_read_timeout(Some(Duration::from_secs(8)))?;
    stream.set_write_timeout(Some(Duration::from_secs(8)))?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let body = match text.find("\r\n\r\n") {
        Some(i) => &text[i + 4..],
        None => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "no header")),
    };
    Ok(body.to_string())
}

/// Translate a wire response into the render model.
pub fn apply(shared: &Shared, response: &HudResponse, now: u64) {
    let mut model = shared.model.lock().unwrap();
    model.room = response.room.clone();
    model.targets = response.targets;
    model.tag_titles = response.tags.iter().map(|t| t.title.clone()).collect();
    model.tag_ages = response.tags.iter().map(|t| t.age).collect();
    model.connected = true;
    model.tick = now;
}

/// Spawn the poll thread.
#[cfg(target_os = "android")]
pub fn spawn(shared: Arc<Shared>) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("hud-poll".into())
        .spawn(move || poll_loop(shared))
}

#[cfg(target_os = "android")]
fn poll_loop(shared: Arc<Shared>) {
    loop {
        match http_get(SERVER_URL) {
            Ok(body) => match serde_json::from_str::<HudResponse>(&body) {
                Ok(parsed) => apply(&shared, &parsed, tick_now()),
                Err(e) => mark_down(&shared, e.into()),
            },
            Err(e) => mark_down(&shared, e.into()),
        }
        std::thread::sleep(Duration::from_secs(POLL_SECS));
    }
}

#[cfg(target_os = "android")]
fn mark_down(shared: &Shared, error: std::io::Error) {
    let mut model = shared.model.lock().unwrap();
    model.connected = false;
    model.tick = tick_now();
    let _ = error;
}

/// Seconds since the epoch (drives the blink cursor).
#[cfg(target_os = "android")]
pub fn tick_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_url_separates_host_and_path() {
        assert_eq!(split_url("http://1.2.3.4:8787/v1/hud"), ("1.2.3.4:8787", "/v1/hud"));
        assert_eq!(split_url("1.2.3.4:8787"), ("1.2.3.4:8787", "/"));
    }

    #[test]
    fn apply_maps_wire_to_model() {
        let shared = Shared::default();
        let response = HudResponse {
            room: "kitchen".into(),
            targets: 2,
            tags: vec![Tag {
                id: "vision:0:socks".into(),
                kind: "VISION".into(),
                title: "socks".into(),
                snippet: String::new(),
                age: 12,
            }],
        };
        apply(&shared, &response, 7);
        let model = shared.model.lock().unwrap();
        assert_eq!(model.room, "kitchen");
        assert_eq!(model.tag_titles, vec!["socks".to_string()]);
        assert!(model.connected);
    }
}
