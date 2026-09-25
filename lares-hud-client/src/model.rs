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

impl Prim {
    /// Translate a primitive down the display (panel band centering).
    pub fn shift_y(&mut self, dy: i32) {
        match self {
            Prim::Dot { y, .. }
            | Prim::HLine { y, .. }
            | Prim::VLine { y, .. }
            | Prim::Frame { y, .. }
            | Prim::Fill { y, .. } => *y += dy,
        }
    }
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
    /// Transient status override for the bottom line ("TICK...").
    pub status: String,
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

/// Glyph scale for a display width: big text on wide waveguides.
///
/// 480px-class displays render each font pixel as a 3x3 block (~26
/// characters per line); the 192px host preview keeps 1x proportions.
fn font_scale(w: i32) -> i32 {
    (w / 160).clamp(1, 3)
}

/// Emit one text run as Dot primitives, each font pixel a scale×scale block.
fn text(out: &mut Vec<Prim>, s: &str, x: i32, y: i32, scale: i32, level: Level) {
    font::draw_text(s, x, y, |px, py| {
        for dy in 0..scale {
            for dx in 0..scale {
                out.push(Prim::Dot {
                    x: x + px * scale + dx,
                    y: y + py * scale + dy,
                    level,
                });
            }
        }
    });
}

/// Build the full frame for a display of the given pixel size.
///
/// Layout: outer frame; header `ROOM     TGT:n`; separator; the tag list
/// (up to what fits, freshest first); bottom status line with blink cursor.
pub fn build_frame(model: &HudModel, w: i32, h: i32) -> Vec<Prim> {
    let scale = font_scale(w);
    let margin = MARGIN * scale;
    let line = LINE * scale;
    let gap = GAP * scale;
    let pitch = PITCH * scale;

    let mut out = Vec::with_capacity(4096);
    out.push(Prim::Frame { x: 0, y: 0, w, h, level: Level::Mid });

    // Header: room left, target count right.
    text(&mut out, &model.room.to_uppercase(), margin, line, scale, Level::Full);
    let tgt = format!("TGT:{}", model.targets);
    let tw = font::text_width(&tgt) * scale;
    text(&mut out, &tgt, w - margin - tw, line, scale, Level::Full);
    out.push(Prim::HLine { x: 2, y: line + gap + 2 * scale, len: w - 4, level: Level::Mid });

    // Tags, freshest first, until the space runs out.
    let mut y = line + gap + 2 * scale + line + gap;
    let status_y = h - line - gap;
    let mut order: Vec<usize> = (0..model.tag_titles.len()).collect();
    order.sort_by_key(|&i| model.tag_ages.get(i).copied().unwrap_or(i64::MAX));
    for &i in order.iter() {
        if y + line > status_y - gap {
            break;
        }
        let title = &model.tag_titles[i];
        let age = model.tag_ages.get(i).copied().unwrap_or(i64::MAX);
        let level = age_level(age);
        out.push(Prim::Fill {
            x: margin,
            y: y + 2 * scale,
            w: 2 * scale,
            h: 2 * scale,
            level: Level::Full,
        });
        let mut label = title.to_uppercase();
        let max_w = w - 2 * margin - pitch;
        while font::text_width(&label) * scale > max_w && label.len() > 1 {
            label.pop();
        }
        text(&mut out, &label, margin + pitch, y, scale, level);
        y += line + gap;
    }

    if model.tag_titles.is_empty() {
        text(&mut out, "CLEAR", margin, y, scale, Level::Dim);
    }

    // Status line: transient status, else connection + blink cursor.
    let (status, level) = if !model.status.is_empty() {
        (model.status.clone(), Level::Full)
    } else if model.connected {
        ("LARES".into(), Level::Dim)
    } else {
        ("NO LINK".into(), Level::Full)
    };
    text(&mut out, &status, margin, status_y, scale, level);
    if model.tick.is_multiple_of(2) {
        out.push(Prim::Fill {
            x: w - margin - 4 * scale,
            y: h - gap - 3 * scale,
            w: 4 * scale,
            h: 3 * scale,
            level: Level::Full,
        });
    }
    out
}
