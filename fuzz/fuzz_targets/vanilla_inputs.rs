//! Fuzz target: arbitrary bytes -> valid-but-adversarial `VanillaInputs` ->
//! `celnet_vanilla::price` + `celnet_vanilla::greeks`, asserting the public
//! contract holds under any in-domain input:
//!
//!   * the call never panics (no `unwrap` on `NaN`, no divide-by-zero blow-up,
//!     no `ln`/`sqrt` of a non-positive that escapes as a panic), and
//!   * every produced number is finite (no `NaN`/`±inf` leaking out of the
//!     pricer or the 14-Greek set).
//!
//! The fuzzer drives a structured decoder (via `arbitrary`) rather than raw
//! field bytes, so it spends its budget on *in-domain* adversarial corners —
//! deep ITM/OTM strikes, near-zero and very long expiries, tiny and huge vols,
//! and large rate differentials — instead of trivially-rejected garbage. This
//! is the Garman-Kohlhagen domain the pricer is contracted for: strictly
//! positive spot, strike, vol and time; finite, bounded rates.
//!
//! Run (Linux nightly):
//!   cargo +nightly fuzz run vanilla_inputs -- -max_total_time=120

#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

use celnet_types::{OptionType, VanillaInputs};

/// A bounded, in-domain draw for the Garman-Kohlhagen pricer.
///
/// Each field is mapped from a raw `f64` into the half-open interval the model
/// is defined on, so the fuzzer cannot waste iterations on inputs the API does
/// not promise to handle (zero/negative spot etc.). The *interesting* stress is
/// the adversarial spread *within* the valid domain.
///
/// `Debug` is required by `libfuzzer-sys` 0.4 so a triggering input can be
/// printed on a crash for reproduction.
#[derive(Debug)]
struct Draw {
    is_call: bool,
    inputs: VanillaInputs,
}

/// Map an arbitrary finite-or-not `f64` into `[lo, hi]`, treating NaN/inf as the
/// midpoint so a degenerate draw still produces a legal-domain value.
fn clamp_into(raw: f64, lo: f64, hi: f64) -> f64 {
    let mid = 0.5 * (lo + hi);
    if !raw.is_finite() {
        return mid;
    }
    // Squash through tanh so the full f64 range folds smoothly into [lo, hi];
    // this keeps extreme magnitudes mapping to the domain edges (the corners we
    // most want to probe) without ever escaping the interval.
    let t = 0.5 * (libm::tanh(raw) + 1.0); // (0, 1)
    lo + t * (hi - lo)
}

impl<'a> Arbitrary<'a> for Draw {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let is_call = bool::arbitrary(u)?;
        // Strictly positive spot/strike/vol/time; finite bounded rates. Bounds
        // span the practical FX-options envelope plus an adversarial margin.
        let spot = clamp_into(f64::arbitrary(u)?, 1e-6, 1e6);
        let strike = clamp_into(f64::arbitrary(u)?, 1e-6, 1e6);
        let vol = clamp_into(f64::arbitrary(u)?, 1e-6, 50.0); // up to 5000 vol
        let t = clamp_into(f64::arbitrary(u)?, 1e-9, 100.0); // ~0 to a century
        let r_dom = clamp_into(f64::arbitrary(u)?, -1.0, 1.0);
        let r_for = clamp_into(f64::arbitrary(u)?, -1.0, 1.0);
        Ok(Draw {
            is_call,
            inputs: VanillaInputs::new(spot, strike, vol, t, r_dom, r_for),
        })
    }
}

fuzz_target!(|draw: Draw| {
    let opt = if draw.is_call {
        OptionType::Call
    } else {
        OptionType::Put
    };
    let i = &draw.inputs;

    // Contract 1: pricing never panics; the premium is finite.
    let px = celnet_vanilla::price(opt, i);
    assert!(px.is_finite(), "non-finite price {px} for {i:?} ({opt:?})");
    // A vanilla premium is non-negative.
    assert!(px >= 0.0, "negative price {px} for {i:?} ({opt:?})");

    // Contract 2: the full 14-Greek pass never panics; every Greek is finite.
    let g = celnet_vanilla::greeks(opt, i);
    for (name, v) in [
        ("price", g.price),
        ("delta_spot", g.delta_spot),
        ("delta_forward", g.delta_forward),
        ("gamma", g.gamma),
        ("vega", g.vega),
        ("theta", g.theta),
        ("rho_dom", g.rho_dom),
        ("rho_for", g.rho_for),
        ("vanna", g.vanna),
        ("volga", g.volga),
        ("charm", g.charm),
        ("speed", g.speed),
        ("zomma", g.zomma),
        ("color", g.color),
    ] {
        assert!(
            v.is_finite(),
            "non-finite greek {name}={v} for {i:?} ({opt:?})"
        );
    }
});
