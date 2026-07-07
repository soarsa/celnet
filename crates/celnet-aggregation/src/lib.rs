//! Cross-asset venue-aggregation core — the pure numerical/data layer that turns
//! N independent venue top-of-book feeds into one continuous **consolidated
//! book** for a tradable cash instrument, then prices a trader's two-way off that
//! consolidated mid.
//!
//! This crate is deliberately edge-free: it owns only the *algebra* of
//! consolidation and risk pricing (no wire contract, no server, no FIX, no GUI —
//! those consume this core in later lanes). It is asset-agnostic over an
//! [`Instrument`] key built from the frozen [`celnet_types`] vocabulary, so the
//! same engine consolidates FX, precious metals, and fixed income without a
//! bespoke per-asset stack.
//!
//! # Lane A — aggregation core
//!
//! - [`VenueId`] / [`Instrument`] / [`VenueQuote`] — the venue-quote vocabulary
//!   ([`instrument`]).
//! - [`VenueFeed`] — the object-safe "subscribe to instruments → stream of
//!   [`VenueQuote`]" trait, plus [`VenueStream`] the deterministic tick-grid
//!   iterator that realises the stream ([`feed`]).
//! - [`SimVenue`] — a **real** deterministic-seeded top-of-book generator (a
//!   sinusoidal mid-drift with a per-venue rich/cheap bias and seeded micro-noise,
//!   configurable spread/size/latency), the legitimate UAT data source
//!   ([`sim`]).
//! - [`ConsolidatedBook`] — continuous best-bid/best-offer across venues with
//!   depth stacking, **time-weighted staleness decay** and **divergent-source
//!   exclusion** adapted from the vol-smile blender
//!   (`celnet_integration::aggregate`) to a cash price/yield, emitting a
//!   consolidated two-way, a confidence measure, and which venues contributed
//!   ([`consolidate`]).
//!
//! # Lane B — risk pricer
//!
//! - [`RiskPricer`] / [`RiskPriceParams`] — the trader's two-way "risk price":
//!   the consolidated mid + **inventory/axe directional skew** + a size-tiered
//!   spread with a warehousing risk charge. The skew is trader-controlled today
//!   via [`RiskPriceParams::skew_bp`], with a real (non-stub) [`AutoSkewSource`]
//!   seam for a later automatic axe read from the rates position store
//!   ([`risk`]).
//!
//! # Reuse & provenance (doc-only)
//!
//! The staleness kernel (`2^{−Δt/τ}`) and the median-consensus + MAD-scaled
//! divergence gate mirror `celnet_integration::aggregate` / `divergence`, adapted
//! from a five-vol smile vector to a scalar cash mid. The BBO total-order
//! tie-break `(price, ts, venue)` and non-finite folding mirror
//! `celnet_rfq::panel`'s ranking. The half-spread's `floor.max(charge)` shape
//! mirrors `celnet_server::spread::SpreadModel::half_spread`; it is re-expressed
//! here in basis points of the cash mid (not option Greeks) and re-implemented
//! rather than imported, because importing it would invert the dependency graph
//! (the server depends on aggregation, never the reverse). Identifiers are
//! purpose-named and vendor/research-neutral.

#![forbid(unsafe_code)]

pub mod consolidate;
pub mod feed;
pub mod instrument;
pub mod risk;
pub mod sim;

pub use consolidate::{
    ConsolidatedBook, ConsolidationConfig, ConsolidationError, DepthLevel, ExclusionReason,
    VenueContribution,
};
pub use feed::{VenueFeed, VenueStream};
pub use instrument::{Instrument, VenueId, VenueQuote};
pub use risk::{AutoSkewSource, NoAutoSkew, RiskPriceParams, RiskPricer, RiskTwoWay, SizeTier};
pub use sim::{SimDynamics, SimVenue};
