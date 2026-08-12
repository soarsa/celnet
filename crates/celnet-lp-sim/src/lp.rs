//! [`SimLp`] — one deterministic-seeded simulated liquidity provider that
//! implements [`celnet_aggregation::VenueFeed`], so a panel of them flows through
//! the REAL consolidation engine.
//!
//! An LP owns a stable venue id, a seed, a set of per-instrument mid sources
//! ([`crate::price::MidSource`]), a set of quoting [`LpParams`] (half-spread,
//! directional skew, size, update cadence, feed latency, self-reported quality),
//! and one injectable [`Fault`]. Its [`VenueFeed::top_of_book`] is a pure function
//! of the query instant, so re-asking at the same instant — or building two LPs
//! with the same seed and configuration — yields byte-identical quotes.

use celnet_aggregation::{Instrument, VenueFeed, VenueId, VenueQuote};

use crate::price::MidSource;
use crate::rng::seeded_unit;

/// The quoting knobs shared across all instruments an LP makes a market in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LpParams {
    /// Half-spread quoted each side of the (skewed) mid, in the instrument's mid
    /// units: `bid = mid + skew − half`, `offer = mid + skew + half`. Positive.
    pub half_spread: f64,
    /// Directional skew/bias applied to the mid before spreading (mid units): a
    /// positive skew lifts both sides (the LP is axed to sell / shows a rich
    /// two-way), a negative skew drops both (axed to buy). Drives cross-LP
    /// divergence in the consolidated composite.
    pub skew: f64,
    /// Firm size quoted on both the bid and the offer. Positive.
    pub size: f64,
    /// Quote-update cadence in nanoseconds: the stochastic mid is resampled once
    /// per tick, so within a tick the top-of-book is stable (a realistic
    /// discretely-updating feed). Clamped to `≥ 1` ns. Ignored by fixed (ladder)
    /// mids, which never move.
    pub tick_nanos: i64,
    /// Feed latency in nanoseconds: a fresh quote's observation `ts` is back-dated
    /// by this amount (`ts = now − latency`), so a laggy LP is aged and
    /// staleness-decayed by the consolidator. Non-negative.
    pub latency_nanos: i64,
    /// The LP's self-reported quality in `[0, 1]`, carried onto every quote.
    pub quality: f64,
}

impl LpParams {
    /// A sensible tight-LP default: a 2 bp half-spread on a ~par (100.0) bond
    /// price, 1 mm size, no skew, a 100 ms update tick, zero latency, full
    /// quality. Callers perturb `skew`/`half_spread`/`latency_nanos` per LP to
    /// build a diverging panel.
    #[must_use]
    pub fn tight() -> Self {
        Self {
            half_spread: 2.0e-2, // ~2 bp of a 100.0 price handle
            skew: 0.0,
            size: 1_000_000.0,
            tick_nanos: 100_000_000, // 100 ms
            latency_nanos: 0,
            quality: 1.0,
        }
    }
}

/// An injectable fault behaviour for one LP — the stress inputs that exercise the
/// consolidator's staleness decay and MAD divergence gating.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fault {
    /// The LP quotes fresh and true (no fault).
    Healthy,
    /// The feed stopped updating at `frozen_at_nanos`: it keeps re-publishing the
    /// price AND observation `ts` it had at that instant, so its age grows without
    /// bound as the valuation clock advances — first staleness-decayed, then hard
    /// excluded once past the consolidator's cutoff.
    Stale {
        /// The last instant at which the feed updated (epoch nanoseconds).
        frozen_at_nanos: i64,
    },
    /// A fat-finger / divergent print: the LP's mid is displaced by `shift` (mid
    /// units) — a mispriced feed the consolidator's median-consensus + MAD gate
    /// must exclude from the composite and the BBO.
    Outlier {
        /// The signed displacement added to the LP's true mid (mid units).
        shift: f64,
    },
}

/// One instrument an LP makes a market in, paired with the mid source that prices
/// it and the quoting conventions of that instrument's market.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentModel {
    /// The instrument key this model quotes.
    pub instrument: Instrument,
    /// How the LP forms its ground-truth mid for the instrument.
    pub mid: MidSource,
    /// A multiplier on the LP's [`half_spread`](LpParams::half_spread) for this
    /// instrument, so one LP can quote markets whose natural width differs by an order
    /// of magnitude — a cash bond's couple of basis points versus a listed future's
    /// single minimum price increment. `1.0` uses the LP's own half-spread.
    pub spread_scale: f64,
    /// A multiplier on the LP's directional [`skew`](LpParams::skew) for this
    /// instrument — how far this member's market leans away from the panel's centre.
    ///
    /// Carried separately from [`spread_scale`](Self::spread_scale) because the two
    /// are not proportional across market structures. On a listed venue every market
    /// maker quotes essentially the same one- or two-tick market and competes on size
    /// and queue position, so the lean must stay a small fraction of the half-spread
    /// or the panel's best bid would print through its best offer — a crossed
    /// composite, which the RFQ resolver rejects outright.
    pub lean_scale: f64,
    /// The minimum price increment this instrument trades on, if any. The bid is
    /// snapped **down** to the grid and the offer **up**, which is what an
    /// exchange-listed market maker shows and can never invert a two-way. `None` for
    /// an off-grid OTC instrument.
    pub tick: Option<f64>,
    /// The amplitude (decimal yield) of this member's **own** time-varying view of
    /// the instrument, drawn on the member's own seed and re-quote cadence and
    /// applied on top of the panel-common market level (see
    /// [`common_market_draw`](Self::common_market_draw)).
    ///
    /// This is the dealer's idiosyncratic axe/micro-view: it is what keeps a member
    /// distinguishable from its peers tick to tick rather than a fixed offset from
    /// them forever, and it is why a member's own re-quote cadence still means
    /// something once the market level is common. It must stay **small relative to
    /// the member's own half-spread** — the caller sizes it in price terms off the
    /// instrument's own DV01 and budgets it inside the tightest member's half-spread
    /// (see [`crate::quoted::QuotedLine::dealer_view`]).
    ///
    /// `0.0` for a market where a sub-increment private view is not meaningful (a
    /// listed contract quoted on a tick grid snaps it away).
    pub dealer_view: f64,
    /// Whether the panel shares one stochastic **market-level** draw (and one
    /// re-quote cadence for it) for this instrument, instead of each member drawing
    /// the level independently.
    ///
    /// The level of a traded security is common information — an exchange print
    /// reaches every market maker at once, and an OTC dealer in a benchmark
    /// government bond marks off the same observable inter-dealer level as its
    /// competitors. Drawing that level *independently* per member is not a model of
    /// bilateral disagreement, it is a model of dealers looking at different markets:
    /// it displaces the members' mids by the instrument's DV01 times the yield
    /// jitter, which on any medium-duration security is an order of magnitude more
    /// than the quoted bid-offer. The panel's best bid then routinely prints through
    /// its best offer, the composite comes out **crossed**, and the server's RFQ
    /// resolver rejects the line outright — so the hedge and the outbound quote it
    /// exists to serve never happen.
    ///
    /// Genuine cross-dealer differentiation is carried instead by the member's
    /// deliberate lean, its own spread width, and its
    /// [`dealer_view`](Self::dealer_view) — all budgeted inside its own half-spread,
    /// because a market whose dealers disagree by more than they quote is not a
    /// market, it is an arbitrage.
    ///
    /// `false` restores fully independent per-member level draws — the primitive
    /// [`InstrumentModel::new`] keeps, for ladder/analytic use.
    pub common_market_draw: bool,
}

/// The shared re-quote cadence of the common market level: the panel steps its
/// common stochastic draw together rather than on each member's own dispersed
/// cadence, because the market level every maker marks off is one observable.
const MARKET_TICK_NANOS: i64 = 250_000_000;

/// The seed and venue label the shared market-level draw is keyed on, so every
/// member of the panel samples the identical value for a given (instrument, tick).
const SHARED_MARKET_SEED: u64 = 0x4C49_5354_4544_0001;

/// A salt keying a member's private [`dealer_view`](InstrumentModel::dealer_view)
/// draw independently of the market-level draw it is added to.
const DEALER_VIEW_SALT: u64 = 0x5649_4557_0000_0001;

/// The venue key of the shared market-level draw (a label, not a real connection).
static SHARED_MARKET_VENUE: std::sync::LazyLock<VenueId> =
    std::sync::LazyLock::new(|| VenueId::new("MARKET-LEVEL"));

impl InstrumentModel {
    /// A model quoting `instrument` off `mid` with the LP's own spread and skew, off
    /// any price grid, forming its market level **independently** of the rest of the
    /// panel — the bare primitive, used for ladder mids and for analytic tests that
    /// want deliberately decorrelated members.
    ///
    /// A member quoting a real instrument alongside a real panel is built with
    /// [`with_quote_shape`](Self::with_quote_shape) instead, which shares the market
    /// level; see [`common_market_draw`](Self::common_market_draw) for why.
    #[must_use]
    pub fn new(instrument: Instrument, mid: MidSource) -> Self {
        Self {
            instrument,
            mid,
            spread_scale: 1.0,
            lean_scale: 1.0,
            tick: None,
            dealer_view: 0.0,
            common_market_draw: false,
        }
    }

    /// This model with a real market's quoting shape: the half-spread scaled by
    /// `spread_scale`, the directional lean by `lean_scale`, quotes snapped to a
    /// `tick` grid when the instrument trades on one, this member's own private
    /// view sized at `dealer_view` (decimal yield), and the **market level shared
    /// across the panel** (see [`common_market_draw`](Self::common_market_draw)).
    #[must_use]
    pub fn with_quote_shape(
        mut self,
        spread_scale: f64,
        lean_scale: f64,
        tick: Option<f64>,
        dealer_view: f64,
    ) -> Self {
        self.spread_scale = spread_scale;
        self.lean_scale = lean_scale;
        self.tick = tick;
        self.dealer_view = if dealer_view.is_finite() && dealer_view > 0.0 {
            dealer_view
        } else {
            0.0
        };
        self.common_market_draw = true;
        self
    }

    /// A configured multiplier, guarding a non-finite or non-positive value back to
    /// `1.0` (a degenerate scale would collapse or invert a two-way rather than
    /// merely quoting it oddly).
    fn scale(configured: f64) -> f64 {
        if configured.is_finite() && configured > 0.0 {
            configured
        } else {
            1.0
        }
    }

    /// The usable price grid for this instrument, if it has one.
    fn grid(&self) -> Option<f64> {
        self.tick.filter(|t| t.is_finite() && *t > 0.0)
    }
}

/// A deterministic-seeded simulated liquidity provider. Implements
/// [`VenueFeed`] as a pure function of the query instant.
#[derive(Debug, Clone)]
pub struct SimLp {
    venue: VenueId,
    seed: u64,
    params: LpParams,
    books: Vec<InstrumentModel>,
    fault: Fault,
}

impl SimLp {
    /// Build an LP from its venue id, seed, quoting params, and the instruments it
    /// makes a market in. The LP starts [`Fault::Healthy`]; inject a fault with
    /// [`SimLp::with_fault`].
    #[must_use]
    pub fn new(venue: VenueId, seed: u64, params: LpParams, books: Vec<InstrumentModel>) -> Self {
        Self {
            venue,
            seed,
            params,
            books,
            fault: Fault::Healthy,
        }
    }

    /// A convenience constructor for a single-instrument **ladder** LP quoting a
    /// fixed mid — the analytic-ground-truth building block for BBO tests.
    #[must_use]
    pub fn quoting_fixed(
        venue: impl Into<String>,
        instrument: Instrument,
        mid: f64,
        params: LpParams,
    ) -> Self {
        Self::new(
            VenueId::new(venue),
            0,
            params,
            vec![InstrumentModel::new(instrument, MidSource::Fixed(mid))],
        )
    }

    /// Return this LP with `fault` injected (builder-style; consumes and returns
    /// `self` so it composes with the constructors).
    #[must_use]
    pub fn with_fault(mut self, fault: Fault) -> Self {
        self.fault = fault;
        self
    }

    /// The LP's currently injected fault.
    #[must_use]
    pub fn fault(&self) -> Fault {
        self.fault
    }

    /// The LP's quoting parameters.
    #[must_use]
    pub fn params(&self) -> LpParams {
        self.params
    }

    /// The mid source this LP quotes for `instrument`, or `None` if it makes no
    /// market in it.
    fn model_for(&self, instrument: &Instrument) -> Option<&InstrumentModel> {
        self.books.iter().find(|b| &b.instrument == instrument)
    }

    /// The seeded per-tick **market-level** noise for a stochastic mid at logical
    /// `t_nanos` (`0.0` for a constant ladder mid, which needs no draw).
    fn market_noise_at(&self, model: &InstrumentModel, t_nanos: i64) -> f64 {
        if !model.mid.is_stochastic() {
            return 0.0;
        }
        if model.common_market_draw {
            // The level is one observable: one draw on one cadence for the panel.
            let tick = t_nanos.div_euclid(MARKET_TICK_NANOS);
            seeded_unit(
                SHARED_MARKET_SEED,
                &SHARED_MARKET_VENUE,
                &model.instrument,
                tick,
            )
        } else {
            let tick = t_nanos.div_euclid(self.params.tick_nanos.max(1));
            seeded_unit(self.seed, &self.venue, &model.instrument, tick)
        }
    }

    /// This member's own private yield displacement at logical `t_nanos`: a seeded
    /// draw on the member's **own** seed and re-quote cadence, scaled by the model's
    /// [`dealer_view`](InstrumentModel::dealer_view) amplitude. Zero when the model
    /// carries no private view (a ladder mid, or a market where one is not
    /// meaningful).
    fn dealer_view_at(&self, model: &InstrumentModel, t_nanos: i64) -> f64 {
        if !model.mid.is_stochastic() || model.dealer_view == 0.0 {
            return 0.0;
        }
        let tick = t_nanos.div_euclid(self.params.tick_nanos.max(1));
        let u = seeded_unit(
            self.seed ^ DEALER_VIEW_SALT,
            &self.venue,
            &model.instrument,
            tick,
        );
        model.dealer_view * u
    }
}

impl VenueFeed for SimLp {
    fn venue(&self) -> &VenueId {
        &self.venue
    }

    fn top_of_book(&self, instrument: &Instrument, now_nanos: i64) -> Option<VenueQuote> {
        let model = self.model_for(instrument)?;

        // A staleness fault freezes BOTH the sampled instant and the observation
        // ts at the moment the feed stopped updating; a healthy feed samples the
        // current tick and reports `now` back-dated by its latency.
        let (sample_nanos, ts) = match self.fault {
            Fault::Stale { frozen_at_nanos } => (frozen_at_nanos, frozen_at_nanos),
            Fault::Healthy | Fault::Outlier { .. } => (
                now_nanos,
                now_nanos.saturating_sub(self.params.latency_nanos),
            ),
        };

        // The market level is (for a real instrument) one shared observable; this
        // member's own private view of the security displaces it by a small amount.
        let noise = self.market_noise_at(model, sample_nanos);
        let view = self.dealer_view_at(model, sample_nanos);
        let mut mid = model.mid.mid_at_view(sample_nanos, noise, view);
        if let Fault::Outlier { shift } = self.fault {
            mid += shift;
        }

        // The instrument's market conventions rescale the LP's quoting knobs, then
        // (for an exchange-listed instrument) snap the two-way onto its price grid:
        // bid down, offer up — never inverting the quote.
        let leaned = mid + self.params.skew * InstrumentModel::scale(model.lean_scale);
        let half = self.params.half_spread * InstrumentModel::scale(model.spread_scale);
        let (bid, offer) = match model.grid() {
            Some(tick) => (
                ((leaned - half) / tick).floor() * tick,
                ((leaned + half) / tick).ceil() * tick,
            ),
            None => (leaned - half, leaned + half),
        };
        Some(VenueQuote {
            venue: self.venue.clone(),
            instrument: instrument.clone(),
            bid,
            offer,
            bid_size: self.params.size,
            offer_size: self.params.size,
            ts,
            quality: self.params.quality,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::price::YieldModel;
    use celnet_bond::{AccrualBasis, Bond, PaymentFrequency, dirty_price, yield_to_maturity};
    use celnet_types::{Ccy, CcyPair, Rate, Tenor};
    use time::{Date, Month};

    const S: i64 = 1_000_000_000;

    fn instr() -> Instrument {
        Instrument::new(CcyPair::new(Ccy::EUR, Ccy::USD), Tenor::Years(5))
    }

    fn ref_bond() -> Bond {
        Bond::new(
            Date::from_calendar_date(2026, Month::June, 15).unwrap(),
            Date::from_calendar_date(2031, Month::June, 15).unwrap(),
            0.05,
            PaymentFrequency::SemiAnnual,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .unwrap()
    }

    fn stochastic_lp(venue: &str, seed: u64) -> SimLp {
        let model = YieldModel {
            bond: ref_bond(),
            long_run_yield: 0.05,
            initial_yield: 0.045,
            reversion_per_sec: 0.02,
            perturbation: 2.0e-4,
        };
        SimLp::new(
            VenueId::new(venue),
            seed,
            LpParams::tight(),
            vec![InstrumentModel::new(
                instr(),
                MidSource::MeanRevertingYield(model),
            )],
        )
    }

    #[test]
    fn same_seed_same_config_is_byte_identical() {
        let a = stochastic_lp("lp", 7);
        let b = stochastic_lp("lp", 7);
        for k in 0..25_i64 {
            let t = k * 137 * S / 10; // arbitrary uneven grid across ticks
            let qa = a.top_of_book(&instr(), t).unwrap();
            let qb = b.top_of_book(&instr(), t).unwrap();
            assert_eq!(qa.bid.to_bits(), qb.bid.to_bits());
            assert_eq!(qa.offer.to_bits(), qb.offer.to_bits());
            assert_eq!(qa.ts, qb.ts);
        }
    }

    #[test]
    fn distinct_seeds_decorrelate_the_stochastic_stream() {
        let a = stochastic_lp("lp", 1);
        let b = stochastic_lp("lp", 2);
        let differs = (0..20).any(|k| {
            let t = k * S;
            a.top_of_book(&instr(), t).unwrap().mid().to_bits()
                != b.top_of_book(&instr(), t).unwrap().mid().to_bits()
        });
        assert!(differs, "distinct seeds must decorrelate the yield noise");
    }

    #[test]
    fn spread_skew_and_size_come_out_as_configured() {
        // A fixed (ladder) mid isolates spread/skew/size from any dynamics.
        let params = LpParams {
            half_spread: 0.03,
            skew: 0.05,
            size: 2_500_000.0,
            ..LpParams::tight()
        };
        let lp = SimLp::quoting_fixed("lp", instr(), 100.0, params);
        let q = lp.top_of_book(&instr(), 42 * S).unwrap();
        // leaned mid = 100.0 + 0.05 = 100.05; bid/offer straddle it by half_spread.
        assert_eq!(q.bid.to_bits(), (100.05_f64 - 0.03).to_bits());
        assert_eq!(q.offer.to_bits(), (100.05_f64 + 0.03).to_bits());
        assert_eq!(q.mid().to_bits(), 100.05_f64.to_bits());
        assert_eq!(q.bid_size, 2_500_000.0);
        assert_eq!(q.offer_size, 2_500_000.0);
    }

    #[test]
    fn latency_back_dates_a_healthy_observation() {
        let params = LpParams {
            latency_nanos: 250 * S / 1000, // 250 ms
            ..LpParams::tight()
        };
        let lp = SimLp::quoting_fixed("slow", instr(), 100.0, params);
        let q = lp.top_of_book(&instr(), 1000 * S).unwrap();
        assert_eq!(q.ts, 1000 * S - 250 * S / 1000);
    }

    #[test]
    fn staleness_fault_freezes_the_observation_ts_and_price() {
        let frozen = 500 * S;
        let lp = SimLp::quoting_fixed("gone", instr(), 100.0, LpParams::tight()).with_fault(
            Fault::Stale {
                frozen_at_nanos: frozen,
            },
        );
        // As the valuation clock advances far past `frozen`, the ts never moves —
        // the quote ages without bound (the consolidator will decay then drop it).
        for now in [frozen, frozen + 60 * S, frozen + 3600 * S] {
            let q = lp.top_of_book(&instr(), now).unwrap();
            assert_eq!(q.ts, frozen, "a stopped feed re-publishes its frozen ts");
        }
    }

    #[test]
    fn outlier_fault_produces_a_divergent_print() {
        let healthy = SimLp::quoting_fixed("lp", instr(), 100.0, LpParams::tight());
        let bad = SimLp::quoting_fixed("lp", instr(), 100.0, LpParams::tight())
            .with_fault(Fault::Outlier { shift: 3.5 });
        let m_ok = healthy.top_of_book(&instr(), S).unwrap().mid();
        let m_bad = bad.top_of_book(&instr(), S).unwrap().mid();
        assert_eq!((m_bad - m_ok).to_bits(), 3.5_f64.to_bits());
    }

    #[test]
    fn no_market_in_unlisted_instrument() {
        let lp = SimLp::quoting_fixed("lp", instr(), 100.0, LpParams::tight());
        let other = Instrument::new(CcyPair::new(Ccy::GBP, Ccy::USD), Tenor::Years(5));
        assert!(lp.top_of_book(&other, 0).is_none());
    }

    #[test]
    fn stochastic_mid_matches_the_reference_yield_to_price_relation() {
        // Validate (not merely assert plausible) that the sim's mid IS the
        // celnet-bond clean price of the sampled yield, and that the relation
        // round-trips through the reference yield solver.
        let model = YieldModel {
            bond: ref_bond(),
            long_run_yield: 0.05,
            initial_yield: 0.045,
            reversion_per_sec: 0.02,
            perturbation: 2.0e-4,
        };
        let now = 3 * S;
        let noise = 0.37;
        let y = model.yield_at(now, noise);
        let clean = model.clean_price_at(now, noise);
        // Reference clean price of that exact yield.
        let s = celnet_bond::clean_price(&ref_bond(), Rate(y)).unwrap();
        assert_eq!(clean.to_bits(), s.to_bits());

        // Round-trip: recovering the yield from the reference dirty price returns y.
        let dirty = dirty_price(&ref_bond(), Rate(y)).unwrap();
        let recovered = yield_to_maturity(&ref_bond(), dirty).unwrap();
        assert!(
            (recovered.0 - y).abs() < 1e-9,
            "ytm round-trip drift {} too large",
            (recovered.0 - y).abs()
        );
    }

    #[test]
    fn clean_price_is_monotone_decreasing_in_yield() {
        // The yield→price map the sim relies on must be strictly decreasing.
        let b = ref_bond();
        let mut prev = f64::INFINITY;
        for bp in 0..20 {
            let y = 0.03 + f64::from(bp) * 0.005;
            let p = celnet_bond::clean_price(&b, Rate(y)).unwrap();
            assert!(p < prev, "price not decreasing at y={y}: {p} !< {prev}");
            prev = p;
        }
    }
}

#[cfg(test)]
mod quote_shape_tests {
    use super::*;
    use crate::price::MidSource;
    use celnet_types::{Ccy, CcyPair, Tenor};

    fn instr(tag: &str) -> Instrument {
        let _ = tag;
        Instrument::new(CcyPair::new(Ccy::EUR, Ccy::USD), Tenor::Years(5))
    }

    /// A tick-gridded model snaps the bid down and the offer up, so a member's own
    /// two-way lands on the grid and is never inverted, however tight the spread.
    #[test]
    fn a_gridded_model_snaps_the_two_way_onto_its_grid() {
        let tick = 1.0 / 64.0;
        let model = InstrumentModel::new(instr("f"), MidSource::Fixed(110.4013)).with_quote_shape(
            1.0,
            1.0,
            Some(tick),
            0.0,
        );
        let params = LpParams {
            half_spread: tick / 2.0,
            skew: 0.0,
            ..LpParams::tight()
        };
        let lp = SimLp::new(VenueId::new("LP-1"), 7, params, vec![model]);
        let q = lp.top_of_book(&instr("f"), 0).expect("quotes");

        for px in [q.bid, q.offer] {
            let ticks = px / tick;
            assert!((ticks - ticks.round()).abs() < 1e-9, "{px} off the grid");
        }
        assert!(q.bid <= q.offer, "a snapped two-way is never inverted");
        assert!(
            q.bid <= 110.4013 && q.offer >= 110.4013,
            "the mid is inside"
        );
        // One or two ticks wide — a listed market, not a cash-bond spread.
        let width = (q.offer - q.bid) / tick;
        assert!((1.0..=2.0).contains(&width.round()), "{width} ticks wide");
    }

    /// The spread and lean multipliers act independently, so a listed line can quote
    /// a tick-wide market while keeping members' leans a small fraction of it.
    #[test]
    fn spread_and_lean_scale_independently() {
        let base = LpParams {
            half_spread: 0.02,
            skew: 0.008,
            ..LpParams::tight()
        };
        let model = InstrumentModel::new(instr("x"), MidSource::Fixed(100.0))
            .with_quote_shape(0.25, 0.05, None, 0.0);
        let lp = SimLp::new(VenueId::new("LP-2"), 1, base, vec![model]);
        let q = lp.top_of_book(&instr("x"), 0).expect("quotes");
        // half-spread 0.02 x 0.25 = 0.005; lean 0.008 x 0.05 = 0.0004.
        assert!(((q.offer - q.bid) / 2.0 - 0.005).abs() < 1e-12);
        assert!(((q.bid + q.offer) / 2.0 - 100.0004).abs() < 1e-12);
    }

    /// A degenerate (non-finite or non-positive) scale falls back to the LP's own
    /// parameters rather than collapsing or inverting the quote.
    #[test]
    fn a_degenerate_scale_falls_back_to_the_lp_parameters() {
        for bad in [0.0, -1.0, f64::NAN] {
            let model = InstrumentModel::new(instr("y"), MidSource::Fixed(100.0))
                .with_quote_shape(bad, bad, None, 0.0);
            let lp = SimLp::new(
                VenueId::new("LP-3"),
                1,
                LpParams {
                    half_spread: 0.02,
                    ..LpParams::tight()
                },
                vec![model],
            );
            let q = lp.top_of_book(&instr("y"), 0).expect("quotes");
            assert!(
                ((q.offer - q.bid) / 2.0 - 0.02).abs() < 1e-12,
                "bad = {bad}"
            );
        }
    }

    /// A listed line shares ONE stochastic draw across the panel — two members with
    /// different seeds, venues and refresh cadences see the identical mid. This is
    /// what stops a tick-wide listed composite from coming out crossed.
    #[test]
    fn a_listed_line_moves_the_whole_panel_together() {
        let yield_model = crate::price::YieldModel {
            bond: celnet_bond::Bond::new(
                time::Date::from_calendar_date(2026, time::Month::June, 1).unwrap(),
                time::Date::from_calendar_date(2033, time::Month::September, 1).unwrap(),
                0.06,
                celnet_bond::PaymentFrequency::SemiAnnual,
                celnet_bond::AccrualBasis::Thirty360BondBasis,
                100.0,
            )
            .unwrap(),
            long_run_yield: 0.042,
            initial_yield: 0.042,
            reversion_per_sec: 0.02,
            perturbation: 3.0e-4,
        };
        let listed = |seed: u64, venue: &str, cadence: i64| {
            let model =
                InstrumentModel::new(instr("z"), MidSource::MeanRevertingYield(yield_model))
                    .with_quote_shape(1.0, 1.0, Some(1.0 / 64.0), 0.0);
            SimLp::new(
                VenueId::new(venue),
                seed,
                LpParams {
                    skew: 0.0,
                    tick_nanos: cadence,
                    ..LpParams::tight()
                },
                vec![model],
            )
        };
        let a = listed(1, "LP-A", 100_000_000);
        let b = listed(999, "LP-B", 600_000_000);
        for t in [0_i64, 400_000_000, 3_000_000_000, 9_100_000_000] {
            let qa = a.top_of_book(&instr("z"), t).expect("quotes");
            let qb = b.top_of_book(&instr("z"), t).expect("quotes");
            assert!(
                (qa.bid - qb.bid).abs() < 1e-12 && (qa.offer - qb.offer).abs() < 1e-12,
                "listed members diverged at t={t}: {qa:?} vs {qb:?}"
            );
        }

        // An OTC line (no grid) keeps the per-member draw — the members DO differ.
        let otc = |seed: u64, venue: &str| {
            SimLp::new(
                VenueId::new(venue),
                seed,
                LpParams {
                    skew: 0.0,
                    ..LpParams::tight()
                },
                vec![InstrumentModel::new(
                    instr("z"),
                    MidSource::MeanRevertingYield(yield_model),
                )],
            )
        };
        let c = otc(1, "LP-A");
        let d = otc(999, "LP-B");
        let qc = c.top_of_book(&instr("z"), 400_000_000).expect("quotes");
        let qd = d.top_of_book(&instr("z"), 400_000_000).expect("quotes");
        assert!(
            (qc.bid - qd.bid).abs() > 1e-9,
            "OTC members should form independent views"
        );
    }
}
