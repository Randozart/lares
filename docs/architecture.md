# Lares — Architecture

## Overview

Lares separates **perception** from **presentation** and keeps all domain logic
in one headless Rust crate (`lares-core`). Clients and inference backends are
replaceable.

```
                ┌───────────────────────────────┐
   Fast loop    │  Android app                  │
   (30–60 FPS)  │  CameraX → freeze frame       │
                │  Compose Canvas overlay       │
                └───────────┬───────────────────┘
                            │ protojson over HTTP (LAN)
                            ▼
                ┌───────────────────────────────┐
   Slow loop    │  lares-server (axum)          │
   (0.5–2 Hz)   │    routes → core              │
                └───────────┬───────────────────┘
                            ▼
                ┌───────────────────────────────┐
                │  lares-core                   │
                │   domain / prompt / diff      │
                │   VisionInferenceEngine       │
                │     ├─ GeminiEngine (cloud)   │
                │     ├─ LocalEngine  (Phase F) │
                │     └─ MockEngine   (tests)   │
                │   nudge::ReminderPolicy (G)   │
                └───────────────────────────────┘
```

## Why two loops

Perception (a VLM call) takes 300–2500 ms. Camera preview must run at 30–60 FPS.
Coupling them is the classic fatal mistake. The fast loop captures a keyframe and
draws boxes on a frozen frame; the slow loop does all inference. Live 60 FPS
tracking can be added later on the fast side without touching the slow side.

## Contract

Protobuf is the single source of truth. `build.rs` runs `prost-build` +
`pbjson-build`, generating Rust structs with serde impls so the server speaks
**protojson**. The Android app compiles the same `.proto` files with the Gradle
protobuf plugin and parses protojson with `protobuf-java-util`'s `JsonFormat`.

Consequence: adding a field to a `.proto` propagates to both sides by
regeneration. No drift.

## Inference abstraction

```rust
#[async_trait]
pub trait VisionInferenceEngine: Send + Sync {
    async fn analyze_scene(
        &self,
        req: AnalyzeSceneRequest,
    ) -> Result<AnalyzeSceneResponse, InferenceError>;
}
```

The server holds `Arc<dyn VisionInferenceEngine>` selected from `LARES_ENGINE`.
Swapping cloud → local → on-device is a one-line change in `server/src/state.rs`.

## Prompting

`prompt.rs` builds two system prompts (DISCOVER and DIFF). Both demand strict
JSON matching the response schema. The Gemini adapter passes
`generationConfig.responseMimeType = "application/json"` and
`responseSchema`, then validates the parse. No prose is allowed into the payload.

- **DISCOVER:** find actionable chores, ignore properly stored items, emit atomic
  actions and subtasks.
- **DIFF:** compare current frame against the room's agreed reference, emit only
  deltas.

## Post-processing

`diff.rs` normalizes engine output:
- clamp/deduplicate overlapping boxes,
- split monolithic actions into atomic `subtasks`,
- assign a stable `id` and room,
- drop low-confidence detections.

This is deterministic and unit-tested without any network.

## Persistence (Phase E)

`sqlx` + SQLite under `LARES_DATA_DIR`. Chores and reference states survive
restarts. Reference images are stored on disk with their metadata in SQLite.

## Nudging (Phase G)

`nudge::ReminderPolicy` takes household state plus an `IdleContext` and returns
an optional `Nudge`. MVP wires `NoopPolicy`. A future policy can use screen idle,
time of day, or home geofence. The contract fields (`priority`, `due_at_unix`,
`cooldown_until_unix`, `context_tags`, `energy_cost`) already exist so no
migration is needed.

## On-device path (Phase F)

`uniffi-rs` generates Kotlin bindings for `lares-core`. The Android app moves
from `http://host/v1/analyze` to an in-process call. Same crate, same contract,
no domain rewrite.
