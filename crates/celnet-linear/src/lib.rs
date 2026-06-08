//! # celnet-linear — the linear FX book
//!
//! First-class, exact, closed-form pricers for the **linear** (non-optional) FX
//! products: the outright [`forward`], the FX [`swap`] (near + far legs), and the
//! non-deliverable forward ([`ndf`]). These are discounted-cashflow identities,
//! not Garman-Kohlhagen option payoffs, so they live in their own leaf crate
//! rather than as arms of the FX option leaf (per `docs/W2-LINEAR-PLAN.md` §1 and
//! ADR-0008's agnostic-payoff layering).
//!
//! ## Asset-class-agnostic by construction (ADR-0008)
//!
//! Every pricer here forms the forward and the discount **only** through the
//! carry producer — [`celnet_types::Carry::forward_factor`] /
//! [`celnet_types::Carry::discount_df`], surfaced as
//! [`LinearInputs::forward`] / [`LinearInputs::discount_df`] — and **never**
//! matches on the [`celnet_types::Carry`] variant. A forward's PV is
//! `side · notional · df · (F − K)` with `F = spot · forward_factor(t)`; this is
//! asset-class-neutral, so an NDF or forward on a (future) metal or digital-asset
//! underlying reuses the identical engine unchanged. Any `match carry { … }`
//! branch in a pricer would be a review blocker (the no-workaround test).
//!
//! ## Exactness
//!
//! Linear PVs and their Greeks are exact analytic closed forms — there is no
//! Monte-Carlo and hence **no `price_std_error`**. The independent oracle (in the
//! `#[cfg(test)]` modules) reaches each reference by a route that shares no
//! intermediate with the production discounted-cashflow algebra, plus structural
//! and limit gates that can disagree (fair-forward ⇒ PV 0, linearity, netting,
//! `t → 0` ⇒ undiscounted intrinsic, NDF == deliverable-forward PV).
//!
//! ## Honest boundary (validation scope — VERIFICATION-CONTRACT §g)
//!
//! The in-repo proof is the **payoff / discounted-cashflow math and the
//! settlement convention identity**. **Live NDF fixing VALUES and metal
//! lease-rate VALUES are ENV** — only the fixing *identity* ([`FixingSource`])
//! and the settlement convention are encoded in-repo; the realized fixing rate
//! and live lease rate are estate-gated market-data feeds and are never sourced
//! from this repository.

pub mod forward;
pub mod inputs;
pub mod ndf;
pub mod swap;

pub use forward::{ForwardGreeks, fair_forward, greeks};
pub use inputs::{LinearInputError, LinearInputs, LinearTerms, Side};
pub use ndf::Ndf;
pub use swap::{SwapError, SwapLegRates, leg_rates, swap_points};

// Re-exported for downstream callers that want the fixing identity vocabulary
// alongside the NDF pricer without depending on `celnet-types` directly.
pub use celnet_types::FixingSource;
