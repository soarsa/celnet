//! The **decision journal** — the audit log of every trader-rule evaluation, including
//! the ones that deliberately did nothing.
//!
//! # Why this exists
//!
//! CelNet has three trader-configurable decision graphs — acceptance
//! ([`celnet_acceptance`]), risk routing ([`celnet_risk_routing`]) and the hedge exit
//! policy ([`celnet_hedge_routing`]). Each already leaves a trail when it *acts*: an
//! accepted lift books a `Deal`, a fired hedge stamps a
//! [`HedgeProvenance`](celnet_proto::HedgeProvenance) carrying the walked `policy_path`.
//!
//! What none of them left was a record when they **did not** act. A green-band book, a
//! graph that walked to a `WAREHOUSE` leaf, a kill-switched desk, a rate guard that
//! downgraded a live fire to advisory, a fill that routed nowhere because no graph was
//! installed — every one of those returned quietly. A trader asking *"why did my hedge
//! rule not fire on that position?"* had nothing to read.
//!
//! This ring closes that gap. **Every** evaluation lands here, `FIRED` and `NO_ACTION`
//! alike, each carrying the exact node path walked and a **stated reason**. Guardrail 2
//! applied to observability: a silent fallback becomes a stated reason.
//!
//! # What it is not
//!
//! It is a **bounded, in-process, lossy** ring — a recent-history control-plane window,
//! exactly like the hedge provenance ring and the lift trace store, not a durable
//! compliance archive. It says so on the wire:
//! [`ListDecisionJournalResponse`](celnet_proto::ListDecisionJournalResponse) carries
//! `total_recorded` and `evicted`, so a client can tell an empty journal from one whose
//! window has rolled past the question, and must never render a truncated log as
//! complete. Durable retention is a separate (unbuilt) concern — see
//! `docs/DECISION-AUDIT.md`.
//!
//! # Off-core (guardrail 11)
//!
//! Recording is a short `push_back` under one write lock on the booking / control tier —
//! the same tier the provenance ring and the intent broadcast already run on. The pinned
//! zero-alloc pricing core cannot reach this type: it lives in `celnet-server`, which
//! `celnet-engine` does not depend on.

use std::collections::VecDeque;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use celnet_proto::{DecisionEngineEnum, DecisionOutcomeEnum, DecisionRecord};

/// Default ring depth — the number of recent evaluations retained before the oldest
/// falls off. Sized to hold a busy desk's recent session while staying bounded memory
/// (guardrail 6). Deeper than the 4 096-trace lift store because a single lift can
/// produce several decision rows (acceptance + routing + hedge).
pub const DEFAULT_JOURNAL_CAPACITY: usize = 16_384;

/// Default and maximum row cap for one `ListDecisionJournal` reply (server-clamped).
const DEFAULT_LIST_LIMIT: usize = 500;
const MAX_LIST_LIMIT: usize = 5_000;

/// The filters one journal query ANDs together. Every field absent ⇒ no restriction.
#[derive(Debug, Clone, Default)]
pub struct JournalQuery {
    /// Restrict to one engine.
    pub engine: Option<DecisionEngineEnum>,
    /// Restrict to fired-only / no-action-only.
    pub outcome: Option<DecisionOutcomeEnum>,
    /// Restrict to one risk book.
    pub book: Option<String>,
    /// Restrict to one instrument / family label.
    pub instrument: Option<String>,
    /// Restrict to one counterparty.
    pub counterparty: Option<String>,
    /// Only records decided at or after this epoch-nanos instant.
    pub since_nanos: Option<i64>,
    /// Row cap (clamped to [`MAX_LIST_LIMIT`]; `None`/`0` ⇒ [`DEFAULT_LIST_LIMIT`]).
    pub limit: Option<u32>,
}

/// What a query returned, plus the lossiness accounting a client needs to be honest
/// about the window it is rendering.
#[derive(Debug, Clone)]
pub struct JournalPage {
    /// The matching rows, newest first (descending `seq`).
    pub records: Vec<DecisionRecord>,
    /// Rows the journal has accepted over the process lifetime.
    pub total_recorded: u64,
    /// Rows evicted at capacity over the process lifetime. Non-zero ⇒ the window is
    /// lossy and the client must say so.
    pub evicted: u64,
}

/// The bounded decision-evaluation ring. Shared behind an `Arc` between every producer
/// (the auto-hedge engine, the acceptance gate, the rates booking path) and the
/// `ListDecisionJournal` / `ListRuleAdvice` handlers.
#[derive(Debug)]
pub struct DecisionJournal {
    ring: RwLock<VecDeque<DecisionRecord>>,
    capacity: usize,
    /// Next `seq` to hand out (starts at 1; 0 is reserved "no record").
    next_seq: AtomicU64,
    /// Lifetime accepted-row count.
    recorded: AtomicU64,
    /// Lifetime evicted-row count.
    evicted: AtomicU64,
}

impl Default for DecisionJournal {
    fn default() -> Self {
        Self::new(DEFAULT_JOURNAL_CAPACITY)
    }
}

impl DecisionJournal {
    /// A fresh journal retaining `capacity` rows.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            ring: RwLock::new(VecDeque::new()),
            capacity: capacity.max(1),
            next_seq: AtomicU64::new(1),
            recorded: AtomicU64::new(0),
            evicted: AtomicU64::new(0),
        }
    }

    /// Append one evaluation, assigning it the next `seq`. Returns the stored row (with
    /// its assigned `seq`), so a caller can carry the citation key onward.
    pub fn record(&self, mut rec: DecisionRecord) -> DecisionRecord {
        rec.seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        self.recorded.fetch_add(1, Ordering::Relaxed);
        let mut g = self.ring.write().expect("decision journal lock poisoned");
        while g.len() >= self.capacity {
            if g.pop_front().is_some() {
                self.evicted.fetch_add(1, Ordering::Relaxed);
            } else {
                break;
            }
        }
        g.push_back(rec.clone());
        rec
    }

    /// Run one filtered query, newest first.
    #[must_use]
    pub fn query(&self, q: &JournalQuery) -> JournalPage {
        let cap = match q.limit {
            None | Some(0) => DEFAULT_LIST_LIMIT,
            Some(n) => (n as usize).min(MAX_LIST_LIMIT),
        };
        let g = self.ring.read().expect("decision journal lock poisoned");
        let records: Vec<DecisionRecord> = g
            .iter()
            .rev()
            .filter(|r| q.engine.is_none_or(|e| r.engine == e as i32))
            .filter(|r| q.outcome.is_none_or(|o| r.outcome == o as i32))
            .filter(|r| q.book.as_deref().is_none_or(|b| r.book == b))
            .filter(|r| q.instrument.as_deref().is_none_or(|i| r.instrument == i))
            .filter(|r| {
                q.counterparty
                    .as_deref()
                    .is_none_or(|c| r.counterparty.as_deref() == Some(c))
            })
            .filter(|r| q.since_nanos.is_none_or(|t| r.decided_at >= t))
            .take(cap)
            .cloned()
            .collect();
        JournalPage {
            records,
            total_recorded: self.recorded.load(Ordering::Relaxed),
            evicted: self.evicted.load(Ordering::Relaxed),
        }
    }

    /// Every retained row, newest first — the input the rule advisor derives from. Kept
    /// separate from [`Self::query`] so the advisor always reads the WHOLE window rather
    /// than a client-clamped page.
    #[must_use]
    pub fn snapshot(&self) -> Vec<DecisionRecord> {
        let g = self.ring.read().expect("decision journal lock poisoned");
        g.iter().rev().cloned().collect()
    }

    /// How many rows are retained right now.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ring
            .read()
            .expect("decision journal lock poisoned")
            .len()
    }

    /// Whether nothing has been recorded (or everything has aged out).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Rows evicted at capacity over the process lifetime.
    #[must_use]
    pub fn evicted(&self) -> u64 {
        self.evicted.load(Ordering::Relaxed)
    }
}

/// Build a `DecisionRecord` skeleton for one engine + verdict. Callers fill the
/// engine-specific fields on the returned value; `seq` is assigned by
/// [`DecisionJournal::record`].
///
/// `reason` is **not** optional by design: a `NO_ACTION` row with an empty reason is
/// exactly the silent fallback this module exists to abolish.
#[must_use]
pub fn decision(
    engine: DecisionEngineEnum,
    outcome: DecisionOutcomeEnum,
    outcome_label: impl Into<String>,
    reason: impl Into<String>,
    decided_at: i64,
) -> DecisionRecord {
    DecisionRecord {
        seq: 0,
        decided_at,
        engine: engine as i32,
        outcome: outcome as i32,
        outcome_label: outcome_label.into(),
        reason: reason.into(),
        ..DecisionRecord::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(engine: DecisionEngineEnum, outcome: DecisionOutcomeEnum, book: &str) -> DecisionRecord {
        DecisionRecord {
            book: book.to_owned(),
            ..decision(engine, outcome, "label", "because", 1_000)
        }
    }

    #[test]
    fn seq_is_monotonic_from_one_and_returned_to_the_caller() {
        let j = DecisionJournal::new(8);
        let a = j.record(row(
            DecisionEngineEnum::DecisionEngineHedge,
            DecisionOutcomeEnum::DecisionOutcomeFired,
            "B",
        ));
        let b = j.record(row(
            DecisionEngineEnum::DecisionEngineHedge,
            DecisionOutcomeEnum::DecisionOutcomeFired,
            "B",
        ));
        assert_eq!(a.seq, 1);
        assert_eq!(b.seq, 2);
        assert_eq!(j.len(), 2);
    }

    #[test]
    fn query_is_newest_first_and_filters_and_clamps() {
        let j = DecisionJournal::new(64);
        j.record(row(
            DecisionEngineEnum::DecisionEngineHedge,
            DecisionOutcomeEnum::DecisionOutcomeNoAction,
            "rates-usd",
        ));
        j.record(row(
            DecisionEngineEnum::DecisionEngineAcceptance,
            DecisionOutcomeEnum::DecisionOutcomeFired,
            "rates-eur",
        ));
        j.record(row(
            DecisionEngineEnum::DecisionEngineHedge,
            DecisionOutcomeEnum::DecisionOutcomeFired,
            "rates-usd",
        ));

        let all = j.query(&JournalQuery::default());
        assert_eq!(all.records.len(), 3);
        assert_eq!(all.records[0].seq, 3, "newest first");
        assert_eq!(all.total_recorded, 3);
        assert_eq!(all.evicted, 0);

        let hedged = j.query(&JournalQuery {
            engine: Some(DecisionEngineEnum::DecisionEngineHedge),
            ..Default::default()
        });
        assert_eq!(hedged.records.len(), 2);

        let idle = j.query(&JournalQuery {
            outcome: Some(DecisionOutcomeEnum::DecisionOutcomeNoAction),
            ..Default::default()
        });
        assert_eq!(idle.records.len(), 1);
        assert_eq!(idle.records[0].seq, 1);

        let by_book = j.query(&JournalQuery {
            book: Some("rates-eur".to_owned()),
            ..Default::default()
        });
        assert_eq!(by_book.records.len(), 1);

        let clamped = j.query(&JournalQuery {
            limit: Some(1),
            ..Default::default()
        });
        assert_eq!(clamped.records.len(), 1);
        assert_eq!(clamped.records[0].seq, 3);
    }

    #[test]
    fn since_filter_excludes_older_rows() {
        let j = DecisionJournal::new(8);
        j.record(DecisionRecord {
            decided_at: 100,
            ..row(
                DecisionEngineEnum::DecisionEngineHedge,
                DecisionOutcomeEnum::DecisionOutcomeFired,
                "B",
            )
        });
        j.record(DecisionRecord {
            decided_at: 300,
            ..row(
                DecisionEngineEnum::DecisionEngineHedge,
                DecisionOutcomeEnum::DecisionOutcomeFired,
                "B",
            )
        });
        let recent = j.query(&JournalQuery {
            since_nanos: Some(200),
            ..Default::default()
        });
        assert_eq!(recent.records.len(), 1);
        assert_eq!(recent.records[0].decided_at, 300);
    }

    /// The window is bounded and SAYS SO: eviction is counted and surfaced, so a client
    /// never renders a rolled-past window as a complete audit trail.
    #[test]
    fn eviction_is_counted_and_reported() {
        let j = DecisionJournal::new(3);
        for _ in 0..5 {
            j.record(row(
                DecisionEngineEnum::DecisionEngineHedge,
                DecisionOutcomeEnum::DecisionOutcomeFired,
                "B",
            ));
        }
        assert_eq!(j.len(), 3);
        let page = j.query(&JournalQuery::default());
        assert_eq!(page.total_recorded, 5);
        assert_eq!(page.evicted, 2);
        assert_eq!(j.evicted(), 2);
        assert_eq!(
            page.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![5, 4, 3],
            "the oldest two rolled off"
        );
    }

    #[test]
    fn counterparty_filter_matches_only_the_named_name() {
        let j = DecisionJournal::new(8);
        j.record(DecisionRecord {
            counterparty: Some("cp-a".to_owned()),
            ..row(
                DecisionEngineEnum::DecisionEngineAcceptance,
                DecisionOutcomeEnum::DecisionOutcomeNoAction,
                "",
            )
        });
        j.record(DecisionRecord {
            counterparty: Some("cp-b".to_owned()),
            ..row(
                DecisionEngineEnum::DecisionEngineAcceptance,
                DecisionOutcomeEnum::DecisionOutcomeNoAction,
                "",
            )
        });
        j.record(row(
            DecisionEngineEnum::DecisionEngineAcceptance,
            DecisionOutcomeEnum::DecisionOutcomeFired,
            "",
        ));
        let only_a = j.query(&JournalQuery {
            counterparty: Some("cp-a".to_owned()),
            ..Default::default()
        });
        assert_eq!(only_a.records.len(), 1);
        assert_eq!(only_a.records[0].counterparty.as_deref(), Some("cp-a"));
    }

    #[test]
    fn snapshot_reads_the_whole_window_regardless_of_the_client_limit() {
        let j = DecisionJournal::new(1_000);
        for _ in 0..600 {
            j.record(row(
                DecisionEngineEnum::DecisionEngineHedge,
                DecisionOutcomeEnum::DecisionOutcomeFired,
                "B",
            ));
        }
        assert_eq!(
            j.query(&JournalQuery::default()).records.len(),
            DEFAULT_LIST_LIMIT,
            "a client page is clamped"
        );
        assert_eq!(j.snapshot().len(), 600, "the advisor reads everything");
    }
}
