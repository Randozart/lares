# Lares — Agent Guidelines

Read `docs/PLAN.md` first. It is the canonical plan.

## Philosophy

- **Contract-first.** `proto/lares/v1/*.proto` is the source of truth. Never
  hand-write DTOs that duplicate it. Regenerate with `make proto`.
- **Dual-loop.** Camera/overlay is the fast loop (Android). Perception is the
  slow loop (Rust). Never couple them.
- **Pluggable inference.** All model calls go through the
  `VisionInferenceEngine` trait. Never call a vendor API from domain logic.
- **No rewrite.** Domain logic lives in `lares-core`. Clients and engines are
  replaceable. If a change would force a rewrite, the abstraction is wrong — fix
  the abstraction.

## Anti-patterns (NEVER DO)

- Calling Gemini/HTTP directly from `domain.rs`, `diff.rs`, or `prompt.rs`.
- Putting inference or domain logic in the Android app.
- Treating chores as ephemeral pixel boxes instead of stored entities.
- Weakening a contract to match lazy code.
- Editing generated protobuf code by hand.

## Praetor enforcement

Praetor is installed as an LSP server and runs on every keystroke. A pre-commit
hook and CI gate run `praetor validate --warn`.

Rules you must satisfy:

- **Intent comment before every function** (missing = ERROR).
- **≤5 parameters.** Bundle into a request/context struct.
- **Cyclomatic ≤15, cognitive ≤15, nesting ≤6.**
- **No O(n²) or worse** without a shadow benchmark.
- Prefer early returns over `if/else if/else` chains.
- No inline `// praetor:ignore`. No `git commit --no-verify`.

If a check truly cannot be satisfied by refactoring, use the shadow escape hatch
(`// praetor-shadow: original=fn`) and run `praetor verify --shadow`. The
benchmark machine decides. See global AGENTS.md for the three-gate process.

## Commands

```bash
make proto        # protoc → Rust types (core/src/gen)
make build        # cargo build --workspace
make test         # cargo test --workspace
make lint         # clippy
make run-server   # run lares-server
make android      # assemble debug APK
make android-install
```

## Environment

- `GEMINI_API_KEY` — required for `LARES_ENGINE=gemini`.
- `LARES_ENGINE` — `mock` (default) or `gemini`.
- `LARES_MODEL` — model id, default set in `core/src/engine/gemini.rs`.
- `LARES_BIND` — server bind address, default `0.0.0.0:8787`.
- `LARES_DATA_DIR` — sqlite + frame storage, default `./data`.
- `JAVA_HOME` — `/home/randozart/brief-tools/jdk-17.0.20+8` for Android builds.
