//! HUD state and the layout that renders it.
//!
//! Pure: state in, primitives out. The Android entry point turns
//! primitives into GL calls; the host preview rasterizes them to a PPM.

use crate::font;

/// One directional overlay mark projected from a detection.
#[derive(Debug, Clone)]
pub struct OverlayMark {
    /// Panel x in pixels.
    pub x: i32,
    /// Panel y in pixels.
    pub y: i32,
    /// Short label under the dot.
    pub label: String,
}

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
    /// Directional overlay marks (panel px + label) from the tick loop.
    pub overlay: Vec<OverlayMark>,
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

    // Directional overlay: crosshair dot + contained label per mark.
    for mark in &model.overlay {
        let mx = mark.x.clamp(4 * scale, w - 4 * scale);
        let my = mark.y.clamp(line + gap + 4 * scale, status_y - gap - 4 * scale);
        let (r, g, bl) = (0.0, 1.0, 0.55);
        let _ = (r, g, bl);
        // Small center dot + crosshair arms (1px strokes, scale 2).
        let cs = 2;
        out.push(Prim::Fill { x: mx - cs, y: my - cs, w: 2 * cs, h: 2 * cs, level: Level::Full });
        out.push(Prim::HLine { x: mx - 3 * cs, y: my, len: 2 * cs, level: Level::Full });
        out.push(Prim::HLine { x: mx + cs, y: my, len: 2 * cs, level: Level::Full });
        out.push(Prim::VLine { x: mx, y: my - 3 * cs, len: 2 * cs, level: Level::Full });
        out.push(Prim::VLine { x: mx, y: my + cs, len: 2 * cs, level: Level::Full });

        // Label at half scale, anchored to stay inside the panel.
        let label_scale = (scale / 2).max(1);
        let mut label = mark.label.clone();
        let max_w = w - 2 * margin;
        while font::text_width(&label) * label_scale > max_w && label.len() > 1 {
            label.pop();
        }
        let lw = font::text_width(&label) * label_scale;
        let lx = if mx + lw > w - margin { mx - lw } else { mx };
        let ly = if my + 5 * scale + line > h { my - line - 3 * scale } else { my + 3 * scale };
        text(&mut out, &label, lx.clamp(margin, w - margin), ly, label_scale, Level::Full);
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

/// Build the calibration frame: marker dot at the stage position + step.
pub fn build_calib_frame(stage: i32, w: i32, h: i32, tick: u64) -> Vec<Prim> {
    let scale = font_scale(w);
    let margin = MARGIN * scale;
    let line = LINE * scale;

    let mut out = Vec::with_capacity(1024);
    out.push(Prim::Frame { x: 0, y: 0, w, h, level: Level::Mid });
    text(&mut out, &format!("CAL {}/5", stage + 1), margin, line, scale, Level::Full);
    out.push(Prim::HLine { x: 2, y: line + GAP * scale + 2 * scale, len: w - 4, level: Level::Mid });

    let positions = crate::calib::CALIB_POSITIONS;
    let stage_idx = stage.clamp(0, 4) as usize;
    for (i, (fx, fy)) in positions.iter().enumerate() {
        let px = (fx * w as f64) as i32;
        let py = (fy * h as f64) as i32;
        let level = if i == stage_idx {
            if tick.is_multiple_of(2) { Level::Full } else { Level::Mid }
        } else {
            Level::Dim
        };
        out.push(Prim::Fill {
            x: px - 2 * scale,
            y: py - 2 * scale,
            w: 4 * scale,
            h: 4 * scale,
            level,
        });
    }

    let status_y = h - line - GAP * scale;
    text(&mut out, "DIE UNDER DOT + TAP", margin, status_y, scale, Level::Mid);
    out
}

/// Menu rows in display order.
pub const MENU_ITEMS: [&str; 4] = ["EXIT", "CALIBRATE", "WIPE", "RESUME"];

/// Build the settings menu frame with the given row selected.
///
/// `wipe_label` renders inside the WIPE row (the pause setting).
pub fn build_menu_frame(
    selected: usize,
    wipe_label: &str,
    w: i32,
    h: i32,
    tick: u64,
) -> Vec<Prim> {
    let scale = font_scale(w);
    let margin = MARGIN * scale;
    let line = LINE * scale;
    let gap = GAP * scale;

    let pitch = PITCH * scale;

    let mut out = Vec::with_capacity(2048);
    out.push(Prim::Frame { x: 0, y: 0, w, h, level: Level::Mid });
    text(&mut out, "LARES", margin, line, scale, Level::Full);
    let hint = format!("{}/{}", selected + 1, MENU_ITEMS.len());
    let hw = font::text_width(&hint) * scale;
    text(&mut out, &hint, w - margin - hw, line, scale, Level::Dim);
    out.push(Prim::HLine { x: 2, y: line + gap + 2 * scale, len: w - 4, level: Level::Mid });

    let mut y = line + gap + 2 * scale + line + gap;
    for (i, item) in MENU_ITEMS.iter().enumerate() {
        let is_selected = i == selected.min(MENU_ITEMS.len() - 1);
        let level = if is_selected { Level::Full } else { Level::Dim };
        if is_selected {
            // Selector: filled arrow marker left of the row.
            let ax = margin;
            out.push(Prim::Fill { x: ax, y: y, w: 2 * scale, h: line, level: Level::Full });
            out.push(Prim::Fill { x: ax + 2 * scale, y: y + scale, w: scale, h: line - 2 * scale, level: Level::Full });
        }
        let label = if *item == "WIPE" {
            format!("WIPE {wipe_label}")
        } else {
            (*item).to_string()
        };
        text(&mut out, &label, margin + pitch * 2, y, scale, level);
        y += line + gap;
    }

    // Footer hint + blink cursor.
    let status_y = h - line - gap;
    text(&mut out, "SWIPE+TAP", margin, status_y, scale, Level::Dim);
    if tick.is_multiple_of(2) {
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
