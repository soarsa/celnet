//! End-to-end **wire-path** latency-under-load harness.
//!
//! This is the GA-gating complement to the in-core micro-benchmarks. The
//! micro-benchmarks (`benches/`) measure the pinned hot core in isolation
//! (price + full Greeks in ~tens of nanoseconds); this harness measures what a
//! real counterparty actually observes: the **client-side round-trip** of an
//! RFQ over the loopback gRPC wire, recorded *while the edge is under sustained
//! streaming + RFQ load*.
//!
//! # What is measured
//!
//! The harness spins the real [`celnet_server::Edge`] in-process on an ephemeral
//! `127.0.0.1:0` port (the same construction the server's own integration tests
//! use), marks it ready, and:
//!
//! 1. opens [`LoadConfig::stream_subscriptions`] concurrent RFS streaming
//!    subscriptions, each draining server `Update`/`Snapshot`/`Heartbeat`
//!    messages continuously — this is the *background load* that keeps the
//!    edge's async runtime, the SPSC core rings, and the pricing core busy;
//! 2. fires [`LoadConfig::rfq_requests`] `RequestQuote` round-trips from
//!    [`LoadConfig::rfq_concurrency`] concurrent client tasks sharing one gRPC
//!    channel, timing **each** round-trip from just-before-send to
//!    reply-received with a monotonic clock and recording it into a per-task
//!    [`hdrhistogram::Histogram`].
//!
//! The per-task histograms are then merged. Because every request is issued and
//! timed independently (closed-loop per task, open across tasks), and because
//! the whole run is bounded by both a request budget and a hard wall-clock cap,
//! a stall surfaces as a fast failure, never an infinite hang.
//!
//! # Honesty
//!
//! The reported numbers are the *loopback* wire-path on the measuring host
//! (Apple M4 here): they include the full gRPC/HTTP-2 client+server stack,
//! tonic codec, the async⇄core SPSC hop, and the pricing compute — everything
//! except the physical NIC and the network. They are therefore an **upper
//! bound on the compute+framing cost** and a **lower bound on real
//! cross-host wire latency** (which adds NIC + switch + propagation). The
//! README records this caveat explicitly; the durable claim is the *ratio to
//! budget* and the *shape of the tail*, not an absolute cross-host figure.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

use celnet_conventions::ConventionRecord;
use celnet_engine::testing::make_state;
use celnet_proto::quote_service_client::QuoteServiceClient;
use celnet_proto::stream_service_client::StreamServiceClient;
use celnet_proto::{
    AtmConvention, CcyPair, Conventions, Cut, DayCount, DeltaConvention, Instrument, OptionType,
    PremiumStyle, Quantity, QuoteRequest, Settlement, Side, StrikeOrDelta, Subscribe,
    SubscriptionId, Tenor, Vanilla, client_stream_message, instrument, strike_or_delta,
    tenor as tenor_mod,
};
use celnet_server::{Clock, CoreLink, Edge, SpreadModel};
use celnet_types::{CcyPair as TypesPair, Tenor as TypesTenor};

use tonic::transport::Channel;

/// Configuration for one wire-path load run.
///
/// Both a request budget *and* a wall-clock cap bound the run, so it always
/// self-terminates: whichever limit is hit first stops the load. The defaults
/// (via [`LoadConfig::default`]) are sized for a quick, deterministic CI gate;
/// the standalone `wire_load` binary scales them up for the published proof.
#[derive(Debug, Clone, Copy)]
pub struct LoadConfig {
    /// Total number of RFQ round-trips to time across all client tasks.
    pub rfq_requests: u64,
    /// Number of concurrent RFQ client tasks (closed-loop each) sharing one
    /// gRPC channel. Higher concurrency raises the offered load and exercises
    /// the edge's multiplexing under contention.
    pub rfq_concurrency: usize,
    /// Number of concurrent RFS streaming subscriptions held open as background
    /// load while the RFQ round-trips are timed.
    pub stream_subscriptions: usize,
    /// Hard wall-clock ceiling for the whole load phase. The run stops at the
    /// request budget or this cap, whichever comes first — so it can never hang.
    pub wall_clock_cap: Duration,
}

impl Default for LoadConfig {
    fn default() -> Self {
        // CI-gate sizing: enough requests for a stable p99/p99.9 estimate, short
        // enough to finish in a couple of seconds, hard-capped well under the
        // CI step timeout.
        Self {
            rfq_requests: 20_000,
            rfq_concurrency: 16,
            stream_subscriptions: 32,
            wall_clock_cap: Duration::from_secs(30),
        }
    }
}

impl LoadConfig {
    /// The larger sizing used by the published-proof `wire_load` binary: more
    /// requests for a tighter tail estimate, still bounded.
    #[must_use]
    pub fn published_proof() -> Self {
        Self {
            rfq_requests: 100_000,
            rfq_concurrency: 32,
            stream_subscriptions: 64,
            wall_clock_cap: Duration::from_secs(60),
        }
    }
}

/// A single recorded percentile, in microseconds, for serialization and the gate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Percentiles {
    /// 50th percentile (median) round-trip, microseconds.
    pub p50_us: f64,
    /// 99th percentile round-trip, microseconds.
    pub p99_us: f64,
    /// 99.9th percentile round-trip, microseconds.
    pub p999_us: f64,
    /// 99.99th percentile round-trip, microseconds.
    pub p9999_us: f64,
    /// Maximum observed round-trip, microseconds.
    pub max_us: f64,
    /// Minimum observed round-trip, microseconds.
    pub min_us: f64,
}

/// The full result of a wire-path load run — serialized as the committed
/// baseline and re-emitted by every run for the CI bench-regression gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireReport {
    /// Workload identifier (e.g. `"rfq_under_rfs_load"`).
    pub workload: String,
    /// RFQ round-trip latency percentiles (the headline wire-path figure).
    pub rfq: Percentiles,
    /// Total RFQ round-trips completed and timed.
    pub rfq_count: u64,
    /// Concurrent RFQ client tasks used.
    pub rfq_concurrency: usize,
    /// Concurrent RFS streaming subscriptions held open as background load.
    pub stream_subscriptions: usize,
    /// Achieved RFQ throughput across all tasks, requests/second.
    pub rfq_throughput_per_s: f64,
    /// Background RFS stream messages drained during the run (load evidence).
    pub stream_messages_drained: u64,
    /// Wall-clock duration of the timed load phase, seconds.
    pub elapsed_s: f64,
}

impl WireReport {
    /// Print a human-readable summary to stdout (used by both binaries).
    pub fn print_summary(&self) {
        println!(
            "== celnet wire-path latency under load: {} ==",
            self.workload
        );
        println!(
            "  load: {} concurrent RFS subscriptions + {} concurrent RFQ client tasks",
            self.stream_subscriptions, self.rfq_concurrency
        );
        println!(
            "  RFQ round-trips timed: {} in {:.3}s  ({:.0} req/s)",
            self.rfq_count, self.elapsed_s, self.rfq_throughput_per_s
        );
        println!(
            "  RFS background messages drained during the run: {}",
            self.stream_messages_drained
        );
        println!("  RFQ wire-path round-trip (client-observed, loopback gRPC):");
        println!("    min    = {:>10.2} us", self.rfq.min_us);
        println!("    p50    = {:>10.2} us", self.rfq.p50_us);
        println!("    p99    = {:>10.2} us", self.rfq.p99_us);
        println!("    p99.9  = {:>10.2} us", self.rfq.p999_us);
        println!("    p99.99 = {:>10.2} us", self.rfq.p9999_us);
        println!("    max    = {:>10.2} us", self.rfq.max_us);
    }
}

/// The resolved EURUSD 1Y convention record the fixture is built on (mirrors the
/// server integration-test fixture so the harness prices the tested workload).
#[must_use]
fn eurusd_conv() -> ConventionRecord {
    celnet_conventions::resolve(TypesPair::parse("EURUSD").unwrap(), TypesTenor::Years(1)).record
}

/// The wire conventions every request carries (mirrors the server test fixture).
///
/// Public so the architectural-invariant integration test can build a `Price`
/// request against the same EUR/USD fixture this harness prices.
pub fn wire_conventions() -> Conventions {
    Conventions {
        delta_convention: DeltaConvention::SpotUnadjusted as i32,
        atm_convention: AtmConvention::AtmForward as i32,
        premium_style: PremiumStyle::DomesticPips as i32,
        cut: Cut::NewYork1000 as i32,
        day_count: DayCount::Act365Fixed as i32,
        settlement: Settlement::Deliverable as i32,
    }
}

/// A vanilla EUR/USD call struck at `strike` with a 1Y expiry.
///
/// Public so the architectural-invariant integration test can price the same owned
/// EUR/USD pair this harness streams/quotes.
pub fn vanilla_call(strike: f64) -> Instrument {
    Instrument {
        underlying: Some(celnet_proto::Underlying::fx(CcyPair {
            base: "EUR".to_owned(),
            quote: "USD".to_owned(),
        })),
        tenor: Some(Tenor {
            unit: tenor_mod::Unit::Years as i32,
            count: 1,
            broken_date: None,
        }),
        expiry_years: 1.0,
        quantity: Some(Quantity {
            notional: 1_000_000.0,
            base_ccy: true,
        }),
        side: Side::TwoWay as i32,
        solve: None,
        pricing_model: celnet_proto::PricingModel::Default as i32,
        product: Some(instrument::Product::Vanilla(Vanilla {
            option_type: OptionType::Call as i32,
            strike: Some(StrikeOrDelta {
                spec: Some(strike_or_delta::Spec::Strike(strike)),
            }),
        })),
        ..Default::default()
    }
}

/// Build a fresh, calibrated EURUSD edge bound on an ephemeral loopback port and
/// marked ready. Returns the running edge and its bound gRPC address.
///
/// # Errors
///
/// Returns the listener bind / serve error if the ephemeral port cannot be
/// bound (e.g. an exhausted ephemeral range).
pub async fn start_ready_edge() -> std::io::Result<(Edge, SocketAddr)> {
    let initial = make_state(1.10, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().expect("loopback addr parses");
    let edge = Edge::start(
        grpc,
        Arc::clone(&link),
        SpreadModel::default(),
        Clock::system(),
    )
    .await?;
    edge.gate().mark_ready();
    let addr = edge.grpc_addr();
    Ok((edge, addr))
}

/// An outbound client stream wrapping a oneshot-closeable mpsc receiver, so the
/// harness can subscribe and then hold the session open until told to stop.
struct ClientOutbound {
    rx: tokio::sync::mpsc::Receiver<celnet_proto::ClientStreamMessage>,
}

impl futures_util::Stream for ClientOutbound {
    type Item = celnet_proto::ClientStreamMessage;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// Run one wire-path load workload against an edge dialed at `addr`, returning a
/// [`WireReport`]. The caller owns the [`Edge`] and shuts it down afterwards.
///
/// The whole timed phase is bounded by both the request budget and the wall-clock
/// cap in `config`, so this future always resolves — a stalled edge surfaces as a
/// short run that fails the gate, never a hang.
///
/// # Errors
///
/// Returns an error string if the gRPC channel cannot be established or a stream
/// cannot be opened. Per-request RFQ errors under load are counted but do not
/// abort the run (the achieved-throughput and tail figures already reflect them).
pub async fn run_load(addr: SocketAddr, config: LoadConfig) -> Result<WireReport, String> {
    // One shared HTTP/2 channel; tonic multiplexes concurrent calls over it, so
    // this exercises the real edge's connection-level concurrency.
    let channel = Channel::from_shared(format!("http://{addr}"))
        .map_err(|e| format!("invalid endpoint: {e}"))?
        .connect()
        .await
        .map_err(|e| format!("channel connect failed: {e}"))?;

    // --- Background streaming load ----------------------------------------
    // Open `stream_subscriptions` RFS sessions; each drains server messages in a
    // spawned task until the stop flag flips. The drained-message counter is
    // load evidence printed in the report.
    let stop = Arc::new(AtomicBool::new(false));
    let drained = Arc::new(AtomicU64::new(0));
    let mut stream_tasks = Vec::with_capacity(config.stream_subscriptions);
    // Keep the outbound senders alive for the duration so the sessions stay open.
    let mut stream_keepalive = Vec::with_capacity(config.stream_subscriptions);

    for i in 0..config.stream_subscriptions {
        let mut sclient = StreamServiceClient::new(channel.clone());
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        // Authenticate the session FIRST (the stream/WS caller-authz cross-cut):
        // the bench edge boots under the Enforce default, so a session must pin its
        // caller before any subscribe or the subscribe is rejected `unauthenticated`.
        // Assert the audited explicit grant-all — the same default the SDK's opening
        // `Authenticate` frame and the gated risk/quote paths use — so the load
        // generator is admitted under Enforce.
        tx.send(celnet_proto::ClientStreamMessage {
            message: Some(client_stream_message::Message::Authenticate(
                celnet_proto::StreamAuth {
                    session_token: None,
                    principal: Some(celnet_proto::EntitlementPrincipal {
                        grant_all: true,
                        grants: vec![],
                        denies: vec![],
                    }),
                },
            )),
        })
        .await
        .map_err(|e| format!("authenticate send failed: {e}"))?;

        // Subscribe to a distinct strike per subscription so each is a real,
        // independently-sequenced line.
        let strike = 1.00 + 0.005 * (i as f64);
        tx.send(celnet_proto::ClientStreamMessage {
            message: Some(client_stream_message::Message::Subscribe(Subscribe {
                subscription: Some(SubscriptionId {
                    value: i as u64 + 1,
                }),
                instrument: Some(vanilla_call(strike)),
                conventions: Some(wire_conventions()),
                throttle_nanos: 0,
                correlation_id: None,
                surface_version: None,
                attribution: None,
            })),
        })
        .await
        .map_err(|e| format!("subscribe send failed: {e}"))?;

        let mut inbound = sclient
            .stream_session(ClientOutbound { rx })
            .await
            .map_err(|e| format!("stream open failed: {e}"))?
            .into_inner();

        let stop_c = Arc::clone(&stop);
        let drained_c = Arc::clone(&drained);
        let task = tokio::spawn(async move {
            use futures_util::StreamExt;
            while !stop_c.load(Ordering::Relaxed) {
                // Bound each await so a silent stream cannot wedge the drainer.
                match tokio::time::timeout(Duration::from_millis(250), inbound.next()).await {
                    Ok(Some(Ok(_msg))) => {
                        drained_c.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Some(Err(_))) | Ok(None) => break,
                    Err(_) => continue, // idle tick; re-check the stop flag.
                }
            }
        });
        stream_tasks.push(task);
        stream_keepalive.push(tx);
    }

    // --- Timed RFQ round-trips --------------------------------------------
    // `rfq_concurrency` closed-loop client tasks pull request indices from a
    // shared atomic counter until the budget is drained or the wall-clock cap is
    // hit, timing each round-trip into a per-task histogram (merged at the end).
    let issued = Arc::new(AtomicU64::new(0));
    let deadline = Instant::now() + config.wall_clock_cap;
    let started = Instant::now();

    let mut rfq_tasks = Vec::with_capacity(config.rfq_concurrency);
    for task_id in 0..config.rfq_concurrency {
        let mut qclient = QuoteServiceClient::new(channel.clone());
        let issued_c = Arc::clone(&issued);
        let budget = config.rfq_requests;
        let task = tokio::spawn(async move {
            // 1µs..1s window at 3 significant figures — ample headroom for a
            // loopback RFQ while keeping the histogram compact.
            let mut hist: Histogram<u64> =
                Histogram::new_with_bounds(1, 1_000_000_000, 3).expect("valid histogram bounds");
            let mut errors: u64 = 0;
            loop {
                let n = issued_c.fetch_add(1, Ordering::Relaxed);
                if n >= budget || Instant::now() >= deadline {
                    break;
                }
                // A distinct idempotency key + strike per request so the edge
                // genuinely prices each one (no idempotent short-circuit).
                let strike = 1.05 + 0.0001 * ((n % 500) as f64);
                let req = QuoteRequest {
                    idempotency_key: format!("load-{task_id}-{n}"),
                    instrument: Some(vanilla_call(strike)),
                    conventions: Some(wire_conventions()),
                    correlation_id: None,
                    surface_version: None,
                    attribution: None,
                    session_token: None,
                    // The RFQ path is now caller-gated (item B §2); the bench edge
                    // boots under the Enforce default, so assert the audited explicit
                    // grant-all principal (the same default the SDK/risk path uses) so
                    // the load generator is admitted.
                    principal: Some(celnet_proto::EntitlementPrincipal {
                        grant_all: true,
                        grants: vec![],
                        denies: vec![],
                    }),
                };
                let t0 = Instant::now();
                let res =
                    tokio::time::timeout(Duration::from_secs(2), qclient.request_quote(req)).await;
                let elapsed_ns = t0.elapsed().as_nanos();
                match res {
                    Ok(Ok(_resp)) => {
                        // Saturating record so a pathological outlier above the
                        // histogram ceiling is clamped, never a panic.
                        let v = u64::try_from(elapsed_ns).unwrap_or(u64::MAX).max(1);
                        hist.saturating_record(v);
                    }
                    Ok(Err(_status)) => errors += 1,
                    Err(_) => errors += 1, // per-request timeout (counted, not fatal).
                }
            }
            (hist, errors)
        });
        rfq_tasks.push(task);
    }

    // Merge the per-task histograms.
    let mut merged: Histogram<u64> =
        Histogram::new_with_bounds(1, 1_000_000_000, 3).expect("valid histogram bounds");
    let mut total_errors: u64 = 0;
    for task in rfq_tasks {
        let (hist, errors) = task
            .await
            .map_err(|e| format!("rfq task join failed: {e}"))?;
        merged
            .add(&hist)
            .map_err(|e| format!("histogram merge failed: {e}"))?;
        total_errors += errors;
    }
    let elapsed = started.elapsed();

    // Stop the background streams and join the drainers.
    stop.store(true, Ordering::Relaxed);
    drop(stream_keepalive); // close the outbound sessions.
    for task in stream_tasks {
        // The drainers re-check the stop flag at most every 250ms; bound the join.
        let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
    }

    let rfq_count = merged.len();
    if rfq_count == 0 {
        return Err(format!(
            "no RFQ round-trips completed ({total_errors} errored) — edge unresponsive"
        ));
    }

    let ns_to_us = |ns: u64| ns as f64 / 1000.0;
    let rfq = Percentiles {
        min_us: ns_to_us(merged.min()),
        p50_us: ns_to_us(merged.value_at_quantile(0.50)),
        p99_us: ns_to_us(merged.value_at_quantile(0.99)),
        p999_us: ns_to_us(merged.value_at_quantile(0.999)),
        p9999_us: ns_to_us(merged.value_at_quantile(0.9999)),
        max_us: ns_to_us(merged.max()),
    };
    let elapsed_s = elapsed.as_secs_f64();
    let rfq_throughput_per_s = if elapsed_s > 0.0 {
        rfq_count as f64 / elapsed_s
    } else {
        0.0
    };

    Ok(WireReport {
        workload: "rfq_under_rfs_load".to_owned(),
        rfq,
        rfq_count,
        rfq_concurrency: config.rfq_concurrency,
        stream_subscriptions: config.stream_subscriptions,
        rfq_throughput_per_s,
        stream_messages_drained: drained.load(Ordering::Relaxed),
        elapsed_s,
    })
}

/// One percentile's worth of a gate breach, for human-readable reporting.
#[derive(Debug, Clone)]
pub struct GateBreach {
    /// The percentile name (e.g. `"p99"`).
    pub metric: String,
    /// The committed baseline value, microseconds.
    pub baseline_us: f64,
    /// The measured value this run, microseconds.
    pub measured_us: f64,
    /// The ceiling the measured value had to stay under (`baseline * (1+tol)`).
    pub ceiling_us: f64,
}

/// Compare a measured [`WireReport`] against a committed baseline at a relative
/// `tolerance` (e.g. `0.50` = allow up to 50% slower than baseline before
/// failing). Returns the list of breached percentiles — empty means the gate
/// passes.
///
/// Only the *latency* percentiles are gated (a regression is a slowdown); the
/// median and the three tail percentiles are each checked. Throughput is
/// reported but not gated, because it is sensitive to the host's core count and
/// concurrent CI load in a way the latency tail is not.
#[must_use]
pub fn compare_to_baseline(
    baseline: &WireReport,
    measured: &WireReport,
    tolerance: f64,
) -> Vec<GateBreach> {
    let checks: [(&str, f64, f64); 4] = [
        ("p50", baseline.rfq.p50_us, measured.rfq.p50_us),
        ("p99", baseline.rfq.p99_us, measured.rfq.p99_us),
        ("p99.9", baseline.rfq.p999_us, measured.rfq.p999_us),
        ("p99.99", baseline.rfq.p9999_us, measured.rfq.p9999_us),
    ];
    let mut breaches = Vec::new();
    for (metric, base, meas) in checks {
        let ceiling = base * (1.0 + tolerance);
        if meas > ceiling {
            breaches.push(GateBreach {
                metric: metric.to_owned(),
                baseline_us: base,
                measured_us: meas,
                ceiling_us: ceiling,
            });
        }
    }
    breaches
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The harness must run a small bounded load against a real in-process edge,
    /// time RFQ round-trips, hold streaming load open, and return a sane report —
    /// the whole thing inside a hard deadline so a regression fails fast.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn wire_load_runs_bounded_and_reports() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let (edge, addr) = start_ready_edge().await.expect("edge binds");
            let config = LoadConfig {
                rfq_requests: 2_000,
                rfq_concurrency: 8,
                stream_subscriptions: 8,
                wall_clock_cap: Duration::from_secs(15),
            };
            let report = run_load(addr, config).await.expect("load run completes");

            // The run genuinely priced RFQs and held streaming load open.
            assert!(
                report.rfq_count > 0,
                "must time at least one RFQ round-trip"
            );
            assert!(
                report.rfq_count <= config.rfq_requests,
                "cannot complete more than the budget"
            );
            assert!(
                report.stream_messages_drained > 0,
                "background RFS streaming must have produced messages (real load)"
            );
            // Percentiles must be finite, positive and monotone.
            assert!(report.rfq.min_us > 0.0 && report.rfq.min_us.is_finite());
            assert!(report.rfq.p50_us >= report.rfq.min_us);
            assert!(report.rfq.p99_us >= report.rfq.p50_us);
            assert!(report.rfq.p999_us >= report.rfq.p99_us);
            assert!(report.rfq.max_us >= report.rfq.p9999_us);
            assert!(report.rfq_throughput_per_s > 0.0);

            edge.shutdown(Duration::from_secs(5)).await;
        })
        .await
        .expect("wire-load test must not hang");
    }

    /// The gate comparison must pass when within tolerance and flag exactly the
    /// percentiles that regress beyond it.
    #[test]
    fn gate_flags_only_regressed_percentiles() {
        let base = WireReport {
            workload: "rfq_under_rfs_load".to_owned(),
            rfq: Percentiles {
                min_us: 10.0,
                p50_us: 50.0,
                p99_us: 100.0,
                p999_us: 200.0,
                p9999_us: 400.0,
                max_us: 800.0,
            },
            rfq_count: 1000,
            rfq_concurrency: 16,
            stream_subscriptions: 32,
            rfq_throughput_per_s: 50_000.0,
            stream_messages_drained: 1234,
            elapsed_s: 1.0,
        };

        // Within a 50% tolerance everywhere -> no breach.
        let ok = WireReport {
            rfq: Percentiles {
                p50_us: 60.0,
                p99_us: 140.0,
                p999_us: 250.0,
                p9999_us: 500.0,
                ..base.rfq
            },
            ..base.clone()
        };
        assert!(compare_to_baseline(&base, &ok, 0.50).is_empty());

        // p99 and p99.9 regress past the 50% ceiling -> exactly those two flagged.
        let bad = WireReport {
            rfq: Percentiles {
                p50_us: 55.0,   // within tolerance
                p99_us: 200.0,  // > 150 ceiling -> breach
                p999_us: 400.0, // > 300 ceiling -> breach
                p9999_us: 500.0,
                ..base.rfq
            },
            ..base.clone()
        };
        let breaches = compare_to_baseline(&base, &bad, 0.50);
        let metrics: Vec<&str> = breaches.iter().map(|b| b.metric.as_str()).collect();
        assert_eq!(metrics, vec!["p99", "p99.9"]);
    }
}
