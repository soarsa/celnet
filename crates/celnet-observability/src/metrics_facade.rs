//! A thin, vendor-neutral metrics facade over the [`metrics`] crate's
//! counters / gauges / histograms.
//!
//! Lives on the **edge / drain side**. The hot path never emits a metric
//! directly — it pushes POD [`HotSample`](crate::record::HotSample)s through the
//! SPSC ring; the drain converts those into the metrics declared here. Centralis-
//! ing the metric *keys* in one place keeps names stable and purpose-named, and
//! lets `celnet-server` install whatever recorder (Prometheus exporter, OTLP,
//! …) it likes behind the facade without the rest of the system knowing.

use crate::record::{ErrorClass, OpKind};

/// The stable metric key namespace prefix for all Celnet metrics.
pub const NAMESPACE: &str = "celnet";

/// Canonical metric keys (the only place metric names are spelled).
pub mod keys {
    /// Counter: pricing operations completed, labelled by `op` and `class`.
    pub const OPS_TOTAL: &str = "celnet.ops.total";
    /// Counter: telemetry samples drained from the SPSC ring.
    pub const TELEMETRY_DRAINED: &str = "celnet.telemetry.drained.total";
    /// Counter: telemetry samples dropped because the ring was full.
    pub const TELEMETRY_DROPPED: &str = "celnet.telemetry.dropped.total";
    /// Gauge: current depth of work-in-flight on the engine (set by the edge).
    pub const INFLIGHT: &str = "celnet.engine.inflight";
    /// Histogram: per-operation latency in nanoseconds, labelled by `op`.
    pub const OP_LATENCY_NS: &str = "celnet.op.latency.ns";
    /// Counter: quotes streamed to RFS subscribers.
    pub const QUOTES_STREAMED: &str = "celnet.quotes.streamed.total";
    /// Counter: audit records committed to the lossless audit log.
    pub const AUDIT_COMMITTED: &str = "celnet.audit.committed.total";
}

/// Record that one pricing operation completed, incrementing the labelled
/// `ops_total` counter and observing its latency on the `op_latency_ns`
/// histogram.
///
/// Call from the **drain**, once per drained [`HotSample`](crate::record::HotSample).
pub fn record_op(kind: OpKind, class: ErrorClass, latency_ns: u64) {
    metrics::counter!(
        keys::OPS_TOTAL,
        "op" => kind.label(),
        "class" => class.label(),
    )
    .increment(1);
    metrics::histogram!(keys::OP_LATENCY_NS, "op" => kind.label()).record(latency_ns as f64);
}

/// Record that `n` telemetry samples were drained this cycle.
pub fn record_drained(n: u64) {
    metrics::counter!(keys::TELEMETRY_DRAINED).increment(n);
}

/// Record that `n` telemetry samples were dropped (ring full).
pub fn record_dropped(n: u64) {
    metrics::counter!(keys::TELEMETRY_DROPPED).increment(n);
}

/// Set the engine in-flight gauge to `depth`.
pub fn set_inflight(depth: u64) {
    #[allow(clippy::cast_precision_loss)]
    metrics::gauge!(keys::INFLIGHT).set(depth as f64);
}

/// Increment the quotes-streamed counter by `n`.
pub fn record_quotes_streamed(n: u64) {
    metrics::counter!(keys::QUOTES_STREAMED).increment(n);
}

/// Increment the audit-committed counter by one.
pub fn record_audit_committed() {
    metrics::counter!(keys::AUDIT_COMMITTED).increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    // A minimal in-test recorder that captures counter increments and histogram
    // observations, proving the facade routes to the `metrics` machinery with
    // the right keys/labels — without pulling in an exporter dependency.
    #[derive(Default)]
    struct CaptureRecorder {
        counters: Mutex<Vec<(String, u64)>>,
        histos: Mutex<Vec<(String, f64)>>,
        gauges: Mutex<Vec<(String, f64)>>,
    }

    struct CapCounter(&'static CaptureRecorder, String);
    struct CapGauge(&'static CaptureRecorder, String);
    struct CapHisto(&'static CaptureRecorder, String);

    impl metrics::CounterFn for CapCounter {
        fn increment(&self, v: u64) {
            self.0.counters.lock().unwrap().push((self.1.clone(), v));
        }
        fn absolute(&self, v: u64) {
            self.0.counters.lock().unwrap().push((self.1.clone(), v));
        }
    }
    impl metrics::GaugeFn for CapGauge {
        fn increment(&self, v: f64) {
            self.0.gauges.lock().unwrap().push((self.1.clone(), v));
        }
        fn decrement(&self, v: f64) {
            self.0.gauges.lock().unwrap().push((self.1.clone(), -v));
        }
        fn set(&self, v: f64) {
            self.0.gauges.lock().unwrap().push((self.1.clone(), v));
        }
    }
    impl metrics::HistogramFn for CapHisto {
        fn record(&self, v: f64) {
            self.0.histos.lock().unwrap().push((self.1.clone(), v));
        }
    }

    impl metrics::Recorder for &'static CaptureRecorder {
        fn describe_counter(
            &self,
            _: metrics::KeyName,
            _: Option<metrics::Unit>,
            _: metrics::SharedString,
        ) {
        }
        fn describe_gauge(
            &self,
            _: metrics::KeyName,
            _: Option<metrics::Unit>,
            _: metrics::SharedString,
        ) {
        }
        fn describe_histogram(
            &self,
            _: metrics::KeyName,
            _: Option<metrics::Unit>,
            _: metrics::SharedString,
        ) {
        }
        fn register_counter(
            &self,
            key: &metrics::Key,
            _: &metrics::Metadata<'_>,
        ) -> metrics::Counter {
            metrics::Counter::from_arc(std::sync::Arc::new(CapCounter(self, key.name().to_owned())))
        }
        fn register_gauge(&self, key: &metrics::Key, _: &metrics::Metadata<'_>) -> metrics::Gauge {
            metrics::Gauge::from_arc(std::sync::Arc::new(CapGauge(self, key.name().to_owned())))
        }
        fn register_histogram(
            &self,
            key: &metrics::Key,
            _: &metrics::Metadata<'_>,
        ) -> metrics::Histogram {
            metrics::Histogram::from_arc(std::sync::Arc::new(CapHisto(self, key.name().to_owned())))
        }
    }

    // Leak a single recorder so it has 'static lifetime and serialize the test
    // (global recorder is process-wide and install-once).
    static GUARD: Mutex<()> = Mutex::new(());
    static INSTALLED: AtomicU64 = AtomicU64::new(0);

    fn recorder() -> &'static CaptureRecorder {
        // Install exactly once; subsequent calls reuse the leaked recorder.
        static CELL: Mutex<Option<&'static CaptureRecorder>> = Mutex::new(None);
        let mut cell = CELL.lock().unwrap();
        if let Some(r) = *cell {
            return r;
        }
        let r: &'static CaptureRecorder = Box::leak(Box::new(CaptureRecorder::default()));
        if INSTALLED.swap(1, Ordering::SeqCst) == 0 {
            let _ = metrics::set_global_recorder(r);
        }
        *cell = Some(r);
        r
    }

    #[test]
    fn facade_routes_to_recorder() {
        let _g = GUARD.lock().unwrap();
        let r = recorder();
        r.counters.lock().unwrap().clear();
        r.histos.lock().unwrap().clear();
        r.gauges.lock().unwrap().clear();

        record_op(OpKind::VanillaPrice, ErrorClass::Ok, 1500);
        record_drained(7);
        record_dropped(2);
        set_inflight(11);
        record_quotes_streamed(4);
        record_audit_committed();

        let counters = r.counters.lock().unwrap();
        assert!(
            counters
                .iter()
                .any(|(k, v)| k == keys::OPS_TOTAL && *v == 1)
        );
        assert!(
            counters
                .iter()
                .any(|(k, v)| k == keys::TELEMETRY_DRAINED && *v == 7)
        );
        assert!(
            counters
                .iter()
                .any(|(k, v)| k == keys::TELEMETRY_DROPPED && *v == 2)
        );
        assert!(
            counters
                .iter()
                .any(|(k, v)| k == keys::QUOTES_STREAMED && *v == 4)
        );
        assert!(
            counters
                .iter()
                .any(|(k, v)| k == keys::AUDIT_COMMITTED && *v == 1)
        );

        let histos = r.histos.lock().unwrap();
        assert!(
            histos
                .iter()
                .any(|(k, v)| k == keys::OP_LATENCY_NS && (*v - 1500.0).abs() < 1e-9)
        );

        let gauges = r.gauges.lock().unwrap();
        assert!(
            gauges
                .iter()
                .any(|(k, v)| k == keys::INFLIGHT && (*v - 11.0).abs() < 1e-9)
        );
    }
}
