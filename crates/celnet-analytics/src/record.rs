//! The neutral input record — one quote/RFQ/execution's contribution to
//! client-flow analytics.
//!
//! A [`FlowRecord`] is a **server-, wire-, and market-data-free** value: the
//! server layer (a later phase) populates it from its own quote/execution
//! records + the `PricingProvenance` waterfall + the RFQ panel, then this crate
//! folds a `&[FlowRecord]` into per-client metrics. Every economically
//! meaningful quantity is an **input** here — nothing is re-priced or
//! re-derived from a market model — so the fold stays a pure, deterministic
//! function of its inputs (guardrail 5, oracle-testable).

/// Which side of the instrument the **desk** took on a fill (the desk's own
/// perspective, not the client's).
///
/// The adverse-selection signal is carried already-signed on
/// [`FlowRecord::markout`], so `side` is not needed to compute the metrics in
/// this crate; it is captured because it is intrinsic to a fill and keys future
/// per-side inventory grouping. It is part of the input contract, not a stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The desk **bought** the instrument from the client.
    Buy,
    /// The desk **sold** the instrument to the client.
    Sell,
}

/// One quote/RFQ/execution's contribution to a client's flow.
///
/// # Sign & unit conventions (locked)
///
/// - Currency amounts (`margin`, `quoted_spread`, `markout`, `hedge_cost`) are
///   in one consistent settlement currency (USD on the FX/FI desks), expressed
///   as an **absolute cash amount for this record's notional** — *not*
///   per-unit and *not* in bps — so they sum directly and normalise against
///   `notional` to yield `$/mm`.
/// - `notional` is a **non-negative magnitude** (face / USD-equivalent); the
///   direction is carried by [`Side`], never by the notional's sign.
/// - `margin` is signed so that **positive = margin earned in the desk's
///   favour** (the outbound-vs-fair delta the desk added — the
///   `PricingProvenance` `outbound − raw` amount). It is only realised on a
///   fill: a record with `was_traded == false` contributes **nothing** to
///   P&L, whatever its `margin` field holds.
/// - `markout` is signed so that **positive = adverse selection (a cost/loss to
///   us)** — the post-trade drift of the fair mid *against* our fill. It is
///   therefore *subtracted* to get net P&L. `None` when no markout horizon has
///   resolved yet.
/// - `hedge_cost` is signed so that **positive = a cost** to hedge/warehouse the
///   risk; also subtracted. `None` when not attributed.
/// - `quoted_spread` is the **full** bid/offer spread we showed on this
///   record's notional, as a positive cash amount — the denominator of
///   captured-vs-offered.
/// - `cover_distance` is our price versus the cover (second-best panel) quote,
///   signed so that **positive = we were more competitive than the cover** (we
///   improved on it). `None` when there was no panel/cover to compare against.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowRecord {
    /// Client identity (e.g. the authenticated requester / FIX session owner)
    /// — the primary rollup key.
    pub client: String,
    /// Counterparty or desk the flow was against — a secondary rollup key.
    pub counterparty: String,
    /// Instrument identifier (symbol / ISIN) — a secondary rollup key.
    pub instrument: String,
    /// Product / asset-class label this flow belongs to — an **opaque** rollup
    /// dimension the server tags each record with (e.g. `"fxo"` for FX options,
    /// `"fi"` for the fixed-income desk). Kept a free-form `String` — like
    /// [`client`](Self::client) / [`instrument`](Self::instrument) — so this pure
    /// crate carries **no** product taxonomy of its own; the meaning lives at the
    /// boundary that populates it. Grouped by [`crate::group_by_asset`] so a
    /// cross-product surface can slice FI vs FXO.
    pub asset: String,
    /// Non-negative notional magnitude (face / USD-equivalent) of this record.
    pub notional: f64,
    /// The desk-perspective side of the fill (see [`Side`]).
    pub side: Side,
    /// `true` if this record is a quote/RFQ response we issued (counts toward
    /// the quote/fishing denominator).
    pub was_quoted: bool,
    /// `true` if this record resulted in a trade/execution (counts toward
    /// traded notional, gross P&L, and the hit-rate numerator).
    pub was_traded: bool,
    /// Realised gross margin the desk added on this fill, in cash (positive =
    /// in our favour). Ignored unless `was_traded`. See the sign conventions.
    pub margin: f64,
    /// Full quoted bid/offer spread on this notional, in cash (positive).
    pub quoted_spread: f64,
    /// Our price versus the cover (second-best) quote (positive = we improved
    /// on the cover). `None` when there was no cover.
    pub cover_distance: Option<f64>,
    /// Post-trade adverse-selection cost in cash (positive = adverse to us).
    /// `None` when no markout horizon has resolved.
    pub markout: Option<f64>,
    /// Cost to hedge/warehouse this fill's risk, in cash (positive = a cost).
    /// `None` when not attributed.
    pub hedge_cost: Option<f64>,
}
