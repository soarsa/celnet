//! Build-time code generation for the Celnet wire contract.
//!
//! The `.proto` schema is compiled with the **pure-Rust** `protox` compiler — it
//! parses the source into a `FileDescriptorSet` with no dependency on a system
//! `protoc` binary — and that descriptor set is handed to `prost-build` to emit
//! the Rust types. This keeps the build hermetic and reproducible on every
//! platform (CI, containers, Apple Silicon) per the determinism discipline.

use std::path::PathBuf;

fn main() {
    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let proto_file = proto_dir.join("celnet.proto");

    // Rebuild only when the schema changes.
    println!("cargo:rerun-if-changed={}", proto_file.display());

    // Parse the schema to a FileDescriptorSet with the pure-Rust compiler.
    let file_descriptor_set = protox::compile([&proto_file], [&proto_dir])
        .expect("celnet.proto must compile with protox (no system protoc required)");

    // Emit Rust types from the descriptor set; no protoc invocation.
    prost_build::Config::new()
        .compile_fds(file_descriptor_set)
        .expect("prost-build must generate Rust types from the descriptor set");
}
