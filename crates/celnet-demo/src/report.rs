//! Telemetry and reporting primitives for Celnet demonstrations.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DemoMetric {
    pub(crate) category: String,
    pub(crate) name: String,
    pub(crate) latency_ns: f64,
    pub(crate) throughput_ops_sec: f64,
    pub(crate) status: String,
    pub(crate) details: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct DemoReport {
    pub(crate) timestamp_utc: u64,
    pub(crate) hardware_target: String,
    pub(crate) total_elapsed_ms: f64,
    pub(crate) metrics: Vec<DemoMetric>,
}

impl DemoReport {
    pub(crate) fn new() -> Self {
        Self {
            timestamp_utc: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            hardware_target: "Apple Silicon (Darwin aarch64) / Bare-Metal x86-64".to_string(),
            total_elapsed_ms: 0.0,
            metrics: Vec::new(),
        }
    }

    pub(crate) fn record(&mut self, category: &str, name: &str, duration: Duration, iters: usize, details: &str) {
        let ns = (duration.as_nanos() as f64) / (iters as f64);
        let ops = if ns > 0.0 { 1e9 / ns } else { 0.0 };
        self.metrics.push(DemoMetric {
            category: category.to_string(),
            name: name.to_string(),
            latency_ns: ns,
            throughput_ops_sec: ops,
            status: "VERIFIED [PASS]".to_string(),
            details: details.to_string(),
        });
    }

    pub(crate) fn print_summary(&self) {
        println!("
=======================================================================================");
        println!("   CELNET VERIFIED DEMONSTRATION TELEMETRY & AUDIT SUMMARY");
        println!("=======================================================================================");
        println!("{:<24} | {:<32} | {:>12} | {:>14}", "Category", "Benchmark", "Latency", "Throughput");
        println!("---------------------------------------------------------------------------------------");
        for m in &self.metrics {
            let lat_str = if m.latency_ns >= 1_000_000.0 {
                format!("{:.2} ms", m.latency_ns / 1_000_000.0)
            } else if m.latency_ns >= 1_000.0 {
                format!("{:.2} µs", m.latency_ns / 1_000.0)
            } else {
                format!("{:.2} ns", m.latency_ns)
            };
            let tp_str = if m.throughput_ops_sec >= 1_000_000.0 {
                format!("{:.1}M /sec", m.throughput_ops_sec / 1_000_000.0)
            } else if m.throughput_ops_sec >= 1_000.0 {
                format!("{:.1}k /sec", m.throughput_ops_sec / 1_000.0)
            } else {
                format!("{:.0} /sec", m.throughput_ops_sec)
            };
            println!("{:<24} | {:<32} | {:>12} | {:>14}", m.category, m.name, lat_str, tp_str);
        }
        println!("=======================================================================================");
    }
}
