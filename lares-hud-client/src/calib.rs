//! Boresight calibration: display↔camera homography + dice pip solving.
//!
//! Camera and display are rigidly fixed to the glasses, so a single 2D
//! projective mapping (per distance band) converts camera-image positions
//! to display-panel positions. Solved from >= 4 point correspondences via
//! a normalized DLT; the marker is a die pip (dark round blob) found by
//! the vision sidecar.

use std::sync::Mutex;

/// One point correspondence for calibration: display px + camera px,
/// both as frame fractions (0..1) so the mapping is resolution-free.
#[derive(Debug, Clone, Copy)]
pub struct Pair {
    /// Display panel position, fractions.
    pub display: (f64, f64),
    /// Camera image position, fractions.
    pub camera: (f64, f64),
}

/// A 3x3 projective transform, row-major with h33 normalized to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Homography {
    pub m: [f64; 9],
}

/// Solve display→camera and camera→display homographies from pairs.
///
/// Uses a normalized DLT (Hartley: center + scale each plane to unit
/// RMS) for numerical stability with pixel-scale inputs. Returns None
/// for fewer than 4 pairs or degenerate (near-collinear) geometry.
pub fn solve(pairs: &[Pair]) -> Option<(Homography, Homography)> {
    if pairs.len() < 4 {
        return None;
    }
    let to_camera = dlt(
        &pairs.iter().map(|p| p.display).collect::<Vec<_>>(),
        &pairs.iter().map(|p| p.camera).collect::<Vec<_>>(),
    )?;
    let to_display = dlt(
        &pairs.iter().map(|p| p.camera).collect::<Vec<_>>(),
        &pairs.iter().map(|p| p.display).collect::<Vec<_>>(),
    )?;
    Some((to_display, to_camera))
}

/// Project a camera position into display panel coordinates.
pub fn project(h: &Homography, camera: (f64, f64)) -> Option<(f64, f64)> {
    let [a, b, c, d, e, f, g, hh, _] = h.m;
    let w = g * camera.0 + hh * camera.1 + 1.0;
    if w.abs() < 1e-9 {
        return None;
    }
    Some(((a * camera.0 + b * camera.1 + c) / w, (d * camera.0 + e * camera.1 + f) / w))
}

/// Format both homographies for the calib file (dc block then cd block).
pub fn format_pair(to_display: &Homography, to_camera: &Homography) -> String {
    format!(
        "cd {}\ndc {}\n",
        row_string(&to_display.m),
        row_string(&to_camera.m)
    )
}

/// Parse a calib file body into (to_display, to_camera).
pub fn parse_file(text: &str) -> Option<(Homography, Homography)> {
    let mut cd = None;
    let mut dc = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        match parts.next()? {
            "cd" => cd = Some(Homography { m: parse_row(&mut parts)? }),
            "dc" => dc = Some(Homography { m: parse_row(&mut parts)? }),
            _ => {}
        }
    }
    Some((cd?, dc?))
}

fn parse_row<'a, I: Iterator<Item = &'a str>>(parts: &mut I) -> Option<[f64; 9]> {
    let mut m = [0.0; 9];
    for slot in &mut m {
        *slot = parts.next()?.parse().ok()?;
    }
    Some(m)
}

fn row_string(m: &[f64; 9]) -> String {
    m.iter().map(|v| format!("{v:.9}")).collect::<Vec<_>>().join(" ")
}

/// Normalized direct linear transform: src plane → dst plane.
#[allow(clippy::needless_range_loop)]
fn dlt(src: &[(f64, f64)], dst: &[(f64, f64)]) -> Option<Homography> {
    let (src_n, t_src) = normalize(src);
    let (dst_n, t_dst) = normalize(dst);

    let mut a = [[0.0f64; 8]; 8];
    let mut b = [0.0f64; 8];
    for i in 0..4 {
        let (x, y) = src_n[i];
        let (u, v) = dst_n[i];
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y];
        b[2 * i] = u;
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y];
        b[2 * i + 1] = v;
    }
    let h_norm = solve_linear(&a, &b)?;

    // Denormalize: H = T_dst^-1 * H_norm * T_src.
    let h_full = [
        h_norm[0], h_norm[1], h_norm[2], h_norm[3], h_norm[4], h_norm[5], h_norm[6], h_norm[7],
        1.0,
    ];
    // H = T_dst^-1 . H_norm . T_src (T_dst is an affine similarity, so
    // its inverse is computed directly from its parts).
    let inv_s = 1.0 / t_dst[0];
    let t_dst_inv = [
        inv_s, 0.0, -t_dst[2] * inv_s,
        0.0, inv_s, -t_dst[5] * inv_s,
        0.0, 0.0, 1.0,
    ];
    let composed = mul(t_dst_inv, mul(h_full, t_src));
    let mut m = composed;
    let scale = 1.0 / m[8];
    for slot in &mut m {
        *slot *= scale;
    }
    Some(Homography { m })
}

/// Hartley normalization: translate centroid to origin, scale mean
/// distance to sqrt(2). Returns normalized points + the transform that
/// produced them (for denormalization).
fn normalize(points: &[(f64, f64)]) -> (Vec<(f64, f64)>, [f64; 9]) {
    let n = points.len() as f64;
    let cx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let cy = points.iter().map(|p| p.1).sum::<f64>() / n;
    let mean_dist = points
        .iter()
        .map(|p| ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt())
        .sum::<f64>()
        / n;
    let scale = if mean_dist > 1e-9 { (2.0f64).sqrt() / mean_dist } else { 1.0 };
    let normalized = points
        .iter()
        .map(|p| ((p.0 - cx) * scale, (p.1 - cy) * scale))
        .collect();
    // T = [scale 0 -scale*cx; 0 scale -scale*cy; 0 0 1]
    (
        normalized,
        [scale, 0.0, -scale * cx, 0.0, scale, -scale * cy, 0.0, 0.0, 1.0],
    )
}

fn mul(a: [f64; 9], b: [f64; 9]) -> [f64; 9] {
    let mut out = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 3 + c] = (0..3)
                .map(|k| a[r * 3 + k] * b[k * 3 + c])
                .sum();
        }
    }
    out
}

/// Gaussian elimination with partial pivoting on an 8x8 system.
fn solve_linear(a: &[[f64; 8]; 8], b: &[f64; 8]) -> Option<[f64; 8]> {
    let mut m = *a;
    let mut rhs = *b;
    for col in 0..8 {
        let pivot = (col..8).fold(col, |best, r| if m[r][col].abs() > m[best][col].abs() { r } else { best });
        if m[pivot][col].abs() < 1e-12 {
            return None;
        }
        m.swap(pivot, col);
        rhs.swap(pivot, col);
        let inv = 1.0 / m[col][col];
        for r in 0..8 {
            if r == col {
                continue;
            }
            let factor = m[r][col] * inv;
            if factor == 0.0 {
                continue;
            }
            let row: [f64; 8] = m[col];
            for (c, val) in row.iter().enumerate() {
                m[r][c] -= factor * val;
            }
            rhs[r] -= factor * rhs[col];
        }
    }
    let mut out = [0.0; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = rhs[i] * (1.0 / m[i][i]);
    }
    Some(out)
}

// ── Device state ─────────────────────────────────────────────────────

use std::sync::atomic::{AtomicI32, Ordering};

/// -1 = off; 0..=4 = awaiting capture at that stage.
pub static CALIB_STAGE: AtomicI32 = AtomicI32::new(-1);

static PAIRS: Mutex<Vec<Pair>> = Mutex::new(Vec::new());
static CALIB_H: Mutex<Option<(Homography, Homography)>> = Mutex::new(None);

/// Panel size used for calibration positions and overlay clamping.
pub const CALIB_PANEL: i32 = 480;

/// Calib file path on the device.
pub const CALIB_FILE: &str = "/sdcard/lares-calib.txt";

/// Marker display positions as panel fractions: center + 4 quadrants.
pub const CALIB_POSITIONS: [(f64, f64); 5] =
    [(0.5, 0.5), (0.28, 0.28), (0.72, 0.28), (0.28, 0.72), (0.72, 0.72)];

/// Cancel an in-progress session.
pub fn cancel() {
    CALIB_STAGE.store(-1, Ordering::Relaxed);
    PAIRS.lock().unwrap().clear();
}

/// Start a calibration session at stage 0.
pub fn start_calib() {
    PAIRS.lock().unwrap().clear();
    CALIB_STAGE.store(0, Ordering::Relaxed);
}

/// Whether a calibration session is in progress.
pub fn calib_active() -> bool {
    CALIB_STAGE.load(Ordering::Relaxed) >= 0
}

/// Current stage (0..5), or -1 when off.
pub fn calib_stage() -> i32 {
    CALIB_STAGE.load(Ordering::Relaxed)
}

/// Panel pixel position of the current stage's marker dot.
pub fn stage_marker_px() -> (i32, i32) {
    let stage = calib_stage().clamp(0, 4) as usize;
    let (fx, fy) = CALIB_POSITIONS[stage];
    ((fx * CALIB_PANEL as f64) as i32, (fy * CALIB_PANEL as f64) as i32)
}

/// Record a correspondence for the current stage from camera fractions.
pub fn push_camera_point(camera: (f64, f64)) -> i32 {
    let stage = calib_stage();
    if stage < 0 {
        return -1;
    }
    let (fx, fy) = CALIB_POSITIONS[stage as usize];
    PAIRS.lock().unwrap().push(Pair {
        display: (fx, fy),
        camera,
    });
    let next = stage + 1;
    CALIB_STAGE.store(if next > 4 { 5 } else { next }, Ordering::Relaxed);
    next
}

/// Solve both homographies from the collected pairs and stash them.
pub fn finish_calib() -> Option<(Homography, Homography)> {
    let pairs = PAIRS.lock().unwrap().clone();
    let pair = solve(&pairs)?;
    *CALIB_H.lock().unwrap() = Some(pair);
    Some(pair)
}

/// Load homographies from a calib file (e.g. /sdcard/lares-calib.txt).
pub fn load_from_file(path: &str) -> Option<(Homography, Homography)> {
    let text = std::fs::read_to_string(path).ok()?;
    let pair = parse_file(&text)?;
    *CALIB_H.lock().unwrap() = Some(pair);
    Some(pair)
}

/// Persist the current homographies to a calib file.
pub fn save_to_file(path: &str) -> Option<()> {
    let guard = CALIB_H.lock().unwrap();
    let (cd, dc) = guard.as_ref()?;
    std::fs::write(path, format_pair(cd, dc)).ok()
}

/// Project a camera-fraction position into panel pixels via the loaded
/// camera→display homography; None when uncalibrated or degenerate.
pub fn project_panel(camera: (f64, f64)) -> Option<(i32, i32)> {
    let guard = CALIB_H.lock().unwrap();
    let (cd, _) = guard.as_ref()?;
    let (fx, fy) = project(cd, camera)?;
    Some(((fx * CALIB_PANEL as f64) as i32, (fy * CALIB_PANEL as f64) as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_solves_and_projects() {
        // Non-collinear spread (a grid) — collinear sets are degenerate.
        let pts = [(0.1, 0.1), (0.8, 0.15), (0.15, 0.8), (0.85, 0.9), (0.5, 0.5)];
        let pairs: Vec<Pair> = pts
            .iter()
            .map(|p| Pair { display: *p, camera: *p })
            .collect();
        let (to_display, _) = solve(&pairs).unwrap();
        let (x, y) = project(&to_display, (0.3, 0.15)).unwrap();
        assert!((x - 0.3).abs() < 1e-6 && (y - 0.15).abs() < 1e-6);
    }

    #[test]
    fn synthetic_perspective_recovers() {
        // A genuine projective map with perspective terms.
        let h = Homography { m: [1.2, 0.1, 0.05, -0.05, 0.9, 0.2, 0.0004, -0.0003, 1.0] };
        let cams = [(0.1, 0.9), (0.5, 0.1), (0.9, 0.8), (0.2, 0.3), (0.7, 0.5)];
        let pairs: Vec<Pair> = cams
            .iter()
            .map(|camera| {
                let display = project(&h, *camera).unwrap();
                Pair { display, camera: *camera }
            })
            .collect();
        let (to_display, _) = solve(&pairs).unwrap();
        for pair in &pairs {
            let (x, y) = project(&to_display, pair.camera).unwrap();
            assert!((x - pair.display.0).abs() < 1e-6, "x mismatch: {x}");
            assert!((y - pair.display.1).abs() < 1e-6, "y mismatch: {y}");
        }
    }

    #[test]
    fn fewer_than_four_pairs_is_none() {
        let pairs = vec![Pair { display: (0.1, 0.1), camera: (0.2, 0.2) }];
        assert!(solve(&pairs).is_none());
    }

    #[test]
    fn degenerate_collinear_is_none() {
        let pairs: Vec<Pair> = (0..5)
            .map(|i| Pair {
                display: (i as f64 * 0.1, i as f64 * 0.1),
                camera: (i as f64 * 0.2, i as f64 * 0.2),
            })
            .collect();
        assert!(solve(&pairs).is_none());
    }

    #[test]
    fn file_round_trip() {
        let text = format!(
            "cd {}\ndc {}\n",
            row_string(&[1.0; 9]),
            row_string(&[2.0; 9])
        );
        let (cd, dc) = parse_file(&text).unwrap();
        assert_eq!(cd.m, [1.0; 9]);
        assert_eq!(dc.m, [2.0; 9]);
    }
}
