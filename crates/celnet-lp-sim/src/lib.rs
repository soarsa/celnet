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
//! ## Named simulated counterparties (see [`roster`])
//!
//! The panel is **not** an anonymous `LP-SIM-01…0N` fleet. Every member is a named
//! simulated counterparty from [`OTC_ROSTER`] — `marketaccess-sim`,
//! `traderweb-sim`, `citigroup-sim`, `jpm-sim` — each with its own persistent,
//! reproducible pricing personality (spread, directional axe, size appetite, quoted
//! depth, response latency, refresh cadence). A booked fill therefore stamps a name
//! a trader recognises into `HedgeProvenance.lp_won`, and the counterparties really
//! do fill different amounts of the same order.
//!
//! ## Orders and fills (see [`execution`] and [`orders`])
//!
//! The simulators do not only publish prices: they **take orders and report fills**
//! over the same FIX `NewOrderSingle(D)` → `ExecutionReport(8)` contract the rest of
//! the estate speaks. [`execution`] is the pure matching core (market / limit /
//! previously-quoted, `TimeInForce` FOK and IOC, per-counterparty quoted depth);
//! [`orders`] binds it to a real socket. Every rejection and every partial fill
//! carries a machine-readable reason — nothing is ever dropped silently.
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
//! - [`OrderVenue`] / [`serve_orders`] — the FIX order acceptor a simulator binds so
//!   a taker can actually trade against the prices it streams.
//!
//! ## Scope note — listed futures live in their own simulator
//!
//! Listed Treasury futures used to be priced here, as one more arm of the generic
//! LP feed. They are now a **dedicated venue simulator** (`celnet-cme-sim`), because
//! a listed contract is not a dealer quote: it has one central market rather than a
//! panel, it trades in whole contracts on a published tick grid, and it rolls across
//! delivery months. This crate keeps the over-the-counter universe (cash government
//! bonds, the swap/OIS curve and the listed STIR strip anchored to it) and lends the
//! futures simulator its shared price/quote vocabulary.

#![forbid(unsafe_code)]

pub mod books;
pub mod credentials;
pub mod execution;
pub mod fleet;
pub mod lp;
pub mod lpsim;
pub mod net;
pub mod ois;
pub mod orders;
pub mod price;
pub mod quoted;
mod rng;
pub mod roster;
pub mod stir;
pub mod universe;

pub use books::{
    BookScope, BookView, StreamDiff, StreamKey, StreamPlan, resolve_from_descs, resolve_plan,
};
pub use credentials::{CredentialError, ServiceCredentials};
pub use execution::{
    DepthLadder, DepthLevel, Execution, Fill, OrderRequest, OrderType, PartialReason, RejectReason,
    Side, TimeInForce, VenueRules, execute,
};
pub use fleet::{FleetConfig, FleetInstrument, fleet, into_feeds};
pub use lp::{Fault, InstrumentModel, LpParams, SimLp};
pub use lpsim::{
    BondComposite, DEFAULT_LP_NAME, LpQuoteSnapshot, LpSimConfig, build_fleet, composite_for,
    depth_for, quotable_lines,
};
pub use net::{BookFeedOptions, FaultSchedule, LoginCredentials, run_book_aware_feed};
pub use ois::{OisCurvePoint, USD_OIS_CURVE, load_ois_universe, ois_instrument_id, ois_lines};
pub use orders::{OrderVenue, QuotedMarket, run_order_acceptor, serve_orders};
pub use price::{MidSource, RateModel, YieldModel};
pub use quoted::QuotedLine;
pub use roster::{LISTED_ROSTER, OTC_ROSTER, SimLpProfile, VenueArchetype};
pub use stir::{StirContract, load_stir_universe, stir_lines};
pub use universe::{
    SecurityType, TreasuryBond, bond_lines, load_coupon_universe, load_curated_universe,
    load_government_universe, load_universe, parse_universe,
};

// Re-export the seam types a caller builds LPs and reads books with, so the crate
// is usable without also reaching into `celnet-aggregation` for the vocabulary.
pub use celnet_aggregation::{
    ConsolidatedBook, ConsolidationConfig, Instrument, VenueFeed, VenueId, VenueQuote,
};
