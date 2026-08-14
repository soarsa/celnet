//! **Auto-hedging / risk-internalisation** (Phase B) — the server-side engine + wire
//! bridge that turns a book's warehoused-risk state into an exit decision, off the pinned
//! pricing core (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md`).
//!
//! - [`engine`] — the [`AutoHedgeEngine`]: resolves the trader's `HedgeGraph` to an
//!   [`ExitAction`](celnet_hedge_routing::ExitAction), sizes + nets the shed against the
//!   [`WarehouseThreshold`](celnet_hedge_routing::WarehouseThreshold), applies the safety
//!   guards, and stamps an immutable `HedgeProvenance` + a live `HedgeIntent`.
//! - [`executor`] — the [`HedgeExecutor`](executor::HedgeExecutor) booking seam: the
//!   shadow-run [`AdvisoryExecutor`](executor::AdvisoryExecutor) (books nothing) and the
//!   real cap-gated [`LedgerExecutor`](executor::LedgerExecutor) for internal crosses.
//! - [`wire`] — the domain ⇄ `celnet_proto` converters for the hedge policy graph,
//!   thresholds, config, provenance and intents (shared by the engine + the RPC handlers).
//!
//! The `AuthService` hedge RPC handlers (in [`crate::services::auth`]) persist / validate
//! the policy graph, thresholds and config on the [`IdentityStore`](crate::config::identity::IdentityStore)
//! and read the engine's provenance ring; the entitlement gate for authoring is the narrow
//! `hedge` capability × asset (`celnet_entitlements::Action::Hedge`), distinct from the
//! `book` capability the engine's own booking legs gate on.

pub mod engine;
pub mod executor;
pub mod suggestion;
pub mod wire;

pub use engine::{
    AutoHedgeEngine, DecisionMeta, HedgeOutcome, PROVENANCE_RING_CAPACITY, SCOPE_NONE,
};
pub use executor::{
    AdvisoryExecutor, ExecOutcome, ExternalHedgeFill, ExternalHedgeRequest, HEDGE_ORD_TYPE,
    HEDGE_TIME_IN_FORCE, HedgeExecutor, HedgeLeg, HedgeVenue, LedgerExecutor, LpFill,
    LpHedgeSource, NO_ROUTER_REASON, NoLpSource, NoStreetRouter, RouteAnswer, RouteOutcome,
    RouteRecord, RoutedFill, StreetOrderIntent, StreetOrderRouter, VENUE_NO_RESPONSE_REASON,
    composite_hedge_price, execute_external,
};
pub use suggestion::{StandingSuggestion, SuggestionExec, SuggestionStore};
