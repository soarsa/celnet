//! Fixed-income cash-bond analytics — the pure numeric leaf for a fixed-coupon bond.
//!
//! This crate is step 1 of the FI pricing engine (`docs/fixed-income/FI-PRICING-ENGINE-DESIGN.md` §6.1,
//! `docs/adr/ADR-0018-fixed-income-as-a-new-asset-class-leaf.md`): a **settlement-aware** cash-bond
//! analytics leaf that complements the *spot-starting* relative-value analytics already in
//! [`celnet_rates::bond`] (yield / Z-spread / G-spread / asset-swap spread, which deliberately price
//! at the curve origin with zero accrued). Here we add the pieces a bond desk needs at a real
//! settlement date:
//!
//! - **Accrued interest** from the last coupon to settlement (day-count fraction × coupon).
//! - **Dirty / clean price from a flat street yield** with the fractional between-coupon first period
//!   handled explicitly (the common textbook error — see [`price`]).
//! - **Dirty price from a discount curve** — PV each cashflow at [`celnet_rates::Curve`].
//! - **Yield to maturity** by a safeguarded Newton–Raphson solve with an analytic derivative and a
//!   bisection fallback so a bad start cannot diverge ([`yield_solve`]).
//! - **Risk** — DV01, Macaulay and modified duration, and convexity ([`risk`]).
//!
//! It reuses — and never reinvents — the shared conventions: day-count and year fractions come from
//! [`celnet_rates::AccrualBasis`], month arithmetic and the discount curve from `celnet-rates` /
//! `celnet-calendar`.
//!
//! # Conventions and coordinate systems
//!
//! Two discounting conventions coexist, exactly as on a real desk:
//!
//! - The **street yield** ([`price::dirty_price`]) uses idealised equal coupon periods: a cashflow
//!   `k` periods-and-a-fraction from settlement is discounted by `(1 + y/f)^(-(w + k - 1))`, where
//!   `f` is coupons per year and `w ∈ (0, 1]` is the fraction of the current coupon period still
//!   remaining after settlement. The regular coupon is `coupon_rate / f · redemption`.
//! - **Accrued interest** ([`price::accrued_interest`]) uses the actual **day-count** fraction from
//!   the last coupon to settlement — so `clean = dirty − accrued` blends the idealised yield with the
//!   actual accrual basis, which is standard market practice.
//! - **Curve pricing** ([`price::price_from_curve`]) discounts each cashflow at the curve's own
//!   year-fraction time axis, measured ACT/365F from settlement (the curve reference date), matching
//!   how `celnet-rates` builds its discount-time axis.
//!
//! # Scope of this increment (no stubs — coordinated follow-ups)
//!
//! - **Regular schedule only.** Coupon dates are the regular month-step dates rolled back from
//!   maturity (end-of-month-aware via [`celnet_calendar::add_months`]); the buyer of a bond settling
//!   mid-period receives the full next coupon and compensates the seller through accrued interest.
//!   Odd first/last coupons (long/short stubs), explicit issue/first-coupon dates, EOM and IMM roll,
//!   and business-day adjustment of the coupon dates are out of scope for v1 and rejected or simply
//!   not modelled rather than faked.
//! - **Single (risk-free) discount curve.** Credit spreads / Z-spread discounting live in the
//!   sibling `celnet-credit` leaf (step 2); this leaf is curve-agnostic and takes whatever
//!   [`celnet_rates::Curve`] it is handed.
//!
//! Method/paper provenance lives in prose only — never in identifiers (CLAUDE.md §8). Every public
//! output is validated against an independent oracle (closed-form annuity, published street vectors,
//! and internal identities), not merely asserted plausible; see the crate's test suites.

mod bond;
pub mod price;
pub mod risk;
mod schedule;
pub mod spread;
pub mod yield_solve;

pub use bond::{Bond, BondError};
pub use price::{accrued_interest, clean_price, dirty_price, price_from_curve};
pub use risk::{BondRisk, bond_risk, convexity, dv01, macaulay_duration, modified_duration};
pub use spread::{cs01, spread_duration, z_spread};
pub use yield_solve::yield_to_maturity;

// Re-export the shared convention types a caller needs to build a [`Bond`], so the crate is usable
// without also reaching into `celnet-rates` for them.
pub use celnet_rates::{AccrualBasis, PaymentFrequency};
