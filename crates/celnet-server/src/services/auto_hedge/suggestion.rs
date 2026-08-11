//! The **standing hedge-suggestion** store — the manual half of suggest-then-exit
//! (`docs/HEDGING-AND-RISK-EXIT.md` §6.5).
//!
//! When a scope's [`HedgeExitMode`](celnet_hedge_routing::HedgeExitMode) is `Suggest`, the
//! booking path does **every** thing it does under `Auto` — measure the book, classify the
//! band, resolve the exit policy, resolve the vehicle, size the DV01 ratio — and then
//! stops short of the venue. What it produces instead is a row in this store: a complete,
//! ready-to-fire hedge with the exact economics it *would* have traded.
//!
//! # Why a store, and not a dialog
//!
//! The desk was explicit: *"we don't want traders to receive a popup."* A modal is a
//! **transient interrupt** — it seizes focus, blocks the surface behind it, and is gone
//! the moment it is dismissed, taking the information with it. A standing suggestion is
//! the opposite in every respect: it is addressed to a `(book, instrument)` cell rather
//! than to whoever happens to be looking, it renders inline on the risk surface next to
//! the risk it describes, ignoring it costs nothing, and it is still there when the desk
//! comes back. That is the behaviour this store exists to provide.
//!
//! # What a row carries
//!
//! Two halves, deliberately separated:
//!
//! - [`HedgeSuggestion`] — the **wire projection** a trader reads: the instruction, the
//!   band, the sized vehicle plan, the honest residual, the policy path.
//! - [`SuggestionExec`] — the **server-private** execution recipe: the venue request
//!   parameters and the originating fill, so firing the suggestion later executes exactly
//!   the hedge that was shown rather than re-deriving a different one from a moved market.
//!   (The realised *price* is of course whatever the venue gives at fire time; it is the
//!   size and the instrument that are pinned.)
//!
//! # Latest-wins per cell
//!
//! One suggestion per `(book, instrument)`. A later fill on the same cell **supersedes**
//! the earlier suggestion rather than stacking a second one — a desk should see "the
//! current hedge for this risk", not a pile of stale sizes from a moving book. Superseding
//! keeps the original `suggestion_id` invalid, so a fire against a stale id fails loudly
//! rather than trading an out-of-date clip.

use std::collections::BTreeMap;
use std::sync::RwLock;

use celnet_hedge_routing::HedgeRatioPlan;
use celnet_proto::{ExitActionDesc, HedgeSuggestion, RatesPosition};

/// The server-private execution recipe pinned alongside a published suggestion — enough
/// to fire exactly the hedge the trader was shown.
#[derive(Debug, Clone)]
pub struct SuggestionExec {
    /// The risk book the hedge reduces.
    pub book: String,
    /// The **family label** the threshold / provenance are scoped by (`"BOND"`, `"OIS"`).
    pub instrument: String,
    /// The **tradeable security** the venue is asked for. For a vehicle hedge this is the
    /// vehicle's own id (the future / benchmark); for a self-hedge it is the position's
    /// security, falling back to the family label when none resolved.
    pub execution_instrument: String,
    /// The signed net risk at raise (its sign selects the hedge side).
    pub net_risk: f64,
    /// The external size to shed, in the budget metric's native units.
    pub size: f64,
    /// The reference mid captured at raise.
    pub mid: f64,
    /// One bp in the instrument's price convention.
    pub bp_scale: f64,
    /// The originating fill — the template the offsetting leg is built from.
    pub fill: RatesPosition,
    /// The originating fill's own risk in the budget metric — the DENOMINATOR of the
    /// offsetting leg's scale factor (`filled / fill_risk`), exactly as the automatic path
    /// computes it. Because `size` above is already reduced by whole-lot rounding, the
    /// book reduces by what the vehicle trade really removes and the rounding residual
    /// honestly stays behind.
    pub fill_risk: f64,
    /// The budget metric ordinal (for the provenance record).
    pub metric: i32,
    /// The resolved warehouse cap.
    pub threshold: f64,
    /// `|net_risk| / threshold` at raise.
    pub utilization: f64,
    /// The RAG band label at raise.
    pub band: String,
    /// The exact policy path walked.
    pub policy_path: Vec<u32>,
    /// The resolved exit action (carrying its vehicle choice).
    pub action: Option<ExitActionDesc>,
    /// The effective LP set the hedge fans to.
    pub lps: Vec<String>,
    /// The internally-crossed portion of the decision (recorded on provenance).
    pub internal_crossed: f64,
    /// The sized vehicle plan, when the leaf hedges with something other than the
    /// position's own security. `None` for a self-hedge (ratio identically 1).
    pub plan: Option<HedgeRatioPlan>,
}

/// One standing suggestion: what the trader sees, plus how to fire it.
#[derive(Debug, Clone)]
pub struct StandingSuggestion {
    /// The wire projection served to clients.
    pub desc: HedgeSuggestion,
    /// The server-private execution recipe.
    pub exec: SuggestionExec,
}

/// The bounded, in-memory store of standing suggestions — one per `(book, instrument)`.
///
/// Shared behind an `Arc` between the booking path (which raises and supersedes) and the
/// `ListHedgeSuggestions` / `ExecuteHedgeSuggestion` handlers (which read and consume).
#[derive(Debug, Default)]
pub struct SuggestionStore {
    inner: RwLock<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Keyed `(book, instrument)` — latest wins, so a cell never stacks stale sizes.
    by_cell: BTreeMap<(String, String), StandingSuggestion>,
    next_id: u64,
}

impl SuggestionStore {
    /// A fresh, empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Raise (or **supersede**) the suggestion for a cell, minting a fresh id. Returns the
    /// stored row, whose `desc.suggestion_id` is the fire key.
    ///
    /// Superseding is deliberate: the earlier id stops resolving, so a client holding a
    /// stale suggestion cannot fire a size the book has since moved past.
    pub fn raise(&self, mut desc: HedgeSuggestion, exec: SuggestionExec) -> StandingSuggestion {
        let mut g = self.lock();
        let id = g.next_id.saturating_add(1);
        g.next_id = id;
        desc.suggestion_id = format!("SUG-{id}");
        let row = StandingSuggestion { desc, exec };
        g.by_cell.insert(
            (row.desc.book.clone(), row.desc.instrument.clone()),
            row.clone(),
        );
        row
    }

    /// Every standing suggestion, newest first, optionally filtered by book.
    #[must_use]
    pub fn list(&self, book: Option<&str>) -> Vec<HedgeSuggestion> {
        let g = self.lock();
        let mut out: Vec<HedgeSuggestion> = g
            .by_cell
            .values()
            .filter(|s| book.is_none_or(|b| s.desc.book == b))
            .map(|s| s.desc.clone())
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.raised_at));
        out
    }

    /// Look one up by id **without** consuming it.
    #[must_use]
    pub fn get(&self, suggestion_id: &str) -> Option<StandingSuggestion> {
        let g = self.lock();
        g.by_cell
            .values()
            .find(|s| s.desc.suggestion_id == suggestion_id)
            .cloned()
    }

    /// Remove a suggestion by id, returning it. The single consume point shared by both
    /// **fire** and **dismiss** — a fired suggestion must not remain fireable.
    pub fn take(&self, suggestion_id: &str) -> Option<StandingSuggestion> {
        let mut g = self.lock();
        let key = g
            .by_cell
            .iter()
            .find(|(_, s)| s.desc.suggestion_id == suggestion_id)
            .map(|(k, _)| k.clone())?;
        g.by_cell.remove(&key)
    }

    /// How many suggestions stand right now.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().by_cell.len()
    }

    /// Whether nothing is standing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner
            .write()
            .expect("hedge-suggestion store lock poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec(book: &str) -> SuggestionExec {
        SuggestionExec {
            book: book.into(),
            instrument: "BOND".into(),
            execution_instrument: "TY-DEC26".into(),
            net_risk: 95_000.0,
            size: 15_000.0,
            mid: 100.0,
            bp_scale: 1e-2,
            fill: RatesPosition::default(),
            fill_risk: 30_000.0,
            metric: 0,
            threshold: 100_000.0,
            utilization: 0.95,
            band: "red".into(),
            policy_path: vec![0, 2],
            action: None,
            lps: vec!["LP-1".into()],
            internal_crossed: 0.0,
            plan: None,
        }
    }

    fn desc(book: &str, raised_at: i64) -> HedgeSuggestion {
        HedgeSuggestion {
            book: book.into(),
            instrument: "BOND".into(),
            raised_at,
            ..HedgeSuggestion::default()
        }
    }

    #[test]
    fn raising_mints_a_stable_fire_key() {
        let s = SuggestionStore::new();
        let a = s.raise(desc("B1", 10), exec("B1"));
        let b = s.raise(desc("B2", 20), exec("B2"));
        assert_eq!(a.desc.suggestion_id, "SUG-1");
        assert_eq!(b.desc.suggestion_id, "SUG-2");
        assert_eq!(s.len(), 2);
    }

    /// A later fill on the SAME cell replaces the earlier suggestion rather than stacking
    /// a second one, and the superseded id stops resolving — a client holding it cannot
    /// fire a size the book has moved past.
    #[test]
    fn a_later_suggestion_supersedes_the_same_cell_and_invalidates_the_old_id() {
        let s = SuggestionStore::new();
        let first = s.raise(desc("B1", 10), exec("B1"));
        let second = s.raise(desc("B1", 20), exec("B1"));
        assert_eq!(s.len(), 1, "one suggestion per (book, instrument)");
        assert!(
            s.get(&first.desc.suggestion_id).is_none(),
            "the superseded id must no longer resolve"
        );
        assert!(s.get(&second.desc.suggestion_id).is_some());
    }

    #[test]
    fn list_is_newest_first_and_filters_by_book() {
        let s = SuggestionStore::new();
        s.raise(desc("B1", 10), exec("B1"));
        s.raise(desc("B2", 30), exec("B2"));
        s.raise(desc("B3", 20), exec("B3"));
        let all = s.list(None);
        assert_eq!(
            all.iter().map(|x| x.book.as_str()).collect::<Vec<_>>(),
            vec!["B2", "B3", "B1"]
        );
        assert_eq!(s.list(Some("B2")).len(), 1);
        assert!(s.list(Some("NOPE")).is_empty());
    }

    /// Taking is the single consume point both fire and dismiss go through, so a fired
    /// suggestion can never be fired twice.
    #[test]
    fn taking_consumes_exactly_once() {
        let s = SuggestionStore::new();
        let a = s.raise(desc("B1", 10), exec("B1"));
        assert!(s.take(&a.desc.suggestion_id).is_some());
        assert!(
            s.take(&a.desc.suggestion_id).is_none(),
            "a consumed suggestion is not fireable again"
        );
        assert!(s.is_empty());
    }

    #[test]
    fn taking_an_unknown_id_is_none() {
        let s = SuggestionStore::new();
        s.raise(desc("B1", 10), exec("B1"));
        assert!(s.take("SUG-999").is_none());
        assert_eq!(s.len(), 1, "an unknown take removes nothing");
    }

    #[test]
    fn the_execution_recipe_is_pinned_alongside_the_projection() {
        let s = SuggestionStore::new();
        let row = s.raise(desc("B1", 10), exec("B1"));
        let back = s.get(&row.desc.suggestion_id).expect("stored");
        assert_eq!(back.exec.execution_instrument, "TY-DEC26");
        assert!((back.exec.size - 15_000.0).abs() < 1e-12);
        assert!((back.exec.fill_risk - 30_000.0).abs() < 1e-12);
    }
}
