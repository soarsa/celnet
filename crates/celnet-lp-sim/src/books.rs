//! The **book resolver** — the pure core of the book-aware feed.
//!
//! Given (a) the enabled [`AggregatedBookDesc`]s the server currently holds
//! (from `AuthService.ListAggregatedBooks`), (b) the set of `LP-SIM-0N` member
//! connection ids *this* process impersonates, and (c) the canonical instrument ids
//! (CUSIPs) the sim can actually price (its loaded Treasury universe), it computes
//! the exact set of **(LP member × instrument)** streams the sim should be emitting.
//!
//! The rules mirror the server's own consolidation contract (ADR-0022):
//!
//! - A **disabled** book stands up no engine, so it is ignored.
//! - A book whose `member_connection_ids` name **none** of our impersonated members
//!   is ignored (its liquidity comes from other connections, not from us).
//! - For a book we *do* participate in, the members we stream *as* are exactly the
//!   intersection of the book's `member_connection_ids` with our impersonated set.
//! - The instruments we stream are the book's scope:
//!   - `ALL_MEMBERS_QUOTE` ⇒ our full priceable universe (the composite tracks the
//!     union of what its members quote, so we quote everything we can);
//!   - `EXPLICIT` ⇒ the listed `instrument_ids`, intersected with what we can price
//!     (an id we have no bond for is silently skipped — we cannot quote it).
//!
//! The result is a [`StreamPlan`]: a deduplicated, deterministically-ordered set of
//! [`StreamKey`]s. Re-resolving on each poll and [`StreamPlan::diff`]-ing against the
//! previous plan yields the add/remove set as books are created, edited, disabled, or
//! deleted — so a newly-created book is priced on the next poll and a deleted one
//! stops within one poll interval.
//!
//! This module is deliberately free of I/O and async so it is exhaustively unit- and
//! integration-testable without a server (see `tests/book_resolver.rs`).

use std::collections::{BTreeMap, BTreeSet};

use celnet_proto::{AggregatedBookDesc, AggregationScopeMode};

/// Which instruments a book's composite is produced for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookScope {
    /// Aggregate every instrument any member quotes — the sim quotes its whole
    /// priceable universe for this book.
    AllMembersQuote,
    /// Aggregate only these canonical instrument ids (CUSIPs).
    Explicit(Vec<String>),
}

/// A book reduced to just the fields the resolver needs, decoupled from the wire
/// [`AggregatedBookDesc`] so it is trivially constructible in tests and stable
/// against unrelated proto growth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookView {
    /// The book's stable id (for logging the add/remove diff).
    pub id: String,
    /// Whether the book is active (a disabled book stands up no engine).
    pub enabled: bool,
    /// The inbound liquidity members whose quotes feed the composite, by connection id.
    pub member_connection_ids: Vec<String>,
    /// Which instruments the composite is produced for.
    pub scope: BookScope,
}

impl BookView {
    /// Project a wire [`AggregatedBookDesc`] onto the resolver's view. An unknown
    /// scope enum value defaults to `ALL_MEMBERS_QUOTE` (the proto3 zero default),
    /// exactly as the server's own `scope_from_wire` does.
    #[must_use]
    pub fn from_desc(desc: &AggregatedBookDesc) -> Self {
        let scope = match AggregationScopeMode::try_from(desc.scope_mode).unwrap_or_default() {
            AggregationScopeMode::Explicit => BookScope::Explicit(desc.instrument_ids.clone()),
            AggregationScopeMode::AllMembersQuote => BookScope::AllMembersQuote,
        };
        Self {
            id: desc.id.clone(),
            enabled: desc.enabled,
            member_connection_ids: desc.member_connection_ids.clone(),
            scope,
        }
    }
}

/// One `(LP member, instrument)` stream the sim emits: member `lp_name` streams a
/// two-way for canonical `instrument_id`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StreamKey {
    /// The LP connection name / consolidation `VenueId` (e.g. `LP-SIM-01`).
    pub lp_name: String,
    /// The canonical server `instrument_id` (the bond's CUSIP).
    pub instrument_id: String,
}

/// The full set of streams the sim should currently emit — deduplicated and
/// deterministically ordered (a `BTreeSet`), so two resolutions of the same books
/// compare equal and `diff` is stable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamPlan {
    keys: BTreeSet<StreamKey>,
}

/// The change between two plans across a poll: the streams to start and to stop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamDiff {
    /// Streams present in the new plan but not the old — start emitting these.
    pub added: Vec<StreamKey>,
    /// Streams present in the old plan but not the new — stop emitting these (the
    /// server ages the last quote out past the book's max-age cutoff).
    pub removed: Vec<StreamKey>,
}

impl StreamDiff {
    /// Whether nothing changed between the two plans.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

impl StreamPlan {
    /// The number of `(member, instrument)` streams in the plan.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the plan has no streams (no participating enabled book).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Whether the plan contains a specific `(member, instrument)` stream.
    #[must_use]
    pub fn contains(&self, lp_name: &str, instrument_id: &str) -> bool {
        self.keys
            .iter()
            .any(|k| k.lp_name == lp_name && k.instrument_id == instrument_id)
    }

    /// Iterate the plan's stream keys in deterministic order.
    pub fn keys(&self) -> impl Iterator<Item = &StreamKey> {
        self.keys.iter()
    }

    /// The distinct LP member names that appear in the plan.
    #[must_use]
    pub fn members(&self) -> BTreeSet<&str> {
        self.keys.iter().map(|k| k.lp_name.as_str()).collect()
    }

    /// The distinct instrument ids that appear in the plan.
    #[must_use]
    pub fn instruments(&self) -> BTreeSet<&str> {
        self.keys.iter().map(|k| k.instrument_id.as_str()).collect()
    }

    /// Group the plan as `member → { instrument ids }` — the shape the streaming
    /// layer iterates to emit one `LpQuote` per pair each round.
    #[must_use]
    pub fn grouped_by_member(&self) -> BTreeMap<&str, BTreeSet<&str>> {
        let mut out: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for k in &self.keys {
            out.entry(k.lp_name.as_str())
                .or_default()
                .insert(k.instrument_id.as_str());
        }
        out
    }

    /// The add/remove diff to move from `self` (the currently-streamed plan) to
    /// `next` (the freshly-resolved plan).
    #[must_use]
    pub fn diff(&self, next: &StreamPlan) -> StreamDiff {
        StreamDiff {
            added: next.keys.difference(&self.keys).cloned().collect(),
            removed: self.keys.difference(&next.keys).cloned().collect(),
        }
    }
}

/// Resolve the streaming plan from the current books, the members we impersonate, and
/// the instrument ids we can price. See the module docs for the exact rules.
///
/// `members` and `priceable_ids` are sets so membership tests are `O(log n)` and the
/// result is order-independent of the inputs.
#[must_use]
pub fn resolve_plan(
    books: &[BookView],
    members: &BTreeSet<String>,
    priceable_ids: &BTreeSet<String>,
) -> StreamPlan {
    let mut keys = BTreeSet::new();
    for book in books {
        if !book.enabled {
            continue;
        }
        // The members we stream *as* for this book: the ones it lists that we own.
        let mut ours: Vec<&String> = book
            .member_connection_ids
            .iter()
            .filter(|m| members.contains(*m))
            .collect();
        ours.sort();
        ours.dedup();
        if ours.is_empty() {
            continue; // No LP-SIM members in this book — not ours to price.
        }
        // The instruments to quote for this book, intersected with what we can price.
        let ids: Vec<&String> = match &book.scope {
            BookScope::AllMembersQuote => priceable_ids.iter().collect(),
            BookScope::Explicit(list) => list
                .iter()
                .filter(|id| priceable_ids.contains(*id))
                .collect(),
        };
        for m in &ours {
            for id in &ids {
                keys.insert(StreamKey {
                    lp_name: (*m).clone(),
                    instrument_id: (*id).clone(),
                });
            }
        }
    }
    StreamPlan { keys }
}

/// Resolve directly from the wire [`AggregatedBookDesc`]s returned by
/// `ListAggregatedBooks` — the convenience the daemon calls each poll.
#[must_use]
pub fn resolve_from_descs(
    descs: &[AggregatedBookDesc],
    members: &BTreeSet<String>,
    priceable_ids: &BTreeSet<String>,
) -> StreamPlan {
    let views: Vec<BookView> = descs.iter().map(BookView::from_desc).collect();
    resolve_plan(&views, members, priceable_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(ns: &[&str]) -> BTreeSet<String> {
        ns.iter().map(|s| (*s).to_string()).collect()
    }
    fn ids(ns: &[&str]) -> BTreeSet<String> {
        ns.iter().map(|s| (*s).to_string()).collect()
    }
    fn book(id: &str, enabled: bool, members: &[&str], scope: BookScope) -> BookView {
        BookView {
            id: id.to_string(),
            enabled,
            member_connection_ids: members.iter().map(|s| (*s).to_string()).collect(),
            scope,
        }
    }

    #[test]
    fn all_members_quote_fans_out_over_the_whole_priceable_universe() {
        let books = [book(
            "b1",
            true,
            &["LP-SIM-01", "LP-SIM-02"],
            BookScope::AllMembersQuote,
        )];
        let plan = resolve_plan(
            &books,
            &members(&["LP-SIM-01", "LP-SIM-02", "LP-SIM-03"]),
            &ids(&["CUSIP-A", "CUSIP-B"]),
        );
        // 2 participating members × 2 instruments = 4 streams; the non-listed member
        // LP-SIM-03 does not appear.
        assert_eq!(plan.len(), 4);
        assert!(plan.contains("LP-SIM-01", "CUSIP-A"));
        assert!(plan.contains("LP-SIM-02", "CUSIP-B"));
        assert!(!plan.members().contains("LP-SIM-03"));
    }

    #[test]
    fn explicit_scope_is_intersected_with_priceable_ids() {
        let books = [book(
            "b1",
            true,
            &["LP-SIM-04"],
            BookScope::Explicit(vec!["CUSIP-A".into(), "CUSIP-Z".into()]),
        )];
        // CUSIP-Z is not priceable → skipped; only CUSIP-A remains.
        let plan = resolve_plan(
            &books,
            &members(&["LP-SIM-04"]),
            &ids(&["CUSIP-A", "CUSIP-B"]),
        );
        assert_eq!(plan.len(), 1);
        assert!(plan.contains("LP-SIM-04", "CUSIP-A"));
        assert!(!plan.contains("LP-SIM-04", "CUSIP-Z"));
    }

    #[test]
    fn disabled_and_memberless_books_are_ignored() {
        let books = [
            book(
                "disabled",
                false,
                &["LP-SIM-01"],
                BookScope::AllMembersQuote,
            ),
            book(
                "no-members",
                true,
                &["OTHER-LP"],
                BookScope::AllMembersQuote,
            ),
        ];
        let plan = resolve_plan(&books, &members(&["LP-SIM-01"]), &ids(&["CUSIP-A"]));
        assert!(plan.is_empty());
    }

    #[test]
    fn diff_tracks_added_and_removed_streams() {
        let empty = StreamPlan::default();
        let one = resolve_plan(
            &[book("b1", true, &["LP-SIM-01"], BookScope::AllMembersQuote)],
            &members(&["LP-SIM-01"]),
            &ids(&["CUSIP-A"]),
        );
        let d = empty.diff(&one);
        assert_eq!(d.added.len(), 1);
        assert!(d.removed.is_empty());
        // Removing the book reverses the diff.
        let back = one.diff(&empty);
        assert!(back.added.is_empty());
        assert_eq!(back.removed.len(), 1);
    }
}
