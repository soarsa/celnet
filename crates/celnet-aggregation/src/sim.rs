//! A real, deterministic-seeded simulated venue feed — the legitimate UAT data
//! source (a genuine synthetic top-of-book generator, not a mock).
//!
//! # What it models
//!
//! Each [`SimVenue`] publishes a top-of-book two-way whose mid evolves as a
//! **deterministic sinusoidal drift** plus **seeded micro-noise**:
//!
//! ```text
//!   mid(t) = base_mid · (1 + bias + amp·sin(2π·t/period + phase) + noise·u(t))
//! ```
//!
//! where `u(t) ∈ [−1, 1)` is drawn from a `SplitMix64` PRNG keyed on
//! `(seed, venue, instrument, tick)`. Because the mid is a pure function of the
//! logical time `t` (given the venue's fixed configuration and seed), the feed is
//! **stateless and O(1)** per query and **exactly reproducible**: two venues built
//! with the same seed and configuration emit byte-identical quotes at every `t`.
//!
//! Multiple venues diverge realistically from one another: a per-venue `bias`
//! makes one venue systematically rich and another cheap, and a per-venue `phase`
//! decorrelates their drift, so N venues produce genuinely diverging two-ways over
//! time — the input a consolidator must reconcile. Per-venue `half_spread`, `size`
//! and `latency_nanos` (which back-dates the observation `ts`, modelling a laggy
//! feed) let a scenario stack a tight fast venue against a wide slow one.
//!
//! # Determinism provenance (doc-only)
//!
//! `SplitMix64` is a well-known, fast, statelessly-seedable mixing PRNG (Steele
//! et al., "Fast Splittable Pseudorandom Number Generators"). The instrument hash
//! uses the standard-library `DefaultHasher` (SipHash with fixed zero keys), which
//! is deterministic across processes and platforms. `sin` is `libm::sin`
//! (correctly-rounded, cross-platform-identical), so the whole feed is
//! reproducible bit-for-bit. Identifiers are purpose-named and vendor-neutral.

use std::f64::consts::PI;
use std::hash::{Hash, Hasher};

use crate::feed::VenueFeed;
use crate::instrument::{Instrument, VenueId, VenueQuote};

/// The configurable dynamics of one simulated venue — the knobs that make N
/// venues produce realistic diverging two-ways.
#[derive(Debug, Clone, PartialEq)]
pub struct SimDynamics {
    /// Absolute half-spread quoted each side of mid (bid = mid − half, offer =
    /// mid + half). Widens the venue's two-way; must be positive.
    pub half_spread: f64,
    /// Firm size quoted on both bid and offer.
    pub size: f64,
    /// Systematic per-venue mid bias as a fraction of `base_mid` (e.g. `+2e-4`
    /// makes this venue quote ~2 bp rich). Drives cross-venue divergence.
    pub bias: f64,
    /// Fractional amplitude of the sinusoidal mid drift (e.g. `1e-3` = ±10 bp).
    pub drift_amp: f64,
    /// Period of the sinusoidal mid drift, in nanoseconds (must be positive).
    pub drift_period_nanos: i64,
    /// Phase offset of the drift sinusoid, in radians — decorrelates venues.
    pub phase: f64,
    /// Seeded micro-noise amplitude as a fraction of `base_mid` (e.g. `1e-4` =
    /// ±1 bp of jitter). The noise is piecewise-constant within a quote tick.
    pub noise: f64,
    /// Quote-tick cadence, in nanoseconds: the mid's seeded noise is resampled
    /// once per tick, so within a tick the top-of-book is stable (a realistic
    /// discretely-updating feed). Must be positive.
    pub tick_nanos: i64,
    /// Feed latency, in nanoseconds: the published observation `ts` is
    /// back-dated by this amount (`ts = now − latency`), so a laggy venue's
    /// quotes are aged and staleness-decayed by the consolidator.
    pub latency_nanos: i64,
    /// The venue's self-reported quality in `[0, 1]`.
    pub quality: f64,
}

impl SimDynamics {
    /// A sensible tight-venue default: a 1-tick spread, 1 M size, no bias, a
    /// modest ±10 bp / 60 s drift with ±1 bp noise on a 100 ms tick, zero latency.
    /// Callers perturb `bias`/`phase`/`half_spread`/`latency_nanos` per venue to
    /// build a diverging panel.
    #[must_use]
    pub fn tight() -> Self {
        Self {
            half_spread: 5.0e-5,
            size: 1_000_000.0,
            bias: 0.0,
            drift_amp: 1.0e-3,
            drift_period_nanos: 60_000_000_000, // 60 s
            phase: 0.0,
            noise: 1.0e-4,
            tick_nanos: 100_000_000, // 100 ms
            latency_nanos: 0,
            quality: 1.0,
        }
    }
}

/// A deterministic-seeded simulated venue. Holds its identity, a seed, its
/// dynamics, and the set of instruments it makes a market in (each with a base
/// mid). Implements [`VenueFeed`] as a pure function of the query time.
#[derive(Debug, Clone)]
pub struct SimVenue {
    venue: VenueId,
    seed: u64,
    dynamics: SimDynamics,
    /// `(instrument, base_mid)` — the venue only quotes these instruments.
    books: Vec<(Instrument, f64)>,
}

impl SimVenue {
    /// Build a simulated venue from its id, seed, dynamics, and its per-instrument
    /// base mids.
    #[must_use]
    pub fn new(
        venue: VenueId,
        seed: u64,
        dynamics: SimDynamics,
        books: Vec<(Instrument, f64)>,
    ) -> Self {
        Self {
            venue,
            seed,
            dynamics,
            books,
        }
    }

    /// The base mid this venue is configured to quote for `instrument`, or `None`
    /// if it makes no market in it.
    fn base_mid(&self, instrument: &Instrument) -> Option<f64> {
        self.books
            .iter()
            .find(|(i, _)| i == instrument)
            .map(|(_, m)| *m)
    }

    /// The seeded mid for `instrument` at logical `now_nanos`.
    fn mid_at(&self, instrument: &Instrument, base_mid: f64, now_nanos: i64) -> f64 {
        let d = &self.dynamics;
        let period = d.drift_period_nanos.max(1) as f64;
        let phase_t = 2.0 * PI * (now_nanos as f64) / period + d.phase;
        let drift = d.drift_amp * libm::sin(phase_t);

        let tick = now_nanos.div_euclid(d.tick_nanos.max(1));
        let noise = d.noise * seeded_unit(self.seed, &self.venue, instrument, tick);

        base_mid * (1.0 + d.bias + drift + noise)
    }
}

impl VenueFeed for SimVenue {
    fn venue(&self) -> &VenueId {
        &self.venue
    }

    fn top_of_book(&self, instrument: &Instrument, now_nanos: i64) -> Option<VenueQuote> {
        let base_mid = self.base_mid(instrument)?;
        let mid = self.mid_at(instrument, base_mid, now_nanos);
        let half = self.dynamics.half_spread;
        Some(VenueQuote {
            venue: self.venue.clone(),
            instrument: instrument.clone(),
            bid: mid - half,
            offer: mid + half,
            bid_size: self.dynamics.size,
            offer_size: self.dynamics.size,
            ts: now_nanos.saturating_sub(self.dynamics.latency_nanos),
            quality: self.dynamics.quality,
        })
    }
}

/// One `SplitMix64` mixing step.
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A stable, cross-process-deterministic `u64` hash of an instrument, via the
/// standard-library `DefaultHasher` (SipHash keyed on fixed zero keys).
fn instrument_hash(instrument: &Instrument) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    instrument.hash(&mut h);
    h.finish()
}

/// A stable `u64` hash of a venue id.
fn venue_hash(venue: &VenueId) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    venue.0.hash(&mut h);
    h.finish()
}

/// A deterministic noise draw in `[−1, 1)` keyed on `(seed, venue, instrument,
/// tick)`. Stateless: identical inputs always yield the identical value.
fn seeded_unit(seed: u64, venue: &VenueId, instrument: &Instrument, tick: i64) -> f64 {
    let mut x = seed;
    x ^= venue_hash(venue).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x = splitmix64(x);
    x ^= instrument_hash(instrument).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x = splitmix64(x);
    x ^= (tick as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    let bits = splitmix64(x);
    // Top 53 bits → a uniform in [0, 1); map to [−1, 1).
    let unit = (bits >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0); // 2^-53
    2.0 * unit - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::{Ccy, CcyPair, Tenor};

    fn eurusd() -> Instrument {
        Instrument::new(CcyPair::new(Ccy::EUR, Ccy::USD), Tenor::Months(3))
    }

    fn venue(id: &str, seed: u64, dyn_: SimDynamics) -> SimVenue {
        SimVenue::new(VenueId::new(id), seed, dyn_, vec![(eurusd(), 1.10)])
    }

    #[test]
    fn same_seed_same_config_is_byte_identical() {
        let a = venue("v", 42, SimDynamics::tight());
        let b = venue("v", 42, SimDynamics::tight());
        for t in [0_i64, 123_456_789, 5_000_000_000, 61_000_000_000] {
            let qa = a.top_of_book(&eurusd(), t).unwrap();
            let qb = b.top_of_book(&eurusd(), t).unwrap();
            assert_eq!(qa.bid.to_bits(), qb.bid.to_bits());
            assert_eq!(qa.offer.to_bits(), qb.offer.to_bits());
            assert_eq!(qa.ts, qb.ts);
        }
    }

    #[test]
    fn different_seed_diverges() {
        let a = venue("v", 1, SimDynamics::tight());
        let b = venue("v", 2, SimDynamics::tight());
        // With noise enabled, distinct seeds must produce distinct mids at some t.
        let differs = (0..10).any(|k| {
            let t = k * 100_000_000;
            a.top_of_book(&eurusd(), t).unwrap().mid().to_bits()
                != b.top_of_book(&eurusd(), t).unwrap().mid().to_bits()
        });
        assert!(differs, "distinct seeds must decorrelate the noise");
    }

    #[test]
    fn latency_back_dates_observation() {
        let mut d = SimDynamics::tight();
        d.latency_nanos = 250_000_000;
        let v = venue("slow", 7, d);
        let q = v.top_of_book(&eurusd(), 1_000_000_000).unwrap();
        assert_eq!(q.ts, 1_000_000_000 - 250_000_000);
    }

    #[test]
    fn quote_is_a_well_formed_two_way() {
        let v = venue("v", 3, SimDynamics::tight());
        let q = v.top_of_book(&eurusd(), 2_000_000_000).unwrap();
        assert!(q.bid < q.offer);
        assert!(q.is_finite());
        assert!(q.bid_size > 0.0 && q.offer_size > 0.0);
    }

    #[test]
    fn no_market_in_unlisted_instrument() {
        let v = venue("v", 3, SimDynamics::tight());
        let other = Instrument::new(CcyPair::new(Ccy::GBP, Ccy::USD), Tenor::Months(3));
        assert!(v.top_of_book(&other, 0).is_none());
    }
}
