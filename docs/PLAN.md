# Lares — Full Project Plan

> Canonical plan record. Nothing intentionally omitted. If a decision was made in
> discussion, it is logged here with its rationale.

## 0. Name

**Lares** — Roman guardian spirits of the household. Chosen because the system is
broader than chores: it is *ambient household-state awareness*. The camera is one
sensor; proactive nudging is one actuator. "chore-vision" was rejected as too
narrow (it described only the camera loop).

- Folder: `Desktop/Projects/lares`
- Android app id: `dev.randozart.lares`
- Rust crates: `lares-core`, `lares-server`
- Protobuf package: `lares.v1`

Alternatives considered and rejected: `domus`, `hestia`, `mundus`, `oikos`.

## 1. Problem

Household chores are easy to miss and hard to start. Some tasks are simply not
noticed, and broad instructions like "clean the counter" have no measurable
Definition of Done, so the task cannot begin.

The real problem is two-fold:
1. **Discovery** — not seeing what needs doing.
2. **Ambiguity** — not knowing what "done" means, so the task cannot start.

Success = reduced task friction, measurable by whether users actually use it.

## 2. Thesis

Point a phone camera at a room. A vision-language model returns bounding boxes
plus atomic chore instructions overlaid on the frame. A stored "agreed target
state" per room enables diffing, giving a measurable Definition of Done. Chores
are persistent entities, not ephemeral pixels. Later, the same domain data drives
proactive nudges when the user is idling.

## 3. Prior art (what exists, what does not)

**No turnkey open-source "household chore HUD" exists.** Adjacent work:

1. **Toy clean-vs-messy classifiers (2018–2022).** Binary "room is 82% messy" or
   generic object boxes. No affordance semantics (a cup on a coaster is fine; a
   cup on the floor is a chore).
2. **Robotics rearrangement benchmarks (e.g. TIDEE).** Heavy PyTorch simulation
   for mobile manipulators. Not an app.
3. **XR prototypes (e.g. Google `xr-objects`, UIST 2024).** ARCore + Vision LLM
   anchors context menus over physical objects. Closest conceptual cousin.

**Key unlock:** do NOT train traditional object detectors (YOLO/SSD) on classes
like `pile_of_dirty_clothes`. Intra-class variance of "mess" is infinite. Use
open-vocabulary grounded VLMs that return normalized bounding boxes and
understand affordances.

## 4. Decisions log

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | Android only (MVP) | Device + NDK already present; avoids Apple provisioning tax. |
| D2 | Cloud VLM first (Gemini), local later | Fastest path to validating the thesis. `LocalEngine` drops into the same trait. |
| D3 | Phone camera, not laptop | Walking the phone around the house is easier than a laptop. |
| D4 | Reference-state diffing in scope | Gives a measurable DoD; removes ambiguity. |
| D5 | Contract-first, protobuf before code | Prevents rewrite when swapping clients/engines. |
| D6 | Rust core, axum server now, UniFFI later | Keeps domain logic headless and portable. |
| D7 | `protojson` over HTTP | Debuggable with curl; Android parses via protobuf-java-util `JsonFormat`. |
| D8 | Jetpack Compose for Android UI | Modern, less XML boilerplate. |
| D9 | Full scaffold A–F now, G seam only | Build the whole frame; fill phases over sessions. |
| D10 | Name: `lares` | Captures ambient awareness + nudging, not just chores. |
| D11 | Proactive nudges architected now, built in G | Extending the contract later is expensive; the fields are cheap now. |

## 5. Architecture

### 5.1 The three fatal couplings to avoid
1. **Frequency coupling** — tying 30–60 FPS display to 0.5–2 Hz perception.
2. **Platform coupling** — baking inference/domain logic into OS UI frameworks.
3. **State coupling** — treating chores as ephemeral pixel boxes.

### 5.2 Dual-loop design

```
┌────────────────────────────────────────────────────────────┐
│                    FAST LOOP (30–60 FPS)                   │
│  Android: CameraX PreviewView + Compose Canvas overlay     │
│                                                            │
│  [Camera] ──> [Capture keyframe] ──> [Frozen frame + HUD]  │
│                      │                      ▲              │
│                      │ POST /v1/analyze      │ boxes        │
└──────────────────────┼──────────────────────┼──────────────┘
                       │ protojson            │
                       ▼                      │
┌────────────────────────────────────────────────────────────┐
│                    SLOW LOOP (0.5–2 Hz)                    │
│  Rust: lares-server (axum) → lares-core                    │
│                                                            │
│  [Frame decimator] → [Prompt builder] → [VisionInference]  │
│        → [Structured JSON parse] → [Dedupe/split] → store  │
└────────────────────────────────────────────────────────────┘
```

The slow loop never cares about screen refresh. The fast loop never does
inference. Swapping either side does not touch the other.

### 5.3 Contract-first
`proto/lares/v1/*.proto` is the source of truth. Rust types are generated by
`prost` + `pbjson` (serde impls for protojson). Android consumes the same
protojson with generated Java classes via the Gradle protobuf plugin. No
hand-written DTOs on either side.

### 5.4 Pluggable inference
```rust
#[async_trait]
pub trait VisionInferenceEngine: Send + Sync {
    async fn analyze_scene(&self, req: AnalyzeSceneRequest)
        -> Result<AnalyzeSceneResponse, InferenceError>;
}
```
Implementations: `GeminiEngine` (B), `MockEngine` (tests), `LocalEngine` (F).
Changing backend = one line of initialization.

### 5.5 Why Rust will not be the bottleneck (Amdahl)
| Stage | Latency | Bottleneck | Rust helps? |
|-------|---------|-----------|-------------|
| Frame capture | 16–33 ms | OS ISP / hardware decoder | No |
| Preprocess (resize/norm) | 2–5 ms | CPU/SIMD | Marginally (~10ms→~1ms, negligible) |
| VLM inference | 300–2500 ms | GPU/NPU bandwidth or network I/O | No |
| Box/label overlay | <1 ms | 2D compositor | No |

>95% of wall time is accelerator matmul or network I/O. Rust is chosen for
correctness, portability, and domain modelling — not speed.

### 5.6 Portability path (no rewrite)
- **Now:** `lares-core` wrapped by `lares-server` (axum). Phone is a thin client.
- **Later:** same crate compiled to a static lib via `uniffi-rs`, generating
  Swift/Kotlin bindings. The client moves from `http://host/v1/analyze` to
  `lares_core::analyze_scene()` in-process. Domain logic untouched.

## 6. Contract / domain model

`proto/lares/v1/chore.proto`
- `BoundingBox{ymin,xmin,ymax,xmax}` — normalized 0–1000, Gemini-native order.
- `ChoreStatus{UNSPECIFIED, DISCOVERED, IN_PROGRESS, DONE, DISMISSED}`.
- `ChoreEntity{ id, room_id, target, action, estimated_seconds, status, box,
  confidence, subtasks[], priority?, due_at_unix?, cooldown_until_unix?,
  last_seen_unix?, context_tags[], energy_cost? }`.
  Optional scheduling fields are unused in MVP and consumed by Phase G.
- `ReferenceState{room_id, image_id, description, captured_at_unix}`.

`proto/lares/v1/inference.proto`
- `AnalyzeMode{UNSPECIFIED, DISCOVER, DIFF}`.
- `AnalyzeSceneRequest{room_id, frame_jpeg, reference_jpeg?, mode}`.
- `AnalyzeSceneResponse{chores[], model, latency_ms}`.

`proto/lares/v1/nudge.proto`
- `Nudge{chore_id, reason, created_at_unix}`.
- `IdleContext{screen_on, seconds_idle, hour_local, at_home}`.
- `NudgeRequest` / `NudgeResponse` (Phase G transport).

### Endpoints
| Method | Path | Purpose |
|--------|------|---------|
| POST | `/v1/analyze` | Analyze a frame (DISCOVER or DIFF). |
| POST | `/v1/rooms/{room_id}/reference` | Set agreed target state. |
| GET  | `/v1/rooms/{room_id}/reference` | Fetch agreed target state. |
| GET  | `/v1/chores` | List chores (optional `?room_id=`). |
| PATCH| `/v1/chores/{id}/status` | Transition chore lifecycle. |
| GET  | `/v1/health` | Liveness. |

### Prompt strategy
Strict grounding to remove ambiguity. System prompt instructs an assistive
vision system to identify *actionable physical chores*, ignore properly stored
items, return a JSON array with `box_2d` (normalized 0–1000), `target`, atomic
imperative `action`, `estimated_seconds`, and `subtasks`. Gemini is called with
`responseMimeType: application/json` and a `responseSchema` so no prose leaks
into the payload.

### Usability design
- **Reference-state diffing** — store 3–4 agreed "target state" photos per room.
  Send `[reference, current]`; return only deltas. Gives an unambiguous
  Definition of Done for a room.
- **Atomic micro-tasks** — post-processor splits monolithic chores into
  single-step actions. "Clean the sink" → "Move 2 bowls to the dishwasher rack".
  Tap a box to dismiss once done.

## 7. Repo layout

```
lares/
├── AGENTS.md              # conventions + Praetor rules
├── Makefile               # proto / build / test / run / android
├── README.md
├── .gitignore
├── docs/
│   ├── PLAN.md            # this file
│   └── architecture.md
├── proto/lares/v1/
│   ├── chore.proto
│   ├── inference.proto
│   └── nudge.proto
├── core/                  # lares-core
│   ├── Cargo.toml
│   ├── build.rs           # prost-build + pbjson-build
│   └── src/
│       ├── lib.rs
│       ├── domain.rs
│       ├── prompt.rs
│       ├── diff.rs
│       ├── engine/{mod.rs, gemini.rs, mock.rs, local.rs}
│       └── nudge/{mod.rs, noop.rs}
├── server/                # lares-server
│   ├── Cargo.toml
│   └── src/{main.rs, routes.rs, state.rs}
├── android/               # Kotlin + Compose + CameraX
│   ├── settings.gradle.kts
│   ├── build.gradle.kts
│   ├── gradle.properties
│   └── app/
│       ├── build.gradle.kts
│       └── src/main/{AndroidManifest.xml, java/dev/randozart/lares/*, res/*}
└── data/                  # gitignored: sqlite + frames + references
```

## 8. Phases

### Phase A — Foundation
- Folder, git, workspace, proto contracts, Rust domain types, `VisionInferenceEngine`
  trait, `MockEngine`, axum server with all endpoints (mock-backed), Makefile, tests.
- **Verify:** `cargo test` green; `curl /v1/analyze` returns deterministic mock boxes.

### Phase B — Real inference
- `GeminiEngine`: async `reqwest`, structured output via `responseSchema`, retry.
- **Verify:** `curl` a real messy-room JPEG → real chore boxes.

### Phase C — Android thin client
- Compose + CameraX, permission flow, capture keyframe, POST to server, Canvas
  overlay draws normalized boxes + labels. `adb reverse` for dev.
- **Verify:** walk the house, boxes render.

### Phase D — Reference-state diffing
- Capture agreed DoD per room; DIFF-mode prompt compares reference vs current.
- **Verify:** only deltas returned.

### Phase E — Lifecycle
- sqlite (sqlx): chores persist across restarts, tap-to-done, micro-task split.
- **Verify:** kill app, state survives.

### Phase F — Local + on-device (future)
- `LocalEngine` (Florence-2 / Qwen2.5-VL via `candle` or `ort`) into the trait.
- Then UniFFI: compile `lares-core` to a static lib, generate Kotlin bindings,
  move inference in-process. Same crate, same contract.

### Phase G — Proactive nudges (seam now, logic later)
- `nudge/`: `trait ReminderPolicy { fn next_nudge(&self, state, ctx) -> Option<Nudge> }`
  plus `NoopPolicy` and `IdleContext`.
- Android: notification channel + WorkManager dependency declared; no logic yet.
- **Verify (now):** trait compiles, NoopPolicy always returns `None`.

## 9. Praetor compliance (enforced)

- Intent (doc) comment before **every** function — missing = ERROR.
- ≤5 params; bundle into context/request structs.
- Cyclomatic ≤15, cognitive ≤15, nesting ≤6.
- No O(n²) or worse without a shadow benchmark.
- Early returns over `if/else if/else` chains.
- Pre-commit runs `praetor validate --warn`; CI runs the same gate.
- No inline `// praetor:ignore`. Shadow escape hatch only as last resort.

## 10. Environment / setup

Present toolchain:
- Rust 1.94.1, cargo 1.94.1 ✓
- Praetor 0.1.0 ✓
- protoc (libprotoc) 36.1 ✓
- Android SDK `/home/randozart/Android/Sdk`: `cmdline-tools`, NDK r27c, `platform-tools/adb` ✓
- JDK 17 `/home/randozart/brief-tools/jdk-17.0.20+8` ✓
- **Missing:** `platforms;android-35`, `build-tools`, Gradle wrapper, emulator
- **Device:** `a90682c5` connected but **unauthorized**
- **Missing:** `buf` (use protoc directly), `GEMINI_API_KEY`

Setup steps before Phase C:
1. `sdkmanager --licenses` and install `platforms;android-35`, `build-tools;35.0.0`.
2. `export JAVA_HOME=/home/randozart/brief-tools/jdk-17.0.20+8`.
3. Bootstrap Gradle wrapper (one-time distribution fetch).
4. Accept USB debugging on `a90682c5`.
5. `export GEMINI_API_KEY=...`.

## 11. Risks & mitigations

| Risk | Mitigation |
|------|-----------|
| Home photos leave network (cloud) | Accepted for MVP; `LocalEngine` in Phase F. |
| VLM box drift while panning | Freeze-frame on capture; live tracking is post-MVP (ARCore/optical flow, no core change). |
| Overlay is on a frozen still, not live | Honest MVP; tap to resume preview. |
| Android toolchain setup eats the project | Scaffold is thin; core+server validated headlessly first (A/B). |
| Model names/APIs change fast | Engine trait + config; model id is env-driven. |
| Structured output leaks prose | `responseSchema` + strict JSON parse + validation. |
| Cost | Gemini Flash tier; frame decimation (0.5–2 Hz). |

## 12. Non-goals (MVP)

- Live 60 FPS tracked AR boxes.
- Multi-user accounts.
- Push notifications and idle detection (Phase G seam only).
- On-device inference (Phase F).
- Training custom models.
