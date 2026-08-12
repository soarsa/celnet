//! The hedge **execution** seam — how a resolved hedge decision's legs are *applied*.
//!
//! [`AutoHedgeEngine`](super::engine::AutoHedgeEngine) is the complete decision +
//! provenance layer; the *booking* of the legs it decides is delegated here so the
//! decision core stays pure and the live-booking wiring is one explicit, swappable seam
//! (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §7).
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
/// venue. Returns the best fill for a hedge of `size` reducing `net_risk` on
/// `instrument`, or `None` when no LP filled within the bounded attempt (a miss). When
/// no live panel is wired the [`NoLpSource`] is used and every attempt honestly misses,
/// so `LpPanelThenComposite` falls back to the composite venue and pure `LpPanel`
/// records an honest miss (never a fabricated fill — guardrail 2).
pub trait LpHedgeSource: Send + Sync {
    /// The best LP fill for hedging `size` of `instrument` (reducing `net_risk`), or
    /// `None` on a miss (no LP filled in time / no panel wired).
    fn best_fill(&self, instrument: &str, net_risk: f64, size: f64) -> Option<LpFill>;
}

/// The honest default LP source: never fills. Used until a live outbound RFQ/FIX panel
/// is injected — `LpPanelThenComposite` then always reaches the composite backstop, and
/// pure `LpPanel` records an honest miss rather than a fabricated fill.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLpSource;

impl LpHedgeSource for NoLpSource {
    fn best_fill(&self, _instrument: &str, _net_risk: f64, _size: f64) -> Option<LpFill> {
        None
    }
}

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
    /// The external size to hedge (native metric units; `≤ 0` ⇒ nothing to do).
    pub size: f64,
    /// The reference composite mid at fire.
    pub mid: f64,
    /// One bp in the instrument's price convention (rate `1e-4`, clean price `1e-2`).
    pub bp_scale: f64,
    /// The desk's execution mode (which venues are allowed).
    pub mode: HedgeExecutionMode,
    /// The composite spread (bp) applied on a composite fill.
    pub composite_spread_bp: f64,
}

/// Execute one external hedge per the policy's [`HedgeExecutionMode`] against an injected
/// LP source (§6.2). PURE venue-selection + pricing — books nothing (the caller books the
/// offsetting leg off the returned [`ExternalHedgeFill`]):
///
/// - `Advisory` (or a non-positive size) → an honest miss (nothing executes).
/// - `LpPanel` → the LP source's best fill, else an honest miss (no composite fallback).
/// - `Composite` → always fills at [`composite_hedge_price`].
/// - `LpPanelThenComposite` → the LP source first; on a miss, fall back to the composite.
#[must_use]
pub fn execute_external(
    req: &ExternalHedgeRequest<'_>,
    lp: &dyn LpHedgeSource,
) -> ExternalHedgeFill {
    if req.size <= 0.0 || req.mode.is_advisory() {
        return ExternalHedgeFill::miss(req.size, req.mid);
    }

    // Try the LP panel first when the mode allows it.
    if req.mode.tries_lp_panel()
        && let Some(fill) = lp.best_fill(req.instrument, req.net_risk, req.size)
    {
        let slippage_bp = if req.bp_scale != 0.0 {
            (fill.price - req.mid) / req.bp_scale
        } else {
            0.0
        };
        return ExternalHedgeFill {
            filled: req.size,
            residual: 0.0,
            hedge_price: fill.price,
            mid_at_fire: req.mid,
            slippage_bp,
            lp_won: Some(fill.lp_id),
            venue: Some(HedgeVenue::LpPanel),
        };
    }

    // Composite (as the primary venue, or the LP-panel fallback).
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
        };
    }

    // Pure `LpPanel` with no LP fill — an honest miss (the shed is warehoused / advisory).
    ExternalHedgeFill::miss(req.size, req.mid)
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
        fn best_fill(&self, _instrument: &str, _net_risk: f64, _size: f64) -> Option<LpFill> {
            Some(LpFill {
                lp_id: self.lp.to_owned(),
                price: self.price,
            })
        }
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
        );
        assert_eq!(
            f.venue,
            Some(HedgeVenue::LpPanel),
            "LP fill wins over the fallback"
        );
        assert_eq!(f.lp_won.as_deref(), Some("LP-1"));
    }
}
