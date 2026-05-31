//! Celnet convention canonicalization + common-numeraire conversion
//! (`docs/RISK-HIERARCHY.md` §2.2/§2.3, `docs/EXPERIENCE-ARCHITECTURE.md` P2-6).
//!
//! # Why this crate exists
//!
//! Celnet's risk **cube** (`docs/RISK-HIERARCHY.md` §2.1) rolls atomic per-position
//! Greeks up an organizational hierarchy (trader → book → desk → ccy-pair →
//! location → entity → firm). The cube above the leaf is **convention-free**: a
//! roll-up is a plain group-by + sum. That is only valid because every position's
//! risk has first been re-derived into **one canonical convention** and a single
//! **reporting numeraire**. This crate is that single place where convention and
//! numeraire live; everything above it adds raw numbers.
//!
//! Two transforms, both **pure, deterministic, `libm`-routed** (no IO, no clock,
//! no allocation on the conversion path beyond the leg vector):
//!
//! 1. **Convention canonicalization** ([`canonicalize`]) — §2.2. Two books may
//!    both report "delta 0.25" under *different* delta conventions; adding them is
//!    meaningless. Given a position's full pricing inputs and its quoted
//!    conventions, we re-derive its risk into the canonical internal convention:
//!    **spot-unadjusted delta, premium excluded**, with the premium carried as a
//!    *separate* monetary line so premium-adjusted views remain reconstructable.
//!    The result is a [`CanonicalLeaf`] — a convention-free fact the cube can sum.
//!
//! 2. **Common-numeraire conversion** ([`CanonicalLeaf::currency_exposure`] +
//!    [`Numeraire`]) — §2.3. Delta is a *currency amount*, not a scalar: for
//!    EURUSD it is a EUR (CCY1) hedge amount; for USDJPY a USD amount. To net
//!    across books and pairs you resolve every position into a **per-currency
//!    exposure vector** (one signed amount per currency leg) and convert to a
//!    chosen **reporting numeraire** at spot. Vega is normalized to a 1-vol-point
//!    move and, because vega P&L is in **premium-currency terms**, requires the
//!    *same* premium-currency normalization as delta before cross-book netting
//!    (§2.2 and §2.3 are coupled — `vega_premium_ccy` carries the premium ccy so
//!    the conversion is correct).
//!
//! # What this crate does NOT do (honest scope)
//!
//! - It does **not** aggregate. Summing canonical leaves across the hierarchy is
//!   `celnet-risk-cube`'s job; this crate only produces the convention-free,
//!   numeraire-resolved leaf and the reusable conversion primitives.
//! - It does **not** model **cross-pair correlation / vol-triangulation** sign
//!   resolution (§2.4) — that is a netting-time concern owned by the cube's
//!   non-additive reducers, and is deliberately left out here so this crate stays
//!   a pure leaf transform.
//! - It does **not** fetch spot/cross rates. Reporting-numeraire conversion takes
//!   an explicit [`SpotResolver`] supplied by the caller (the cube wires it to the
//!   live or IPV-pinned surface), so this crate has no market-data dependency and
//!   stays deterministic for a given resolver.
//!
//! # Provenance
//!
//! Spot/forward and premium-adjusted delta formulae follow standard FX-options
//! practice (Wystup, *FX Options and Structured Products*, 2nd ed., §1.5;
//! `docs/CONVENTIONS.md`). The canonical-convention choice (spot-unadjusted,
//! premium-excluded) is the Celnet design decision recorded in
//! `docs/RISK-HIERARCHY.md` §2.2. Provenance lives in doc comments only; no
//! method/person name appears in an identifier (guardrail #8).

#![forbid(unsafe_code)]

mod leaf;
mod numeraire;

pub use leaf::{CanonicalGreeks, CanonicalLeaf, PositionRisk, canonicalize};
pub use numeraire::{
    CcyExposure, CurrencyExposure, MonetaryAmount, Numeraire, NumeraireError, SpotResolver,
    StaticSpotResolver,
};
