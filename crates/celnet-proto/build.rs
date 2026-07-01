//! Build-time code generation for the Celnet wire contract.
//!
//! The `.proto` schema is compiled with the **pure-Rust** `protox` compiler — it
//! parses the source into a `FileDescriptorSet` with no dependency on a system
//! `protoc` binary — and that descriptor set is handed to `tonic-build` (driving
//! `prost-build`) to emit both the message types and the gRPC service stubs
//! (client + server). This keeps the build hermetic and reproducible on every
//! platform (CI, containers, Apple Silicon) per the determinism discipline:
//! `skip_protoc_run()` guarantees no `protoc` is ever invoked.
//!
//! ## WS wire-contract manifest (arch item G — `ws-codec-from-proto`)
//!
//! The **same** descriptor set additionally drives a generated `wire_contract`
//! module ([`emit_wire_contract`]): a descriptor-derived manifest of the WS
//! mirror's verb vocabulary (the request/response RPCs and the RFS
//! stream-control oneof arms), the message/enum surface, and a per-message
//! **field table** (`MessageFields` / `MESSAGE_FIELDS` / `fields_for`) —
//! every field's proto name, JSON key, type, presence/cardinality label and
//! real-oneof group, walked from the same descriptor. The field tables are the
//! first descriptor-side input to the deferred unary codec swap (the curated
//! JSON-key override table and the generated mechanical codecs build on them;
//! see `docs/INTERFACES.md` → "WS codec from proto descriptor (item G)"). The
//! WebSocket
//! mirror is not a second contract — it is a second *encoding* of this one
//! `celnet.proto` contract — so its routing vocabulary is now sourced from the
//! descriptor rather than a hand-maintained literal that could silently drift
//! from the schema. The manifest is generated deterministically (descriptor
//! declaration order), so the emitted file is byte-identical on every build.

use std::path::PathBuf;

use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::{DescriptorProto, FieldDescriptorProto, FileDescriptorSet, OneofDescriptorProto};

fn main() {
    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let proto_file = proto_dir.join("celnet.proto");

    // Rebuild only when the schema changes.
    println!("cargo:rerun-if-changed={}", proto_file.display());

    // Parse the schema to a FileDescriptorSet with the pure-Rust compiler.
    let file_descriptor_set = protox::compile([&proto_file], [&proto_dir])
        .expect("celnet.proto must compile with protox (no system protoc required)");

    // Generate the WS wire-contract manifest from the descriptor set BEFORE it is
    // moved into tonic-build (`compile_fds` consumes it by value). One descriptor
    // set, two generated artifacts — they cannot diverge.
    emit_wire_contract(&file_descriptor_set);

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

/// The `ClientStreamMessage` oneof is the RFS stream-control surface — its arm
/// names are exactly the WS `type` discriminators the router classifies as
/// stream-control (forwarded to the live session driver, not the unary edge).
const STREAM_CONTROL_MESSAGE: &str = "ClientStreamMessage";

/// Generate `$OUT_DIR/wire_contract.rs` — the descriptor-derived manifest of the
/// WS mirror's contract surface. Deterministic in descriptor declaration order so
/// the emitted file is byte-identical on every build.
fn emit_wire_contract(fds: &FileDescriptorSet) {
    let mut rpcs: Vec<(String, String, String, String)> = Vec::new();
    let mut stream_control: Vec<String> = Vec::new();
    let mut messages: Vec<String> = Vec::new();
    let mut enums: Vec<String> = Vec::new();
    // Per-message field tables, in the SAME declaration order as `messages` (the
    // two vecs are pushed in lockstep by `collect_message`), so the emitted
    // `MESSAGE_FIELDS` aligns index-for-index with `MESSAGES`.
    let mut message_fields: Vec<(String, Vec<FieldData>)> = Vec::new();

    for file in &fds.file {
        let package = file.package();
        // Services → unary request/response verbs.
        for service in &file.service {
            let svc = service.name();
            for method in &service.method {
                rpcs.push((
                    svc.to_owned(),
                    method.name().to_owned(),
                    last_segment(method.input_type()).to_owned(),
                    last_segment(method.output_type()).to_owned(),
                ));
            }
        }
        // Messages (recursively, incl. nested) → names + field tables + the
        // stream-control arms.
        for message in &file.message_type {
            collect_message(
                message,
                package,
                &mut messages,
                &mut stream_control,
                &mut message_fields,
            );
        }
        // Enums.
        for e in &file.enum_type {
            enums.push(e.name().to_owned());
        }
    }

    let mut out = String::new();
    out.push_str(
        "// @generated by celnet-proto/build.rs from proto/celnet.proto — DO NOT EDIT.\n\
         // The descriptor-derived WS wire-contract manifest (arch item G).\n\n",
    );

    // ---- stream-control verbs (the ClientStreamMessage oneof arm names) -------
    out.push_str(
        "/// The RFS stream-control verbs: the `ClientStreamMessage` oneof arm names\n\
         /// (proto field snake_case), i.e. the exact WS `type` discriminators the\n\
         /// router forwards to the live session driver rather than the unary edge.\n\
         /// Generated from the descriptor, so a new oneof arm is classified with no\n\
         /// hand edit to the router.\n\
         pub const STREAM_CONTROL_VERBS: &[&str] = &[\n",
    );
    for verb in &stream_control {
        out.push_str(&format!("    {verb:?},\n"));
    }
    out.push_str("];\n\n");

    // ---- unary RPC verbs ------------------------------------------------------
    out.push_str(
        "/// One WS request/response RPC, derived from a proto service method: the\n\
         /// service + method names and the request/response message type names. The\n\
         /// descriptor-side source of truth for WS unary-verb coverage.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct WireRpc {\n\
         \x20   /// The proto service the method belongs to (e.g. `PricingService`).\n\
         \x20   pub service: &'static str,\n\
         \x20   /// The proto RPC method name (e.g. `Price`).\n\
         \x20   pub method: &'static str,\n\
         \x20   /// The request message type name (e.g. `PriceRequest`).\n\
         \x20   pub request: &'static str,\n\
         \x20   /// The response message type name (e.g. `PriceResponse`).\n\
         \x20   pub response: &'static str,\n\
         }\n\n\
         /// Every WS request/response RPC, in proto declaration order.\n\
         pub const WIRE_RPCS: &[WireRpc] = &[\n",
    );
    for (svc, method, req, resp) in &rpcs {
        out.push_str(&format!(
            "    WireRpc {{ service: {svc:?}, method: {method:?}, request: {req:?}, response: {resp:?} }},\n"
        ));
    }
    out.push_str("];\n\n");

    // ---- message + enum name surface (coverage / drift guards) ----------------
    out.push_str("/// Every message type name in the contract (incl. nested), in declaration order.\npub const MESSAGES: &[&str] = &[\n");
    for m in &messages {
        out.push_str(&format!("    {m:?},\n"));
    }
    out.push_str("];\n\n");

    out.push_str("/// Every top-level enum type name in the contract, in declaration order.\npub const ENUMS: &[&str] = &[\n");
    for e in &enums {
        out.push_str(&format!("    {e:?},\n"));
    }
    out.push_str("];\n\n");

    // ---- per-message field tables --------------------------------------------
    emit_field_tables(&mut out, &message_fields);

    let out_path = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR set by cargo"))
        .join("wire_contract.rs");
    std::fs::write(&out_path, out).expect("write generated wire_contract.rs");
}

/// The descriptor-projected data for one message field, gathered at build time
/// and rendered into a `WireField` literal by [`emit_field_tables`].
struct FieldData {
    /// The proto field name as declared (snake_case in `celnet.proto`).
    proto_name: String,
    /// The JSON key the WS codec reads/writes this field under. Increment 1: the
    /// proto snake_case name verbatim (the curated override table remaps the
    /// exceptions in a later increment).
    json_key: String,
    /// The proto field type — a scalar kind string or the package-stripped
    /// message/enum type name.
    proto_type: String,
    /// The rendered `WireLabel` variant name (`"Singular"` / `"Optional"` /
    /// `"Repeated"`).
    label: &'static str,
    /// The real (non-synthetic) oneof group this field belongs to, if any.
    oneof_group: Option<String>,
}

/// Project one proto field onto its [`FieldData`]. `oneof_decls` is the enclosing
/// message's `oneof_decl` list, used to resolve a real oneof arm's group name; a
/// proto3 `optional` field carries a *synthetic* single-arm oneof in the
/// descriptor, which is reported as [`WireLabel::Optional`] with **no** group
/// (`field.proto3_optional()` is the authoritative discriminator).
fn field_data(
    field: &FieldDescriptorProto,
    oneof_decls: &[OneofDescriptorProto],
    package: &str,
) -> FieldData {
    let proto_name = field.name().to_owned();
    // Increment 1 (item G — ws-codec-from-proto): the JSON key is the proto
    // snake_case name verbatim. The curated override table (docs/INTERFACES.md →
    // "WS codec from proto descriptor (item G)" → Activation plan) remaps the
    // exceptions in a later increment; this stays a pure descriptor projection.
    let json_key = proto_name.clone();

    let proto_type = match field.r#type() {
        Type::Double => "double".to_owned(),
        Type::Float => "float".to_owned(),
        Type::Int64 => "int64".to_owned(),
        Type::Uint64 => "uint64".to_owned(),
        Type::Int32 => "int32".to_owned(),
        Type::Fixed64 => "fixed64".to_owned(),
        Type::Fixed32 => "fixed32".to_owned(),
        Type::Bool => "bool".to_owned(),
        Type::String => "string".to_owned(),
        Type::Group => "group".to_owned(),
        // Message / enum fields carry the referenced type name in `type_name`;
        // report it package-stripped (nesting preserved so it stays unambiguous).
        Type::Message | Type::Enum => simplify_type_name(field.type_name(), package),
        Type::Bytes => "bytes".to_owned(),
        Type::Uint32 => "uint32".to_owned(),
        Type::Sfixed32 => "sfixed32".to_owned(),
        Type::Sfixed64 => "sfixed64".to_owned(),
        Type::Sint32 => "sint32".to_owned(),
        Type::Sint64 => "sint64".to_owned(),
    };

    // proto3 `optional` is a synthetic single-arm oneof in the descriptor: report
    // it as explicit-presence Optional, never a real oneof group.
    let is_proto3_optional = field.proto3_optional();
    let label = if is_proto3_optional {
        "Optional"
    } else {
        match field.label() {
            Label::Repeated => "Repeated",
            // proto3 singular fields carry LABEL_OPTIONAL in the descriptor; the
            // presence distinction is `proto3_optional`, handled above.
            Label::Optional | Label::Required => "Singular",
        }
    };
    let oneof_group = if is_proto3_optional {
        None
    } else {
        field
            .oneof_index
            .and_then(|i| oneof_decls.get(i as usize))
            .map(|d| d.name().to_owned())
    };

    FieldData {
        proto_name,
        json_key,
        proto_type,
        label,
        oneof_group,
    }
}

/// Strip a fully-qualified proto type name (`.celnet.wire.CcyPair`,
/// `.celnet.wire.Tenor.Unit`) down to its package-relative form (`CcyPair`,
/// `Tenor.Unit`) — keeping any nesting so nested types stay unambiguous.
fn simplify_type_name(type_name: &str, package: &str) -> String {
    let trimmed = type_name.strip_prefix('.').unwrap_or(type_name);
    let pkg_prefix = format!("{package}.");
    trimmed
        .strip_prefix(&pkg_prefix)
        .unwrap_or(trimmed)
        .to_owned()
}

/// Emit the field-table section of `wire_contract.rs`: the `WireLabel` enum, the
/// `WireField` / `MessageFields` structs, the `MESSAGE_FIELDS` const (in
/// declaration order, aligned index-for-index with `MESSAGES`) and the
/// `fields_for` lookup. Deterministic in descriptor order ⇒ byte-identical on
/// every build.
fn emit_field_tables(out: &mut String, message_fields: &[(String, Vec<FieldData>)]) {
    out.push_str(
        "/// A proto field's presence / cardinality, projected from the descriptor.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub enum WireLabel {\n\
         \x20   /// A singular proto3 field (implicit presence): a scalar, message or\n\
         \x20   /// enum field that is neither `repeated` nor an explicit `optional`.\n\
         \x20   Singular,\n\
         \x20   /// An explicit-presence proto3 `optional` field. In the descriptor this\n\
         \x20   /// is a synthetic single-arm oneof; it is NOT a real oneof group\n\
         \x20   /// (`oneof_group` is `None`).\n\
         \x20   Optional,\n\
         \x20   /// A `repeated` field.\n\
         \x20   Repeated,\n\
         }\n\n",
    );

    out.push_str(
        "/// One field of a wire message, projected from the proto descriptor: the\n\
         /// proto (snake_case) name, the JSON key the WS codec uses, the proto type,\n\
         /// the presence/cardinality label, and — for a real (non-synthetic) `oneof`\n\
         /// arm — the oneof group name. The descriptor-side input the deferred unary\n\
         /// codec swap builds its curated override table and mechanical codecs on\n\
         /// (arch item G — `ws-codec-from-proto`).\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct WireField {\n\
         \x20   /// The proto field name as declared (snake_case).\n\
         \x20   pub proto_name: &'static str,\n\
         \x20   /// The JSON key the WS codec reads/writes this field under.\n\
         \x20   ///\n\
         \x20   /// Increment 1 (`ws-codec-from-proto`): this is the proto snake_case\n\
         \x20   /// name verbatim (`json_key == proto_name`). The curated override\n\
         \x20   /// table — see `docs/INTERFACES.md` \"WS codec from proto descriptor\n\
         \x20   /// (item G)\" → Activation plan — remaps the exceptions (the FX-legacy\n\
         \x20   /// `pair`/`r_dom`/`r_for` keys, the flat `rho_dom`/`rho_for`\n\
         \x20   /// projection, camelCase `brokenDate`, the bespoke oneof tagging) in a\n\
         \x20   /// later increment so the generated codec stays byte-identical to the\n\
         \x20   /// hand codec.\n\
         \x20   pub json_key: &'static str,\n\
         \x20   /// The proto field type: a scalar kind (`double`, `string`, `bool`,\n\
         \x20   /// `uint32`, …) or the package-stripped message/enum type name.\n\
         \x20   pub proto_type: &'static str,\n\
         \x20   /// The field's presence / cardinality label.\n\
         \x20   pub label: WireLabel,\n\
         \x20   /// The real `oneof` group this field belongs to, or `None` for a plain\n\
         \x20   /// field or a proto3 `optional` (synthetic single-arm oneof).\n\
         \x20   pub oneof_group: Option<&'static str>,\n\
         }\n\n",
    );

    out.push_str(
        "/// Every field of one wire message, in proto declaration order.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq)]\n\
         pub struct MessageFields {\n\
         \x20   /// The (simple) message type name — matches an entry in [`MESSAGES`].\n\
         \x20   pub message: &'static str,\n\
         \x20   /// The message's fields, in proto declaration order.\n\
         \x20   pub fields: &'static [WireField],\n\
         }\n\n",
    );

    out.push_str(
        "/// The per-message field table for every message in the contract (incl.\n\
         /// nested messages), in the same declaration order as [`MESSAGES`].\n\
         pub const MESSAGE_FIELDS: &[MessageFields] = &[\n",
    );
    for (message, fields) in message_fields {
        out.push_str(&format!(
            "    MessageFields {{ message: {message:?}, fields: &[\n"
        ));
        for f in fields {
            let oneof = match &f.oneof_group {
                Some(g) => format!("Some({g:?})"),
                None => "None".to_owned(),
            };
            out.push_str(&format!(
                "        WireField {{ proto_name: {pn:?}, json_key: {jk:?}, proto_type: {pt:?}, label: WireLabel::{label}, oneof_group: {oneof} }},\n",
                pn = f.proto_name,
                jk = f.json_key,
                pt = f.proto_type,
                label = f.label,
            ));
        }
        out.push_str("    ] },\n");
    }
    out.push_str("];\n\n");

    out.push_str(
        "/// The field table for `message` (its simple type name), or `None` if the\n\
         /// name is not a contract message.\n\
         pub fn fields_for(message: &str) -> Option<&'static [WireField]> {\n\
         \x20   MESSAGE_FIELDS\n\
         \x20       .iter()\n\
         \x20       .find(|m| m.message == message)\n\
         \x20       .map(|m| m.fields)\n\
         }\n",
    );
}

/// Collect a message's name, its field table, and its nested messages
/// (recursively) — and, for the `ClientStreamMessage`, its oneof arm field names
/// (the stream-control verbs). `messages` and `message_fields` are pushed in
/// lockstep (name then table, before recursion) so they stay index-aligned.
fn collect_message(
    message: &DescriptorProto,
    package: &str,
    messages: &mut Vec<String>,
    stream_control: &mut Vec<String>,
    message_fields: &mut Vec<(String, Vec<FieldData>)>,
) {
    let name = message.name();
    messages.push(name.to_owned());
    let fields: Vec<FieldData> = message
        .field
        .iter()
        .map(|f| field_data(f, &message.oneof_decl, package))
        .collect();
    message_fields.push((name.to_owned(), fields));
    if name == STREAM_CONTROL_MESSAGE {
        for field in &message.field {
            // The control surface is the message's oneof — every arm carries an
            // `oneof_index`. Field names are already proto snake_case == the WS verb.
            if field.oneof_index.is_some() {
                stream_control.push(field.name().to_owned());
            }
        }
    }
    for nested in &message.nested_type {
        collect_message(nested, package, messages, stream_control, message_fields);
    }
}

/// The last dotted segment of a fully-qualified proto type (`.celnet.wire.Foo` →
/// `Foo`) — the bare message type name.
fn last_segment(qualified: &str) -> &str {
    qualified.rsplit('.').next().unwrap_or(qualified)
}
