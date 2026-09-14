//! Lares core.
//!
//! Headless, client-agnostic domain logic:
//!
//! - [`domain`] — the generated protobuf contract plus domain helpers.
//! - [`engine`] — the pluggable [`engine::VisionInferenceEngine`] abstraction
//!   (mock, Gemini cloud, and a local stub).
//! - [`prompt`] — system prompts and the structured-output schema.
//! - [`diff`] — deterministic post-processing of raw engine output.
//! - [`store`] — SQLite persistence for chores and room references.
//! - [`nudge`] — the Phase G proactive-reminder seam.

// The generated protobuf types and their protojson serde impls live together
// under `lares::v1`; the pbjson-generated impls reference the prost types by
// unqualified name, so both must be in the same module scope.
pub mod lares {
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/lares.v1.rs"));
        include!(concat!(env!("OUT_DIR"), "/lares.v1.serde.rs"));
    }
}

pub mod diff;
pub mod domain;
pub mod engine;
pub mod nudge;
pub mod prompt;
pub mod store;
