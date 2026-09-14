# Accuracy, Tracking, Immersive UI, Panoramic Sweep

> Build record for the post-MVP polish pass. Extends `docs/live-tracking.md`.

## 1. Accuracy (false positives, misclassification)

Root cause: `gemini-2.5-flash-lite` guesses when unsure (glass of soda → "can").

- Default model → `gemini-2.5-flash` (thinking already off; measured 2.5s vs 2.8s).
- `temperature: 0` in generationConfig — deterministic output.
- Conservative prompt: report only clearly-visible items; use generic labels
  ("drink container") when the specific type is uncertain; omit items that do
  not clearly need attention.
- Cap results at top 6 chores by confidence.
- Confidence floor `0.25 → 0.45` as a backstop.

## 2. Tracking (boxes drift off objects)

- `search_half` 32 → 48 px (survives faster pans).
- EMA smoothing on each box position (kills jitter).
- `TrackedBox` gains a `confidence` field so the overlay can fade lost boxes
  instead of letting them sit wrong; the next pan-settle re-anchors.
- Later (optional): adaptive re-anchor when >50% of boxes are lost.

## 3. Immersive UI — no scroll on the main screen

- `Box(fillMaxSize)`: full-bleed live preview (`FILL_CENTER`) + tracker overlay.
- Top overlay: status + room + mode chips + gear → settings dialog (server URL,
  room, reference description).
- Bottom overlay: **Sweep**, **Scan**, **Set Reference**, **Chores (N)** →
  `ModalBottomSheet` (each chore: How / Done).
- Edge boxes approximate under FILL_CENTER (exact mapping deferred).

## 4. Panoramic sweep (multi-frame, one VLM call)

No stitching. Capture ~4–8 keyframes while panning, send them all in one
`generateContent` request, merge results.

- Contract: `AnalyzeSceneRequest.sweep_jpegs[]`; `ChoreEntity.image_index`
  (which frame each chore came from, 0-based).
- Prompt: "frames are consecutive views of one room; report each chore once, in
  the frame where it is clearest, and set `image` to that frame's index."
- Engine builds N image parts; parses `image` → `image_index`.
- `diff.rs` dedupes across frames (same target + coarse position) and caps at 6.
- Client: capture keyframes every ~1.2s (fingerprint-gated), stop → one analyze
  call → anchor boxes to their frames; overlay shows the latest frame's boxes;
  full list in the sheet.
- Cost: one call per sweep, still gated by scene-change fingerprint.

## Verification

- Accuracy: live curl lite vs flash on the test image.
- Sweep: one curl with N frames → one deduped chore set.
- UI/tracking: on-device.