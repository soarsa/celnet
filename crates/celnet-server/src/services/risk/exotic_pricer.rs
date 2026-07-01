//! The server-side [`ExoticLegPricer`] implementation: the concrete
//! `celnet-exotics` closed forms behind the risk cube's injected exotic seam.
//!
//! The risk cube (`celnet-risk-cube`) names a booked exotic by
//! [`celnet_types::ExoticKind`] and re-prices it under scenario shocks, but does
//! **not** depend on the heavy `celnet-exotics` pricing crate — the dependency is
//! inverted through the [`celnet_core::ExoticLegPricer`] trait (arch-program item E,
//! `docs/INTERFACES.md` one-way edges). The server already owns the exotics estate,
//! so it implements that trait here and injects it into every cube reducer that
//! re-prices an exotic leg (additive leaf build, non-additive VaR/ES, curvature,
//! FRTB vega buckets).
//!
//! The arithmetic is the **identical** `celnet-exotics` closed form the cube
//! formerly called directly — the same `VanillaInputs → ExoticInputs` projection
//! (`From<&VanillaInputs>`, byte-identical FX two-rate accessors) and the same
//! `single_barrier_price` / `digital_price` / `digital_greeks` calls in the same
//! order — so the risk output is byte-for-byte unchanged.

use celnet_core::ExoticLegPricer;
use celnet_exotics::{ExoticInputs, digital_greeks, digital_price, single_barrier_price};
use celnet_types::{DigitalKind, ExoticKind, VanillaInputs};

/// The concrete exotic-leg pricer over the `celnet-exotics` closed forms.
///
/// Zero-sized: it holds no state, so a `&ExoticEngine` is a free seam to pass into
/// the cube's reducers exactly alongside the carry pricer.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExoticEngine;

impl ExoticLegPricer for ExoticEngine {
    #[inline]
    fn unit_price(&self, kind: ExoticKind, inputs: &VanillaInputs) -> f64 {
        let i: ExoticInputs = inputs.into();
        match kind {
            ExoticKind::SingleBarrier(spec) => single_barrier_price(&i, spec),
            ExoticKind::Digital(k) => digital_price(k, &i),
        }
    }

    #[inline]
    fn digital_greeks(&self, kind: DigitalKind, inputs: &VanillaInputs) -> (f64, f64, f64) {
        let dg = digital_greeks(kind, &inputs.into());
        (dg.delta, dg.gamma, dg.vega)
    }
}
