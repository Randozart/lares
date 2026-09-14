# Lares

Ambient household-state awareness. Point your phone at a room; a vision-language
model returns bounding boxes plus atomic chore instructions overlaid on the
frame. Reference-state diffing gives a measurable definition of "done". Chores
are persistent entities, not ephemeral pixels.

Named after the Roman guardian spirits of the household.

> Full plan: [`docs/PLAN.md`](docs/PLAN.md). Architecture: [`docs/architecture.md`](docs/architecture.md).

## Status

Scaffold. Phases A–F built, Phase G seam in place. See `docs/PLAN.md` §8.

## Layout

| Path | What |
|------|------|
| `proto/lares/v1/` | Contract — source of truth |
| `core/` | `lares-core`: domain, engines, prompt, diff, nudge |
| `server/` | `lares-server`: axum, protojson over HTTP |
| `android/` | Kotlin + Compose + CameraX thin client |
| `docs/` | Plan and architecture |

## Quick start

```bash
make proto        # generate Rust types from protobuf
make build        # build workspace
make test         # run tests
make run-server   # start the API on :8787
```

Analyze a frame (mock engine, no key needed):

```bash
curl -s localhost:8787/v1/health
curl -s -X POST localhost:8787/v1/analyze \
  -H 'content-type: application/json' \
  -d '{"roomId":"kitchen","mode":"ANALYZE_MODE_DISCOVER","frameJpeg":"<base64>"}'
```

Real inference (Phase B):

```bash
export GEMINI_API_KEY=...
export LARES_ENGINE=gemini
make run-server
```

Android (Phase C): see `android/README.md`.

## Design in one line

Contract-first, dual-loop: a fast camera/overlay loop on the phone, a slow
perception loop in Rust behind a `VisionInferenceEngine` trait, swappable from
cloud to local to on-device without rewriting domain logic.
