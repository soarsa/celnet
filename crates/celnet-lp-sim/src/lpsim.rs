//! The **LP-SIM feed assembly** — the glue that turns the loaded Treasury universe
//! into a running, consolidating liquidity-provider feed named `LP-SIM`.
//!
//! This is the shared core behind both the `lp-sim` runnable binary and the
//! Treasury aggregation integration test: it builds a panel of decorrelated
//! [`SimLp`]s (the `LP-SIM` connection and, optionally, sibling members
//! `LP-SIM-01`…) that each stream a stochastic two-way for every selected
//! [`TreasuryBond`], then consolidates the panel per bond through the REAL
//! [`celnet_aggregation::ConsolidatedBook`] engine — the same engine an
//! operator-defined FI Aggregated Book uses server-side.
//!
//! The per-bond mid is the reference-seeded mean-reverting yield priced through the
//! `celnet-bond` analytics leaf (see [`TreasuryBond::yield_model`]), so the prices
//! are oracle-anchored, and each member leans/decorrelates so the consolidated
//! best-bid/offer and per-LP contributions are non-trivial.

use celnet_aggregation::{
    ConsolidatedBook, ConsolidationConfig, ConsolidationError, VenueId, VenueQuote,
};
use celnet_types::BrokenDate;

use crate::lp::{InstrumentModel, LpParams, SimLp};
use crate::price::MidSource;
use crate::quoted::QuotedLine;
use crate::rng::{child_seed, seeded_unit, unit01};
use crate::universe::TreasuryBond;

/// The default LP connection name the feed advertises — the venue id a subscriber
/// sees as the price contributor.
pub const DEFAULT_LP_NAME: &str = "LP-SIM";

/// How the `LP-SIM` feed is assembled: the LP identity, the number of decorrelated
/// member connections, the stochastic-yield knobs, the quoting shape, and the
/// consolidation tuning.
#[derive(Debug, Clone)]
pub struct LpSimConfig {
    /// The base LP connection name (venue id). With one member the venue is exactly
    /// this; with N > 1 the members are `"{lp_name}-01"`, `"{lp_name}-02"`, ….
    pub lp_name: String,
    /// Number of decorrelated LP member connections feeding the book (`≥ 1`). More
    /// members ⇒ a richer composite (median-consensus gating needs ≥ 3 to be
    /// decidable).
    pub members: usize,
    /// The root seed; the same `(seed, config, universe)` yields a byte-identical
    /// feed.
    pub seed: u64,
    /// The settlement/valuation date used to build each bond's reference schedule
    /// and invert its reference yield.
    pub settlement: BrokenDate,
    /// Mean-reversion speed `κ` per second of the yield process.
    pub reversion_per_sec: f64,
    /// Yield jitter amplitude `σ` (decimal yield), e.g. `2e-4` = ±2 bp.
    pub perturbation: f64,
    /// The half-spread each member quotes around its (skewed) mid, in price points
    /// per 100 face.
    pub half_spread: f64,
    /// The firm size each member shows on both sides (face units).
    pub size: f64,
    /// The per-member increment of directional skew (price points): the panel is
    /// centred so the middle member is unbiased and the ends lean opposite ways.
    pub skew_step: f64,
    /// The per-member dispersion of the initial yield (decimal yield), decorrelating
    /// the members' starting mids.
    pub yield_dispersion: f64,
    /// Consolidation staleness half-life in seconds.
    pub tau_secs: f64,
    /// Consolidation hard staleness cutoff in seconds.
    pub cutoff_secs: f64,
    /// Consolidation divergence tolerance in price points per 100 face.
    pub divergence_tolerance: f64,
}

impl Default for LpSimConfig {
    fn default() -> Self {
        Self {
            lp_name: DEFAULT_LP_NAME.to_string(),
            members: 4,
            seed: 0x1234_5678,
            settlement: BrokenDate::new(2026, 4, 16),
            reversion_per_sec: 0.02,
            perturbation: 3.0e-4,
            half_spread: 2.0e-2, // ~2 bp of a 100.0 price handle
            size: 1_000_000.0,
            // Per-member skew + starting-yield dispersion kept to realistic
            // competing-dealer levels (~sub-bp to ~1.5 bp of yield). Wider values
            // push a *fresh* member's mid past the absolute divergence tolerance on
            // long-duration bonds (dev ≈ duration × Δy × price), which would falsely
            // gate a legitimate quote — the divergence gate is for outliers, not for
            // normal cross-dealer spread. See ConsolidationConfig::divergence_tolerance.
            skew_step: 4.0e-3,
            yield_dispersion: 1.5e-4,
            tau_secs: 30.0,
            cutoff_secs: 60.0,
            divergence_tolerance: 0.50,
        }
    }
}

impl LpSimConfig {
    /// The consolidation config the engine consumes for this feed.
    #[must_use]
    pub fn consolidation(&self) -> ConsolidationConfig {
        ConsolidationConfig {
            staleness_half_life_secs: self.tau_secs.max(f64::MIN_POSITIVE),
            staleness_cutoff_secs: self.cutoff_secs,
            divergence_tolerance: self.divergence_tolerance,
        }
    }

    /// The venue id of member `i` (`0`-based). With a single member this is exactly
    /// [`lp_name`](Self::lp_name); otherwise it is suffixed `-01`, `-02`, … so every
    /// contribution is attributable to a named connection.
    #[must_use]
    pub fn member_venue(&self, i: usize) -> String {
        if self.members <= 1 {
            self.lp_name.clone()
        } else {
            format!("{}-{:02}", self.lp_name, i + 1)
        }
    }
}

/// The full set of [`QuotedLine`]s the sim can price for `cfg`: every cash bond in
/// `bonds` that models at the config's settlement, followed by every listed Treasury
/// futures contract anchored to that same cash curve.
///
/// This is the sim's **quotable** universe, and it is deliberately built from the
/// same `celnet-refdata` sources the server seeds its **tradeable** registry from —
/// an instrument that is tradeable but not quotable never reaches an aggregated book
/// (see the [`crate::quoted`] module docs).
#[must_use]
pub fn quotable_lines(cfg: &LpSimConfig, bonds: &[TreasuryBond]) -> Vec<QuotedLine> {
    let mut lines = crate::universe::bond_lines(
        bonds,
        cfg.settlement,
        cfg.half_spread,
        cfg.skew_step,
        cfg.reversion_per_sec,
        cfg.perturbation,
    );
    let contracts = crate::futures::load_futures_universe(bonds, cfg.settlement);
    lines.extend(crate::futures::futures_lines(
        &contracts,
        cfg.half_spread,
        cfg.skew_step,
        cfg.reversion_per_sec,
        cfg.perturbation,
    ));
    lines
}

/// Build the `LP-SIM` panel: one [`SimLp`] per member, each quoting every
/// [`QuotedLine`] it is given (a stochastic reference-seeded yield, priced through
/// the real analytics leaf). Reproducible for a fixed `(config, lines)`.
///
/// Each instrument carries its own market's quoting conventions
/// ([`QuotedLine::spread_scale`] / [`QuotedLine::lean_scale`] /
/// [`QuotedLine::yield_dispersion`] / [`QuotedLine::tick`]), so one panel quotes cash
/// bonds a couple of basis points wide and listed futures one or two minimum price
/// increments wide, on the exchange's grid.
#[must_use]
pub fn build_fleet(cfg: &LpSimConfig, lines: &[QuotedLine]) -> Vec<SimLp> {
    let n = cfg.members.max(1);
    let centre = (n as f64 - 1.0) / 2.0;
    (0..n)
        .map(|i| {
            let venue = VenueId::new(cfg.member_venue(i));
            let lp_seed = child_seed(cfg.seed, i);
            let skew = (i as f64 - centre) * cfg.skew_step;

            let books = lines
                .iter()
                .map(|line| {
                    let instrument = line.instrument.clone();
                    // Disperse this member's initial yield around the reference,
                    // keyed the same way the runtime tick noise is (tick 0).
                    let u = seeded_unit(lp_seed, &venue, &instrument, 0);
                    let mut model = line.model;
                    // Every real line sizes its own dispersion off its own DV01 (a
                    // flat yield budget cannot serve instruments of different
                    // duration); the fleet default only covers a hand-built line.
                    model.initial_yield +=
                        line.yield_dispersion.unwrap_or(cfg.yield_dispersion) * u;
                    InstrumentModel::new(instrument, MidSource::MeanRevertingYield(model))
                        .with_quote_shape(
                            line.spread_scale,
                            line.lean_scale,
                            line.tick,
                            line.dealer_view,
                        )
                })
                .collect();

            let params = member_params(cfg, lp_seed, skew);
            SimLp::new(venue, lp_seed, params, books)
        })
        .collect()
}

/// Derive one member's distinct, seeded quoting *character* from its child seed and
/// centred panel `skew`. Each member disperses its half-spread, firm size, refresh
/// cadence, and self-reported quality reproducibly off `lp_seed`, so the five LPs are
/// visibly different market-makers (and the consolidated best-bid/best-offer across
/// them is a meaningful winner rather than five identical two-ways). Feed latency is
/// left at zero here — occasional per-round staleness/outlier faults are injected at
/// *stream* time (see [`crate::net`]) so the built panel is always fresh and its
/// analytic BBO is exact for the ground-truth tests.
fn member_params(cfg: &LpSimConfig, lp_seed: u64, skew: f64) -> LpParams {
    // Salts key four independent draws off the same member seed (any distinct set
    // works; these are arbitrary odd constants).
    const SALT_SPREAD: u64 = 0x0000_0000_0000_00A1;
    const SALT_SIZE: u64 = 0x0000_0000_0000_00B3;
    const SALT_TICK: u64 = 0x0000_0000_0000_00C7;
    const SALT_QUALITY: u64 = 0x0000_0000_0000_00D9;

    // Half-spread in [0.6, 1.4)× the base — some LPs quote tighter than others.
    let half_spread = cfg.half_spread * (0.6 + 0.8 * unit01(lp_seed, SALT_SPREAD));
    // Firm size in [0.5, 1.5)× the base, snapped to the nearest 100k (min 100k) so
    // sizes read like real quantities.
    let raw_size = cfg.size * (0.5 + unit01(lp_seed, SALT_SIZE));
    let size = ((raw_size / 100_000.0).round() * 100_000.0).max(100_000.0);
    // Refresh cadence in [100 ms, 600 ms) — a faster LP re-quotes its stochastic mid
    // more often within an emission interval.
    let tick_nanos = 100_000_000 + (unit01(lp_seed, SALT_TICK) * 500_000_000.0) as i64;
    // Self-reported quality in [0.85, 1.0).
    let quality = 0.85 + 0.15 * unit01(lp_seed, SALT_QUALITY);

    LpParams {
        half_spread,
        skew,
        size,
        tick_nanos,
        latency_nanos: 0,
        quality,
    }
}

/// One instrument's consolidated composite plus the identity a subscriber renders.
#[derive(Debug, Clone)]
pub struct BondComposite {
    /// The consolidated book from the real engine.
    pub book: ConsolidatedBook,
    /// The best-bid two-way as a human line (identity + price + per-LP legs).
    pub rendered: String,
}

/// Consolidate the panel for `line` at `now_nanos` and render the composite as a
/// subscriber-facing line: the instrument identity (name + cross-refs), the composite
/// best bid/offer + size, the confidence, and each contributing LP's mid.
///
/// # Errors
/// Propagates [`ConsolidationError`] (e.g. every member excluded / no market).
pub fn composite_for(
    feeds: &[Box<dyn celnet_aggregation::VenueFeed>],
    line: &QuotedLine,
    now_nanos: i64,
    cfg: &ConsolidationConfig,
) -> Result<BondComposite, ConsolidationError> {
    let book = ConsolidatedBook::consolidate(feeds, &line.instrument, now_nanos, cfg)?;
    let legs: Vec<String> = book
        .contributions
        .iter()
        .map(|c| {
            let tag = match c.excluded {
                None => format!("{:.4}", c.mid),
                Some(reason) => format!("{:.4}✗{reason:?}", c.mid),
            };
            format!("{}={tag}", c.venue.as_str())
        })
        .collect();
    let rendered = format!(
        "{name} [{identity}]  bid {bid:.4} x{bsz:.0}  offer {offer:.4} x{osz:.0}  \
         mid {mid:.4}  conf {conf:.2}  [{legs}]",
        name = line.display_name,
        identity = line.identity,
        bid = book.best_bid,
        bsz = book.best_bid_size,
        offer = book.best_offer,
        osz = book.best_offer_size,
        mid = book.composite_mid,
        conf = book.confidence,
        legs = legs.join(", "),
    );
    Ok(BondComposite { book, rendered })
}

/// A quote snapshot for one member on one instrument at an instant — the shape an LP
/// pushes to a server ingest (`lp_name`, `instrument_id`, two-way, ts). Exposed so
/// the binary can also emit the raw per-LP wire view, and to make the mapping from
/// a [`VenueQuote`] to the canonical `instrument_id` explicit.
#[derive(Debug, Clone, PartialEq)]
pub struct LpQuoteSnapshot {
    /// The LP connection name (venue id).
    pub lp_name: String,
    /// The canonical server `instrument_id` (a bond's CUSIP / slug, or a futures
    /// contract code).
    pub instrument_id: String,
    /// Bid / offer per 100 face and firm sizes.
    pub bid: f64,
    /// Offer per 100 face.
    pub offer: f64,
    /// Firm bid size.
    pub bid_size: f64,
    /// Firm offer size.
    pub offer_size: f64,
    /// Observation instant (epoch nanoseconds).
    pub ts: i64,
}

impl LpQuoteSnapshot {
    /// Build a snapshot from a member's [`VenueQuote`] and the line it prices — the
    /// bridge that stamps the canonical `instrument_id` onto the wire view.
    #[must_use]
    pub fn from_quote(q: &VenueQuote, line: &QuotedLine) -> Self {
        Self {
            lp_name: q.venue.as_str().to_string(),
            instrument_id: line.instrument_id.clone(),
            bid: q.bid,
            offer: q.offer,
            bid_size: q.bid_size,
            offer_size: q.offer_size,
            ts: q.ts,
        }
    }
}
