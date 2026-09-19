//! HUD state and the layout that renders it.
//!
//! Pure: state in, primitives out. The Android entry point turns
//! primitives into GL calls; the host preview rasterizes them to a PPM.

use crate::font;

/// Brightness level for a primitive (mono waveguide: luminance only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    /// Full luminance — fresh, primary.
    Full,
    /// Two-thirds — secondary structure.
    Mid,
    /// One-third — aged or tertiary.
    Dim,
}

/// One drawable primitive in display space (y down).
#[derive(Debug, Clone, Copy)]
pub enum Prim {
    /// Single lit pixel.
    Dot { x: i32, y: i32, level: Level },
    /// Horizontal line, inclusive.
    HLine { x: i32, y: i32, len: i32, level: Level },
    /// Vertical line, inclusive.
    VLine { x: i32, y: i32, len: i32, level: Level },
    /// Rectangle outline.
    Frame { x: i32, y: i32, w: i32, h: i32, level: Level },
    /// Filled rectangle (cursor, badges).
    Fill { x: i32, y: i32, w: i32, h: i32, level: Level },
}

/// The live data a HUD frame renders.
#[derive(Debug, Clone, Default)]
pub struct HudModel {
    /// Current room name, uppercase ("KITCHEN").
    pub room: String,
    /// Live scan-target count.
    pub targets: u32,
    /// Titles of the live targets.
    pub tag_titles: Vec<String>,
    /// Ages of the tags, parallel to `tag_titles` (seconds).
    pub tag_ages: Vec<i64>,
    /// Whether the last poll succeeded (drives the status line).
    pub connected: bool,
    /// Monotonic seconds, for the blink cursor.
    pub tick: u64,
}

const PITCH: i32 = 6;
const MARGIN: i32 = 3;
const LINE: i32 = 9;
const GAP: i32 = 4;

/// Brightness for a tag of the given age.
fn age_level(age: i64) -> Level {
    match age {
        a if a < 60 => Level::Full,
        a if a < 300 => Level::Mid,
        _ => Level::Dim,
    }
}

/// Emit one text run as Dot primitives.
fn text(out: &mut Vec<Prim>, s: &str, x: i32, y: i32, level: Level) {
    font::draw_text(s, x, y, |px, py| out.push(Prim::Dot { x: px, y: py, level }));
}

/// Build the full frame for a display of the given pixel size.
///
/// Layout: outer frame; header `ROOM     TGT:n`; separator; the tag list
/// (up to what fits, freshest first); bottom status line with blink cursor.
pub fn build_frame(model: &HudModel, w: i32, h: i32) -> Vec<Prim> {
    let mut out = Vec::with_capacity(2048);
    out.push(Prim::Frame { x: 0, y: 0, w, h, level: Level::Mid });

    // Header.
    text(&mut out, &model.room.to_uppercase(), MARGIN, LINE, Level::Full);
    let tgt = format!("TGT:{}", model.targets);
    let tw = font::text_width(&tgt);
    text(&mut out, &tgt, w - MARGIN - tw, LINE, Level::Full);
    out.push(Prim::HLine { x: 2, y: LINE + GAP + 5, len: w - 4, level: Level::Mid });

    // Tags, freshest first, until the space runs out.
    let mut y = LINE + GAP + 5 + LINE + GAP;
    let status_y = h - LINE - GAP;
    let mut order: Vec<usize> = (0..model.tag_titles.len()).collect();
    order.sort_by_key(|&i| model.tag_ages.get(i).copied().unwrap_or(i64::MAX));
    for &i in order.iter() {
        if y + LINE > status_y - GAP {
            break;
        }
        let title = &model.tag_titles[i];
        let age = model.tag_ages.get(i).copied().unwrap_or(i64::MAX);
        let level = age_level(age);
        out.push(Prim::Dot {
            x: MARGIN,
            y: y + 2,
            level: Level::Full,
        });
        let mut label = title.to_uppercase();
        let max_w = w - 2 * MARGIN - PITCH;
        while font::text_width(&label) > max_w && label.len() > 1 {
            label.pop();
        }
        text(&mut out, &label, MARGIN + PITCH, y, level);
        y += LINE + GAP;
    }

    if model.tag_titles.is_empty() {
        text(&mut out, "CLEAR", MARGIN, y, Level::Dim);
    }

    // Status line: connection + blink cursor.
    let status = if model.connected { "LARES" } else { "NO LINK" };
    text(&mut out, status, MARGIN, status_y, if model.connected { Level::Dim } else { Level::Full });
    if model.tick.is_multiple_of(2) {
        out.push(Prim::Fill { x: w - MARGIN - 6, y: h - GAP - 5, w: 6, h: 5, level: Level::Full });
    }
    out
}
