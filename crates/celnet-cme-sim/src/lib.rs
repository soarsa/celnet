//! # `celnet-cme-sim` — the listed **futures venue** simulator
//!
//! A single simulated exchange, connection id [`VENUE_ID`] (`cme-sim`), that quotes
//! the Treasury futures complex and trades against its own quotes.
//!
//! ## Why this is its own simulator
//!
//! Listed futures used to be one more arm of the generic LP feed
//! (`celnet-lp-sim::futures`), quoted by the same anonymous member panel that quoted
//! cash bonds. That was wrong in three concrete ways, each of which showed up as
//! something an operator could see:
//!
//! 1. **A listed contract has one market, not a panel.** Four competing "dealers"
//!    quoting the same exchange-listed contract is not a market structure that
//!    exists. Worse, the four members' price dispersion had to be squeezed to a
//!    tenth of a tick to stop the panel's best bid printing through its best offer —
//!    a crossed composite, which the RFQ resolver rejects outright, starving exactly
//!    the hedges the contracts exist to fill.
//! 2. **A listed contract trades in whole contracts.** Nothing in the OTC feed knew
//!    that, so a futures market could be quoted — and a futures hedge filled — for a
//!    fraction of a contract.
//! 3. **A listed contract rolls.** A hedge vehicle configured as a product symbol
//!    (`ZF`) has to resolve to whichever delivery month is trading today, and only
//!    the venue knows which cycles are live.
//!
//! Splitting the venue out lets each of those be modelled correctly instead of
//! being averaged against a cash-bond market structure.
//!
//! ## What it is a peer of
//!
//! It is a **first-class peer** of the OTC simulator, not a subordinate: its own
//! crate, its own `cme-sim` binary, its own launcher (`deploy/start-cme-sim.sh`) and
//! its own control script (`cmesimctl`), authenticating and publishing through the
//! same `LiquidityFeedService.LpFeed` ingest and accepting orders over the same FIX
//! `NewOrderSingle(D)` → `ExecutionReport(8)` contract. It depends on
//! [`celnet_lp_sim`] only for the *shared simulator vocabulary* — the
//! [`QuotedLine`](celnet_lp_sim::QuotedLine) shape, the seeded mean-reverting price
//! model, the book resolver, the streaming client, the named-counterparty roster,
//! and the order-execution core. The dependency points strictly one way: the OTC
//! simulator knows nothing about this crate.
//!
//! ## Everything it quotes comes from reference data
//!
//! No number here is invented. Contract face value, minimum price increment, tick
//! value, notional coupon, delivery and last-trading dates, the notional deliverable
//! and the derived DV01 per contract all come from
//! [`celnet_refdata::TreasuryFutureSpec`]; the level each contract is anchored at is
//! a real inverted cash-curve yield (see [`contract`]); and the delivery-month roll
//! is [`celnet_refdata::front_contract_id`].
//!
//! ## Operator note — this is a breaking change to an aggregated book
//!
//! Futures lines are no longer contributed by the OTC panel's members. An aggregated
//! book that carries Treasury futures must list `cme-sim` in its
//! `member_connection_ids`, or those lines will have no contributor and every
//! futures hedge routed at them will backstop to the synthetic `COMPOSITE` venue.

#![forbid(unsafe_code)]

pub mod contract;
pub mod feed;
pub mod venue;

pub use contract::{FuturesContract, futures_lines, load_futures_universe};
pub use feed::{FuturesFeedOptions, run_futures_feed};
pub use venue::{
    BASE_HALF_SPREAD, BASE_SIZE, BASE_SKEW_STEP, SymbolResolution, VENUE_ID, as_of,
    build_order_venue, build_venue_feed, contract_lot_size, profile, publish_markets,
    resolve_contract, whole_contract_clip,
};
