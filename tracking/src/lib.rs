//! Lares on-device visual tracking.
//!
//! Provides the fast-loop glue between VLM anchors: normalized cross-tracked
//! chore boxes, whole-frame fingerprints for the cost guard, and landmark
//! patch extraction. Exposed to the Android client via UniFFI so the tracking
//! logic lives in Rust and can be unit-tested headlessly.

uniffi::setup_scaffolding!();

/// A grayscale camera frame (the Y plane of a YUV image).
#[derive(uniffi::Record)]
pub struct GrayFrame {
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in pixels.
    pub height: u32,
    /// Byte distance between consecutive rows (may exceed width).
    pub row_stride: u32,
    /// Grayscale bytes, `row_stride` per row.
    pub data: Vec<u8>,
}

/// A tracked box in normalized coordinates (0..=1000).
#[derive(uniffi::Record, Clone)]
pub struct TrackedBox {
    /// Stable identifier from the VLM anchor.
    pub id: String,
    /// Left edge, normalized 0..=1000.
    pub xmin: f32,
    /// Top edge, normalized 0..=1000.
    pub ymin: f32,
    /// Right edge, normalized 0..=1000.
    pub xmax: f32,
    /// Bottom edge, normalized 0..=1000.
    pub ymax: f32,
    /// Match confidence in the latest frame; fades as the box is lost.
    pub confidence: f32,
}

/// A cropped grayscale region (e.g. a landmark patch).
#[derive(uniffi::Record)]
pub struct ImagePatch {
    /// Patch width in pixels.
    pub width: u32,
    /// Patch height in pixels.
    pub height: u32,
    /// Grayscale bytes, row-major, `width` per row.
    pub data: Vec<u8>,
}

/// Match confidence below which a box is treated as lost.
const MIN_CONFIDENCE: f32 = 0.75;

/// Coarse-to-fine search half-width (pixels) around the last position.
const DEFAULT_SEARCH_HALF: u32 = 48;

/// EMA factor applied toward the matched position each frame.
const EMA_ALPHA: f32 = 0.6;

/// Confidence multiplier applied while a box is lost.
const LOST_DECAY: f32 = 0.5;

/// Grid dimension for the scene fingerprint (produces 64 bits).
const FINGERPRINT_GRID: u32 = 8;

/// A single anchored patch with its last known position.
struct AnchorPatch {
    /// Identifier from the VLM anchor.
    id: String,
    /// Subsampled grayscale patch bytes.
    patch: Vec<u8>,
    /// Patch width in subsampled pixels.
    pw: u32,
    /// Patch height in subsampled pixels.
    ph: u32,
    /// Subsampling stride used when the patch was extracted.
    step: u32,
    /// Physical box width in pixels at anchor time.
    bw_px: u32,
    /// Physical box height in pixels at anchor time.
    bh_px: u32,
    /// Physical half-width in pixels, for search placement.
    hw_px: u32,
    /// Physical half-height in pixels, for search placement.
    hh_px: u32,
    /// Center position in anchor-frame pixels.
    pos: (u32, u32),
    /// Anchor frame dimensions, for normalized <-> pixel mapping.
    frame_w: u32,
    /// Anchor frame height.
    frame_h: u32,
    /// Consecutive lost frames; the next VLM anchor clears it.
    lost: u32,
    /// Latest match confidence; used to fade boxes as they are lost.
    confidence: f32,
}

/// The live tracker. Methods take `&self` with interior mutability because
/// UniFFI shares exported objects behind an `Arc`.
#[derive(uniffi::Object)]
pub struct LaresTracker {
    /// Anchored patches currently being tracked.
    patches: std::sync::Mutex<Vec<AnchorPatch>>,
    /// Subsampling stride for patch extraction and matching.
    subsample: u32,
    /// Search half-width in pixels.
    search_half: u32,
}

/// Recover a poisoned lock's value instead of panicking.
fn lock_patches(
    patches: &std::sync::Mutex<Vec<AnchorPatch>>,
) -> std::sync::MutexGuard<'_, Vec<AnchorPatch>> {
    patches.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A grayscale block region within a frame.
struct Block {
    /// Top row.
    r0: u32,
    /// Left column.
    c0: u32,
    /// Row past the bottom (exclusive).
    r1: u32,
    /// Column past the right (exclusive).
    c1: u32,
}

/// Sample a grayscale region's mean luminance on a stride grid.
fn block_mean(frame: &GrayFrame, block: &Block, step: u32) -> u8 {
    let step = step.max(1) as usize;
    let r0 = block.r0.min(frame.height) as usize;
    let c0 = block.c0.min(frame.width) as usize;
    let r1 = block.r1.min(frame.height) as usize;
    let c1 = block.c1.min(frame.width) as usize;
    let rows = (r1 - r0).div_ceil(step);
    let cols = (c1 - c0).div_ceil(step);
    let mut sum: u64 = 0;
    let mut n: u64 = 0;
    for i in 0..(rows * cols) {
        let r = r0 + (i / cols) * step;
        let c = c0 + (i % cols) * step;
        let idx = r * frame.row_stride as usize + c;
        sum += frame.data.get(idx).copied().unwrap_or(0) as u64;
        n += 1;
    }
    if n == 0 {
        return 0;
    }
    (sum / n) as u8
}

/// Convert a normalized box to an in-bounds pixel rectangle.
fn norm_to_px(box_: &TrackedBox, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let w = w.max(1) as f32;
    let h = h.max(1) as f32;
    let x0 = (box_.xmin / 1000.0 * w).round().clamp(0.0, w - 1.0) as u32;
    let y0 = (box_.ymin / 1000.0 * h).round().clamp(0.0, h - 1.0) as u32;
    let x1 = (box_.xmax / 1000.0 * w).round().clamp(x0 as f32 + 1.0, w) as u32;
    let y1 = (box_.ymax / 1000.0 * h).round().clamp(y0 as f32 + 1.0, h) as u32;
    (x0, y0, x1, y1)
}

/// Convert a pixel center plus pixel extents back to a normalized box.
impl AnchorPatch {
    /// Map the current pixel center back to a normalized box.
    fn to_norm(&self) -> TrackedBox {
        let (cx, cy) = self.pos;
        let w = self.frame_w.max(1) as f32;
        let h = self.frame_h.max(1) as f32;
        let half_w = (self.bw_px as f32 / 2.0).min(w / 2.0);
        let half_h = (self.bh_px as f32 / 2.0).min(h / 2.0);
        let cx = cx as f32;
        let cy = cy as f32;
        TrackedBox {
            id: String::new(),
            xmin: ((cx - half_w) / w * 1000.0).clamp(0.0, 1000.0),
            ymin: ((cy - half_h) / h * 1000.0).clamp(0.0, 1000.0),
            xmax: ((cx + half_w) / w * 1000.0).clamp(0.0, 1000.0),
            ymax: ((cy + half_h) / h * 1000.0).clamp(0.0, 1000.0),
            confidence: self.confidence,
        }
    }

    /// Convert to a tracked box with the patch's id.
    fn to_tracked_box(&self) -> TrackedBox {
        let mut b = self.to_norm();
        b.id = self.id.clone();
        b
    }
}

/// Apply a match result to a patch: update position, lost count, confidence.
fn update_patch_from_match(patch: &mut AnchorPatch, match_result: Option<(i32, i32, f32)>) {
    let Some((dx, dy, confidence)) = match_result else {
        mark_lost(patch);
        return;
    };
    if confidence < MIN_CONFIDENCE {
        mark_lost(patch);
        return;
    }
    let nx = patch.pos.0 as i32 + dx;
    let ny = patch.pos.1 as i32 + dy;
    let hw = patch.frame_w as i32 / 4;
    let hh = patch.frame_h as i32 / 4;
    let clamped_x = nx.clamp(-hw, patch.frame_w as i32 + hw);
    let clamped_y = ny.clamp(-hh, patch.frame_h as i32 + hh);
    patch.pos.0 = (patch.pos.0 as f32 * (1.0 - EMA_ALPHA) + clamped_x as f32 * EMA_ALPHA)
        .round()
        .max(0.0) as u32;
    patch.pos.1 = (patch.pos.1 as f32 * (1.0 - EMA_ALPHA) + clamped_y as f32 * EMA_ALPHA)
        .round()
        .max(0.0) as u32;
    patch.lost = 0;
    patch.confidence = confidence;
    let leaving = nx < -(patch.frame_w as i32 / 2)
        || ny < -(patch.frame_h as i32 / 2)
        || nx > patch.frame_w as i32 * 3 / 2
        || ny > patch.frame_h as i32 * 3 / 2;
    if leaving {
        patch.confidence *= LOST_DECAY;
    }
}

/// Increment the lost counter and decay confidence.
fn mark_lost(patch: &mut AnchorPatch) {
    patch.lost += 1;
    patch.confidence *= LOST_DECAY;
}

/// Sum of absolute differences between a patch and the frame at an offset.
fn patch_sad(frame: &GrayFrame, patch: &AnchorPatch, cx: i32, cy: i32) -> u64 {
    let w = frame.width as i32;
    let h = frame.height as i32;
    let pw = patch.pw as i32;
    let x0 = cx - patch.hw_px as i32;
    let y0 = cy - patch.hh_px as i32;
    let mut sad: u64 = 0;
    for (i, &pv) in patch.patch.iter().enumerate() {
        let px = x0 + (i as i32 % pw) * patch.step as i32;
        let py = y0 + (i as i32 / pw) * patch.step as i32;
        if px < 0 || py < 0 || px >= w || py >= h {
            sad += pv as u64 * 4;
            continue;
        }
        let fv = frame.data[py as usize * frame.row_stride as usize + px as usize] as u64;
        sad += (pv as i64 - fv as i64).unsigned_abs();
    }
    sad
}

/// Search the window around the last position for the best patch offset.
fn find_best_offset(
    frame: &GrayFrame,
    patch: &AnchorPatch,
    search_half: u32,
) -> Option<(i32, i32, f32)> {
    let sh = search_half as i32;
    let step = patch.step.max(1) as i32;
    let cx = patch.pos.0 as i32;
    let cy = patch.pos.1 as i32;
    let cells = (sh / step).max(1);
    let span = 2 * cells + 1;
    let mut best: Option<(i32, i32, u64)> = None;
    for k in 0..(span * span) {
        let dy = -sh + (k / span) * step;
        let dx = -sh + (k % span) * step;
        let sad = patch_sad(frame, patch, cx + dx, cy + dy);
        let improved = match best {
            Some((_, _, current)) => sad < current,
            None => true,
        };
        if improved {
            best = Some((dx, dy, sad));
        }
    }
    best.map(|(dx, dy, sad)| {
        let max_sad = (patch.pw * patch.ph).max(1) as f32 * 255.0;
        let confidence = 1.0 - (sad as f32) / max_sad;
        (dx, dy, confidence)
    })
}

/// Extract a subsampled patch for a normalized box from a frame.
fn extract_patch(
    frame: &GrayFrame,
    box_: &TrackedBox,
    step: u32,
) -> Option<(Vec<u8>, u32, u32, u32, u32)> {
    let (x0, y0, x1, y1) = norm_to_px(box_, frame.width, frame.height);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let step = step.max(1);
    let rows = ((y1 - y0) as usize).div_ceil(step as usize);
    let cols = ((x1 - x0) as usize).div_ceil(step as usize);
    let mut data = Vec::with_capacity(rows * cols);
    for i in 0..(rows * cols) {
        let r = y0 + (i / cols) as u32 * step;
        let c = x0 + (i % cols) as u32 * step;
        let idx = r as usize * frame.row_stride as usize + c as usize;
        data.push(frame.data.get(idx).copied().unwrap_or(0));
    }
    Some((data, cols as u32, rows as u32, x1 - x0, y1 - y0))
}

#[uniffi::export]
impl LaresTracker {
    /// Create a tracker with the given subsampling and search configuration.
    #[uniffi::constructor]
    pub fn new(subsample: u32) -> Self {
        Self {
            patches: std::sync::Mutex::new(Vec::new()),
            subsample: subsample.max(1),
            search_half: DEFAULT_SEARCH_HALF,
        }
    }

    /// Anchor the tracker to a frame and a set of boxes, cropping patches.
    pub fn anchor(&self, frame: &GrayFrame, boxes: Vec<TrackedBox>) {
        let mut patches = lock_patches(&self.patches);
        patches.clear();
        let w = frame.width;
        let h = frame.height;
        for box_ in boxes {
            let extracted = extract_patch(frame, &box_, self.subsample);
            let Some((data, pw, ph, bw_px, bh_px)) = extracted else {
                continue;
            };
            let cx = ((box_.xmin + box_.xmax) / 2.0 / 1000.0 * w as f32).round() as u32;
            let cy = ((box_.ymin + box_.ymax) / 2.0 / 1000.0 * h as f32).round() as u32;
            patches.push(AnchorPatch {
                id: box_.id,
                patch: data,
                pw,
                ph,
                step: self.subsample,
                bw_px,
                bh_px,
                hw_px: bw_px / 2,
                hh_px: bh_px / 2,
                pos: (cx, cy),
                frame_w: w,
                frame_h: h,
                lost: 0,
                confidence: 1.0,
            });
        }
    }

    /// Track all anchored patches in a new frame; returns updated boxes.
    pub fn track(&self, frame: &GrayFrame) -> Vec<TrackedBox> {
        let mut patches = lock_patches(&self.patches);
        let mut out = Vec::with_capacity(patches.len());
        for patch in patches.iter_mut() {
            update_patch_from_match(patch, find_best_offset(frame, patch, self.search_half));
            out.push(patch.to_tracked_box());
        }
        out
    }

    /// Drop all anchored patches.
    pub fn clear(&self) {
        lock_patches(&self.patches).clear();
    }

    /// Compute the 8x8 edge-hash fingerprint of a frame (8 bytes).
    pub fn fingerprint(&self, frame: &GrayFrame) -> Vec<u8> {
        let cell_w = frame.width.div_ceil(FINGERPRINT_GRID + 1).max(1);
        let cell_h = frame.height.div_ceil(FINGERPRINT_GRID).max(1);
        let mut bits: u64 = 0;
        for k in 0..(FINGERPRINT_GRID * FINGERPRINT_GRID) {
            let row = k / FINGERPRINT_GRID;
            let col = k % FINGERPRINT_GRID;
            let r0 = row * cell_h;
            let c0 = col * cell_w;
            let r1 = r0 + cell_h;
            let left_block = Block { r0, c0, r1, c1: c0 + cell_w };
            let right_block = Block { r0, c0: c0 + cell_w, r1, c1: c0 + 2 * cell_w };
            let left = block_mean(frame, &left_block, 2);
            let right = block_mean(frame, &right_block, 2);
            if left > right {
                bits |= 1u64 << k;
            }
        }
        bits.to_le_bytes().to_vec()
    }

    /// Hamming distance between two fingerprints (0 = identical scene).
    pub fn fingerprint_distance(&self, a: &[u8], b: &[u8]) -> u32 {
        let mut dist: u32 = 0;
        for i in 0..8 {
            let x = a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
            dist += x.count_ones();
        }
        dist
    }

    /// Crop the grayscale region for a normalized box from a frame.
    pub fn patch(&self, frame: &GrayFrame, box_: TrackedBox) -> ImagePatch {
        let (x0, y0, x1, y1) = norm_to_px(&box_, frame.width, frame.height);
        let pw = (x1 - x0).max(1) as usize;
        let ph = (y1 - y0).max(1) as usize;
        let mut data = Vec::with_capacity(pw * ph);
        for i in 0..(pw * ph) {
            let row = y0 + (i / pw) as u32;
            let col = x0 + (i % pw) as u32;
            let idx = row as usize * frame.row_stride as usize + col as usize;
            data.push(frame.data.get(idx).copied().unwrap_or(0));
        }
        ImagePatch {
            width: pw as u32,
            height: ph as u32,
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a gray frame from a generator closure.
    fn frame(w: u32, h: u32, gen: impl Fn(u32, u32) -> u8) -> GrayFrame {
        let mut data = Vec::with_capacity((w * h) as usize);
        for i in 0..(w * h) {
            let (x, y) = (i % w, i / w);
            data.push(gen(x, y));
        }
        GrayFrame { width: w, height: h, row_stride: w, data }
    }

    /// Deterministic pseudo-random texture, so patches are locally unique.
    fn noisy(x: u32, y: u32) -> u8 {
        let mut v = x.wrapping_mul(0x9E37_79B1).wrapping_add(y.wrapping_mul(0x85EB_CA77));
        v = (v ^ (v >> 15)).wrapping_mul(0x2C1B_3C6D);
        v ^= v >> 12;
        (v & 0xFF) as u8
    }

    #[test]
    fn tracks_shifted_patch() {
        let tracker = LaresTracker::new(2);
        let base = frame(320, 240, noisy);
        let box_ = TrackedBox {
            id: "c1".to_string(),
            xmin: 400.0,
            ymin: 400.0,
            xmax: 600.0,
            ymax: 600.0,
            confidence: 1.0,
        };
        tracker.anchor(&base, vec![box_.clone()]);
        // Shift the scene content right by 10px: draw the noise at x-10.
        let shifted = frame(320, 240, |x, y| noisy(x.saturating_sub(10), y));
        // EMA converges over a few frames, as it would at 15fps in reality.
        for _ in 0..12 {
            tracker.track(&shifted);
        }
        let out = tracker.track(&shifted);
        assert_eq!(out.len(), 1);
        let moved = &out[0];
        let dx_px = (moved.xmin - box_.xmin) / 1000.0 * 320.0;
        assert!(
            (dx_px - 10.0).abs() <= 4.0,
            "expected ~10px shift, got {dx_px}"
        );
    }

    #[test]
    fn static_scene_stays_in_place() {
        let tracker = LaresTracker::new(2);
        let base = frame(320, 240, noisy);
        let box_ = TrackedBox {
            id: "c1".to_string(),
            xmin: 400.0,
            ymin: 400.0,
            xmax: 600.0,
            ymax: 600.0,
            confidence: 1.0,
        };
        tracker.anchor(&base, vec![box_.clone()]);
        let out = tracker.track(&base);
        let dx = out[0].xmin - box_.xmin;
        assert!(dx.abs() <= 2.0, "box should stay put, drifted {dx}");
    }

    #[test]
    fn identical_frames_have_zero_fingerprint_distance() {
        let tracker = LaresTracker::new(2);
        let a = frame(320, 240, noisy);
        let b = frame(320, 240, noisy);
        let fa = tracker.fingerprint(&a);
        let fb = tracker.fingerprint(&b);
        assert_eq!(tracker.fingerprint_distance(&fa, &fb), 0);
    }

    #[test]
    fn changed_scene_has_large_fingerprint_distance() {
        let tracker = LaresTracker::new(2);
        let a = frame(320, 240, noisy);
        // Inverted brightness flips every left-vs-right cell comparison.
        let b = frame(320, 240, |x, y| 255 - noisy(x, y));
        let fa = tracker.fingerprint(&a);
        let fb = tracker.fingerprint(&b);
        assert!(tracker.fingerprint_distance(&fa, &fb) > 16, "inversion should flip many bits");
    }

    #[test]
    fn crops_landmark_patch() {
        let tracker = LaresTracker::new(2);
        let base = frame(200, 200, noisy);
        let box_ = TrackedBox {
            id: "lm".to_string(),
            xmin: 250.0,
            ymin: 250.0,
            xmax: 750.0,
            ymax: 750.0,
            confidence: 1.0,
        };
        let patch = tracker.patch(&base, box_);
        assert_eq!(patch.width, 100);
        assert_eq!(patch.height, 100);
        assert_eq!(patch.data.len(), 10_000);
        assert!(patch.data.iter().any(|&b| b > 0));
    }
}