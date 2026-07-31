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
}
