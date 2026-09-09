//! The hedge **execution** seam — how a resolved hedge decision's legs are *applied*.
//!
//! [`AutoHedgeEngine`](super::engine::AutoHedgeEngine) is the complete decision +
//! provenance layer; the *booking* of the legs it decides is delegated here so the
//! decision core stays pure and the live-booking wiring is one explicit, swappable seam
//! (`docs/hedging/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §7).
//!
//! Two implementations ship:
//!
//! - [`AdvisoryExecutor`] — books **nothing** (returns [`ExecOutcome::Advisory`]). The
//!   mandatory shadow-run posture (§8.4), and the safe default while a desk watches the
//!   engine shadow real flow. Whatever it "would" do is still stamped as real provenance
//!   by the engine — it simply never trades.
//! - [`LedgerExecutor`] — a **real** in-memory booking ledger that applies the two
//!   offsetting `CROSS_INTERNAL` legs (flatten the source, open the counterparty),
//!   re-running a caller-supplied **hard-cap gate** on the target before it commits (the
//!   limits-are-never-bypassed rule, §8.3). It is genuinely functional and fully tested;
//!   the P2 server seam swaps its cap-gate + sink closures for the live
//!   [`PositionStore`](crate::services::risk::store::PositionStore) offsetting-booking sink
//!   and `project_risk_book_breach` gate (the same sinks risk-transfer drives). External
//!   `SUBMIT_MARKET_ORDER` / `RFQ_OUT` execution onto the `celnet-rfq` panel is the P3
//!   seam and stays advisory-gated (§8.4).

use std::sync::RwLock;

use crate::config::hedge_policy::HedgeExecutionMode;

// ============================================================================
// External hedge execution — the LIVE seam (§6.2). Where the internal-cross
// executor above books an offsetting leg at the consolidated mid, this seam
// EXTERNALISES a shed onto a venue: the LP-sim RFQ/FIX panel (`LpPanel`), the Agg
// Book COMPOSITE mid (`Composite`), or LP-first with a composite backstop
// (`LpPanelThenComposite`). `Advisory` books nothing. The venue-selection + pricing
// is a PURE function (`execute_external`) so its economics are oracle-testable; the
// caller (`RatesPositionStore::stamp_internalise`) books the offsetting leg + stamps
// the real provenance off the returned [`ExternalHedgeFill`].
// ============================================================================

/// The venue an external hedge actually filled on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HedgeVenue {
    /// Filled on the external LP-sim RFQ/FIX panel.
    LpPanel,
    /// Filled against the Agg Book COMPOSITE mid (spread applied).
    Composite,
}

impl HedgeVenue {
    /// The stable `lp_won` label carried on provenance for a composite fill (an LP fill
    /// carries the winning LP's own id instead).
    pub const COMPOSITE_LABEL: &'static str = "COMPOSITE";

    /// The stable venue label for **structured logging** — the field that tells ops
    /// whether a hedge really crossed the street or backstopped to a synthetic mid.
    /// Before this the venue existed only in the in-memory provenance ring, so nothing in
    /// any log sink distinguished a genuine LP fill from a `COMPOSITE` backstop.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            HedgeVenue::LpPanel => "LP_PANEL",
            HedgeVenue::Composite => Self::COMPOSITE_LABEL,
        }
    }
}

/// One external LP hedge fill — the best two-way an LP returned for a hedge request.
#[derive(Debug, Clone, PartialEq)]
pub struct LpFill {
    /// The winning LP's id (carried into `HedgeProvenance.lp_won`).
    pub lp_id: String,
    /// The realised fill price (in the instrument's quote convention).
    pub price: f64,
}

/// A source of external LP hedge quotes — the injection seam onto the **existing**
/// outbound RFQ/FIX panel (`celnet-rfq` / the LP-sim), so this module invents no new
/// venue.
///
/// The seam returns the **whole ranked panel**, not just the winner. Returning only the
/// winner discards two facts the desk genuinely observed and cannot reconstruct later:
/// which other LPs showed a firm executable price (they were shown the order and we
/// dealt away from them — a real *miss*), and how far the runner-up sat from the price
/// we took (a real *cover distance*). Both were previously thrown away at this seam,
/// which is precisely why the street-side league table could only ever report zeros for
/// missed deals and cover. An empty panel is an honest miss: no LP had a firm price on
/// the required side, or no live panel is wired at all ([`NoLpSource`]), and
/// `LpPanelThenComposite` then reaches the composite backstop while pure `LpPanel`
/// records the miss — never a fabricated fill (guardrail 2).
pub trait LpHedgeSource: Send + Sync {
    /// Every LP showing a firm executable price for hedging `size` of `instrument`
    /// (reducing `net_risk`), ordered **best-first** on the side we need. Empty on a
    /// miss.
    fn rank(&self, instrument: &str, net_risk: f64, size: f64) -> Vec<LpFill>;

    /// The best LP fill — the head of [`Self::rank`]. Provided, so an implementation
    /// only ever defines the ranking and the two can never disagree.
    fn best_fill(&self, instrument: &str, net_risk: f64, size: f64) -> Option<LpFill> {
        self.rank(instrument, net_risk, size).into_iter().next()
    }
}

/// The honest default LP source: never fills. Used until a live outbound RFQ/FIX panel
/// is injected — `LpPanelThenComposite` then always reaches the composite backstop, and
/// pure `LpPanel` records an honest miss rather than a fabricated fill.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLpSource;

impl LpHedgeSource for NoLpSource {
    fn rank(&self, _instrument: &str, _net_risk: f64, _size: f64) -> Vec<LpFill> {
        Vec::new()
    }
}

// ============================================================================
// The ROUTING seam — sending a real order to a named counterparty.
//
// [`LpHedgeSource`] answers "who is showing a firm price, and at what level". That
// is an OBSERVATION of a standing quote: it says nothing about whether the LP would
// actually trade. Turning an observation into a fill without asking anyone is the
// fabrication this seam exists to remove, so the two are deliberately separate
// traits — a quote source that cannot route (the aggregation hub) is unable to
// invent a fill even by accident, because filling now requires a counterparty's
// answer that only a router can obtain.
// ============================================================================

/// One outbound order, addressed to a single named panel member.
///
/// The vocabulary is FIX's, because the estate's only order→fill contract is
/// `NewOrderSingle(D)` → `ExecutionReport(8)` and the street-side record keys off
/// those tags verbatim. The side comes from [`celnet_analytics::StreetSide::shedding`]
/// — the SAME function the record uses — so the order that goes out and the row that
/// records it can never disagree about which way we dealt.
#[derive(Debug, Clone, Copy)]
pub struct StreetOrderIntent<'a> {
    /// The panel member to address (its connection id / `SenderCompID`).
    pub lp_id: &'a str,
    /// The tradeable security (FIX `Symbol(55)`).
    pub instrument: &'a str,
    /// The desk's own side.
    pub side: celnet_analytics::StreetSide,
    /// The quantity to work (FIX `OrderQty(38)`), in the instrument's native units.
    pub quantity: f64,
    /// The price limit (FIX `Price(44)`) — the member's OWN firm price from the
    /// ranking. Sending its own level back is what makes a decline meaningful: an LP
    /// that will not trade where it is showing has pulled at last look.
    pub limit_price: f64,
    /// FIX `OrdType(40)` — see [`celnet_fix::messages::ord_type`].
    pub ord_type: u8,
    /// FIX `TimeInForce(59)` — see [`celnet_fix::messages::time_in_force`].
    pub time_in_force: u8,
}

/// What a counterparty did with one routed order.
///
/// Every variant is an **observed** terminal state carrying its reason. There is no
/// "unknown": an attempt whose answer never arrived is [`Self::Expired`], which is a
/// fact about the venue (it did not answer in time), not an absence of data.
#[derive(Debug, Clone, PartialEq)]
pub enum RouteAnswer {
    /// The venue traded — in full, or in part with the remainder cancelled.
    Traded(RoutedFill),
    /// The venue **accepted** the order and could not satisfy it (FIX `OrdStatus=4`).
    Cancelled {
        /// The venue's machine-readable reason (its `Text(58)` code).
        reason: String,
        /// Measured round trip from send to answer.
        latency_nanos: u64,
    },
    /// The venue **refused** the order (FIX `OrdStatus=8`).
    Rejected {
        /// The venue's machine-readable reason.
        reason: String,
        /// Measured round trip from send to answer.
        latency_nanos: u64,
    },
    /// The venue declined at the price it was itself showing — its market moved
    /// between the quote we ranked and the order we sent. That is a last look, and
    /// it is a materially different fact about a counterparty than a refusal.
    LastLookPulled {
        /// The venue's machine-readable reason.
        reason: String,
        /// Measured round trip from send to answer.
        latency_nanos: u64,
    },
    /// No answer arrived inside the router's deadline. Recorded, never retried
    /// silently and never allowed to look like "the street showed nothing".
    Expired {
        /// How long we actually waited before giving up.
        waited_nanos: u64,
    },
    /// The order could not be sent at all — the member has no configured order
    /// endpoint, or the session could not be established. Carries the stated reason;
    /// this is the one variant with **no** measured latency, because there was no
    /// round trip to measure.
    Unroutable {
        /// Why nothing could be sent.
        reason: String,
    },
}

/// A real fill returned by a counterparty.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutedFill {
    /// The quantity that actually traded (`< requested` on an IOC partial).
    pub filled: f64,
    /// The realised (average) fill price the venue reported.
    pub price: f64,
    /// The venue's qualifier on a partial fill (why it did not complete); absent on
    /// a clean complete fill, which needs no explanation.
    pub reason: Option<String>,
    /// Measured round trip from send to `ExecutionReport`.
    pub latency_nanos: u64,
}

/// Sends a real order to a named panel member and waits for that member's answer.
///
/// Implementations open a genuine session to the counterparty. There is deliberately
/// no implementation that answers from local state: an object that could "route"
/// without a counterparty would reintroduce the synthesised fill.
pub trait StreetOrderRouter: Send + Sync {
    /// Work `intent` against its named member and return what came back. Blocks the
    /// calling thread for at most the router's own deadline; a venue that never
    /// answers yields [`RouteAnswer::Expired`], never a hang.
    fn route(&self, intent: &StreetOrderIntent<'_>) -> RouteAnswer;
}

/// The honest default router: routes nothing, and says so.
///
/// Installed until a live order-routing seam is wired. A desk running on this can
/// still *rank* the panel (that is a real observation) but can never fill on it, so
/// `LpPanelThenComposite` reaches the composite backstop and pure `LpPanel` records
/// the miss — the same honesty posture [`NoLpSource`] holds for quotes.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoStreetRouter;

/// The stated reason [`NoStreetRouter`] gives — a configuration fact, not a market one.
pub const NO_ROUTER_REASON: &str = "no_order_router_configured";

impl StreetOrderRouter for NoStreetRouter {
    fn route(&self, _intent: &StreetOrderIntent<'_>) -> RouteAnswer {
        RouteAnswer::Unroutable {
            reason: NO_ROUTER_REASON.to_owned(),
        }
    }
}

/// The terminal state of one routed attempt, as the street-side record classifies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteOutcome {
    /// The full requested quantity traded.
    Filled,
    /// Part traded; the remainder was cancelled.
    PartiallyFilled,
    /// The venue refused the order.
    Rejected,
    /// The venue accepted it and could not satisfy it.
    Cancelled,
    /// No answer inside the deadline.
    Expired,
    /// The venue would not trade at the price it was showing.
    LastLookPulled,
    /// Nothing could be sent (no endpoint / no session).
    Unroutable,
}

impl RouteOutcome {
    /// Whether any quantity changed hands.
    #[must_use]
    pub const fn is_fill(self) -> bool {
        matches!(self, Self::Filled | Self::PartiallyFilled)
    }

    /// The street-side record's outcome for this terminal state.
    ///
    /// An [`Unroutable`](Self::Unroutable) attempt maps to
    /// [`StreetOutcome::NoLiquidity`](celnet_analytics::StreetOutcome::NoLiquidity):
    /// nobody refused us, because nobody was asked. Collapsing it into `Rejected`
    /// would blame a counterparty for our own missing configuration.
    #[must_use]
    pub const fn street_outcome(self) -> celnet_analytics::StreetOutcome {
        use celnet_analytics::StreetOutcome as S;
        match self {
            Self::Filled => S::Filled,
            Self::PartiallyFilled => S::PartiallyFilled,
            Self::Rejected => S::Rejected,
            Self::Cancelled => S::Cancelled,
            Self::Expired => S::Expired,
            Self::LastLookPulled => S::LastLookPulled,
            Self::Unroutable => S::NoLiquidity,
        }
    }
}

/// One outbound order we actually sent (or could not send), and what came back —
/// the routing half of the street-side execution record.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteRecord {
    /// The member the order was addressed to.
    pub lp_id: String,
    /// The member's own firm price the order was priced at — the level from the ranking
    /// that this order asked it to honour. Retained so the record can name the panel an
    /// order competed against even where the fill itself carries none.
    pub quoted_price: f64,
    /// The terminal state.
    pub outcome: RouteOutcome,
    /// The venue's (or the router's) machine-readable reason. Absent only on a clean
    /// complete fill.
    pub reason: Option<String>,
    /// The quantity that traded on this attempt (`0.0` when nothing did).
    pub filled: f64,
    /// The realised price, when something traded.
    pub price: Option<f64>,
    /// FIX `OrdType(40)` as sent.
    pub order_type: u8,
    /// FIX `TimeInForce(59)` as sent.
    pub time_in_force: u8,
    /// Measured venue round trip. Absent **only** for an
    /// [`Unroutable`](RouteOutcome::Unroutable) attempt, which had no round trip.
    pub response_latency_nanos: Option<u64>,
}

impl RouteRecord {
    /// Classify `answer` to `intent` into a record. The single place a venue's answer
    /// becomes an outcome, so the booked economics and the recorded row are derived
    /// from one decision rather than two.
    #[must_use]
    pub fn from_answer(intent: &StreetOrderIntent<'_>, answer: RouteAnswer) -> Self {
        let base = |outcome, reason, filled, price, latency| Self {
            lp_id: intent.lp_id.to_owned(),
            quoted_price: intent.limit_price,
            outcome,
            reason,
            filled,
            price,
            order_type: intent.ord_type,
            time_in_force: intent.time_in_force,
            response_latency_nanos: latency,
        };
        match answer {
            RouteAnswer::Traded(f) => {
                // "Partial" is decided against what we ASKED for, not against what the
                // venue chose to call it: a fill short of the clip leaves a residual
                // the book must still carry, whatever the report was labelled.
                let outcome = if f.filled + f64::EPSILON * intent.quantity.abs() < intent.quantity {
                    RouteOutcome::PartiallyFilled
                } else {
                    RouteOutcome::Filled
                };
                base(
                    outcome,
                    f.reason,
                    f.filled,
                    Some(f.price),
                    Some(f.latency_nanos),
                )
            }
            RouteAnswer::Cancelled {
                reason,
                latency_nanos,
            } => base(
                RouteOutcome::Cancelled,
                Some(reason),
                0.0,
                None,
                Some(latency_nanos),
            ),
            RouteAnswer::Rejected {
                reason,
                latency_nanos,
            } => base(
                RouteOutcome::Rejected,
                Some(reason),
                0.0,
                None,
                Some(latency_nanos),
            ),
            RouteAnswer::LastLookPulled {
                reason,
                latency_nanos,
            } => base(
                RouteOutcome::LastLookPulled,
                Some(reason),
                0.0,
                None,
                Some(latency_nanos),
            ),
            RouteAnswer::Expired { waited_nanos } => base(
                RouteOutcome::Expired,
                Some(VENUE_NO_RESPONSE_REASON.to_owned()),
                0.0,
                None,
                // The wait IS a measured round trip — an upper bound on the venue's
                // latency, and the only latency datum a silent venue ever yields.
                Some(waited_nanos),
            ),
            RouteAnswer::Unroutable { reason } => {
                base(RouteOutcome::Unroutable, Some(reason), 0.0, None, None)
            }
        }
    }
}

/// The reason stamped on an attempt the venue never answered.
pub const VENUE_NO_RESPONSE_REASON: &str = "venue_no_response";

/// The realised economics of one external hedge attempt (a fill, or an honest miss).
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalHedgeFill {
    /// The size actually hedged externally (`0.0` on a miss / advisory).
    pub filled: f64,
    /// The unfilled residual of the requested shed (`requested − filled`); on a pure
    /// `LpPanel` miss this is the whole request (warehoused / advisory).
    pub residual: f64,
    /// The realised hedge price (the reference mid on a miss — no fabricated level).
    pub hedge_price: f64,
    /// The reference composite mid at fire.
    pub mid_at_fire: f64,
    /// Signed slippage of the fill vs mid, in the instrument's quote-convention bp
    /// (`(hedge_price − mid) / bp_scale`): negative when we sold below mid to shed a
    /// long, positive when we bought above mid to shed a short. `0` on a miss.
    pub slippage_bp: f64,
    /// The winning LP id, or [`HedgeVenue::COMPOSITE_LABEL`] for a composite fill;
    /// `None` on a miss.
    pub lp_won: Option<String>,
    /// The venue the fill landed on; `None` on a miss / advisory.
    pub venue: Option<HedgeVenue>,
    /// **Every** LP that showed a firm executable price on the required side when this
    /// attempt was worked, best-first — the competition the order was ranked against.
    /// The head is the LP we dealt on for an `LpPanel` fill.
    ///
    /// Empty means the street genuinely showed nothing, which is exactly what a
    /// `COMPOSITE` backstop records: the desk did not choose the synthetic mid over a
    /// real price, there was no real price. Carrying the panel through is what lets the
    /// street-side execution record report honest misses and a real cover distance
    /// instead of the zeros a winner-only seam can produce.
    pub panel: Vec<LpFill>,
    /// **Every order actually sent**, in the order it was sent — the routing history
    /// behind this fill.
    ///
    /// Empty means nothing was routed: an advisory run, a zero-size shed, a
    /// composite-only mode, or an empty panel (nobody to address). A non-empty list
    /// means real `NewOrderSingle`s went out and real `ExecutionReport`s came back;
    /// a rejection on the best-priced member followed by a fill on the cover appears
    /// here as two entries, because two orders genuinely left the building.
    ///
    /// This is what separates a fill we *earned* from one we *inferred*: a fill whose
    /// `attempts` is empty was never routed, and the record refuses to claim
    /// otherwise.
    pub attempts: Vec<RouteRecord>,
}

impl ExternalHedgeFill {
    /// An honest miss: nothing filled, the whole request left as residual, price = mid.
    #[must_use]
    pub fn miss(requested: f64, mid: f64) -> Self {
        Self {
            filled: 0.0,
            residual: requested.max(0.0),
            hedge_price: mid,
            mid_at_fire: mid,
            slippage_bp: 0.0,
            lp_won: None,
            venue: None,
            panel: Vec::new(),
            attempts: Vec::new(),
        }
    }

    /// An honest miss that nonetheless *saw* a panel — used when an attempt reached
    /// named LPs and did not trade (so the panel is real even though nothing filled).
    #[must_use]
    pub fn miss_with_panel(requested: f64, mid: f64, panel: Vec<LpFill>) -> Self {
        Self {
            panel,
            ..Self::miss(requested, mid)
        }
    }

    /// Whether this attempt actually externalised risk (a real fill on either venue).
    #[must_use]
    pub fn is_filled(&self) -> bool {
        self.filled > 0.0 && self.venue.is_some()
    }

    /// The venue label for **structured logging**: the filling venue, or `NONE` for an
    /// honest miss / advisory (nothing was externalised). Paired with `lp_won` on the
    /// hedge-execution log line, this is what lets ops answer "did that hedge cross the
    /// street, or backstop to a synthetic mid?" from the logs alone.
    #[must_use]
    pub fn venue_label(&self) -> &'static str {
        self.venue.map_or("NONE", HedgeVenue::label)
    }
}

/// The COMPOSITE hedge price + signed slippage for shedding a position (§6.2).
///
/// The hedge is the aggressor paying away the spread: reducing a **long** (`net_risk > 0`)
/// means SELLING **below** mid; reducing a **short** means BUYING **above** mid. The mid is
/// worsened by `composite_spread_bp` (in the instrument's quote convention via `bp_scale`,
/// matching [`crate::services::internalise::dealer_edge_bps`]):
///
/// ```text
/// signed_bp   = (net_risk > 0 ? −1 : +1) · |composite_spread_bp|
/// hedge_price = mid + signed_bp · bp_scale
/// slippage_bp = (hedge_price − mid) / bp_scale = signed_bp
/// ```
///
/// Returns `(hedge_price, slippage_bp)`. `slippage_bp` is exactly `signed_bp`, so the
/// magnitude is the composite spread paid and the sign records the side.
#[must_use]
pub fn composite_hedge_price(
    mid: f64,
    net_risk: f64,
    composite_spread_bp: f64,
    bp_scale: f64,
) -> (f64, f64) {
    let sell = net_risk > 0.0; // reducing a long ⇒ sell below mid
    let spread = composite_spread_bp.max(0.0);
    let signed_bp = if sell { -spread } else { spread };
    let hedge_price = mid + signed_bp * bp_scale;
    (hedge_price, signed_bp)
}

/// A request to externalise `size` (>0) of a shed, reducing signed `net_risk` on
/// `instrument`, against a reference composite `mid` (in the instrument's quote
/// convention, `bp_scale` = one bp in price units).
#[derive(Debug, Clone)]
pub struct ExternalHedgeRequest<'a> {
    /// The instrument being hedged (for the LP request).
    pub instrument: &'a str,
    /// The signed net risk (its sign selects the hedge side).
    pub net_risk: f64,
    /// The external size to hedge, in the **budget metric** (DV01 for a rates book;
    /// `≤ 0` ⇒ nothing to do). This is the risk-side number: what the book reduces by,
    /// what the residual is measured in, and what the provenance records.
    pub size: f64,
    /// What the VENUE is asked to trade, in the **venue's own units** — contracts for a
    /// listed future, face for a cash bond.
    ///
    /// This is deliberately separate from [`Self::size`]. A vehicle hedge sheds DV01 by
    /// trading a *lot-denominated* instrument, and a venue that trades whole lots rejects
    /// anything else outright: sending the DV01 figure as the order quantity got every
    /// futures shed rejected `NOT_A_WHOLE_LOT` on UAT (a plan for 14 ZTU26 contracts went
    /// to the wire asking for 484.5648), so the book never drained and pinned at its cap.
    ///
    /// Equal to `size` when the two are the same denomination (the self-hedge).
    pub venue_quantity: f64,
    /// How much budget-metric risk ONE venue unit removes — the plan's DV01 per contract.
    ///
    /// The venue reports fills in its own units, so this converts them back to the budget
    /// metric before the book reduces. `1.0` when `venue_quantity == size`.
    pub risk_per_venue_unit: f64,
    /// The reference composite mid at fire.
    pub mid: f64,
    /// One bp in the instrument's price convention (rate `1e-4`, clean price `1e-2`).
    pub bp_scale: f64,
    /// The desk's execution mode (which venues are allowed).
    pub mode: HedgeExecutionMode,
    /// The composite spread (bp) applied on a composite fill.
    pub composite_spread_bp: f64,
}

/// The `TimeInForce(59)` an auto-hedge shed is worked with.
///
/// **Immediate-or-cancel**, deliberately. A shed exists to reduce risk now: taking
/// whatever depth the member has and carrying the rest as an honest `residual` (which
/// the book already models, and which the composite backstop already covers) strictly
/// dominates killing the whole clip because the last few contracts were missing. A
/// resting time-in-force is not an option at all — these are quote-driven venues with
/// no working-order book, and they reject a resting TIF rather than silently downgrade
/// it. `FillOrKill` remains a first-class value of [`StreetOrderIntent`] that every
/// router honours; it is simply not what shedding risk wants.
pub const HEDGE_TIME_IN_FORCE: u8 = celnet_fix::messages::time_in_force::IMMEDIATE_OR_CANCEL;

/// The `OrdType(40)` an auto-hedge shed is worked with.
///
/// **Limit**, priced at the member's OWN firm level from the ranking. A market order
/// would fill at whatever the member happened to be showing on arrival, which makes
/// the recorded slippage a measurement of nothing; pricing the order at the level we
/// ranked means a decline is information — the member has moved away from its own
/// quote, which is exactly a last look.
pub const HEDGE_ORD_TYPE: u8 = celnet_fix::messages::ord_type::LIMIT;

/// Execute one external hedge per the policy's [`HedgeExecutionMode`] (§6.2). Books
/// nothing — the caller books the offsetting leg off the returned [`ExternalHedgeFill`]:
///
/// - `Advisory` (or a non-positive size) → an honest miss (nothing executes, nothing
///   is routed).
/// - `LpPanel` → **route** down the ranked panel; the first member that trades wins.
///   No fill ⇒ an honest miss (no composite fallback).
/// - `Composite` → always fills at [`composite_hedge_price`]; nothing is routed.
/// - `LpPanelThenComposite` → the panel first; only when no member traded does the
///   composite backstop fill, still crediting no LP.
///
/// # Why the panel is walked rather than lifted
///
/// [`LpHedgeSource::rank`] reports who is *showing* a firm price. It cannot report who
/// would *trade*, because nobody has been asked. This function therefore treats the
/// ranking as a routing order and sends a real order to each member in turn until one
/// trades, recording every answer in [`ExternalHedgeFill::attempts`]. A refusal on the
/// best-priced member is a fact about that member — it must not silently become a
/// composite backstop, which would misreport a counterparty problem as an absence of
/// street liquidity.
#[must_use]
pub fn execute_external(
    req: &ExternalHedgeRequest<'_>,
    lp: &dyn LpHedgeSource,
    router: &dyn StreetOrderRouter,
) -> ExternalHedgeFill {
    if req.size <= 0.0 || req.venue_quantity <= 0.0 || req.mode.is_advisory() {
        return ExternalHedgeFill::miss(req.size, req.mid);
    }

    // Read the WHOLE ranked panel (not just the winner) so the street-side execution
    // record can report who else showed a firm price and how far the cover sat — see
    // `ExternalHedgeFill::panel`.
    let panel = if req.mode.tries_lp_panel() {
        // Rank on what the venue is actually asked for: executable depth is a question
        // about the venue's own units, not about DV01.
        lp.rank(req.instrument, req.net_risk, req.venue_quantity)
    } else {
        Vec::new()
    };

    let mut attempts: Vec<RouteRecord> = Vec::with_capacity(panel.len());
    for member in &panel {
        let intent = StreetOrderIntent {
            lp_id: &member.lp_id,
            instrument: req.instrument,
            side: celnet_analytics::StreetSide::shedding(req.net_risk),
            quantity: req.venue_quantity,
            limit_price: member.price,
            ord_type: HEDGE_ORD_TYPE,
            time_in_force: HEDGE_TIME_IN_FORCE,
        };
        let record = RouteRecord::from_answer(&intent, router.route(&intent));
        let traded = record.outcome.is_fill();
        // The venue fills in ITS units; the book reduces in the budget metric. Convert
        // once, here, so every downstream number (fill, residual, the offsetting leg) is
        // denominated the way its consumer expects.
        let filled = record.filled * req.risk_per_venue_unit;
        let price = record.price.unwrap_or(member.price);
        attempts.push(record);
        if traded && filled > 0.0 {
            let slippage_bp = if req.bp_scale != 0.0 {
                (price - req.mid) / req.bp_scale
            } else {
                0.0
            };
            return ExternalHedgeFill {
                filled,
                // An IOC that took only part of the clip leaves a REAL residual: the
                // book did not reduce by what we asked for, and saying it did would
                // make the blotter and the risk diverge.
                residual: (req.size - filled).max(0.0),
                hedge_price: price,
                mid_at_fire: req.mid,
                slippage_bp,
                lp_won: Some(member.lp_id.clone()),
                venue: Some(HedgeVenue::LpPanel),
                panel,
                attempts,
            };
        }
    }

    // Composite (as the primary venue, or the panel fallback). Reached when the panel
    // was empty OR every member we asked declined, so the recorded fill carries no
    // competitors — a backstop is not a price we preferred over the street, it is the
    // absence of a street price we could deal on. The attempts survive so the record
    // can still say WHO declined; without them a full panel of refusals would be
    // indistinguishable from an empty panel.
    if req.mode.allows_composite() {
        let (hedge_price, slippage_bp) =
            composite_hedge_price(req.mid, req.net_risk, req.composite_spread_bp, req.bp_scale);
        return ExternalHedgeFill {
            filled: req.size,
            residual: 0.0,
            hedge_price,
            mid_at_fire: req.mid,
            slippage_bp,
            lp_won: Some(HedgeVenue::COMPOSITE_LABEL.to_owned()),
            venue: Some(HedgeVenue::Composite),
            panel: Vec::new(),
            attempts,
        };
    }

    // Pure `LpPanel` with no fill — an honest miss (the shed is warehoused / advisory).
    ExternalHedgeFill {
        panel,
        attempts,
        ..ExternalHedgeFill::miss(req.size, req.mid)
    }
}

/// One offsetting booking leg an internal cross applies: a signed size booked into a book
/// at the consolidated mid. A `CROSS_INTERNAL` is two of these — the source flattens
/// (`-crossed`) and the counterparty opens (`+crossed`).
#[derive(Debug, Clone, PartialEq)]
pub struct HedgeLeg {
    /// The book the leg is booked into.
    pub book: String,
    /// The instrument.
    pub instrument: String,
    /// The signed size (native metric units; + opens long risk, − flattens).
    pub signed_size: f64,
    /// The price the leg books at (the consolidated mid for an internal cross).
    pub price: f64,
}

/// The outcome of applying a hedge decision's legs.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecOutcome {
    /// Advisory / dry-run: nothing was booked (the shadow-run posture).
    Advisory,
    /// The legs were booked. Carries the applied legs (for provenance / audit).
    Booked {
        /// The offsetting legs actually committed.
        legs: Vec<HedgeLeg>,
    },
    /// The booking was refused by the hard-cap gate (limits are never bypassed, §8.3).
    Rejected {
        /// The human reason surfaced on the notification / audit.
        reason: String,
    },
}

/// Applies a hedge decision's offsetting legs. The one swappable seam between the pure
/// decision engine and the live position store.
pub trait HedgeExecutor: Send + Sync {
    /// Apply an internal cross of `crossed` (>0) units of `instrument` from `source_book`
    /// into `counterparty_book` at `mid`. Returns whether it booked, was advisory, or was
    /// rejected by the target's hard-cap gate.
    fn cross_internal(
        &self,
        source_book: &str,
        counterparty_book: &str,
        instrument: &str,
        crossed: f64,
        mid: f64,
    ) -> ExecOutcome;
}

/// The shadow-run executor: books nothing, always [`ExecOutcome::Advisory`].
#[derive(Debug, Default, Clone, Copy)]
pub struct AdvisoryExecutor;

impl HedgeExecutor for AdvisoryExecutor {
    fn cross_internal(
        &self,
        _source_book: &str,
        _counterparty_book: &str,
        _instrument: &str,
        _crossed: f64,
        _mid: f64,
    ) -> ExecOutcome {
        ExecOutcome::Advisory
    }
}

/// A real in-memory booking ledger for internal crosses, gated by a caller-supplied
/// hard-cap predicate on the **target** book (the P2 live seam swaps the gate + the
/// committed legs for the live `PositionStore` sink + `project_risk_book_breach`).
///
/// `cap_gate(book, projected_signed_size) -> bool` returns whether booking that projected
/// exposure into `book` is within its hard cap. A ledger commit runs the gate on the
/// counterparty (opening) book before it records **both** offsetting legs atomically.
pub struct LedgerExecutor<F>
where
    F: Fn(&str, f64) -> bool + Send + Sync,
{
    booked: RwLock<Vec<HedgeLeg>>,
    cap_gate: F,
}

impl<F> LedgerExecutor<F>
where
    F: Fn(&str, f64) -> bool + Send + Sync,
{
    /// A ledger gated by `cap_gate`. A gate of `|_, _| true` books unconditionally.
    pub fn new(cap_gate: F) -> Self {
        Self {
            booked: RwLock::new(Vec::new()),
            cap_gate,
        }
    }

    /// The legs booked so far (audit / test aid).
    #[must_use]
    pub fn booked(&self) -> Vec<HedgeLeg> {
        self.booked.read().expect("ledger lock poisoned").clone()
    }
}

impl<F> HedgeExecutor for LedgerExecutor<F>
where
    F: Fn(&str, f64) -> bool + Send + Sync,
{
    fn cross_internal(
        &self,
        source_book: &str,
        counterparty_book: &str,
        instrument: &str,
        crossed: f64,
        mid: f64,
    ) -> ExecOutcome {
        let crossed = crossed.max(0.0);
        if crossed == 0.0 {
            return ExecOutcome::Advisory;
        }
        // Re-run the hard-cap gate on the target (opening) book before committing — an
        // auto-hedge can never itself breach a cap (§8.3).
        if !(self.cap_gate)(counterparty_book, crossed) {
            return ExecOutcome::Rejected {
                reason: format!(
                    "internal cross of {crossed} {instrument} would breach {counterparty_book}'s hard cap"
                ),
            };
        }
        let legs = vec![
            HedgeLeg {
                book: source_book.to_owned(),
                instrument: instrument.to_owned(),
                signed_size: -crossed, // the source flattens
                price: mid,
            },
            HedgeLeg {
                book: counterparty_book.to_owned(),
                instrument: instrument.to_owned(),
                signed_size: crossed, // the counterparty opens
                price: mid,
            },
        ];
        {
            let mut g = self.booked.write().expect("ledger lock poisoned");
            g.extend(legs.iter().cloned());
        }
        ExecOutcome::Booked { legs }
    }
}


/// Outcome of an algorithmic execution hedge slice schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct AlgorithmicHedgeOutcome {
    /// Total target hedge size.
    pub target_size: f64,
    /// Total quantity successfully filled across slices.
    pub total_filled: f64,
    /// Volume-weighted average execution price (VWAP).
    pub vwap_price: f64,
    /// Number of slices executed.
    pub slices_count: usize,
    /// Individual fill records for each executed slice.
    pub slice_fills: Vec<ExternalHedgeFill>,
    /// Unfilled residual quantity.
    pub residual: f64,
}

/// Slices a large hedge request using an algorithmic execution strategy
/// ([`celnet_algo::PluggableExecutionStrategy`]), routing each slice through the
/// external LP panel / router to minimize market impact and adverse selection.
pub fn execute_algorithmic_hedge(
    req: &ExternalHedgeRequest<'_>,
    lp: &dyn LpHedgeSource,
    router: &dyn StreetOrderRouter,
    strategy: &mut dyn celnet_algo::PluggableExecutionStrategy,
    total_duration_secs: f64,
    num_steps: usize,
) -> AlgorithmicHedgeOutcome {
    if req.size <= 0.0 || req.mode.is_advisory() || num_steps == 0 {
        return AlgorithmicHedgeOutcome {
            target_size: req.size,
            total_filled: 0.0,
            vwap_price: req.mid,
            slices_count: 0,
            slice_fills: Vec::new(),
            residual: req.size,
        };
    }

    let mut remaining = req.size;
    let mut total_filled = 0.0;
    let mut sum_notional = 0.0;
    let mut slice_fills = Vec::with_capacity(num_steps);
    let step_duration = total_duration_secs / (num_steps as f64);

    let half_spread = req.composite_spread_bp * req.bp_scale * 0.5;
    let book = celnet_algo::MarketBookSnapshot::new(
        req.mid - half_spread,
        1_000_000.0,
        req.mid + half_spread,
        1_000_000.0,
    );

    for step in 0..num_steps {
        if remaining <= 1e-6 {
            break;
        }

        let elapsed = (step as f64) * step_duration;
        let ctx = celnet_algo::StrategyExecutionContext {
            total_order_qty: req.size,
            executed_qty: total_filled,
            remaining_qty: remaining,
            elapsed_seconds: elapsed,
            total_duration_seconds: total_duration_secs,
            book: &book,
            historical_volume_fraction: (step + 1) as f64 / num_steps as f64,
        };

        if let Ok(Some(slice)) = strategy.compute_slice(&ctx) {
            let slice_qty = slice.quantity.min(remaining);
            if slice_qty <= 0.0 {
                continue;
            }

            let slice_venue_qty = if req.size > 0.0 {
                slice_qty * (req.venue_quantity / req.size)
            } else {
                slice_qty
            };
            let slice_req = ExternalHedgeRequest {
                size: slice_qty,
                venue_quantity: slice_venue_qty,
                risk_per_venue_unit: req.risk_per_venue_unit,
                instrument: req.instrument,
                net_risk: req.net_risk,
                mid: req.mid,
                bp_scale: req.bp_scale,
                mode: req.mode,
                composite_spread_bp: req.composite_spread_bp,
            };

            let fill = execute_external(&slice_req, lp, router);
            if fill.is_filled() {
                let filled_qty = fill.filled;
                let price = fill.hedge_price;
                strategy.on_fill(filled_qty, price);
                remaining = (remaining - filled_qty).max(0.0);
                total_filled += filled_qty;
                sum_notional += filled_qty * price;
            }
            slice_fills.push(fill);
        }
    }

    let vwap_price = if total_filled > 0.0 {
        sum_notional / total_filled
    } else {
        req.mid
    };

    AlgorithmicHedgeOutcome {
        target_size: req.size,
        total_filled,
        vwap_price,
        slices_count: slice_fills.len(),
        slice_fills,
        residual: remaining,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisory_books_nothing() {
        let e = AdvisoryExecutor;
        assert_eq!(
            e.cross_internal("src", "tgt", "EURUSD", 5_000.0, 1.1),
            ExecOutcome::Advisory
        );
    }

    #[test]
    fn ledger_books_two_offsetting_legs_at_mid() {
        let e = LedgerExecutor::new(|_book, _size| true);
        let out = e.cross_internal("RATES-EUR", "RATES-USD", "EURUSD", 6_000.0, 1.0850);
        match out {
            ExecOutcome::Booked { legs } => {
                assert_eq!(legs.len(), 2);
                assert_eq!(legs[0].signed_size, -6_000.0, "source flattens");
                assert_eq!(legs[1].signed_size, 6_000.0, "counterparty opens");
                assert!(legs.iter().all(|l| (l.price - 1.0850).abs() < 1e-12));
            }
            other => panic!("expected Booked, got {other:?}"),
        }
        assert_eq!(e.booked().len(), 2);
    }

    #[test]
    fn ledger_rejects_when_cap_gate_fails_and_books_nothing() {
        // Gate refuses any booking into RATES-USD.
        let e = LedgerExecutor::new(|book, _size| book != "RATES-USD");
        let out = e.cross_internal("RATES-EUR", "RATES-USD", "EURUSD", 6_000.0, 1.0);
        assert!(matches!(out, ExecOutcome::Rejected { .. }));
        assert!(e.booked().is_empty(), "a rejected cross books nothing");
    }

    #[test]
    fn zero_cross_is_advisory_noop() {
        let e = LedgerExecutor::new(|_, _| true);
        assert_eq!(
            e.cross_internal("a", "b", "EURUSD", 0.0, 1.0),
            ExecOutcome::Advisory
        );
        assert!(e.booked().is_empty());
    }

    // --- external execution seam --------------------------------------------

    /// An LP source that always fills at a fixed price (for the LP-panel path).
    struct FixedLp {
        lp: &'static str,
        price: f64,
    }
    impl LpHedgeSource for FixedLp {
        fn rank(&self, _instrument: &str, _net_risk: f64, _size: f64) -> Vec<LpFill> {
            vec![LpFill {
                lp_id: self.lp.to_owned(),
                price: self.price,
            }]
        }
    }

    /// A three-deep panel, best-first (for the competition / cover path).
    struct PanelLp;
    impl LpHedgeSource for PanelLp {
        fn rank(&self, _instrument: &str, _net_risk: f64, _size: f64) -> Vec<LpFill> {
            vec![
                LpFill {
                    lp_id: "LP-1".to_owned(),
                    price: 99.99,
                },
                LpFill {
                    lp_id: "LP-2".to_owned(),
                    price: 99.97,
                },
                LpFill {
                    lp_id: "LP-3".to_owned(),
                    price: 99.95,
                },
            ]
        }
    }

    /// A router that trades the full clip at the level it was shown — the "every
    /// member honours its own quote" world, so a test can isolate venue SELECTION
    /// from venue BEHAVIOUR. Real venue behaviour (partials, declines, silence) is
    /// exercised against the actual simulators over a real socket in
    /// `tests/street_routing_e2e.rs`.
    struct AlwaysTrades;
    impl StreetOrderRouter for AlwaysTrades {
        fn route(&self, intent: &StreetOrderIntent<'_>) -> RouteAnswer {
            RouteAnswer::Traded(RoutedFill {
                filled: intent.quantity,
                price: intent.limit_price,
                reason: None,
                latency_nanos: 250_000,
            })
        }
    }

    /// A router where every member refuses — the case that must NOT look like an
    /// empty street.
    struct AlwaysRefuses;
    impl StreetOrderRouter for AlwaysRefuses {
        fn route(&self, _intent: &StreetOrderIntent<'_>) -> RouteAnswer {
            RouteAnswer::Rejected {
                reason: "NOT_MY_AXE".to_owned(),
                latency_nanos: 400_000,
            }
        }
    }

    /// A router that records exactly what quantity it was asked for, and refuses anything
    /// that is not a whole lot — the behaviour a listed-futures venue actually has.
    struct WholeLotVenue {
        seen: std::sync::Mutex<Vec<f64>>,
    }
    impl StreetOrderRouter for WholeLotVenue {
        fn route(&self, intent: &StreetOrderIntent<'_>) -> RouteAnswer {
            self.seen.lock().expect("seen lock").push(intent.quantity);
            if (intent.quantity - intent.quantity.round()).abs() > f64::EPSILON {
                return RouteAnswer::Rejected {
                    reason: "NOT_A_WHOLE_LOT".to_owned(),
                    latency_nanos: 100_000,
                };
            }
            RouteAnswer::Traded(RoutedFill {
                filled: intent.quantity,
                price: intent.limit_price,
                reason: None,
                latency_nanos: 250_000,
            })
        }
    }

    /// **A vehicle hedge asks the venue in CONTRACTS, and books the fill in DV01.**
    ///
    /// Regression for the UAT report of 2026-08-20. The externalised size is denominated in
    /// the budget metric (DV01), and it was being put straight onto the wire as the order
    /// quantity — so a plan for 14 `ZTU26` contracts asked the venue for 484.5648 and CME
    /// rejected every single futures shed `NOT_A_WHOLE_LOT`. Nothing ever filled, the book
    /// never drained, and both risk buckets sat pinned at 100% of limit while the hedge
    /// blotter cheerfully showed hedges firing.
    ///
    /// The two denominations must stay separate end to end: the venue trades lots, the book
    /// accounts in DV01.
    #[test]
    fn a_vehicle_hedge_asks_the_venue_in_whole_contracts_not_dv01() {
        let dv01_per_contract = 34.2707;
        let contracts = 14.0;
        let router = WholeLotVenue {
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let req = ExternalHedgeRequest {
            instrument: "ZTU26",
            net_risk: 50_000.0,
            // The risk-side number — deliberately NOT a whole number.
            size: contracts * dv01_per_contract,
            venue_quantity: contracts,
            risk_per_venue_unit: dv01_per_contract,
            mid: 103.5,
            bp_scale: 1e-2,
            mode: HedgeExecutionMode::LpPanel,
            composite_spread_bp: 0.5,
        };
        let fill = execute_external(
            &req,
            &FixedLp {
                lp: "cme-sim",
                price: 103.5,
            },
            &router,
        );

        let seen = router.seen.lock().expect("seen lock").clone();
        assert_eq!(
            seen,
            vec![contracts],
            "the venue must be asked in contracts"
        );
        assert!(
            fill.lp_won.is_some(),
            "a whole-lot order fills; before the fix this was rejected NOT_A_WHOLE_LOT"
        );
        // …and what comes back is converted to the budget metric, so the book reduces by
        // the DV01 the contracts actually removed.
        assert!(
            (fill.filled - contracts * dv01_per_contract).abs() < 1e-9,
            "fill must be reported in DV01, got {}",
            fill.filled
        );
        assert!(fill.residual.abs() < 1e-9, "a full fill leaves no residual");
    }

    /// A plan that rounds BELOW one whole contract puts no order on the wire at all.
    ///
    /// The second half of the 2026-08-20 regression, and the subtler half: the first fix
    /// only used the plan's contract count when it was non-zero, so a sub-one-lot target
    /// fell back to the DV01 figure and went to the venue as `ZFU26 qty=38.3346` — the
    /// exact shape the whole-lot rounding exists to prevent. Caught on UAT by checking the
    /// live wire after the deploy rather than trusting the fix.
    #[test]
    fn a_plan_below_one_whole_contract_sends_nothing() {
        let router = WholeLotVenue {
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let fill = execute_external(
            &ExternalHedgeRequest {
                instrument: "ZFU26",
                net_risk: 50_000.0,
                size: 38.3346,
                // The plan rounded to zero contracts — there is nothing tradeable here.
                venue_quantity: 0.0,
                risk_per_venue_unit: 40.1992,
                mid: 107.39,
                bp_scale: 1e-2,
                mode: HedgeExecutionMode::LpPanel,
                composite_spread_bp: 0.5,
            },
            &FixedLp {
                lp: "cme-sim",
                price: 107.39,
            },
            &router,
        );
        assert!(
            router.seen.lock().expect("seen lock").is_empty(),
            "nothing may reach the venue when the plan rounds to zero lots"
        );
        assert!(fill.lp_won.is_none(), "a no-op shed did not trade");
        assert!(
            (fill.residual - 38.3346).abs() < 1e-9,
            "the whole target stays an honest residual"
        );
    }

    /// The self-hedge shape is unchanged: with the two denominations equal, the venue is
    /// asked for the size itself and the fill needs no conversion.
    #[test]
    fn a_self_hedge_still_asks_the_venue_in_the_budget_metric() {
        let router = WholeLotVenue {
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let fill = execute_external(
            &req(
                50_000.0,
                25.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanel,
                0.5,
            ),
            &FixedLp {
                lp: "LP-1",
                price: 99.99,
            },
            &router,
        );
        assert_eq!(router.seen.lock().expect("seen lock").clone(), vec![25.0]);
        assert!((fill.filled - 25.0).abs() < 1e-9);
    }

    fn req(
        net_risk: f64,
        size: f64,
        mid: f64,
        bp_scale: f64,
        mode: HedgeExecutionMode,
        spread: f64,
    ) -> ExternalHedgeRequest<'static> {
        ExternalHedgeRequest {
            instrument: "USSW10",
            net_risk,
            size,
            // These fixtures hedge in the budget metric itself (the self-hedge shape), so
            // the venue and risk denominations coincide.
            venue_quantity: size,
            risk_per_venue_unit: 1.0,
            mid,
            bp_scale,
            mode,
            composite_spread_bp: spread,
        }
    }

    /// Composite fill price + signed slippage against a HAND-WORKED reference (guardrail 5).
    ///
    /// Long bond, clean-price mid 100.0, `bp_scale = 1e-2` (1bp of price = 0.01), spread 0.5bp.
    /// Reducing a long ⇒ SELL below mid: `signed_bp = −0.5`,
    /// `hedge_price = 100 + (−0.5)·0.01 = 99.995`, `slippage_bp = −0.5`.
    #[test]
    fn composite_price_long_bond_matches_hand_worked_reference() {
        let (price, slip) = composite_hedge_price(100.0, 50_000.0, 0.5, 1e-2);
        assert!((price - 99.995).abs() < 1e-12, "hedge_price {price}");
        assert!((slip - (-0.5)).abs() < 1e-12, "slippage_bp {slip}");
        // Slippage is exactly the signed price deviation over one bp of price.
        assert!(((price - 100.0) / 1e-2 - slip).abs() < 1e-12);
    }

    /// The rate side: short swap, par-rate mid 3% (0.03), `bp_scale = 1e-4`, spread 0.5bp.
    /// Reducing a short ⇒ BUY above mid: `signed_bp = +0.5`,
    /// `hedge_price = 0.03 + 0.5·1e-4 = 0.03005`, `slippage_bp = +0.5`.
    #[test]
    fn composite_price_short_swap_matches_hand_worked_reference() {
        let (price, slip) = composite_hedge_price(0.03, -50_000.0, 0.5, 1e-4);
        assert!((price - 0.03005).abs() < 1e-15, "hedge_price {price}");
        assert!((slip - 0.5).abs() < 1e-12, "slippage_bp {slip}");
    }

    #[test]
    fn advisory_mode_never_executes() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::Advisory,
                0.5,
            ),
            &NoLpSource,
            &NoStreetRouter,
        );
        assert!(!f.is_filled());
        assert_eq!(f.filled, 0.0);
        assert_eq!(f.residual, 10_000.0);
        assert_eq!(
            f.hedge_price, 100.0,
            "a miss carries the mid, never a fabricated level"
        );
    }

    #[test]
    fn composite_mode_always_fills() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::Composite,
                0.5,
            ),
            &NoLpSource,
            &NoStreetRouter,
        );
        assert!(f.is_filled());
        assert_eq!(f.venue, Some(HedgeVenue::Composite));
        assert_eq!(f.lp_won.as_deref(), Some("COMPOSITE"));
        assert_eq!(f.filled, 10_000.0);
        assert!((f.hedge_price - 99.995).abs() < 1e-12);
    }

    #[test]
    fn lp_panel_fills_from_the_source() {
        let lp = FixedLp {
            lp: "LP-2",
            price: 99.99,
        };
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanel,
                0.5,
            ),
            &lp,
            &AlwaysTrades,
        );
        assert!(f.is_filled());
        assert_eq!(f.venue, Some(HedgeVenue::LpPanel));
        assert_eq!(f.lp_won.as_deref(), Some("LP-2"));
        // slippage vs mid = (99.99 - 100)/1e-2 = -1.0 bp.
        assert!(
            (f.slippage_bp - (-1.0)).abs() < 1e-12,
            "slippage {}",
            f.slippage_bp
        );
    }

    #[test]
    fn pure_lp_panel_no_fill_is_an_honest_miss() {
        // No composite fallback in pure LpPanel — an unfilled clip is warehoused/advisory.
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanel,
                0.5,
            ),
            &NoLpSource,
            &NoStreetRouter,
        );
        assert!(
            !f.is_filled(),
            "pure LpPanel with no LP fill must NOT invent a fill"
        );
        assert_eq!(f.residual, 10_000.0);
    }

    #[test]
    fn lp_panel_then_composite_falls_back_when_lp_misses() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanelThenComposite,
                0.5,
            ),
            &NoLpSource,
            &NoStreetRouter,
        );
        assert!(
            f.is_filled(),
            "LP miss falls back to the composite backstop"
        );
        assert_eq!(f.venue, Some(HedgeVenue::Composite));
        assert_eq!(f.lp_won.as_deref(), Some("COMPOSITE"));
    }

    #[test]
    fn lp_panel_then_composite_prefers_a_real_lp_fill() {
        let lp = FixedLp {
            lp: "LP-1",
            price: 99.98,
        };
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanelThenComposite,
                0.5,
            ),
            &lp,
            &AlwaysTrades,
        );
        assert_eq!(
            f.venue,
            Some(HedgeVenue::LpPanel),
            "LP fill wins over the fallback"
        );
        assert_eq!(f.lp_won.as_deref(), Some("LP-1"));
    }

    /// The WHOLE panel survives the fill — the winner AND the LPs we dealt away from.
    /// This is the datum the street-side league table's "missed" and "cover" columns
    /// are computed from; a winner-only seam can only ever report zeros for both.
    #[test]
    fn a_panel_fill_carries_every_competing_firm_price() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanelThenComposite,
                0.5,
            ),
            &PanelLp,
            &AlwaysTrades,
        );
        assert_eq!(f.lp_won.as_deref(), Some("LP-1"), "the head of the ranking");
        assert_eq!(f.panel.len(), 3, "every firm price is retained");
        assert_eq!(f.panel[0].lp_id, "LP-1");
        assert_eq!(f.panel[1].lp_id, "LP-2", "the cover");
        assert_eq!(f.panel[2].lp_id, "LP-3");
    }

    /// A composite backstop is the ABSENCE of street liquidity, so it carries no
    /// competitors — nothing can later be attributed to an LP off it.
    #[test]
    fn a_composite_backstop_carries_no_panel() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanelThenComposite,
                0.5,
            ),
            &NoLpSource,
            &NoStreetRouter,
        );
        assert_eq!(f.venue, Some(HedgeVenue::Composite));
        assert!(f.panel.is_empty());
    }

    /// `best_fill` is the head of `rank` by construction — the two can never disagree.
    #[test]
    fn best_fill_is_the_head_of_the_ranking() {
        let best = PanelLp
            .best_fill("USSW10", 50_000.0, 10_000.0)
            .expect("a panel of three has a best");
        assert_eq!(
            best.lp_id,
            PanelLp.rank("USSW10", 50_000.0, 10_000.0)[0].lp_id
        );
        assert!(NoLpSource.best_fill("USSW10", 1.0, 1.0).is_none());
    }

    // --- routing: a firm price is not a fill -------------------------------------

    /// The regression that motivates this whole seam. A three-deep panel of firm
    /// prices with NO order router must **not** produce an LP fill: nobody was asked,
    /// so nobody agreed. It backstops to the composite and credits no LP, and the
    /// attempt log says exactly why.
    #[test]
    fn a_firm_price_with_no_router_is_never_an_lp_fill() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanelThenComposite,
                0.5,
            ),
            &PanelLp,
            &NoStreetRouter,
        );
        assert_eq!(
            f.venue,
            Some(HedgeVenue::Composite),
            "a quote nobody was asked to honour must not become an LP fill"
        );
        assert_eq!(f.lp_won.as_deref(), Some(HedgeVenue::COMPOSITE_LABEL));
        assert_eq!(f.attempts.len(), 3, "every member was tried in turn");
        assert!(
            f.attempts
                .iter()
                .all(|a| a.outcome == RouteOutcome::Unroutable
                    && a.reason.as_deref() == Some(NO_ROUTER_REASON)),
            "an unroutable attempt states the configuration reason, never a market one"
        );
        assert!(
            f.attempts
                .iter()
                .all(|a| a.response_latency_nanos.is_none()),
            "no round trip happened, so no latency may be reported"
        );
    }

    /// A routed fill carries the venue's own economics — the filled quantity and price
    /// the counterparty reported, plus the typed order fields actually sent.
    #[test]
    fn a_routed_fill_carries_the_typed_order_and_measured_latency() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanel,
                0.5,
            ),
            &PanelLp,
            &AlwaysTrades,
        );
        assert_eq!(f.venue, Some(HedgeVenue::LpPanel));
        assert_eq!(f.lp_won.as_deref(), Some("LP-1"));
        assert_eq!(f.attempts.len(), 1, "the best member traded, so we stopped");
        let a = &f.attempts[0];
        assert_eq!(a.outcome, RouteOutcome::Filled);
        assert_eq!(a.order_type, HEDGE_ORD_TYPE);
        assert_eq!(a.time_in_force, HEDGE_TIME_IN_FORCE);
        assert_eq!(a.response_latency_nanos, Some(250_000));
    }

    /// A refusal on the best-priced member is not the end of the street: the order
    /// walks to the cover, and BOTH orders are recorded because both really went out.
    #[test]
    fn a_refusal_on_the_best_member_walks_to_the_cover() {
        /// Refuses `LP-1`, trades on anyone else.
        struct RefusesTheBest;
        impl StreetOrderRouter for RefusesTheBest {
            fn route(&self, intent: &StreetOrderIntent<'_>) -> RouteAnswer {
                if intent.lp_id == "LP-1" {
                    return RouteAnswer::LastLookPulled {
                        reason: "NOT_MARKETABLE".to_owned(),
                        latency_nanos: 900,
                    };
                }
                RouteAnswer::Traded(RoutedFill {
                    filled: intent.quantity,
                    price: intent.limit_price,
                    reason: None,
                    latency_nanos: 1_100,
                })
            }
        }

        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanel,
                0.5,
            ),
            &PanelLp,
            &RefusesTheBest,
        );
        assert_eq!(f.lp_won.as_deref(), Some("LP-2"), "the cover traded");
        assert_eq!(
            f.attempts.len(),
            2,
            "two orders genuinely left the building"
        );
        assert_eq!(f.attempts[0].outcome, RouteOutcome::LastLookPulled);
        assert_eq!(f.attempts[0].lp_id, "LP-1");
        assert_eq!(f.attempts[1].outcome, RouteOutcome::Filled);
        // Priced on LP-2's own level (99.97), not LP-1's — the price we actually got.
        assert!((f.hedge_price - 99.97).abs() < 1e-12, "{}", f.hedge_price);
    }

    /// A partial fill leaves a REAL residual. Reporting the whole clip as hedged when
    /// only part traded would make the risk book and the blotter disagree.
    #[test]
    fn a_partial_fill_leaves_an_honest_residual() {
        struct HalfFills;
        impl StreetOrderRouter for HalfFills {
            fn route(&self, intent: &StreetOrderIntent<'_>) -> RouteAnswer {
                RouteAnswer::Traded(RoutedFill {
                    filled: intent.quantity / 2.0,
                    price: intent.limit_price,
                    reason: Some("IOC_DEPTH_EXHAUSTED".to_owned()),
                    latency_nanos: 700,
                })
            }
        }
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanel,
                0.5,
            ),
            &PanelLp,
            &HalfFills,
        );
        assert_eq!(f.filled, 5_000.0);
        assert_eq!(f.residual, 5_000.0, "the unfilled half is not hedged");
        assert_eq!(f.attempts[0].outcome, RouteOutcome::PartiallyFilled);
        assert_eq!(f.attempts[0].reason.as_deref(), Some("IOC_DEPTH_EXHAUSTED"));
    }

    /// A panel that unanimously refuses backstops to the composite — but the refusals
    /// SURVIVE on the record. Without them a street that said no would be
    /// indistinguishable from a street that said nothing.
    #[test]
    fn a_unanimous_refusal_backstops_but_keeps_every_refusal_on_the_record() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanelThenComposite,
                0.5,
            ),
            &PanelLp,
            &AlwaysRefuses,
        );
        assert_eq!(f.venue, Some(HedgeVenue::Composite));
        assert_eq!(
            f.lp_won.as_deref(),
            Some(HedgeVenue::COMPOSITE_LABEL),
            "a backstop credits no counterparty"
        );
        assert!(
            f.panel.is_empty(),
            "a backstop fill competed against nothing it could deal on"
        );
        assert_eq!(f.attempts.len(), 3);
        assert!(
            f.attempts
                .iter()
                .all(|a| a.outcome == RouteOutcome::Rejected)
        );
    }

    /// Pure `LpPanel` with a unanimously refusing street is an honest miss that still
    /// names who refused.
    #[test]
    fn pure_lp_panel_refusals_are_a_miss_that_names_the_refusers() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanel,
                0.5,
            ),
            &PanelLp,
            &AlwaysRefuses,
        );
        assert!(!f.is_filled());
        assert_eq!(f.residual, 10_000.0);
        assert_eq!(f.attempts.len(), 3);
        assert_eq!(f.panel.len(), 3, "the panel we asked is still the panel");
    }

    /// A silent venue is a recorded outcome, not a hang and not a market fact.
    #[test]
    fn a_silent_venue_expires_with_its_measured_wait() {
        struct NeverAnswers;
        impl StreetOrderRouter for NeverAnswers {
            fn route(&self, _intent: &StreetOrderIntent<'_>) -> RouteAnswer {
                RouteAnswer::Expired {
                    waited_nanos: 250_000_000,
                }
            }
        }
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanel,
                0.5,
            ),
            &FixedLp {
                lp: "LP-9",
                price: 99.99,
            },
            &NeverAnswers,
        );
        assert!(!f.is_filled());
        let a = &f.attempts[0];
        assert_eq!(a.outcome, RouteOutcome::Expired);
        assert_eq!(a.reason.as_deref(), Some(VENUE_NO_RESPONSE_REASON));
        assert_eq!(
            a.response_latency_nanos,
            Some(250_000_000),
            "the wait is the only latency datum a silent venue yields"
        );
        assert_eq!(
            a.outcome.street_outcome(),
            celnet_analytics::StreetOutcome::Expired
        );
    }

    /// A composite-only desk routes NOTHING — it never asks the street, so it must
    /// never claim to have.
    #[test]
    fn a_composite_only_desk_routes_nothing() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::Composite,
                0.5,
            ),
            &PanelLp,
            &AlwaysTrades,
        );
        assert_eq!(f.venue, Some(HedgeVenue::Composite));
        assert!(
            f.attempts.is_empty(),
            "no order was sent, so none is logged"
        );
    }

    /// An advisory desk routes nothing either — the shadow-run posture must not put
    /// live orders on the wire.
    #[test]
    fn an_advisory_desk_never_puts_an_order_on_the_wire() {
        let f = execute_external(
            &req(
                50_000.0,
                10_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::Advisory,
                0.5,
            ),
            &PanelLp,
            &AlwaysTrades,
        );
        assert!(!f.is_filled());
        assert!(f.attempts.is_empty());
    }

    /// Every routing outcome maps onto a distinct street-side outcome; in particular
    /// an unroutable attempt is NOT a rejection (nobody refused us).
    #[test]
    fn routing_outcomes_map_onto_distinct_street_outcomes() {
        use celnet_analytics::StreetOutcome as S;
        assert_eq!(RouteOutcome::Filled.street_outcome(), S::Filled);
        assert_eq!(
            RouteOutcome::PartiallyFilled.street_outcome(),
            S::PartiallyFilled
        );
        assert_eq!(RouteOutcome::Rejected.street_outcome(), S::Rejected);
        assert_eq!(RouteOutcome::Cancelled.street_outcome(), S::Cancelled);
        assert_eq!(RouteOutcome::Expired.street_outcome(), S::Expired);
        assert_eq!(
            RouteOutcome::LastLookPulled.street_outcome(),
            S::LastLookPulled
        );
        assert_eq!(
            RouteOutcome::Unroutable.street_outcome(),
            S::NoLiquidity,
            "our missing configuration is never a counterparty's refusal"
        );
    }

    /// Algorithmic execution slices a large hedge across multiple child steps.
    #[test]
    fn algorithmic_execution_slices_hedge_across_steps() {
        let mut strategy = celnet_algo::AdaptiveSpreadStrategy::new(1.0);
        let outcome = execute_algorithmic_hedge(
            &req(
                100_000.0,
                50_000.0,
                100.0,
                1e-2,
                HedgeExecutionMode::LpPanel,
                0.5,
            ),
            &PanelLp,
            &AlwaysTrades,
            &mut strategy,
            60.0,
            5,
        );

        assert!(outcome.total_filled > 0.0);
        assert!(outcome.slices_count > 0);
        assert_eq!(outcome.target_size, 50_000.0);
        assert!(outcome.residual < 50_000.0);
    }

}
