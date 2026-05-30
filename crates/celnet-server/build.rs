//! Build-time gRPC code generation for the Celnet service edge.
//!
//! The `edge.proto` service contract is parsed with the **pure-Rust** `protox`
//! compiler — it produces a `FileDescriptorSet` with no dependency on a system
//! `protoc` binary — and that descriptor set is handed to `tonic-build` to emit
//! the message types *and* the async client/server service stubs. This keeps the
//! build hermetic and reproducible on every platform (CI, containers, Apple
//! Silicon), matching the determinism discipline used by `celnet-proto`.

use std::path::PathBuf;

fn main() {
    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let proto_file = proto_dir.join("edge.proto");

    // Rebuild only when the schema changes.
    println!("cargo:rerun-if-changed={}", proto_file.display());

    // Parse the schema to a FileDescriptorSet with the pure-Rust compiler — no
    // system `protoc` is invoked.
    let file_descriptor_set = protox::compile([&proto_file], [&proto_dir])
        .expect("edge.proto must compile with protox (no system protoc required)");

    // Emit message types plus async gRPC client/server stubs from the descriptor
    // set. We generate both ends so the in-process integration tests can dial the
    // server through the generated client over a real TCP socket.
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_fds(file_descriptor_set)
        .expect("tonic-build must generate gRPC stubs from the descriptor set");
}
