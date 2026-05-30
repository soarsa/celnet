//! Integration test: the structured JSON subscriber emits valid,
//! line-delimited JSON with the expected fields, and audit records render as
//! structured events.
//!
//! We build a scoped subscriber over an in-memory capture writer (so the test
//! never touches stdout, never blocks, and is fully deterministic) and parse the
//! emitted lines as JSON.

use std::io;
use std::sync::{Arc, Mutex};

use celnet_observability::logging::{
    AuditRecord, AuditStage, LogClass, LogConfig, build_json_subscriber,
};
use celnet_observability::record::OpKind;
use tracing::Level;

/// An in-memory `MakeWriter` capturing everything written, for assertion.
#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl CaptureWriter {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).expect("utf8")
    }
}

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
    type Writer = CaptureWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn emits_structured_json_lines() {
    let writer = CaptureWriter::default();
    let cfg = LogConfig {
        max_level: Level::INFO,
        filter: Some("info".to_string()),
        with_location: false,
        with_spans: true,
    };
    let subscriber = build_json_subscriber(&cfg, writer.clone());

    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("price_request", request_id = 42u64);
        let _g = span.enter();
        tracing::info!(
            class = LogClass::Lifecycle.label(),
            instrument = "EURUSD",
            premium = 0.0123_f64,
            "priced"
        );

        let rec = AuditRecord::new(AuditStage::QuoteIssued, 42, "idem-9", "tenant-x", "EURUSD")
            .with_priced(OpKind::VanillaPrice, 0.0123);
        rec.emit();
    });

    let out = writer.contents();
    let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        lines.len() >= 2,
        "expected at least two log lines, got: {out}"
    );

    // Every emitted line must be valid JSON.
    let mut events = Vec::new();
    for line in &lines {
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("line not JSON ({e}): {line}"));
        events.push(v);
    }

    // The first event: structured fields are flattened in (flatten_event=true),
    // carries the timestamp/level, the span list, and our custom fields.
    let priced = events
        .iter()
        .find(|v| v.get("message").and_then(|m| m.as_str()) == Some("priced"))
        .expect("a 'priced' event");
    assert_eq!(priced["level"], "INFO");
    assert!(priced.get("timestamp").is_some(), "must carry a timestamp");
    assert_eq!(priced["class"], "lifecycle");
    assert_eq!(priced["instrument"], "EURUSD");
    // The active span must be attached.
    let spans = priced
        .get("spans")
        .and_then(|s| s.as_array())
        .expect("span list present");
    assert!(
        spans
            .iter()
            .any(|s| s.get("request_id").and_then(|r| r.as_u64()) == Some(42)),
        "active span with request_id=42 must be attached: {priced}"
    );

    // The audit event: class=security, audit_stage present, premium present.
    let audit = events
        .iter()
        .find(|v| v.get("message").and_then(|m| m.as_str()) == Some("audit"))
        .expect("an 'audit' event");
    assert_eq!(audit["class"], "security");
    assert_eq!(audit["audit_stage"], "quote_issued");
    assert_eq!(audit["instrument"], "EURUSD");
    assert_eq!(audit["op"], "vanilla_price");
}

#[test]
fn filter_directive_suppresses_below_threshold() {
    let writer = CaptureWriter::default();
    let cfg = LogConfig {
        max_level: Level::INFO,
        filter: Some("warn".to_string()),
        with_location: false,
        with_spans: false,
    };
    let subscriber = build_json_subscriber(&cfg, writer.clone());

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("should_be_filtered");
        tracing::warn!(class = LogClass::Degraded.label(), "kept");
    });

    let out = writer.contents();
    assert!(
        !out.contains("should_be_filtered"),
        "info must be filtered out"
    );
    assert!(out.contains("kept"), "warn must pass the filter");
    // The surviving line is still valid JSON.
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        let _: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
    }
}
