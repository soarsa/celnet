//! Celnet latency & throughput benchmark fixtures.
//!
//! This crate turns the microsecond pricing budgets stated in
//! `docs/ARCHITECTURE.md` §1.2 into *reproducible proof*. The benchmark
//! harnesses themselves live under `benches/` (driven by the `divan`
//! micro-benchmark harness); this library hosts the shared, allocation-free
//! input fixtures so that the benches and the unit tests price exactly the same
//! representative workload.
//!
//! ## Budgets under test (`docs/ARCHITECTURE.md` §1.2)
//!
//! | Workload | Target |
//! |---|---|
//! | Vanilla price + full 13-Greek set (cached surface), hot path | p50 ≤ 2 µs, p99 ≤ 10 µs, p99.9 ≤ 25 µs |
//! | Streaming quote throughput | ≥ 1M price updates/s/core |
//!
//! `divan` reports the **median** and **min** per operation; the median is the
//! quantity compared against the p50 ≤ 2 µs budget, while the min approximates
//! the warm-cache floor. The batched bench amortizes a many-strike surface
//! slice and is the basis for the throughput comparison.
//!
//! ## Absolute per-option §1.2 truth-gate
//!
//! `divan`'s median is a central tendency, not a tail. The [`core_load`] module
//! (driven by the `core_load` binary) turns the **absolute** §1.2 ceilings
//! (p50 ≤ 2 µs, p99 ≤ 10 µs, p99.9 ≤ 25 µs) into a *measured, asserted* gate: it
//! times each individual price + full-Greek call into a coordinated-omission-
//! aware [`hdrhistogram::Histogram`] under a low-jitter, priority-elevated,
//! core-pinned regime over the [`sweep_inputs`] working set, and exits non-zero
//! if any measured percentile exceeds its committed budget. The op is ~tens of
//! ns, so the budgets pass with large margin. The [`sched`] module hosts the
//! safe (no-`unsafe`) priority-elevation affordance.
//!
//! ## Fixtures
//!
//! [`representative_inputs`] is a single realistic at-the-money EUR/USD-style
//! option. [`representative_batch`] is a many-strike slice spanning the liquid
//! delta range (deep OTM put wing through deep OTM call wing), which is the unit
//! of work a surface rebuild or a portfolio repricing iterates over. Both are
//! shared verbatim by the benches and by the in-crate unit test so the numbers
//! published as proof are the numbers exercised by the test suite.
//!
//! ## Wire-path latency-under-load (end-to-end proof)
//!
//! The micro-benchmarks above measure the *pinned hot core* in isolation (no
//! wire, ~tens of nanoseconds). The complementary, GA-gating figure is the
//! **wire-path** latency: the round-trip a real counterparty observes when it
//! dials the running service edge over the network and prices an option *while
//! the edge is under sustained streaming + RFQ load*. The [`wire`] module hosts
//! that harness — it spins the real [`celnet_server::Edge`] in-process on an
//! ephemeral loopback port, opens many concurrent RFS streaming subscriptions
//! to load the edge, fires a fixed budget of RFQ round-trips, and records each
//! client-observed round-trip into an [`hdrhistogram::Histogram`]
//! (coordinated-omission aware). It reports p50 / p99 / p99.9 / p99.99 and the
//! achieved throughput, and serializes a [`wire::WireReport`] the CI
//! bench-regression gate compares against a committed baseline.
#![forbid(unsafe_code)]

pub mod core_load;
pub mod fleet_slo;
pub mod sched;
pub mod wire;

use celnet_core::math::ln;
use celnet_types::VanillaInputs;

/// Number of strikes in the representative surface-slice batch.
///
/// Chosen to mirror a liquid FX smile densely sampled across the wings: a
/// realistic per-tenor strike ladder a portfolio/surface rebuild loops over.
pub const BATCH_STRIKES: usize = 64;

/// A single representative at-the-money vanilla option (EUR/USD-style).
///
/// Spot `1.10`, struck at the money, 9.5 vol, 6-month expiry, with a modest
/// positive domestic-over-foreign carry — squarely in the regime the hot-path
/// budget in `docs/ARCHITECTURE.md` §1.2 targets.
#[must_use]
pub fn representative_inputs() -> VanillaInputs {
    VanillaInputs::new(1.10, 1.10, 0.095, 0.5, 0.025, 0.015)
}

/// A representative many-strike surface slice: [`BATCH_STRIKES`] strikes spread
/// across the liquid delta range for one pair/tenor.
///
/// The strikes fan symmetrically around the at-the-money forward, from a deep
/// out-of-the-money put wing to a deep out-of-the-money call wing, which is the
/// shape a surface rebuild or a portfolio repricing iterates over. The vector is
/// built once (outside the timed region) and then priced in a tight loop, so the
/// benchmark measures per-option compute, not allocation.
#[must_use]
pub fn representative_batch() -> Vec<VanillaInputs> {
    let base = representative_inputs();
    let forward = base.forward();
    // Span roughly ±35% in moneyness around the forward — comfortably covering
    // the 10-delta wings of a 6M smile.
    let lo = 0.65;
    let hi = 1.35;
    let n = BATCH_STRIKES;
    (0..n)
        .map(|k| {
            // Linear moneyness ladder in [lo, hi], inclusive of both endpoints.
            let frac = k as f64 / (n as f64 - 1.0);
            let moneyness = lo + (hi - lo) * frac;
            let strike = forward * moneyness;
            // A light smile: vol rises toward the wings (symmetric quadratic in
            // log-moneyness) so wing strikes exercise the full d1/d2 range.
            let lm = ln(strike / forward);
            let vol = base.vol + 0.6 * lm * lm;
            VanillaInputs {
                strike,
                vol,
                ..base
            }
        })
        .collect()
}

/// Number of distinct options in the in-core latency sweep.
///
/// A prime length, deliberately co-prime with the 2-element call/put cycle, so
/// stepping `idx = k % SWEEP_LEN` together with `opt = opts[k % 2]` walks every
/// (option-type, input) combination over the run rather than locking one parity
/// to one input — every sample prices a genuinely different option.
pub const SWEEP_LEN: usize = 257;

/// A deterministic sweep of representative vanilla inputs spanning the liquid
/// regime (varied spot, vol and strike around an at-the-money EUR/USD forward).
///
/// This is the working set the in-core latency truth-gate (`core_load`) cycles
/// through under sustained injection. It varies all three of spot, vol and strike
/// so the optimizer cannot constant-fold `greeks(..)` to a single value, yet
/// stays inside the smooth, in-the-budget regime the §1.2 hot-path target governs
/// (no degenerate near-zero-vol or near-expiry inputs that would price a
/// different, slower branch). Built once, outside any timed region.
#[must_use]
pub fn sweep_inputs() -> Vec<VanillaInputs> {
    let base = representative_inputs();
    let forward = base.forward();
    let n = SWEEP_LEN;
    (0..n)
        .map(|k| {
            let frac = k as f64 / (n as f64 - 1.0); // [0, 1]
            // Spot drifts ±5% across the sweep; strike fans ±25% in moneyness;
            // vol ranges ~7%..~13% — a realistic liquid window, all smooth.
            let spot = base.spot * (0.95 + 0.10 * frac);
            let moneyness = 0.75 + 0.50 * frac;
            let strike = forward * moneyness;
            let vol = 0.07 + 0.06 * frac;
            VanillaInputs {
                spot,
                strike,
                vol,
                ..base
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::OptionType;
    use celnet_vanilla::{greeks, price};

    #[test]
    fn sweep_inputs_is_varied_and_smooth() {
        let sweep = sweep_inputs();
        assert_eq!(sweep.len(), SWEEP_LEN);
        // All three driving inputs genuinely vary across the sweep (so the
        // optimizer cannot fold the priced result to a constant).
        let spots: std::collections::BTreeSet<u64> =
            sweep.iter().map(|i| i.spot.to_bits()).collect();
        let vols: std::collections::BTreeSet<u64> = sweep.iter().map(|i| i.vol.to_bits()).collect();
        let strikes: std::collections::BTreeSet<u64> =
            sweep.iter().map(|i| i.strike.to_bits()).collect();
        assert!(spots.len() > SWEEP_LEN / 2, "spot must vary");
        assert!(vols.len() > SWEEP_LEN / 2, "vol must vary");
        assert!(strikes.len() > SWEEP_LEN / 2, "strike must vary");
        // Every fixture is in the smooth, finite-price regime.
        for i in &sweep {
            assert!(i.spot > 0.0 && i.strike > 0.0 && i.vol > 0.0 && i.t > 0.0);
            let g = greeks(OptionType::Call, i);
            assert!(g.price.is_finite() && g.price >= 0.0);
            assert!(g.vega.is_finite() && g.gamma.is_finite());
        }
    }

    #[test]
    fn batch_builder_is_sane() {
        let batch = representative_batch();
        assert_eq!(batch.len(), BATCH_STRIKES);

        let base = representative_inputs();
        let forward = base.forward();

        // Strikes must be strictly increasing, all positive, and bracket the
        // forward (so the slice genuinely spans both wings).
        let mut prev = f64::NEG_INFINITY;
        let mut saw_below = false;
        let mut saw_above = false;
        for i in &batch {
            assert!(i.strike > 0.0, "strike must be positive");
            assert!(i.strike > prev, "strikes must be strictly increasing");
            assert!(i.vol > 0.0, "vol must be positive");
            assert!(i.t > 0.0, "expiry must be positive");
            prev = i.strike;
            saw_below |= i.strike < forward;
            saw_above |= i.strike > forward;

            // Every fixture must produce finite, non-negative, bounded prices —
            // the same sanity envelope the vanilla property tests enforce.
            let c = price(OptionType::Call, i);
            let p = price(OptionType::Put, i);
            assert!(c.is_finite() && c >= 0.0 && c <= i.spot * i.df_for() + 1e-12);
            assert!(p.is_finite() && p >= 0.0 && p <= i.strike * i.df_dom() + 1e-12);

            // The full Greek pass must also be finite for every fixture.
            let g = greeks(OptionType::Call, i);
            for v in [
                g.price,
                g.delta_spot,
                g.delta_forward,
                g.gamma,
                g.vega,
                g.theta,
                g.rho_dom,
                g.rho_for,
                g.vanna,
                g.volga,
                g.charm,
                g.speed,
                g.zomma,
                g.color,
            ] {
                assert!(v.is_finite(), "every Greek must be finite");
            }
        }
        assert!(saw_below && saw_above, "batch must bracket the forward");
    }

    #[test]
    fn representative_inputs_is_atm() {
        let i = representative_inputs();
        // ATM-struck-on-spot: strike equals spot for this fixture.
        celnet_core::assert_close!(i.strike, i.spot, 0.0, 0.0);
        // Call and put are both strictly positive and finite.
        assert!(price(OptionType::Call, &i) > 0.0);
        assert!(price(OptionType::Put, &i) > 0.0);
    }
}
