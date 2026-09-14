# Live Tracking & Household State Memory

> Design record for the fast-loop tracking work and the landmark/sameness layer.
> Supersedes the "frozen frame overlay" MVP described in `PLAN.md` §5.2.

## 1. Goal

Boxes live on the camera preview and stay glued to objects while the user pans.
The VLM re-anchors every few seconds; a lightweight tracker carries boxes
frame-to-frame between anchors. Inference calls are gated so cost stays low.

Second goal: the app remembers *state*, not just frames. It can answer "has the
kitchen changed since yesterday's clean?" and "where does the hamper live?" —
without burning inference calls.

## 2. Measured latency baseline

| Config | Latency |
|--------|---------|
| gemini-2.5-flash, thinking ON (original) | 20.3 s |
| gemini-2.5-flash, thinking OFF | 2.5 s |
| **gemini-2.5-flash-lite, thinking OFF (default)** | **2.8 s** |

Levers already applied: `thinkingBudget: 0` for the 2.5 family, server-side
downscale to ≤1280px (`downscale_jpeg`), default model switched to
`gemini-2.5-flash-lite`.

Remaining budget is dominated by the model round-trip itself; further cuts come
from calling it less often (cost guards) or from local inference (Phase F).

## 3. Cost guards (client-side, decided before upload)

Three gates gate every auto-scan:

- **G1 — Settle dwell.** Device must be stable ≥1.5 s (`SensorManager`
  accelerometer variance) before a scan is allowed.
- **G2 — Scene change.** Current 8×8 grid fingerprint vs the last-analyzed
  fingerprint; skip if Hamming distance ≤ 6 bits (≈95% unchanged).
- **G3 — Rate limit.** ≥5 s between calls; one analysis in flight.

Manual "Capture" bypasses G1–G3.

Free work that runs regardless of gates: the tracker and the fingerprint
comparison (local Rust, no API).

## 4. The capability ladder (sameness & landmarks)

| Tier | What | Cost | Where |
|------|------|------|-------|
| T0 | Whole-frame dHash | ~free | gating only (superseded by T1) |
| T1 | 8×8 grid edge-hash fingerprint | free, local Rust | sameness-with-location, cross-session "changed since X", delta localization |
| T2 | Semantic landmarks via existing VLM (`landmarks: [{label, box}]`) | a few tokens/scan | persistent per-room anchors: sink basin, hamper, sofa… |
| T3 | Local place descriptors (NetVLAD/DINO), Phase F+ | heavy (on-device inference) | only if T1+T2 underperform |

### Honest limits
- **No metric localization** (meters/pose) without ARCore/SLAM — out of scope.
  2D normalized boxes + semantic labels answer the questions we care about.
- Stored boxes are **priors, not anchors**: lighting/angle change every session,
  so the VLM re-localizes landmarks each scan; the tracker glues within a
  session.
- Pixel fingerprints break under lighting change → use edge-based hashes;
  day/night variants possible later.

## 5. Contract changes

`proto/lares/v1/chore.proto`:
- New `Landmark { string label = 1; BoundingBox box = 2; }`.
- `AnalyzeSceneResponse.landmarks` (`repeated Landmark landmarks = 4;`).
  **DISCOVER mode only** — DIFF stays delta-focused, no extra tokens.

`response_schema` in `prompt.rs` gains `landmarks: [{label, box_2d}]`.
`gemini.rs` parses landmarks into the contract.

Fingerprints and patches are **not** in the wire contract:
- Fingerprints: opaque byte blobs (`Vec<u8>`) uploaded via their own endpoint.
- Landmark gray patches: **client-side only** (kept in app storage, cropped from
  the anchor frame) — never uploaded, keeps the analyze payload lean.

## 6. `tracking/` crate (`lares-tracking`, Rust + UniFFI)

```
GrayFrame   { width, height, row_stride, data: Vec<u8> }   // grayscale Y plane
TrackedBox  { id, xmin, ymin, xmax, ymax }                 // normalized 0..1000

LaresTracker
  new(patch_half: u32, search_half: u32, subsample: u32)
  anchor(frame: &GrayFrame, boxes: Vec<TrackedBox>)        // crop + store patches
  track(frame: &GrayFrame) -> Vec<TrackedBox>              // NCC search, update
  clear()
  fingerprint(frame: &GrayFrame) -> Vec<u8>                // 8×8 edge-hash (~128B)
  fingerprint_distance(a: &[u8], b: &[u8]) -> u32          // hamming across cells
  patch(frame: &GrayFrame, box) -> Vec<u8>                 // landmark gray crop
```

- NCC on a subsampled grid, bounded search window; low-confidence matches keep
  the last position (stale-fade); the next VLM anchor corrects.
- `crate-type = ["cdylib", "lib"]`; a `uniffi-bindgen` bin shim pins the
  bindgen version to the crate.
- Pure Rust, no proto/network deps. Praetor-clean: intent comments, bundled
  params, small helpers.

## 7. Server persistence

SQLite additions:

```
room_landmarks(room_id, label, ymin, xmin, ymax, xmax, updated_at)
room_fingerprints(id, room_id, kind, grid_hash, captured_at)
    kind ∈ {latest, history, clean}; history capped at 20
```

Endpoints:
- `GET  /v1/rooms/{id}/landmarks` — current landmark set
- `POST /v1/rooms/{id}/fingerprint` — store a fingerprint
- `GET  /v1/rooms/{id}/fingerprint?kind=latest|history|clean`
- `POST /v1/rooms/{id}/clean-fingerprint` — store the "agreed clean" state

`diff::postprocess` passes landmarks through unchanged.

## 8. Android fast loop

- **Fully live.** Drop freeze/resume. Overlay draws tracker boxes over the
  live `PreviewView`. `ImageCapture` + `ImageAnalysis` forced to the same
  resolution (1280×720) and display rotation so boxes share aspect.
- **Auto-scan state machine** with gates G1–G3 above.
- **On VLM response:** `anchor(savedGrayFrame, boxes)`; crop + save landmark
  patches; update cached fingerprint; POST fingerprint to server; draw
  landmarks subtly; refresh chore list.
- **Every frame:** `track()` throttled to ~15fps → Compose state.
- **Reference flow:** unchanged — capture + POST; works off the live preview.

## 9. Toolchain (Android → Rust)

Present: cargo-ndk 4.1.2, `aarch64-linux-android` rustup target, NDK r27c.
Add: `ANDROID_NDK_ROOT` in the Gradle build env; `x86_64-linux-android` for
emulator (optional).

Gradle (app module):
- `cargoNdkBuild` — `cargo ndk -t arm64-v8a -o src/main/jniLibs build --release
  -p lares-tracking` (AGP packages the `.so`).
- `generateUniffiKotlin` — `uniffi-bindgen generate --library <cdylib>
  --language kotlin --out-dir src/main/java` (ordered after the NDK build).
- Both wired into `preBuild`. `uniffi` pinned 0.32 (crate + bindgen match).

## 10. Verification

- **Headless:** `tracking` unit tests (shifted patch → shifted box, lost match,
  fingerprint distance, patch crop); `cargo test --workspace`;
  `praetor validate --warn`; `assembleDebug` bundles `liblarestracking.so` and
  the generated Kotlin compiles; curl round-trips for the new endpoints.
- **On-device (needs phone):** sweep a room — boxes stick while panning;
  settle triggers exactly one scan; an unchanged scene triggers **zero** scans;
  a moved object triggers one; landmarks persist across restarts.

## 11. Out of scope / later

- T3 local place descriptors (NetVLAD/DINO) — Phase F+, only if T1+T2 lag.
- ARCore/SLAM metric localization.
- Live 60fps tracking of arbitrary new objects before first VLM anchor.