//! Build-time code generation for the Celnet wire contract.
//!
//! The `.proto` schema is compiled with the **pure-Rust** `protox` compiler — it
//! parses the source into a `FileDescriptorSet` with no dependency on a system
//! `protoc` binary — and that descriptor set is handed to `tonic-build` (driving
//! `prost-build`) to emit both the message types and the gRPC service stubs
//! (client + server). This keeps the build hermetic and reproducible on every
//! platform (CI, containers, Apple Silicon) per the determinism discipline:
//! `skip_protoc_run()` guarantees no `protoc` is ever invoked.

use std::path::PathBuf;

fn main() {
    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let proto_file = proto_dir.join("celnet.proto");

    // Rebuild only when the schema changes.
    println!("cargo:rerun-if-changed={}", proto_file.display());

    // Parse the schema to a FileDescriptorSet with the pure-Rust compiler.
    let file_descriptor_set = protox::compile([&proto_file], [&proto_dir])
        .expect("celnet.proto must compile with protox (no system protoc required)");

    // Emit Rust message types AND tonic service stubs from the descriptor set.
    // `skip_protoc_run` keeps the build hermetic — protox already produced the
    // descriptor set, so no `protoc` binary is invoked here either.
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .skip_protoc_run()
        .compile_fds(file_descriptor_set)
        .expect("tonic-build must generate message + service stubs from the descriptor set");
}
