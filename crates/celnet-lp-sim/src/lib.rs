//! # `celnet-lp-sim` — a fixed-income liquidity-provider (LP) feed simulator
//!
//! A fleet of deterministic-seeded synthetic **bond LPs** we point the FI
//! Aggregated Book at to emulate "various LP connections into the book". Each LP
//! implements [`celnet_aggregation::VenueFeed`], so the fleet flows through the
//! **real** [`celnet_aggregation`] consolidation engine — the same
//! [`ConsolidatedBook`](celnet_aggregation::ConsolidatedBook) that turns N venue
//! top-of-book feeds into one composite book, with staleness decay
//! (`2^{−Δt/τ}`), median-consensus + MAD divergence gating, depth stacking and a
//! confidence measure. This crate never modifies `celnet-aggregation`; it only
//! feeds it.
//!
//! ## Research finding & chosen approach (Step 1 rationale)
//!
//! The codebase already ships a deterministic synthetic top-of-book generator,
//! [`celnet_aggregation::sim::SimVenue`], plus [`celnet_rfq`]'s synthetic
//! `SYNTH-LP-*` responders and the [`celnet_fix`] quote simulator. `SimVenue` is
//! an excellent *FX/cash* generator (a sinusoidal mid drift with per-venue
//! rich/cheap bias and seeded micro-noise), but it is **not** the right thing to
//! extend for this task, for three reasons:
//!
//! 1. **Asset model.** A bond desk quotes off a **yield**, not a price handle; the
//!    controllable knob is the yield, mapped to a clean price by the real
//!    analytics leaf. `SimVenue` has no yield/bond notion.
//! 2. **Ground-truth testability.** The point of *this* crate is a strong,
//!    analytic check of the consolidator. That needs a **ladder** mode where each
//!    LP's bid/offer is exactly specified, so `best_bid = max(fresh bids)` and
//!    `best_offer = min(fresh offers)` can be asserted without recomputing the
//!    stochastic path. `SimVenue` is always stochastic.
//! 3. **Fault injection.** Exercising the consolidator's staleness decay and MAD
//!    gate needs first-class, injectable **staleness** and **outlier/fat-finger**
//!    faults per LP — a testing concern that does not belong inside the
//!    production `celnet-aggregation` core.
//!
//! So — following the task's recommendation — this is a **dedicated crate that
//! depends on `celnet-aggregation`** (path dep) and implements its `VenueFeed`,
//! reusing the house determinism convention (SplitMix64 seeding, `libm`
//! transcendentals; see [`rng`]) rather than reinventing it, and reusing the real
//! [`celnet_bond`] analytics leaf as the yield→price relation rather than
//! re-deriving a bond price. Depending on the real leaf means the simulator's
//! stochastic prices **are** the reference oracle — satisfying the
//! "validate against a reference, never merely assert plausible" guardrail by
//! construction (the unit tests additionally round-trip the price↔yield map
//! through [`celnet_bond::yield_to_maturity`] and confirm monotonicity).
//!
//! ## Two price modes (see [`price`])
//!
//! - **Ground-truth ladder** ([`MidSource::Fixed`]) — an exact, constant mid; an
//!   LP's bid/offer are exact functions of its half-spread and skew, giving the
//!   analytic ground truth the integration test asserts against.
//! - **Stochastic** ([`MidSource::MeanRevertingYield`]) — a seeded mean-reverting
//!   yield (the closed-form Ornstein–Uhlenbeck / Vasicek conditional mean plus a
//!   bounded seeded perturbation) priced to a clean bond price via
//!   [`celnet_bond::clean_price`], for soak/demo realism.
//!
//! ## Public surface
//!
//! - [`SimLp`] — one LP implementing [`VenueFeed`](celnet_aggregation::VenueFeed),
//!   parameterised by venue id, [`LpParams`] (half-spread, directional skew, size,
//!   update cadence, feed latency, quality) and one injectable [`Fault`]
//!   ([`Fault::Stale`] / [`Fault::Outlier`]).
//! - [`fleet`] / [`FleetConfig`] / [`FleetInstrument`] — a factory that spins up N
//!   decorrelated, reproducible LPs; [`into_feeds`] boxes them for the
//!   consolidator.
//!
//! ## Scope note — real-socket LP adapter is future work
//!
//! This crate is library-only and models LPs as in-process `VenueFeed`s. A
//! **FIX-emitting adapter** (a real-socket LP over [`celnet_fix`], as the FIX
//! quote sim does) is deliberately **out of scope** here; it would sit behind the
//! same `VenueFeed` seam — a network `VenueFeed` implementation that returns its
//! most recent tick — so nothing in the consolidator or this simulator's fleet
//! model would need to change to admit it later.

#![forbid(unsafe_code)]

pub mod fleet;
pub mod lp;
pub mod price;
mod rng;

pub use fleet::{FleetConfig, FleetInstrument, fleet, into_feeds};
pub use lp::{Fault, InstrumentModel, LpParams, SimLp};
pub use price::{MidSource, YieldModel};

// Re-export the seam types a caller builds LPs and reads books with, so the crate
// is usable without also reaching into `celnet-aggregation` for the vocabulary.
pub use celnet_aggregation::{
    ConsolidatedBook, ConsolidationConfig, Instrument, VenueFeed, VenueId, VenueQuote,
};
