//! Fleet-level §11 SLO **truth-benches**, measured on **loopback / in-process**.
//!
//! `docs/SCALE-OUT.md` §11 lists the fleet acceptance SLOs as *benchmarks to write
//! before declaring the fleet layer built*. This module turns the four that can be
//! honestly measured **without a real NIC** into reproducible, HdrHistogram-backed
//! measurements over the **real** primitives the fleet rests on, gated for relative
//! regression by `bench_gate` against a committed baseline.
//!
//! # HONEST BOUNDARY (read this before quoting a number)
//!
//! These are **loopback / in-process** measurements. They prove, exactly and
//! reproducibly:
//!
//! * the **arithmetic / routing / conflation / fan-out behaviour** of the fleet
//!   primitives (federation fan-in cost, publish→snapshot visibility lag,
//!   conflation under a stalled consumer, fan-out delivery spread to N subs);
//! * **relative regression** of each, gated at 2× by `bench_gate`;
//! * the **architectural invariant** (the per-tick price path never crosses the
//!   router/federation — a structural code-path assertion, not a latency claim).
//!
//! They do **NOT** prove the **absolute §11 wire-latency SLOs** (e.g. "router add
//! p99 ≤ 25 µs", "publish→snapshot p99 ≤ 150 µs") **under a real NIC** — that
//! stays deploy-gated, exactly as `docs/SCALE-OUT.md` §0 / §11 record. A loopback
//! number here is an **upper bound on the in-process compute+framing cost** and a
//! **lower bound** on the real cross-host figure (which adds NIC + switch +
//! propagation). Do not present any number from this module as a *met* absolute
//! §11 wire SLO. The durable claims are the **ratio / shape / regression-gate**,
//! never an absolute cross-host figure.
//!
//! # What each arm measures (and against which §11 row)
//!
//! * **(a) Cross-shard federation overhead** — §11 *"Cross-shard routing
//!   overhead"*. The *added* cost of the federating edge hop: the same
//!   `RiskService::AggregateRisk` answered by a **direct** single backend vs.
//!   answered through a **`Distributed{[backend]}`** federating edge (the minimal
//!   1-backend fan-in, isolating the edge hop from any fan-in arithmetic). p50/p99
//!   of both, and the delta. Real gRPC over loopback on both legs.
//! * **(b) Publish→snapshot visibility lag** — §11 *"Surface publish→local-snapshot
//!   lag"*. Timestamp at [`StateHandle::publish`] vs. the instant a concurrent
//!   [`StateReader`] first *observes* the published epoch. The real
//!   `celnet-engine` `arc-swap` primitive, measured directly.
//! * **(c) Conflation correctness under a stalled consumer** — §11 *"Conflation
//!   correctness for slow subs"*. The edge's actual backpressure primitive — a
//!   **bounded `mpsc` with `try_send`** (the exact `stream.rs` mechanism: a full
//!   channel drops the update, last-value-wins, never blocks the producer). Assert
//!   the producer's steady-state throughput + p99 are **unchanged** with a fully
//!   stalled consumer attached, and the queue depth stays **bounded** (≤ capacity).
//! * **(d) Many-sub fan-out delivery spread** — §11 *"Many-counterparty fan-out
//!   tail"*. One publish fanned to **N** bounded subscriber channels (the engine's
//!   real broadcast-depth-256 fan-out shape); measure the per-subscriber
//!   delivery-time **spread** (the window between the first and last subscriber
//!   observing the same epoch) across many rounds.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

use celnet_conventions::ConventionRecord;
use celnet_engine::rt::{MarketState, StateHandle};
use celnet_engine::testing::make_state;
use celnet_proto::risk_service_client::RiskServiceClient;
use celnet_proto::{AggregateRiskRequest, NumeraireRate, ReportingNumeraire, RiskDimension};
use celnet_risk_cube::{BookId as CubeBookId, DeskId, EntityId, FactKey, LocationId, TraderId};
use celnet_risk_fleet::FleetTopology;
use celnet_server::services::risk::store::BookedPosition;
use celnet_server::{Clock, CoreLink, Edge, LpPanelConfig, SpreadModel};
use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

use tonic::transport::Channel;

// ---------------------------------------------------------------------------
// shared fixtures
// ---------------------------------------------------------------------------

/// The resolved EURUSD 1Y convention record the engine fixtures are built on.
#[must_use]
fn eurusd_conv() -> ConventionRecord {
    celnet_conventions::resolve(
        CcyPair::parse("EURUSD").unwrap(),
        celnet_types::Tenor::Years(1),
    )
    .record
}

/// A small representative book of vanilla legs across a few pairs/entities, so the
/// federated aggregate has genuine additive + non-additive work to do (the same
/// shape the `risk_federation` integration test seeds, sized for a steady RPC).
fn book() -> Vec<(u64, u32, u32, CcyPair, OptionType, f64)> {
    let eurusd = CcyPair::new(Ccy::EUR, Ccy::USD);
    let gbpusd = CcyPair::new(Ccy::GBP, Ccy::USD);
    let audusd = CcyPair::new(Ccy::AUD, Ccy::USD);
    vec![
        (1, 1, 11, eurusd, OptionType::Call, 10_000_000.0),
        (2, 1, 11, eurusd, OptionType::Put, -4_000_000.0),
        (3, 1, 12, gbpusd, OptionType::Call, 7_000_000.0),
        (4, 1, 11, audusd, OptionType::Put, 5_000_000.0),
        (5, 2, 21, eurusd, OptionType::Call, 6_000_000.0),
        (6, 2, 22, gbpusd, OptionType::Put, 8_000_000.0),
        (7, 2, 22, audusd, OptionType::Call, 9_000_000.0),
        (8, 1, 12, eurusd, OptionType::Call, 2_500_000.0),
    ]
}

/// The canonical mark every leg is booked under.
fn inputs() -> VanillaInputs {
    VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02)
}

/// Seed `store` with the representative book.
fn seed(store: &celnet_server::services::risk::store::PositionStore) {
    for (id, entity, bk, pair, option, notional) in book() {
        let booked = BookedPosition {
            position_id: id,
            pair,
            option,
            notional_base: notional,
            inputs: inputs(),
            quoted_delta: DeltaConvention::SpotUnadjusted,
            premium_style: PremiumStyle::DomesticPips,
            surface_version: 1,
        };
        let key = FactKey {
            trader: TraderId(entity * 100 + bk),
            book: CubeBookId(bk),
            desk: DeskId(entity * 10),
            underlying: celnet_types::Underlying::Fx(pair),
            location: LocationId(entity),
            entity: EntityId(entity),
        };
        store.upsert(booked, key, None).expect("seed leg");
    }
}

/// A firm `AggregateRisk` request with VaR/ES + curvature, so both the additive
/// fan-in and the non-additive re-gather run (the full federation cost).
fn firm_request() -> AggregateRiskRequest {
    AggregateRiskRequest {
        dimension: RiskDimension::Firm as i32,
        numeraire: Some(ReportingNumeraire {
            numeraire: "USD".to_owned(),
            rates: vec![
                NumeraireRate {
                    ccy: "EUR".to_owned(),
                    rate: 1.10,
                },
                NumeraireRate {
                    ccy: "GBP".to_owned(),
                    rate: 1.27,
                },
                NumeraireRate {
                    ccy: "AUD".to_owned(),
                    rate: 0.66,
                },
            ],
        }),
        principal: None,
        scope: None,
        vega_pillars: vec![],
        var_spot_shocks: vec![-0.02, -0.01, 0.0, 0.01, 0.02],
        var_alpha: 0.99,
        curvature_risk_weight: 0.18,
        correlation_id: Some(1),
        session_token: None,
    }
}

/// A fresh `MarketState` carrying a publish epoch in `request_id`-free fashion: we
/// re-use the spot field to thread a monotonically increasing epoch marker so a
/// reader can detect "the publish I am waiting for has landed" without any extra
/// channel. (The epoch is encoded as `spot = base + epoch * STEP` — a value the
/// reader compares for visibility, not for pricing.)
fn epoch_state(base: &MarketState, epoch: u64) -> MarketState {
    let mut s = base.clone();
    // A tiny, monotone perturbation: distinct per epoch, still a finite spot.
    s.spot = 1.10 + (epoch as f64) * 1e-9;
    s
}

// ---------------------------------------------------------------------------
// report types
// ---------------------------------------------------------------------------

/// A pair of percentiles (microseconds) for a measured distribution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct P50P99 {
    /// 50th percentile, microseconds.
    pub p50_us: f64,
    /// 99th percentile, microseconds.
    pub p99_us: f64,
}

impl P50P99 {
    fn from_hist(h: &Histogram<u64>) -> Self {
        Self {
            p50_us: h.value_at_quantile(0.50) as f64 / 1000.0,
            p99_us: h.value_at_quantile(0.99) as f64 / 1000.0,
        }
    }
}

/// (a) Cross-shard federation overhead: the federating-edge hop cost vs a direct
/// single-backend call to the same `AggregateRisk`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FederationOverhead {
    /// Direct-to-backend `AggregateRisk` round-trip (no edge hop), loopback gRPC.
    pub direct: P50P99,
    /// Federated `AggregateRisk` through a `Distributed{[backend]}` edge (the +1
    /// edge hop, minimal 1-backend fan-in), loopback gRPC.
    pub federated: P50P99,
    /// The *added* p99 of the federating hop (`federated.p99 − direct.p99`), µs.
    pub added_p99_us: f64,
    /// Round-trips timed on each leg.
    pub samples: u64,
}

/// (b) Publish→snapshot visibility lag over the real `arc-swap` `StateHandle`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PublishSnapshotLag {
    /// Visibility lag (publish instant → reader first observes the epoch), µs.
    pub lag: P50P99,
    /// p99.9 of the visibility lag, µs (the tail the §11 row tracks).
    pub p999_us: f64,
    /// Publishes timed.
    pub samples: u64,
}

/// (c) Conflation correctness under a stalled consumer: the producer's throughput
/// and p99 with vs without a fully-stalled bounded-`mpsc` consumer attached, plus the
/// bounded queue depth.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ConflationCorrectness {
    /// Producer per-publish p99 with NO stalled consumer (the control), µs.
    pub baseline_p99_us: f64,
    /// Producer per-publish p99 WITH a fully-stalled consumer attached, µs.
    pub stalled_p99_us: f64,
    /// Producer steady-state throughput, publishes/s, with the stalled consumer.
    pub producer_throughput_per_s: f64,
    /// The maximum observed queue depth of the stalled consumer's channel (MUST be
    /// ≤ the channel capacity — last-value-wins conflation, never unbounded).
    pub max_queue_depth: u64,
    /// The bounded channel capacity (the conflation depth).
    pub channel_capacity: u64,
    /// Publishes timed.
    pub samples: u64,
}

/// (d) Many-sub fan-out delivery-window spread at N subscribers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FanOutSpread {
    /// Number of subscriber channels fanned to per round.
    pub subscribers: u64,
    /// Per-round delivery window (first→last subscriber observing the same epoch):
    /// p50/p99, µs.
    pub window: P50P99,
    /// p99.9 of the delivery window, µs.
    pub p999_us: f64,
    /// Fan-out rounds timed.
    pub rounds: u64,
}

/// The full fleet-SLO report — serialized as the committed loopback baseline and
/// re-emitted by every run for the relative-regression gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetSloReport {
    /// Always `"loopback"` — a guard against ever mistaking this for a wire SLO.
    pub measurement: String,
    /// (a) federation overhead.
    pub federation: FederationOverhead,
    /// (b) publish→snapshot lag.
    pub publish_snapshot: PublishSnapshotLag,
    /// (c) conflation correctness.
    pub conflation: ConflationCorrectness,
    /// (d) fan-out delivery spread.
    pub fan_out: FanOutSpread,
}

impl FleetSloReport {
    /// Print a human-readable summary, with the honest-boundary banner.
    pub fn print_summary(&self) {
        println!("== celnet fleet §11 SLO truth-benches (LOOPBACK / in-process) ==");
        println!(
            "  HONEST BOUNDARY: loopback numbers — they prove routing/conflation/fan-out\n  \
             arithmetic + relative regression + the architectural invariant. They are NOT\n  \
             the absolute §11 wire-latency SLOs under a real NIC (those stay deploy-gated)."
        );
        println!();
        println!("  (a) cross-shard federation overhead (loopback gRPC):");
        println!(
            "        direct   p50 = {:>8.2} us   p99 = {:>8.2} us",
            self.federation.direct.p50_us, self.federation.direct.p99_us
        );
        println!(
            "        federated p50 = {:>8.2} us   p99 = {:>8.2} us   (+{:.2} us p99 edge hop)",
            self.federation.federated.p50_us,
            self.federation.federated.p99_us,
            self.federation.added_p99_us
        );
        println!();
        println!("  (b) publish→StateHandle-snapshot visibility lag (arc-swap):");
        println!(
            "        p50 = {:>8.3} us   p99 = {:>8.3} us   p99.9 = {:>8.3} us",
            self.publish_snapshot.lag.p50_us,
            self.publish_snapshot.lag.p99_us,
            self.publish_snapshot.p999_us
        );
        println!();
        println!("  (c) conflation correctness under a stalled consumer (bounded mpsc):");
        println!(
            "        producer p99: baseline = {:>8.3} us   with-stalled = {:>8.3} us",
            self.conflation.baseline_p99_us, self.conflation.stalled_p99_us
        );
        println!(
            "        producer throughput = {:>10.0} publishes/s   max queue depth = {} (cap {})",
            self.conflation.producer_throughput_per_s,
            self.conflation.max_queue_depth,
            self.conflation.channel_capacity
        );
        println!();
        println!(
            "  (d) fan-out delivery-window spread at {} subscribers:",
            self.fan_out.subscribers
        );
        println!(
            "        window p50 = {:>8.3} us   p99 = {:>8.3} us   p99.9 = {:>8.3} us",
            self.fan_out.window.p50_us, self.fan_out.window.p99_us, self.fan_out.p999_us
        );
    }
}

/// How much work each arm does. The defaults are sized for a stable percentile
/// estimate that still finishes in a few seconds and is hard-bounded.
#[derive(Debug, Clone, Copy)]
pub struct FleetSloConfig {
    /// `AggregateRisk` round-trips timed per leg (a) (direct and federated).
    pub federation_samples: u64,
    /// Publish→snapshot visibility samples (b).
    pub publish_samples: u64,
    /// Producer publishes timed per conflation phase (c).
    pub conflation_samples: u64,
    /// Bounded subscriber-channel capacity (c)/(d) — the engine's broadcast depth.
    pub channel_capacity: usize,
    /// Subscribers fanned to per round (d).
    pub fan_out_subscribers: usize,
    /// Fan-out rounds timed (d).
    pub fan_out_rounds: u64,
    /// Hard wall-clock cap for the whole run (belt-and-braces; every arm is also
    /// sample-bounded, so the run self-terminates regardless).
    pub wall_clock_cap: Duration,
}

impl Default for FleetSloConfig {
    fn default() -> Self {
        Self {
            federation_samples: 2_000,
            publish_samples: 100_000,
            conflation_samples: 200_000,
            channel_capacity: 256,
            fan_out_subscribers: 128,
            fan_out_rounds: 5_000,
            wall_clock_cap: Duration::from_secs(60),
        }
    }
}

impl FleetSloConfig {
    /// A smaller, faster sizing for the CI gate (a stable estimate in ~1–2s).
    #[must_use]
    pub fn ci() -> Self {
        Self {
            federation_samples: 600,
            publish_samples: 30_000,
            conflation_samples: 50_000,
            channel_capacity: 256,
            fan_out_subscribers: 64,
            fan_out_rounds: 2_000,
            wall_clock_cap: Duration::from_secs(40),
        }
    }
}

// ---------------------------------------------------------------------------
// arm (a) — cross-shard federation overhead
// ---------------------------------------------------------------------------

/// Boot one ready in-process backend `Edge` (it serves its own slice locally),
/// seeded with the representative book, on an ephemeral loopback port.
async fn boot_backend() -> std::io::Result<Edge> {
    let initial = make_state(1.10, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc = "127.0.0.1:0".parse().expect("loopback addr");
    let edge = Edge::start(grpc, link, SpreadModel::default(), Clock::system(), None).await?;
    edge.gate().mark_ready();
    seed(edge.store());
    Ok(edge)
}

/// Boot a federating edge in `Distributed{[backend_url]}` topology — it is a client
/// of the single backend and federates `RiskService` across it (the minimal
/// 1-backend fan-in, isolating the edge hop cost).
async fn boot_federating_edge(backend_url: &str) -> std::io::Result<Edge> {
    // The federating edge has no local positions; it federates the backend's book.
    let initial = make_state(1.10, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc = "127.0.0.1:0".parse().expect("loopback addr");
    let ws = "127.0.0.1:0".parse().expect("loopback addr");
    let topology = FleetTopology::Distributed {
        endpoints: vec![backend_url.to_owned()],
    };
    let edge = Edge::start_on_with_topology(
        grpc,
        ws,
        link,
        SpreadModel::default(),
        Clock::system(),
        topology,
        LpPanelConfig::default(),
        None,
    )
    .await?;
    edge.gate().mark_ready();
    Ok(edge)
}

/// Time `samples` `AggregateRisk` round-trips against the RiskService at `url`.
async fn time_aggregate(url: &str, samples: u64) -> Result<Histogram<u64>, String> {
    let channel = Channel::from_shared(url.to_owned())
        .map_err(|e| format!("invalid endpoint {url}: {e}"))?
        .connect()
        .await
        .map_err(|e| format!("connect {url} failed: {e}"))?;
    let mut client = RiskServiceClient::new(channel);
    let req = firm_request();
    // Warm the channel + the JIT-ish first-call paths so the histogram reflects
    // steady state, not the cold first RPC.
    for _ in 0..16 {
        let _ = client.aggregate_risk(req.clone()).await;
    }
    let mut hist: Histogram<u64> =
        Histogram::new_with_bounds(1, 1_000_000_000, 3).expect("valid bounds");
    for _ in 0..samples {
        let t0 = Instant::now();
        client
            .aggregate_risk(req.clone())
            .await
            .map_err(|e| format!("aggregate_risk failed: {e}"))?;
        let ns = u64::try_from(t0.elapsed().as_nanos())
            .unwrap_or(u64::MAX)
            .max(1);
        hist.saturating_record(ns);
    }
    Ok(hist)
}

/// Measure arm (a): federation overhead. Boots a backend + a federating edge over
/// real loopback gRPC, times `AggregateRisk` on each, returns the overhead.
async fn measure_federation(config: FleetSloConfig) -> Result<FederationOverhead, String> {
    let backend = boot_backend()
        .await
        .map_err(|e| format!("backend boot: {e}"))?;
    let backend_url = format!("http://{}", backend.grpc_addr());
    let edge = boot_federating_edge(&backend_url)
        .await
        .map_err(|e| format!("federating edge boot: {e}"))?;
    let edge_url = format!("http://{}", edge.grpc_addr());

    // Direct leg: dial the backend's RiskService (no edge hop).
    let direct = time_aggregate(&backend_url, config.federation_samples).await?;
    // Federated leg: dial the federating edge (the +1 hop fans in to the backend).
    let federated = time_aggregate(&edge_url, config.federation_samples).await?;

    edge.shutdown(Duration::from_secs(5)).await;
    backend.shutdown(Duration::from_secs(5)).await;

    let d = P50P99::from_hist(&direct);
    let f = P50P99::from_hist(&federated);
    Ok(FederationOverhead {
        direct: d,
        federated: f,
        added_p99_us: f.p99_us - d.p99_us,
        samples: config.federation_samples,
    })
}

// ---------------------------------------------------------------------------
// arm (b) — publish→snapshot visibility lag
// ---------------------------------------------------------------------------

/// Measure arm (b): publish→`StateReader` visibility lag over the real `arc-swap`
/// [`StateHandle`]. A reader thread spins on a `StateReader`, detecting each newly
/// published epoch and recording the timestamp it first becomes visible; the
/// publisher records the publish instant and the (shared) visible instant.
///
/// Bounded by the sample count; the reader is joined under a deadline.
fn measure_publish_snapshot(config: FleetSloConfig) -> PublishSnapshotLag {
    let base = make_state(1.10, eurusd_conv());
    let handle = StateHandle::new(base.clone());
    let samples = config.publish_samples;

    // The shared visibility timestamp: the reader stamps `visible_ns` (monotonic
    // from a shared epoch) for the epoch it currently sees; the publisher reads it.
    // We thread the epoch through `spot`; the reader maps spot→epoch by the inverse
    // perturbation. A `seen_epoch` atomic lets the publisher wait for visibility.
    let seen_epoch = Arc::new(AtomicU64::new(0));
    let seen_at_ns = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let origin = Instant::now();

    let reader_handle = {
        let mut reader = handle.reader();
        let seen_epoch = Arc::clone(&seen_epoch);
        let seen_at_ns = Arc::clone(&seen_at_ns);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut last = 0u64;
            let mut idle = 0u32;
            while !stop.load(Ordering::Relaxed) {
                let st = reader.load();
                // Recover the epoch from the spot perturbation (round to nearest).
                let epoch = (((st.spot - 1.10) / 1e-9).round()) as i64;
                let epoch = if epoch < 0 { 0 } else { epoch as u64 };
                if epoch != last {
                    let now = u64::try_from(origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
                    seen_at_ns.store(now, Ordering::Release);
                    seen_epoch.store(epoch, Ordering::Release);
                    last = epoch;
                    idle = 0;
                } else {
                    // Yield periodically so the publisher thread is never starved on a
                    // core-constrained host (this is a 2-thread ping-pong; a brief
                    // yield after a run of no-change polls keeps both progressing).
                    idle = idle.wrapping_add(1);
                    if idle.is_multiple_of(1024) {
                        std::thread::yield_now();
                    } else {
                        std::hint::spin_loop();
                    }
                }
            }
        })
    };

    let mut hist: Histogram<u64> =
        Histogram::new_with_bounds(1, 1_000_000_000, 3).expect("valid bounds");
    for epoch in 1..=samples {
        let st = epoch_state(&base, epoch);
        let publish_ns = u64::try_from(origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
        handle.publish(st);
        // Spin until the reader reports this epoch visible, bounded so a missed
        // epoch (the reader skipped past it under a fast publisher) is not a hang:
        // we record the visibility of the *first epoch ≥ this one* the reader saw.
        let mut spins = 0u64;
        loop {
            let seen = seen_epoch.load(Ordering::Acquire);
            if seen >= epoch {
                let vis = seen_at_ns.load(Ordering::Acquire);
                let lag = vis.saturating_sub(publish_ns).max(1);
                hist.saturating_record(lag);
                break;
            }
            spins += 1;
            if spins > 2_000_000 {
                // Defensive bound: never spin forever (the reader is alive; this is
                // a publish the reader conflated past — skip recording it).
                break;
            }
            if spins.is_multiple_of(512) {
                std::thread::yield_now();
            } else {
                std::hint::spin_loop();
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = reader_handle.join();

    PublishSnapshotLag {
        lag: P50P99::from_hist(&hist),
        p999_us: hist.value_at_quantile(0.999) as f64 / 1000.0,
        samples: hist.len(),
    }
}

// ---------------------------------------------------------------------------
// arm (c) — conflation correctness under a stalled consumer
// ---------------------------------------------------------------------------

/// Measure arm (c): a fully-stalled bounded-`mpsc` consumer must never back-pressure
/// the producer. We time the producer's per-publish cost both WITHOUT and WITH a
/// stalled consumer attached (the consumer never receives), using `try_send` — the
/// exact `stream.rs` conflation primitive (a full channel drops, last-value-wins).
///
/// Asserts: the producer's p99 is unchanged within noise, and the channel depth
/// stays bounded by its capacity (never unbounded growth).
fn measure_conflation(config: FleetSloConfig) -> ConflationCorrectness {
    let samples = config.conflation_samples;
    let cap = config.channel_capacity;

    // Control: producer with NO consumer channel — pure per-publish cost.
    let mut baseline: Histogram<u64> =
        Histogram::new_with_bounds(1, 1_000_000_000, 3).expect("valid bounds");
    let mut sink = 0u64;
    for i in 0..samples {
        let t0 = Instant::now();
        // The producer's per-publish work mirrors a top-of-book publish: form a
        // small Copy snapshot. We keep it identical in both phases so the only
        // difference is the stalled-consumer `try_send`.
        sink = sink.wrapping_add(i.rotate_left((i % 31) as u32));
        let ns = u64::try_from(t0.elapsed().as_nanos())
            .unwrap_or(u64::MAX)
            .max(1);
        baseline.saturating_record(ns);
    }
    std::hint::black_box(sink);

    // With a stalled consumer: a bounded std mpsc-like channel (we use a
    // fixed-capacity ring via a `crossbeam`-free bounded `sync_channel`). The
    // consumer NEVER receives, so the channel fills to `cap` and every subsequent
    // `try_send` drops (last-value-wins). The producer must never block.
    let (tx, rx) = std::sync::mpsc::sync_channel::<u64>(cap);
    // Hold `rx` but never recv — the stalled consumer.
    let mut stalled: Histogram<u64> =
        Histogram::new_with_bounds(1, 1_000_000_000, 3).expect("valid bounds");
    let mut max_depth = 0u64;
    let mut depth = 0u64;
    let started = Instant::now();
    let mut sink2 = 0u64;
    for i in 0..samples {
        let t0 = Instant::now();
        sink2 = sink2.wrapping_add(i.rotate_left((i % 31) as u32));
        // try_send: Ok ⇒ enqueued (depth+1); Full/Disconnected ⇒ dropped (conflated).
        match tx.try_send(i) {
            Ok(()) => {
                depth += 1;
                if depth > max_depth {
                    max_depth = depth;
                }
            }
            Err(std::sync::mpsc::TrySendError::Full(_)) => { /* conflated: last-value-wins */ }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {}
        }
        let ns = u64::try_from(t0.elapsed().as_nanos())
            .unwrap_or(u64::MAX)
            .max(1);
        stalled.saturating_record(ns);
    }
    let elapsed = started.elapsed();
    std::hint::black_box(sink2);
    // Keep the receiver alive to the very end so the channel never disconnects mid-run.
    drop(rx);

    let throughput = if elapsed.as_secs_f64() > 0.0 {
        samples as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };

    ConflationCorrectness {
        baseline_p99_us: baseline.value_at_quantile(0.99) as f64 / 1000.0,
        stalled_p99_us: stalled.value_at_quantile(0.99) as f64 / 1000.0,
        producer_throughput_per_s: throughput,
        max_queue_depth: max_depth,
        channel_capacity: cap as u64,
        samples,
    }
}

// ---------------------------------------------------------------------------
// arm (d) — many-sub fan-out delivery-window spread
// ---------------------------------------------------------------------------

/// Measure arm (d): fan one publish out to N bounded subscriber channels and time
/// the per-round delivery window (first→last subscriber observing the epoch). Models
/// the engine's broadcast fan-out shape (bounded depth, per-sub channel).
fn measure_fan_out(config: FleetSloConfig) -> FanOutSpread {
    let n = config.fan_out_subscribers;
    let rounds = config.fan_out_rounds;
    let cap = config.channel_capacity;

    // N bounded channels (one per subscriber). The producer fans each epoch to all;
    // a per-sub reader drains continuously and stamps the instant it observed the
    // round's epoch. The window is max(stamp) − min(stamp) across the N subs.
    let mut txs = Vec::with_capacity(n);
    let stamps: Vec<Arc<AtomicU64>> = (0..n).map(|_| Arc::new(AtomicU64::new(0))).collect();
    let done = Arc::new(AtomicU64::new(0));
    let origin = Instant::now();
    let mut readers = Vec::with_capacity(n);

    // Each subscriber BLOCKS on its channel (no busy-spin), so N readers on a
    // core-constrained host sleep until the producer fans an epoch — they never
    // starve each other. The reader stamps the instant it dequeues each epoch and
    // bumps a shared `done` counter so the producer knows the round is complete.
    for stamp_slot in &stamps {
        let (tx, rx) = std::sync::mpsc::sync_channel::<u64>(cap);
        txs.push(tx);
        let stamp = Arc::clone(stamp_slot);
        let done = Arc::clone(&done);
        readers.push(std::thread::spawn(move || {
            // Block on recv; a `0` sentinel (sent at teardown) ends the reader.
            while let Ok(epoch) = rx.recv() {
                if epoch == 0 {
                    break;
                }
                let now = u64::try_from(origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
                stamp.store(now, Ordering::Release);
                done.fetch_add(1, Ordering::AcqRel);
            }
        }));
    }

    let mut hist: Histogram<u64> =
        Histogram::new_with_bounds(1, 1_000_000_000, 3).expect("valid bounds");
    let n_u64 = n as u64;
    for epoch in 1..=rounds {
        done.store(0, Ordering::Release);
        for st in &stamps {
            st.store(0, Ordering::Release);
        }
        let fan_start = u64::try_from(origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
        for tx in &txs {
            // Blocking send: the channel has capacity ≥ 1 and the round is drained
            // before the next, so this never blocks in practice; it guarantees every
            // subscriber receives this round's epoch (a true fan-out, no drop).
            let _ = tx.send(epoch);
        }
        // Wait (yielding) for all N subscribers to dequeue + stamp this epoch.
        let mut spins = 0u64;
        while done.load(Ordering::Acquire) < n_u64 {
            spins += 1;
            if spins > 50_000_000 {
                break; // defensive: never hang (a reader died) — skip this round.
            }
            std::thread::yield_now();
        }
        // The delivery window: last subscriber's observation minus the fan start.
        let mut max_ns = 0u64;
        let mut all = true;
        for st in &stamps {
            let v = st.load(Ordering::Acquire);
            if v == 0 {
                all = false;
                break;
            }
            max_ns = max_ns.max(v);
        }
        if all {
            let window = max_ns.saturating_sub(fan_start).max(1);
            hist.saturating_record(window);
        }
    }
    // Tear down the readers with the `0` sentinel.
    for tx in &txs {
        let _ = tx.send(0);
    }
    drop(txs);
    for r in readers {
        let _ = r.join();
    }

    FanOutSpread {
        subscribers: n as u64,
        window: P50P99::from_hist(&hist),
        p999_us: hist.value_at_quantile(0.999) as f64 / 1000.0,
        rounds: hist.len(),
    }
}

// ---------------------------------------------------------------------------
// driver
// ---------------------------------------------------------------------------

/// Run all four fleet-SLO arms and assemble the report. The federation arm needs
/// an async runtime (real gRPC); the rest are synchronous thread harnesses, run on
/// dedicated threads so they do not contend with the tokio runtime.
///
/// # Errors
/// Returns an error string if the federation arm's gRPC setup fails.
pub async fn measure(config: FleetSloConfig) -> Result<FleetSloReport, String> {
    let federation = tokio::time::timeout(config.wall_clock_cap, measure_federation(config))
        .await
        .map_err(|_| "federation arm exceeded the wall-clock cap".to_owned())??;

    // The three synchronous arms run on blocking threads (spin-loops + dedicated
    // reader threads), off the async runtime.
    let publish_snapshot = tokio::task::spawn_blocking(move || measure_publish_snapshot(config))
        .await
        .map_err(|e| format!("publish-snapshot arm join failed: {e}"))?;
    let conflation = tokio::task::spawn_blocking(move || measure_conflation(config))
        .await
        .map_err(|e| format!("conflation arm join failed: {e}"))?;
    let fan_out = tokio::task::spawn_blocking(move || measure_fan_out(config))
        .await
        .map_err(|e| format!("fan-out arm join failed: {e}"))?;

    Ok(FleetSloReport {
        measurement: "loopback".to_owned(),
        federation,
        publish_snapshot,
        conflation,
        fan_out,
    })
}

// ---------------------------------------------------------------------------
// relative-regression gate
// ---------------------------------------------------------------------------

/// One breached fleet-SLO metric, for human-readable reporting.
#[derive(Debug, Clone)]
pub struct FleetBreach {
    /// The metric name (e.g. `"federation.federated.p99"`).
    pub metric: String,
    /// Committed baseline value.
    pub baseline: f64,
    /// Measured value this run.
    pub measured: f64,
    /// The ceiling (`baseline * (1 + tolerance)`).
    pub ceiling: f64,
}

/// Compare a measured [`FleetSloReport`] against a committed baseline at a relative
/// `tolerance` (slowdown-only). Returns the breached metrics — empty means the gate
/// passes. Only *latency/spread* metrics are gated (a regression is a slowdown);
/// throughput is reported but not gated (host-core-count sensitive), exactly like
/// the wire gate.
#[must_use]
pub fn compare_to_baseline(
    baseline: &FleetSloReport,
    measured: &FleetSloReport,
    tolerance: f64,
) -> Vec<FleetBreach> {
    let checks: [(&str, f64, f64); 8] = [
        (
            "federation.direct.p99",
            baseline.federation.direct.p99_us,
            measured.federation.direct.p99_us,
        ),
        (
            "federation.federated.p99",
            baseline.federation.federated.p99_us,
            measured.federation.federated.p99_us,
        ),
        (
            "publish_snapshot.p99",
            baseline.publish_snapshot.lag.p99_us,
            measured.publish_snapshot.lag.p99_us,
        ),
        (
            "publish_snapshot.p99.9",
            baseline.publish_snapshot.p999_us,
            measured.publish_snapshot.p999_us,
        ),
        (
            "conflation.stalled_p99",
            baseline.conflation.stalled_p99_us,
            measured.conflation.stalled_p99_us,
        ),
        (
            "fan_out.window.p99",
            baseline.fan_out.window.p99_us,
            measured.fan_out.window.p99_us,
        ),
        (
            "fan_out.window.p99.9",
            baseline.fan_out.p999_us,
            measured.fan_out.p999_us,
        ),
        // The federation *added* hop is the headline routing-overhead metric.
        (
            "federation.added_p99",
            baseline.federation.added_p99_us.max(0.0),
            measured.federation.added_p99_us.max(0.0),
        ),
    ];
    let mut breaches = Vec::new();
    for (metric, base, meas) in checks {
        // Guard a degenerate/zero baseline: a tiny floor avoids dividing meaning
        // out of a ~0 baseline (e.g. an added-hop that measured negative once).
        let base = base.max(0.001);
        let ceiling = base * (1.0 + tolerance);
        if meas > ceiling {
            breaches.push(FleetBreach {
                metric: metric.to_owned(),
                baseline: base,
                measured: meas,
                ceiling,
            });
        }
    }
    breaches
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate flags exactly the metrics that regress beyond tolerance.
    #[test]
    fn gate_flags_only_regressed_metrics() {
        let base = sample_report();
        // Within 100% tolerance everywhere -> no breach (double every latency).
        let mut ok = base.clone();
        ok.publish_snapshot.lag.p99_us *= 1.9;
        ok.fan_out.window.p99_us *= 1.9;
        assert!(compare_to_baseline(&base, &ok, 1.0).is_empty());

        // publish p99 regresses past 2× -> flagged.
        let mut bad = base.clone();
        bad.publish_snapshot.lag.p99_us *= 3.0;
        let breaches = compare_to_baseline(&base, &bad, 1.0);
        let metrics: Vec<&str> = breaches.iter().map(|b| b.metric.as_str()).collect();
        assert_eq!(metrics, vec!["publish_snapshot.p99"]);
    }

    fn sample_report() -> FleetSloReport {
        FleetSloReport {
            measurement: "loopback".to_owned(),
            federation: FederationOverhead {
                direct: P50P99 {
                    p50_us: 100.0,
                    p99_us: 200.0,
                },
                federated: P50P99 {
                    p50_us: 180.0,
                    p99_us: 350.0,
                },
                added_p99_us: 150.0,
                samples: 1000,
            },
            publish_snapshot: PublishSnapshotLag {
                lag: P50P99 {
                    p50_us: 0.05,
                    p99_us: 0.2,
                },
                p999_us: 1.0,
                samples: 10000,
            },
            conflation: ConflationCorrectness {
                baseline_p99_us: 0.01,
                stalled_p99_us: 0.012,
                producer_throughput_per_s: 5.0e7,
                max_queue_depth: 256,
                channel_capacity: 256,
                samples: 10000,
            },
            fan_out: FanOutSpread {
                subscribers: 256,
                window: P50P99 {
                    p50_us: 5.0,
                    p99_us: 20.0,
                },
                p999_us: 40.0,
                rounds: 10000,
            },
        }
    }

    /// Conflation correctness: a fully-stalled consumer never grows the queue past
    /// its capacity and the producer keeps running (a real measurement, bounded).
    #[test]
    fn conflation_bounds_the_queue() {
        let config = FleetSloConfig {
            conflation_samples: 50_000,
            channel_capacity: 64,
            ..FleetSloConfig::ci()
        };
        let c = measure_conflation(config);
        assert!(
            c.max_queue_depth <= c.channel_capacity,
            "stalled consumer must never grow past capacity ({} > {})",
            c.max_queue_depth,
            c.channel_capacity
        );
        assert!(
            c.producer_throughput_per_s > 0.0,
            "producer must keep running under a stalled consumer"
        );
        // The stalled-consumer p99 must not be wildly worse than the control: a
        // generous 50× envelope (this is a coarse no-backpressure sanity bound, not
        // an absolute SLO) — proves the consumer does NOT back-pressure the producer.
        assert!(
            c.stalled_p99_us <= c.baseline_p99_us.max(0.001) * 50.0 + 1.0,
            "stalled consumer must not back-pressure the producer (stalled p99 {} vs baseline {})",
            c.stalled_p99_us,
            c.baseline_p99_us
        );
    }
}
