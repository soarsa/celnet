//! The server-side **linear-rates position book** the dealer-quoting feature books
//! into and the `RiskService` Book/List reads serve.
//!
//! # Why a dedicated store (not the FX [`PositionStore`])
//!
//! The FX [`PositionStore`](super::risk::store::PositionStore) warehouses
//! convention-baked vanilla/exotic [`RiskFact`](celnet_risk_cube::RiskFact)s under a
//! full org [`Hierarchy`](celnet_risk_cube::Hierarchy) and an
//! [`Underlying`](celnet_types::Underlying) axis. A linear-rates position
//! ([`RatesPosition`]) is a far leaner fact: a `(entity, book)` cell plus a
//! `RatesInstrument` priced against a request-supplied `CurveSet`. It has **no**
//! `Underlying`, no Book→Desk hierarchy, and no marking surface — so forcing it
//! through the FX fact key (which *requires* an `Underlying`) would be dishonest.
//! This module is the rates analogue of that store: in-memory, [`RwLock`]-guarded,
//! RAII-clean, with a deterministic monotonic id counter, and an entitlement
//! predicate scoped to the two org axes a rates cell actually carries.
//!
//! The same store instance is shared (behind an [`Arc`](std::sync::Arc)) by the
//! `RiskService` edge (which books via `BookRatesPosition` and reads via
//! `ListRatesPositions`) and the [`RfqDeskService`](super::desk) edge (whose
//! `AcceptDeskQuote` books the dealt position here), so the desk blotter and the
//! Book workspace read one coherent book.

use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_proto::{EntitlementPrincipal, EntitlementRule, RatesPosition, RiskDimension};

/// The shared in-memory linear-rates position book. Cheap to share behind an
/// [`Arc`](std::sync::Arc); every mutation takes the write lock briefly. Lives
/// strictly on the async edge — never the pinned zero-alloc pricing core.
#[derive(Debug)]
pub struct RatesPositionStore {
    inner: RwLock<Vec<RatesPosition>>,
    /// The monotonic id source: the next server-assigned `position_id`. Starts at
    /// `1` (id `0` is the wire "assign me a fresh id" sentinel), so booking is
    /// deterministic and test-reproducible (no wall-clock / randomness).
    next_id: AtomicU64,
}

impl Default for RatesPositionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RatesPositionStore {
    /// An empty rates book with the id counter primed at `1`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Book a rates position, returning the stored fact (with its assigned id).
    ///
    /// `position.position_id == 0` ⇒ the server assigns a fresh monotonic id (a new
    /// booking). A non-zero id **upserts** (supersedes) the current fact for that id
    /// — one current fact per `position_id`, mirroring the FX store's `upsert`
    /// supersede semantics — so a re-book never double-counts.
    #[must_use]
    pub fn book(&self, mut position: RatesPosition) -> RatesPosition {
        let mut g = self
            .inner
            .write()
            .expect("rates position store lock poisoned");
        if position.position_id == 0 {
            position.position_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        } else {
            // Keep the counter ahead of any explicitly-booked id so a later
            // auto-assigned id never collides with a client-chosen one.
            let mut cur = self.next_id.load(Ordering::Relaxed);
            while position.position_id >= cur {
                match self.next_id.compare_exchange(
                    cur,
                    position.position_id + 1,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(observed) => cur = observed,
                }
            }
        }
        if let Some(slot) = g.iter_mut().find(|p| p.position_id == position.position_id) {
            *slot = position;
        } else {
            g.push(position);
        }
        position
    }

    /// A deterministic snapshot of the whole book, ascending by `position_id`.
    #[must_use]
    pub fn snapshot(&self) -> Vec<RatesPosition> {
        let g = self
            .inner
            .read()
            .expect("rates position store lock poisoned");
        let mut out = g.clone();
        out.sort_by_key(|p| p.position_id);
        out
    }

    /// The number of booked rates positions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .expect("rates position store lock poisoned")
            .len()
    }

    /// Whether the rates book is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Whether a rates `(entity, book)` cell is admitted by an asserted entitlement
/// principal — the post-boundary pruning predicate `ListRatesPositions` applies,
/// mirroring the FX `ListPositions` deny-by-default semantics over the **flat**
/// org axes a rates cell carries (no Book→Desk hierarchy, no underlying):
///
/// * an **absent** principal ⇒ grant-all (the post-boundary convention
///   [`convert::principal_of`](super::risk::convert::principal_of) uses: the access
///   boundary already mode-gated the absence; here it means "see everything");
/// * **deny wins**: any deny rule covering the cell denies it;
/// * `grant_all` ⇒ admitted (after deny);
/// * otherwise admitted iff some grant rule covers the cell.
#[must_use]
pub fn admits_rates_cell(principal: Option<&EntitlementPrincipal>, entity: u32, book: u32) -> bool {
    let Some(p) = principal else {
        return true;
    };
    if p.denies.iter().any(|r| rule_covers_cell(r, entity, book)) {
        return false;
    }
    if p.grant_all {
        return true;
    }
    p.grants.iter().any(|r| rule_covers_cell(r, entity, book))
}

/// Whether one entitlement rule covers a rates `(entity, book)` cell. A rule covers
/// the cell iff **every** scope it carries is on a dimension the flat rates cell
/// models (`ENTITY`, `BOOK`, or the `FIRM` apex) and matches. A scope on any other
/// org dimension (`TRADER` / `DESK` / `LOCATION` / `UNDERLYING`) cannot match a
/// rates cell — which is not trader/desk/location/underlying-attributed — so the
/// rule does not cover it. An empty rule (no scopes) is a firm-wide grant: it covers
/// every cell (`all` over an empty iterator is `true`).
fn rule_covers_cell(rule: &EntitlementRule, entity: u32, book: u32) -> bool {
    rule.scopes
        .iter()
        .all(|s| match RiskDimension::try_from(s.dimension) {
            Ok(RiskDimension::Firm) => true,
            Ok(RiskDimension::Entity) => s.value == u64::from(entity),
            Ok(RiskDimension::Book) => s.value == u64::from(book),
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{
        EntitlementRule, OisInstrument, RatesInstrument, RiskScope, Side, rates_instrument,
    };

    fn position(id: u64, entity: u32, book: u32) -> RatesPosition {
        RatesPosition {
            position_id: id,
            entity,
            book,
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: 5,
                    fixed_rate: 0.04,
                    notional: 10_000_000.0,
                    side: Side::Buy as i32,
                })),
            }),
        }
    }

    /// A zero-id booking is assigned a fresh monotonic id; a snapshot reports the
    /// book ascending by id.
    #[test]
    fn booking_assigns_monotonic_ids() {
        let store = RatesPositionStore::new();
        let a = store.book(position(0, 1, 10));
        let b = store.book(position(0, 1, 11));
        assert_eq!(a.position_id, 1);
        assert_eq!(b.position_id, 2);
        assert_eq!(store.len(), 2);
        let snap = store.snapshot();
        assert_eq!(
            snap.iter().map(|p| p.position_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    /// A non-zero id upserts (supersedes) the current fact for that id — one current
    /// fact per id, never a duplicate.
    #[test]
    fn rebooking_an_id_supersedes() {
        let store = RatesPositionStore::new();
        let first = store.book(position(0, 1, 10));
        assert_eq!(first.position_id, 1);
        // Re-book id 1 in a different book — supersede, not duplicate.
        let again = store.book(position(1, 1, 99));
        assert_eq!(again.position_id, 1);
        assert_eq!(store.len(), 1);
        assert_eq!(store.snapshot()[0].book, 99);
    }

    /// An explicit id keeps the auto-id counter ahead, so no later auto-assigned id
    /// collides with a client-chosen one.
    #[test]
    fn explicit_id_advances_the_counter() {
        let store = RatesPositionStore::new();
        let _ = store.book(position(50, 1, 10));
        let next = store.book(position(0, 1, 11));
        assert_eq!(next.position_id, 51);
    }

    /// Absent principal ⇒ grant-all; a grant on `BOOK=10` admits only that book; a
    /// deny wins over a grant.
    #[test]
    fn entitlement_prunes_by_book_cell() {
        assert!(admits_rates_cell(None, 1, 10));

        let grant_book_10 = EntitlementPrincipal {
            grant_all: false,
            grants: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: RiskDimension::Book as i32,
                    value: 10,
                }],
            }],
            denies: vec![],
        };
        assert!(admits_rates_cell(Some(&grant_book_10), 1, 10));
        assert!(!admits_rates_cell(Some(&grant_book_10), 1, 11));

        let grant_all_deny_book_11 = EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: RiskDimension::Book as i32,
                    value: 11,
                }],
            }],
        };
        assert!(admits_rates_cell(Some(&grant_all_deny_book_11), 1, 10));
        assert!(!admits_rates_cell(Some(&grant_all_deny_book_11), 1, 11));
    }

    /// A grant on a dimension a flat rates cell does NOT model (e.g. DESK) cannot
    /// match it — the rate cell is not desk-attributed.
    #[test]
    fn grant_on_unmodelled_dimension_does_not_match() {
        let grant_desk = EntitlementPrincipal {
            grant_all: false,
            grants: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: RiskDimension::Desk as i32,
                    value: 3,
                }],
            }],
            denies: vec![],
        };
        assert!(!admits_rates_cell(Some(&grant_desk), 1, 10));
    }
}
