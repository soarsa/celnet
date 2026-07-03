//! Celnet **multi-dealer RFQ aggregation** (Wave W4 Track B —
//! `docs/W4-STRUCTURED-RFQ-PLAN.md`).
//!
//! Celnet's single-dealer pricing is generalised here into a multi-bank RFQ
//! venue: one [`RfqRequest`] is fanned **concurrently** to a bounded panel of
//! [`QuoteSource`]s — a native in-process [`InternalPricerSource`] (so a market
//! always exists) and any number of external [`FixLpAdapter`]s reaching real LPs
//! over a **real FIX 4.4 session on a socket** — and the two-sided responses are
//! ranked into an audited [`RankedPanel`]:
//!
//! * **best bid** = max bid premium, **best offer** = min offer premium;
//! * **deterministic tie-break** on equal best price: earlier `epoch_nanos`,
//!   then lexicographically smallest stable `lp_id`;
//! * **timeout** — a source past its deadline is dropped (not errored) and
//!   excluded from `lp_count`;
//! * **last-look** — a winner whose `valid_until_nanos` has lapsed is rejected
//!   and the next-best is promoted;
//! * a hard **consistency invariant** — `lp_count` == responder count and every
//!   `lp_won_*` references a real responder.
//!
//! The aggregation algebra lives in [`panel`]; the two source kinds in
//! [`internal`] and [`lp_fix`]. This is a one-way leaf crate
//! (`celnet-rfq → {celnet-types, celnet-proto, celnet-fix}`); it does NOT depend
//! on the hot `celnet-engine`, and it owns only the *aggregation* — the pricing
//! model is injected (native source) or supplied by the LP (FIX source), exactly
//! as the `celnet-fix` dialect injects its pricer.
//!
//! # Honest boundary (verbatim — deploy/ENV-gated)
//!
//! **Live LP-panel connectivity (real bank sessions over WAN FIX) and the
//! regulated-venue / MAS-RMO status are ENV — designed, seamed and ADR'd
//! in-repo, validated at deploy, NEVER claimed in-repo.** In-repo this crate
//! proves the aggregation / ranking / tie-break / last-look ALGORITHM plus the
//! FIX framing / dialect round-trip over a **loopback** socket only (≥ 3
//! synthetic LP responders, at least one a real loopback [`FixLpAdapter`], gated
//! against an independently-computed injected ground-truth ladder). Live
//! endpoints are selected by the edge's `CELNET_LP_PANEL` environment
//! configuration, defaulting to the synthetic in-repo panel; this crate carries
//! no live endpoint and makes no WAN-latency or venue-status claim.
#![forbid(unsafe_code)]

pub mod internal;
pub mod lp_fix;
pub mod panel;

pub use internal::InternalPricerSource;
pub use lp_fix::{FixLpAdapter, FixLpConfig};
pub use panel::{
    DealerQuote, FxOptionLeg, MultiDealerEngine, PanelError, QuoteSource, QuoteSourceReply,
    RankedPanel, RatesLeg, RfqLeg, RfqRequest, TwoWay,
};
