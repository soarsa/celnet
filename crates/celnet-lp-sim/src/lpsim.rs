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
use crate::quoted::QuotedLine;
use crate::rng::{child_seed, seeded_unit};
use crate::roster::{OTC_ROSTER, SimLpProfile};
use crate::universe::TreasuryBond;

/// The feed's own label in banners and logs. It is **not** a venue id: every
/// contribution is attributed to a named simulated counterparty from
/// [`crate::roster::OTC_ROSTER`], never to a numbered `LP-SIM-0N` connection.
pub const DEFAULT_LP_NAME: &str = "OTC-SIM";

/// How the `LP-SIM` feed is assembled: the LP identity, the number of decorrelated
/// member connections, the stochastic-yield knobs, the quoting shape, and the
/// consolidation tuning.
#[derive(Debug, Clone)]
pub struct LpSimConfig {
    /// The feed's label for banners and logs. **Not** a venue id — see
    /// [`DEFAULT_LP_NAME`].
    pub lp_name: String,
    /// The roster of named simulated counterparties this panel is drawn from, in
    /// panel order. Defaults to [`OTC_ROSTER`].
    pub roster: &'static [SimLpProfile],
    /// How many of `roster`'s counterparties participate (`≥ 1`, clamped to the
    /// roster length by [`LpSimConfig::panel_size`]). More members ⇒ a richer
    /// composite (median-consensus gating needs ≥ 3 to be decidable).
    ///
    /// It is deliberately **not** possible to exceed the roster: two members sharing
    /// a `VenueId` collapse into one contribution inside the consolidator, so a
    /// wrapped panel would look like N feeds and consolidate like one.
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
            roster: OTC_ROSTER,
            members: OTC_ROSTER.len(),
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

    /// The number of counterparties that actually participate: the requested
    /// [`members`](Self::members) clamped to the roster (never wrapped).
    #[must_use]
    pub fn panel_size(&self) -> usize {
        self.members.clamp(1, self.roster.len().max(1))
    }

    /// The roster profile of panel position `i` (`0`-based), or `None` past the end.
    #[must_use]
    pub fn member_profile(&self, i: usize) -> Option<&'static SimLpProfile> {
        if i >= self.panel_size() {
            return None;
        }
        crate::roster::profile_at(self.roster, i)
    }

    /// The venue id of panel position `i` — the named simulated counterparty's own
    /// stable id (e.g. `citigroup-sim`), which is what a composite leg, an LP panel
    /// row and a booked hedge's `lp_won` are all attributed to.
    ///
    /// Falls back to the feed label only for a position past the roster, which
    /// [`panel_size`](Self::panel_size) already makes unreachable.
    #[must_use]
    pub fn member_venue(&self, i: usize) -> String {
        self.member_profile(i)
            .map_or_else(|| self.lp_name.clone(), |p| p.id.to_string())
    }

    /// Every participating counterparty's venue id, in panel order.
    #[must_use]
    pub fn member_venues(&self) -> Vec<String> {
        (0..self.panel_size())
            .map(|i| self.member_venue(i))
            .collect()
    }
}

/// The full set of [`QuotedLine`]s the OTC sim can price for `cfg`: every cash bond
/// in `bonds` that models at the config's settlement, the swap/OIS curve points, and
/// the listed SOFR STIR strip anchored to those same curve points.
///
/// This is the sim's **quotable** universe, and it is deliberately built from the
/// same `celnet-refdata` sources the server seeds its **tradeable** registry from —
/// an instrument that is tradeable but not quotable never reaches an aggregated book
/// (see the [`crate::quoted`] module docs).
///
/// **Listed Treasury futures are not here.** They are quoted by the dedicated
/// futures venue simulator (`celnet-cme-sim`) under its own `cme-sim` connection —
/// see the crate docs. The tradeable-vs-quotable invariant above still holds across
/// the estate, but it now holds across *two* simulators, so an aggregated book that
/// carries futures must list `cme-sim` as a member.
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
    // The swap/OIS curve points. Without these no aggregated book ever carries a swap
    // line, and the venue's OIS arm has no composite to price an RFS against (see the
    // `crate::ois` module docs).
    let ois_points = crate::ois::load_ois_universe();
    lines.extend(crate::ois::ois_lines(
        &ois_points,
        cfg.half_spread,
        cfg.skew_step,
        cfg.reversion_per_sec,
        cfg.perturbation,
    ));
    // The listed SOFR STIR strip, anchored to the SAME curve the swap lines stream so
    // the front end and the swap strip cannot drift apart (see `crate::stir`).
    let stir = crate::stir::load_stir_universe(&ois_points, as_of(cfg.settlement));
    lines.extend(crate::stir::stir_lines(
        &stir,
        cfg.half_spread,
        cfg.skew_step,
        cfg.reversion_per_sec,
        cfg.perturbation,
    ));
    lines
}

/// The sim's settlement date as a reference-data civil date — the valuation instant
/// the listed universes are filtered against, so a contract that has stopped trading
/// is never quoted.
fn as_of(settlement: BrokenDate) -> celnet_refdata::CivilYmd {
    celnet_refdata::CivilYmd::new(
        settlement.year,
        u32::from(settlement.month),
        u32::from(settlement.day),
    )
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
    let n = cfg.panel_size();
    (0..n)
        .filter_map(|i| {
            let profile = cfg.member_profile(i)?;
            let venue = VenueId::new(profile.id);
            let lp_seed = child_seed(cfg.seed, i);

            let books = lines
                .iter()
                .map(|line| {
                    let instrument = line.instrument.clone();
                    // Disperse this member's initial yield around the reference,
                    // keyed the same way the runtime tick noise is (tick 0).
                    let u = seeded_unit(lp_seed, &venue, &instrument, 0);
                    // Every real line sizes its own dispersion off its own DV01 (a
                    // flat yield budget cannot serve instruments of different
                    // duration); the fleet default only covers a hand-built line.
                    let dispersion = line.yield_dispersion.unwrap_or(cfg.yield_dispersion) * u;
                    let mid = line.mid.with_initial_displaced(dispersion);
                    InstrumentModel::new(instrument, mid).with_quote_shape(
                        line.spread_scale,
                        line.lean_scale,
                        line.tick,
                        line.dealer_view,
                    )
                })
                .collect();

            Some(SimLp::new(
                venue,
                lp_seed,
                member_params(cfg, profile),
                books,
            ))
        })
        .collect()
}

/// Project one named counterparty's [`SimLpProfile`] onto the quoting knobs the
/// price model consumes.
///
/// The character is a property of the **roster**, not of a draw: who is tight, who
/// is axed, who shows size and who is slow is fixed and reproducible across every
/// run, which is what makes an LP panel readable to a trader. Only a narrow seeded
/// jitter (±10% on the spread, ±15% on the size — see
/// [`SimLpProfile::half_spread`] / [`SimLpProfile::firm_size`]) varies with the run
/// seed, and it is far too small to reorder the roster's characters.
///
/// Unlike the previous numbered fleet, feed latency is **real** here: each
/// counterparty back-dates its observation by its own archetype's response latency,
/// so a slow principal dealer is staleness-decayed by the consolidator exactly as a
/// slow real feed is. The latencies are sub-10 ms — far inside any book's staleness
/// cutoff — so this changes the panel's freshness ordering without ever excluding a
/// healthy member. Injected staleness/outlier *faults* remain a stream-time concern
/// (see [`crate::net`]).
fn member_params(cfg: &LpSimConfig, profile: &SimLpProfile) -> LpParams {
    LpParams {
        half_spread: profile.half_spread(cfg.half_spread, cfg.seed),
        skew: profile.axe(cfg.skew_step),
        size: profile.firm_size(cfg.size, cfg.seed),
        tick_nanos: profile.refresh_nanos(),
        latency_nanos: profile.latency_nanos(),
        quality: profile.quality,
    }
}

/// The [`crate::execution::DepthLadder`] a taker walks when it sends an order to
/// panel member `i` for `line`, given that member's current top-of-book.
///
/// This is the bridge between what a counterparty **quotes** and what it will
/// **trade**: the ladder's touch is exactly the streamed level the composite was
/// built from, and everything behind it is that counterparty's own roster depth
/// shape. A simulator therefore never fills against liquidity it never showed.
///
/// Returns `None` for a panel position outside the roster.
#[must_use]
pub fn depth_for(
    cfg: &LpSimConfig,
    member: usize,
    side: crate::execution::Side,
    touch_price: f64,
    touch_size: f64,
    lot_size: Option<f64>,
) -> Option<crate::execution::DepthLadder> {
    let profile = cfg.member_profile(member)?;
    Some(crate::execution::DepthLadder::from_quote(
        side,
        touch_price,
        touch_size,
        profile.half_spread(cfg.half_spread, cfg.seed),
        profile,
        lot_size,
    ))
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
