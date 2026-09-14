# Phase F — Local & on-device inference

Cloud-first is deliberate. This document records the no-rewrite path to local and
on-device inference.

## The swap point

Every model call goes through one trait in `lares-core`:

```rust
#[async_trait]
pub trait VisionInferenceEngine: Send + Sync {
    async fn analyze_scene(
        &self,
        req: AnalyzeSceneRequest,
    ) -> Result<AnalyzeSceneResponse, InferenceError>;
}
```

Backends are selected once in `server/src/main.rs::build_engine` from
`LARES_ENGINE`. Swapping cloud → local → on-device changes one line.

## Step 1 — Local server (`LocalEngine`)

`core/src/engine/local.rs` is a stub that returns `Unavailable` until Phase F.

Candidates, in order of preference:
- **`ort`** (ONNX Runtime bindings) running Florence-2 or a Qwen2.5-VL ONNX
  export on the workstation GPU. Rust-native, no Python process.
- **`candle`** (pure-Rust Hugging Face inference) — cleanest dependency story,
  slower at parity.
- A small FastAPI/ONNX service on the LAN, if the Rust model runtime proves
  immature. The trait hides the choice.

`LocalEngine::analyze_scene` then: downscale JPEG, run the model, map normalized
boxes into the same `AnalyzeSceneResponse` shape, and return. All post-processing
in `diff.rs` is unchanged.

## Step 2 — On-device (UniFFI)

Same crate, embedded in the app:

1. Add `uniffi` to `lares-core` and generate Kotlin bindings
   (`uniffi-bindgen kotlin`). The crate is already headless and client-agnostic.
2. The Android client moves from `http://host/v1/analyze` to
   `LaresCore().analyzeScene(...)` in-process. `LaresClient` keeps the same
   protojson signature, so the UI does not notice.
3. Compile `lares-core` for `aarch64-linux-android` (the NDK r27c target is
   already installed at `/home/randozart/Android/Sdk/ndk`).

## What must NOT change

- `proto/lares/v1/*` — the contract is stable across all three backends.
- `diff.rs`, `prompt.rs`, `store.rs`, `nudge/` — none care where inference runs.
- The Android overlay, chore list, and lifecycle transitions.

## When to trigger Phase F

Not at MVP. Pull the trigger when one of these bites:
- Home-photo privacy becomes unacceptable (cloud path sends frames off-network).
- LAN latency (typically ~1 s round trip) stops being tolerable.
- You want to walk outside your Wi-Fi coverage with the app.