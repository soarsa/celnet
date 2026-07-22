//! A factory that spins up N decorrelated [`SimLp`]s with distinct, reproducible
//! characteristics — the "various LP connections into the book" a soak or demo
//! points the aggregated book at.
//!
//! [`fleet`] derives a child seed per LP (so members decorrelate), spreads a
//! centred directional skew across the panel, and disperses each LP's initial
//! yield deterministically, then returns ready-to-quote LPs in stochastic mode.
//! [`into_feeds`] boxes any LP collection into the `Vec<Box<dyn VenueFeed>>` the
//! consolidator consumes.

use celnet_aggregation::{Instrument, VenueFeed, VenueId};

use crate::lp::{InstrumentModel, LpParams, SimLp};
use crate::price::{MidSource, YieldModel};
use crate::rng::{child_seed, seeded_unit};

/// One instrument the whole fleet makes a market in, described as a stochastic
/// yield template. Each LP instantiates it with a per-LP dispersed initial yield
/// so the members' mids diverge realistically.
#[derive(Debug, Clone)]
pub struct FleetInstrument {
    /// The instrument key every LP quotes.
    pub instrument: Instrument,
    /// The yield-model template (its `initial_yield` is the panel centre; each LP
    /// disperses around it — see [`FleetConfig::yield_dispersion`]).
    pub template: YieldModel,
}

/// The knobs a [`fleet`] is built from.
#[derive(Debug, Clone)]
pub struct FleetConfig {
    /// Number of LPs to spin up (`≥ 1`).
    pub lps: usize,
    /// The base quoting params shared by every LP (each LP's `skew` is then
    /// overridden with its centred panel skew — see `skew_step`).
    pub params: LpParams,
    /// The instruments the fleet makes a market in.
    pub instruments: Vec<FleetInstrument>,
    /// The per-LP increment of directional skew (mid units). LP `i` is assigned
    /// `skew = (i − (lps−1)/2)·skew_step`, so the panel is centred: the middle LP
    /// is unbiased, the ends lean opposite ways.
    pub skew_step: f64,
    /// The per-LP dispersion of the initial yield (decimal yield units): LP `i`'s
    /// `initial_yield` is displaced from the template by `yield_dispersion·u`,
    /// with `u ∈ [−1, 1)` a seeded per-LP draw. Decorrelates the starting mids.
    pub yield_dispersion: f64,
}

/// Spin up `cfg.lps` decorrelated LPs from a root `seed`. Reproducible: the same
/// `(seed, cfg)` always yields byte-identical LPs (and hence quote streams).
///
/// LP `i` gets a distinct venue id (`"lp-00"`, `"lp-01"`, …), a
/// [`child_seed`]-derived seed, a centred panel skew, and per-instrument yield
/// models whose initial yield is dispersed by a seeded draw.
#[must_use]
pub fn fleet(seed: u64, cfg: &FleetConfig) -> Vec<SimLp> {
    let n = cfg.lps.max(1);
    let centre = (n as f64 - 1.0) / 2.0;
    (0..n)
        .map(|i| {
            let venue = VenueId::new(format!("lp-{i:02}"));
            let lp_seed = child_seed(seed, i);
            let skew = (i as f64 - centre) * cfg.skew_step;

            let books = cfg
                .instruments
                .iter()
                .map(|fi| {
                    // A per-(LP, instrument) seeded displacement of the initial
                    // yield, keyed the same way the runtime noise is (tick 0).
                    let u = seeded_unit(lp_seed, &venue, &fi.instrument, 0);
                    let mut model = fi.template;
                    model.initial_yield += cfg.yield_dispersion * u;
                    InstrumentModel {
                        instrument: fi.instrument.clone(),
                        mid: MidSource::MeanRevertingYield(model),
                    }
                })
                .collect();

            let params = LpParams { skew, ..cfg.params };
            SimLp::new(venue, lp_seed, params, books)
        })
        .collect()
}

/// Box a collection of LPs into the `Vec<Box<dyn VenueFeed>>` panel the
/// consolidator ([`celnet_aggregation::ConsolidatedBook::consolidate`]) consumes.
#[must_use]
pub fn into_feeds<I>(lps: I) -> Vec<Box<dyn VenueFeed>>
where
    I: IntoIterator<Item = SimLp>,
{
    lps.into_iter()
        .map(|lp| Box::new(lp) as Box<dyn VenueFeed>)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_bond::{AccrualBasis, Bond, PaymentFrequency};
    use celnet_types::{Ccy, CcyPair, Tenor};
    use time::{Date, Month};

    const S: i64 = 1_000_000_000;

    fn instr() -> Instrument {
        Instrument::new(CcyPair::new(Ccy::EUR, Ccy::USD), Tenor::Years(7))
    }

    fn template() -> YieldModel {
        YieldModel {
            bond: Bond::new(
                Date::from_calendar_date(2026, Month::June, 15).unwrap(),
                Date::from_calendar_date(2033, Month::June, 15).unwrap(),
                0.04,
                PaymentFrequency::SemiAnnual,
                AccrualBasis::Thirty360BondBasis,
                100.0,
            )
            .unwrap(),
            long_run_yield: 0.04,
            initial_yield: 0.04,
            reversion_per_sec: 0.01,
            perturbation: 1.0e-4,
        }
    }

    fn cfg(n: usize) -> FleetConfig {
        FleetConfig {
            lps: n,
            params: LpParams::tight(),
            instruments: vec![FleetInstrument {
                instrument: instr(),
                template: template(),
            }],
            skew_step: 0.01,
            yield_dispersion: 5.0e-4,
        }
    }

    #[test]
    fn fleet_is_reproducible_for_same_seed() {
        let a = fleet(99, &cfg(5));
        let b = fleet(99, &cfg(5));
        for (x, y) in a.iter().zip(b.iter()) {
            let qx = x.top_of_book(&instr(), 3 * S).unwrap();
            let qy = y.top_of_book(&instr(), 3 * S).unwrap();
            assert_eq!(qx.bid.to_bits(), qy.bid.to_bits());
            assert_eq!(qx.offer.to_bits(), qy.offer.to_bits());
            assert_eq!(qx.venue, qy.venue);
        }
    }

    #[test]
    fn fleet_members_are_decorrelated() {
        let lps = fleet(3, &cfg(5));
        // Distinct venue ids and distinct mids at a common instant.
        let mids: Vec<u64> = lps
            .iter()
            .map(|lp| lp.top_of_book(&instr(), 5 * S).unwrap().mid().to_bits())
            .collect();
        let unique: std::collections::HashSet<u64> = mids.iter().copied().collect();
        assert_eq!(unique.len(), mids.len(), "fleet mids must be distinct");
    }

    #[test]
    fn fleet_skew_is_centred() {
        let lps = fleet(7, &cfg(5));
        // Middle LP (index 2 of 5) is unskewed; ends lean opposite ways.
        assert_eq!(lps[2].params().skew.to_bits(), 0.0_f64.to_bits());
        assert!(lps[0].params().skew < 0.0);
        assert!(lps[4].params().skew > 0.0);
    }

    #[test]
    fn into_feeds_boxes_the_panel() {
        let feeds = into_feeds(fleet(1, &cfg(4)));
        assert_eq!(feeds.len(), 4);
        assert!(feeds[0].top_of_book(&instr(), 0).is_some());
    }
}
