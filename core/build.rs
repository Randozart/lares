//! Build script: compile the protobuf contract into Rust types and generate
//! protojson serde implementations.
//!
//! Contract-first: `proto/lares/v1/*.proto` is the single source of truth. This
//! script runs `prost-build` for the message types and `pbjson-build` for
//! protojson-compatible `serde` impls, so the server can speak JSON that the
//! Android client parses with `protobuf-java-util`'s `JsonFormat`.

use std::path::PathBuf;

/// Compile the proto contract and emit prost + pbjson code into `OUT_DIR`.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let proto_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../proto");
    let descriptor_path = out_dir.join("lares_descriptor.bin");

    let mut prost_config = prost_build::Config::new();
    prost_config.file_descriptor_set_path(&descriptor_path);
    prost_config.compile_protos(
        &[
            "lares/v1/chore.proto",
            "lares/v1/inference.proto",
            "lares/v1/nudge.proto",
        ],
        &[proto_root],
    )?;

    let descriptor_bytes = std::fs::read(&descriptor_path)?;
    pbjson_build::Builder::new()
        .register_descriptors(&descriptor_bytes)?
        .build(&[".lares.v1"])?;

    Ok(())
}
