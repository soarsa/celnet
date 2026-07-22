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
/// it.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentModel {
    /// The instrument key this model quotes.
    pub instrument: Instrument,
    /// How the LP forms its ground-truth mid for the instrument.
    pub mid: MidSource,
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
            vec![InstrumentModel {
                instrument,
                mid: MidSource::Fixed(mid),
            }],
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

    /// The seeded per-tick noise for a stochastic mid at logical `t_nanos`
    /// (`0.0` for a constant ladder mid, which needs no draw).
    fn noise_at(&self, model: &InstrumentModel, t_nanos: i64) -> f64 {
        if model.mid.is_stochastic() {
            let tick = t_nanos.div_euclid(self.params.tick_nanos.max(1));
            seeded_unit(self.seed, &self.venue, &model.instrument, tick)
        } else {
            0.0
        }
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

        let noise = self.noise_at(model, sample_nanos);
        let mut mid = model.mid.mid_at(sample_nanos, noise);
        if let Fault::Outlier { shift } = self.fault {
            mid += shift;
        }

        let leaned = mid + self.params.skew;
        Some(VenueQuote {
            venue: self.venue.clone(),
            instrument: instrument.clone(),
            bid: leaned - self.params.half_spread,
            offer: leaned + self.params.half_spread,
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
            vec![InstrumentModel {
                instrument: instr(),
                mid: MidSource::MeanRevertingYield(model),
            }],
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
