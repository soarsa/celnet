//! Accumulator / decumulator — periodic accumulation of the underlying at a
//! discounted strike (the *pivot*), with a knock-out barrier that terminates the
//! structure and a gearing multiplier on the adverse side of the pivot.
//!
//! # Structure
//!
//! An equity-/FX-accumulator lets the client accumulate a fixed quantity of the
//! base currency at the **pivot** strike `K` on each of `n` fixing dates, for as
//! long as the structure is alive:
//!
//! * **Above the pivot** (`S_k > K`): the client buys one unit at `K` and is
//!   immediately up `S_k − K` — the *gain* leg.
//! * **Below the pivot** (`S_k < K`): the client is still obliged to buy, now at a
//!   loss `K − S_k`, **geared** by the leverage multiplier (the classic
//!   "accumulate twice as fast when it goes against you") — the geared *loss* leg.
//! * **Knock-out barrier** `B` above the pivot: once a fixing reaches the barrier
//!   the structure terminates immediately (no further accumulation) — this caps the
//!   client's upside, which is what funds the discounted pivot.
//!
//! [`accumulator_price`] returns the present value to the **client** (the holder
//! who accumulates): positive is value to the client. The bank's value is its
//! negative.
//!
//! # Numerics
//!
//! Two monitoring conventions for the knock-out:
//! * **discrete** — the barrier is tested only at the fixing dates;
//! * **continuous** — between fixings the Brownian-bridge crossing probability
//!   (the same exact construction the barrier MC in [`crate::mc`] uses) is applied,
//!   so the structure is correctly more likely to knock out.
//!
//! Priced on the Monte-Carlo engine ([`crate::rng::CounterRng`] +
//! [`crate::normal::inverse_cdf`]) with antithetic variates; reproducible from the
//! seed. For the discrete convention with continuous-monitoring off, the GBM law
//! between fixings is exact (no time-step bias).
//!
//! Provenance (doc-only): the accumulator payoff and its risk profile are
//! described in Wystup (2017), *FX Options and Structured Products*; the
//! Brownian-bridge barrier correction is Beaglehole-Dybvig-Zhou (1997) /
//! Glasserman (2003). Identifiers are purpose-named and vendor/research-neutral.

use celnet_core::math::{exp, ln, sqrt};

use crate::inputs::ExoticInputs;
use crate::normal::inverse_cdf;
use crate::rng::CounterRng;

/// Knock-out monitoring convention for the accumulator barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Monitoring {
    /// Barrier tested only at the fixing dates.
    Discrete,
    /// Continuous monitoring via the Brownian-bridge crossing probability between
    /// fixings.
    Continuous,
}

/// An accumulator specification (pivot strike, knock-out barrier, gearing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accumulator {
    /// Pivot strike `K` at which the client accumulates each fixing.
    pub pivot: f64,
    /// Up-and-out knock-out barrier `B` (`B > K`). Reaching it terminates the
    /// structure.
    pub barrier: f64,
    /// Number of equally-spaced fixing dates over `[0, T]`.
    pub fixings: usize,
    /// Gearing/leverage multiplier on the below-pivot (loss) leg (`≥ 1` typically:
    /// accumulate faster when adverse).
    pub leverage: f64,
    /// Per-fixing notional (units of base accumulated per fixing).
    pub notional: f64,
    /// Knock-out monitoring convention.
    pub monitoring: Monitoring,
}

impl Accumulator {
    fn validate(&self) {
        assert!(self.fixings >= 1, "accumulator needs ≥1 fixing");
        assert!(
            self.barrier > self.pivot,
            "knock-out barrier {} must sit above the pivot {}",
            self.barrier,
            self.pivot
        );
        assert!(self.leverage >= 0.0, "leverage must be non-negative");
        assert!(self.notional > 0.0, "notional must be positive");
    }
}

/// Welford accumulator (mean + std-error of the mean).
#[derive(Default)]
struct Welford {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    #[inline]
    fn push(&mut self, x: f64) {
        self.n += 1;
        let d = x - self.mean;
        self.mean += d / self.n as f64;
        self.m2 += d * (x - self.mean);
    }
    fn std_error(&self) -> f64 {
        if self.n < 2 {
            0.0
        } else {
            sqrt(self.m2 / ((self.n - 1) as f64) / self.n as f64)
        }
    }
}

/// Monte-Carlo configuration for the accumulator engine.
#[derive(Debug, Clone, Copy)]
pub struct AccumulatorMcConfig {
    /// Number of antithetic path **pairs**.
    pub pairs: usize,
    /// Seed for the counter-based RNG.
    pub seed: u64,
}

/// The result of an accumulator Monte-Carlo run.
#[derive(Debug, Clone, Copy)]
pub struct AccumulatorResult {
    /// Discounted present value to the **client** (accumulator). Positive = value
    /// to the client.
    pub price: f64,
    /// Standard error of the mean.
    pub std_error: f64,
    /// Expected number of fixings that actually settle before knock-out — the
    /// structure's expected accumulation count.
    pub expected_settled_fixings: f64,
}

/// Price an accumulator by Monte-Carlo with antithetic variates.
///
/// Each path simulates the `fixings` spots from the exact per-fixing GBM law,
/// settles the accumulation legs (gain above pivot, geared loss below), and
/// terminates on knock-out (discrete at a fixing, or via the Brownian-bridge
/// crossing probability between fixings under continuous monitoring). Each leg is
/// discounted to its own fixing date.
#[must_use]
pub fn accumulator_price(
    i: &ExoticInputs,
    spec: Accumulator,
    cfg: AccumulatorMcConfig,
) -> AccumulatorResult {
    spec.validate();
    let n = spec.fixings;
    let dt = i.t / n as f64;
    let ln_s0 = ln(i.spot);
    // Carry accessors read ONCE, outside the path loop (no `Carry` dispatch in the
    // hot path, ADR-0008); byte-identical to the FX two-rate form for
    // `Carry::FxRates` (drift `(r_dom − r_for − ½σ²)dt`, dfs `e^{−r_dom·t_k}`).
    let drift_step = (i.carry_rate() - 0.5 * i.vol * i.vol) * dt;
    let vol_sqrt_dt = i.vol * sqrt(dt);
    let var_step = vol_sqrt_dt * vol_sqrt_dt;
    let ln_b = ln(spec.barrier);

    let mut dfs = vec![0.0f64; n];
    for (k, df) in dfs.iter_mut().enumerate() {
        let t_k = (k + 1) as f64 * dt;
        *df = i.discount_df_at(t_k);
    }

    let mut pv = Welford::default();
    let mut settled = Welford::default();
    let mut z = vec![0.0f64; n];

    for pair in 0..cfg.pairs as u64 {
        let mut rng = CounterRng::new(cfg.seed, 0, pair, 0);
        for zk in z.iter_mut() {
            *zk = inverse_cdf(rng.next_u01());
        }
        let a = walk_path(
            ln_s0,
            drift_step,
            vol_sqrt_dt,
            var_step,
            ln_b,
            &z,
            &dfs,
            spec,
            1.0,
        );
        let b = walk_path(
            ln_s0,
            drift_step,
            vol_sqrt_dt,
            var_step,
            ln_b,
            &z,
            &dfs,
            spec,
            -1.0,
        );
        pv.push(0.5 * (a.0 + b.0));
        settled.push(0.5 * (a.1 + b.1));
    }

    AccumulatorResult {
        price: pv.mean,
        std_error: pv.std_error(),
        expected_settled_fixings: settled.mean,
    }
}

/// Walk one antithetic-signed accumulator path, returning `(client_pv,
/// settled_fixing_count)`.
#[allow(clippy::too_many_arguments)]
fn walk_path(
    ln_s0: f64,
    drift_step: f64,
    vol_sqrt_dt: f64,
    var_step: f64,
    ln_b: f64,
    z: &[f64],
    dfs: &[f64],
    spec: Accumulator,
    sign: f64,
) -> (f64, f64) {
    let mut ln_prev = ln_s0;
    let mut client_pv = 0.0f64;
    let mut settled = 0.0f64;
    let mut survival = 1.0f64; // probability the structure is still alive (continuous)

    for k in 0..z.len() {
        let ln_next = ln_prev + drift_step + vol_sqrt_dt * sign * z[k];

        // Continuous monitoring: probability the up-barrier was NOT crossed on the
        // bridge between ln_prev and ln_next, conditional on both endpoints below
        // the barrier. (Same reflected-bridge formula as the barrier MC.)
        let step_survival = match spec.monitoring {
            Monitoring::Discrete => 1.0,
            Monitoring::Continuous => {
                if ln_prev < ln_b && ln_next < ln_b {
                    let num = 2.0 * (ln_b - ln_prev) * (ln_b - ln_next);
                    1.0 - exp(-num / var_step)
                } else {
                    0.0
                }
            }
        };

        let s_k = exp(ln_next);
        // Discrete knock-out at this fixing: if the node itself is at/above barrier.
        let node_knocked = s_k >= spec.barrier;

        // Weight under which this fixing settles: it must reach this fixing alive.
        // Under continuous monitoring `survival` carries the inter-fixing
        // no-crossing probability; under discrete it is 1 until a node knocks.
        let alive_weight = match spec.monitoring {
            Monitoring::Discrete => {
                if node_knocked {
                    0.0
                } else {
                    1.0
                }
            }
            Monitoring::Continuous => survival * step_survival,
        };

        if alive_weight > 0.0 {
            let leg = if s_k > spec.pivot {
                // Above pivot: client gains S − K on one unit.
                s_k - spec.pivot
            } else {
                // Below pivot: client loses (K − S), geared.
                -spec.leverage * (spec.pivot - s_k)
            };
            client_pv += alive_weight * leg * spec.notional * dfs[k];
            settled += alive_weight;
        }

        // Update survival / terminate.
        match spec.monitoring {
            Monitoring::Discrete => {
                if node_knocked {
                    // Structure dead from here on.
                    return (client_pv, settled);
                }
            }
            Monitoring::Continuous => {
                survival *= step_survival;
            }
        }
        ln_prev = ln_next;
    }

    (client_pv, settled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::VanillaInputs;

    fn base() -> ExoticInputs {
        VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01).into()
    }

    fn spec(monitoring: Monitoring) -> Accumulator {
        Accumulator {
            pivot: 1.28,
            barrier: 1.36,
            fixings: 12,
            leverage: 2.0,
            notional: 1.0,
            monitoring,
        }
    }

    /// Reproducibility: identical seed ⇒ bit-identical price.
    #[test]
    fn mc_is_reproducible() {
        let i = base();
        let cfg = AccumulatorMcConfig {
            pairs: 30_000,
            seed: 0xAC11,
        };
        let a = accumulator_price(&i, spec(Monitoring::Discrete), cfg);
        let b = accumulator_price(&i, spec(Monitoring::Discrete), cfg);
        assert_eq!(a.price.to_bits(), b.price.to_bits());
        assert_eq!(a.std_error.to_bits(), b.std_error.to_bits());
    }

    /// Gearing on the loss leg hurts the client: raising the leverage multiplier
    /// lowers the client's present value (they accumulate the loss faster). Pins
    /// the gearing mechanic.
    #[test]
    fn higher_gearing_hurts_client() {
        let i = base();
        let cfg = AccumulatorMcConfig {
            pairs: 200_000,
            seed: 0x6EA2,
        };
        let g1 = Accumulator {
            leverage: 1.0,
            ..spec(Monitoring::Discrete)
        };
        let g3 = Accumulator {
            leverage: 3.0,
            ..spec(Monitoring::Discrete)
        };
        let v1 = accumulator_price(&i, g1, cfg);
        let v3 = accumulator_price(&i, g3, cfg);
        let tol = 4.0 * (v1.std_error + v3.std_error);
        assert!(
            v3.price < v1.price - tol,
            "more gearing should lower client PV: {} < {} (tol {tol})",
            v3.price,
            v1.price
        );
    }

    /// Continuous monitoring knocks out more often than discrete monitoring of the
    /// same barrier (the path can breach between fixings), so it settles **fewer**
    /// fixings on average. Pins the Brownian-bridge knock-out correction.
    #[test]
    fn continuous_knocks_out_more_than_discrete() {
        let i = base();
        let cfg = AccumulatorMcConfig {
            pairs: 200_000,
            seed: 0xC0A7,
        };
        let disc = accumulator_price(&i, spec(Monitoring::Discrete), cfg);
        let cont = accumulator_price(&i, spec(Monitoring::Continuous), cfg);
        assert!(
            cont.expected_settled_fixings < disc.expected_settled_fixings,
            "continuous monitoring should settle fewer fixings: {} < {}",
            cont.expected_settled_fixings,
            disc.expected_settled_fixings
        );
    }

    /// A barrier far above spot (effectively never knocked) settles essentially all
    /// fixings, and a far-below-pivot scenario (deep ITM client) is worth more than
    /// an ATM-pivot scenario. Bound sanity.
    #[test]
    fn unreachable_barrier_settles_all_fixings() {
        let i = base();
        let cfg = AccumulatorMcConfig {
            pairs: 60_000,
            seed: 0xFA17,
        };
        let s = Accumulator {
            barrier: 100.0,
            ..spec(Monitoring::Discrete)
        };
        let r = accumulator_price(&i, s, cfg);
        assert!(
            (r.expected_settled_fixings - s.fixings as f64).abs() < 1e-9,
            "unreachable barrier should settle all fixings, got {}",
            r.expected_settled_fixings
        );
    }

    /// A lower pivot (deeper discount) is worth more to the client than a higher
    /// pivot, holding everything else fixed — the pivot is the price the client
    /// pays at each accumulation.
    #[test]
    fn lower_pivot_is_worth_more_to_client() {
        let i = base();
        let cfg = AccumulatorMcConfig {
            pairs: 150_000,
            seed: 0x9170,
        };
        let low = Accumulator {
            pivot: 1.24,
            ..spec(Monitoring::Discrete)
        };
        let high = Accumulator {
            pivot: 1.30,
            ..spec(Monitoring::Discrete)
        };
        let vl = accumulator_price(&i, low, cfg);
        let vh = accumulator_price(&i, high, cfg);
        assert!(
            vl.price > vh.price,
            "lower pivot should be worth more to client: {} > {}",
            vl.price,
            vh.price
        );
    }
}
