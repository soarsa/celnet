//! The **rule advisor** — rule suggestions derived from the recorded
//! [decision journal](super::decision_journal), and from nothing else.
//!
//! # The honesty contract
//!
//! Every suggestion this module emits is a **counted pattern over rows the server
//! actually wrote**. A suggestion carries:
//!
//! * `occurrences` — the true number of supporting rows;
//! * `first_seen` / `last_seen` — the real timestamps of the first and last of them;
//! * `evidence_seqs` — the `DecisionRecord.seq` values themselves, so a trader can pull
//!   those exact rows up in the audit table and check the reasoning before acting;
//! * `editor` — which existing rule editor to open, because every suggestion must be
//!   something the trader can go and *do*, not an oracle pronouncement.
//!
//! Nothing is modelled, extrapolated, or scored by a heuristic that outruns the data. If
//! a pattern is not visible in the journal, no advice is produced for it.
//!
//! # What is deliberately NOT derived here
//!
//! *Counterparty adverse-selection skew* ("this name's fills are consistently followed by
//! an adverse mark move ⇒ skew them"). That derivation needs a post-fill mark trajectory
//! per fill — the mid at `t+n` for several `n` — which nothing in CelNet retains today:
//! the journal records the state **at** the decision, the deal blotter records the dealt
//! level, and the aggregation book keeps only the live top-of-book. Rather than fabricate
//! a plausible-looking skew recommendation from data that does not exist, this advisor
//! emits no such suggestion. See `docs/DECISION-AUDIT.md` §5 for the store that would
//! have to be built first.

use std::collections::BTreeMap;

use celnet_proto::{DecisionEngineEnum, DecisionOutcomeEnum, DecisionRecord, RuleAdvice};

/// How many supporting rows a pattern needs before it is worth a trader's attention. One
/// occurrence is an event, not a pattern; the journal table already shows single events.
const MIN_OCCURRENCES: usize = 3;

/// How many `seq` citations to carry per suggestion (the newest ones). `occurrences`
/// always reports the true count.
const EVIDENCE_SAMPLE: usize = 12;

/// Substrings the emitting sites write into `DecisionRecord.reason`. Matching on these is
/// safe because the same crate owns both ends: the constants below are the *only* way
/// these reasons are produced (see `auto_hedge::engine` and `desk::mod`).
pub(crate) mod reasons {
    /// No hedge policy graph resolved for the scope at all.
    pub(crate) const NO_POLICY: &str = "no hedge policy";
    /// A policy existed but was structurally broken.
    pub(crate) const BROKEN_POLICY: &str = "policy graph did not resolve";
    /// The desk kill-switch / disabled flag suppressed the decision.
    pub(crate) const HALTED: &str = "halted:";
    /// The graph walked to a WAREHOUSE leaf — the rule ran and chose to hold.
    pub(crate) const WAREHOUSE_LEAF: &str = "WAREHOUSE";
    /// A rate / notional guard downgraded a live external fire to advisory.
    pub(crate) const GUARD: &str = "→ advisory";
    /// The named hedge vehicle could not be resolved and the fire fell back to a
    /// self-hedge.
    pub(crate) const VEHICLE_UNRESOLVED: &str = "hedge vehicle did not resolve";
    /// No risk-routing graph was installed, so the fill booked unrouted.
    pub(crate) const NO_ROUTING: &str = "no routing policy";
}

/// A band label that means the book is at or past its warning level.
fn is_hot_band(band: &str) -> bool {
    matches!(band, "amber" | "red" | "breach")
}

/// Accumulator for one suggestion group while the rows are folded.
#[derive(Debug, Default)]
struct Group {
    occurrences: u32,
    first_seen: i64,
    last_seen: i64,
    seqs: Vec<u64>,
    book: String,
    instrument: String,
    counterparty: Option<String>,
    /// The peak utilisation observed across the group (hedge patterns only) — real,
    /// read off the rows, used to make the rationale concrete.
    peak_utilization: f64,
}

impl Group {
    fn fold(&mut self, r: &DecisionRecord) {
        if self.occurrences == 0 {
            self.first_seen = r.decided_at;
            self.book = r.book.clone();
            self.instrument = r.instrument.clone();
            self.counterparty = r.counterparty.clone();
        }
        self.occurrences = self.occurrences.saturating_add(1);
        self.first_seen = self.first_seen.min(r.decided_at);
        self.last_seen = self.last_seen.max(r.decided_at);
        self.peak_utilization = self.peak_utilization.max(r.utilization);
        if self.seqs.len() < EVIDENCE_SAMPLE {
            self.seqs.push(r.seq);
        }
    }
}

/// The identity of one suggestion group: its derivation kind plus the scope it is about.
type GroupKey = (&'static str, String, String, Option<String>);

/// Derive the suggestions supported by `rows` (a journal snapshot, newest first),
/// optionally narrowed to one `book`.
///
/// Returns the suggestions ordered strongest-evidence-first, and the number of rows the
/// derivation actually read (the honest denominator a client renders beside them).
#[must_use]
pub fn derive(rows: &[DecisionRecord], book: Option<&str>) -> (Vec<RuleAdvice>, u32) {
    let considered: Vec<&DecisionRecord> = rows
        .iter()
        .filter(|r| book.is_none_or(|b| r.book == b))
        .collect();

    let mut groups: BTreeMap<GroupKey, Group> = BTreeMap::new();
    let mut add = |kind: &'static str, r: &DecisionRecord, by_cp: bool| {
        let key: GroupKey = (
            kind,
            r.book.clone(),
            r.instrument.clone(),
            if by_cp { r.counterparty.clone() } else { None },
        );
        groups.entry(key).or_default().fold(r);
    };

    for r in &considered {
        let hedge = r.engine == DecisionEngineEnum::DecisionEngineHedge as i32;
        let acceptance = r.engine == DecisionEngineEnum::DecisionEngineAcceptance as i32;
        let routing = r.engine == DecisionEngineEnum::DecisionEngineRiskRouting as i32;
        let idle = r.outcome == DecisionOutcomeEnum::DecisionOutcomeNoAction as i32;
        let reason = r.reason.as_str();

        let unpoliced = reason.contains(reasons::NO_POLICY);
        // A book governed by no authored policy at any scope is worth flagging whether or
        // not the built-in default graph happened to act on this particular evaluation.
        if hedge && unpoliced {
            add("hedge_policy_missing", r, false);
        }
        if hedge && idle && reason.contains(reasons::BROKEN_POLICY) {
            add("hedge_policy_broken", r, false);
        }
        if hedge && idle && reason.contains(reasons::HALTED) {
            add("desk_halted", r, false);
        }
        // A WAREHOUSE leaf reached while the book is ALREADY hot is the interesting one:
        // the rule ran, walked the trader's own graph, and chose to hold risk that is at
        // or past the warning band. A green-band warehouse hold is correct and is not
        // advice-worthy.
        // (Suppressed when there is no authored policy at all — that is the stronger,
        // more actionable finding and is already reported above; reporting both would
        // just say the same thing twice.)
        if hedge
            && idle
            && !unpoliced
            && reason.contains(reasons::WAREHOUSE_LEAF)
            && is_hot_band(&r.band)
        {
            add("hedge_warehouses_hot_book", r, false);
        }
        if hedge && reason.contains(reasons::GUARD) {
            add("hedge_guard_suppressing", r, false);
        }
        if hedge && reason.contains(reasons::VEHICLE_UNRESOLVED) {
            add("hedge_vehicle_unresolved", r, false);
        }
        if acceptance && idle && r.outcome_label == "hold" {
            add("acceptance_hold_recurring", r, true);
        }
        if acceptance && idle && r.outcome_label == "reject" {
            add("acceptance_reject_recurring", r, true);
        }
        if routing && idle && reason.contains(reasons::NO_ROUTING) {
            add("routing_unrouted", r, false);
        }
    }

    let mut advice: Vec<RuleAdvice> = groups
        .into_iter()
        .filter(|(_, g)| g.occurrences as usize >= MIN_OCCURRENCES)
        .map(|((kind, ..), g)| build(kind, &g))
        .collect();
    // Strongest evidence first; ties broken by recency, then by id so the order is total
    // and a client's table never reshuffles between identical responses.
    advice.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then(b.last_seen.cmp(&a.last_seen))
            .then(a.advice_id.cmp(&b.advice_id))
    });
    (advice, u32::try_from(considered.len()).unwrap_or(u32::MAX))
}

/// Render one accumulated group as its wire suggestion. Every number in the prose comes
/// off the group; nothing is invented.
fn build(kind: &'static str, g: &Group) -> RuleAdvice {
    let n = g.occurrences;
    let scope_label = match (g.book.is_empty(), g.instrument.is_empty()) {
        (false, false) => format!("{} · {}", g.book, g.instrument),
        (false, true) => g.book.clone(),
        (true, false) => g.instrument.clone(),
        (true, true) => "the firm scope".to_owned(),
    };
    let cp = g.counterparty.clone().unwrap_or_default();
    let peak = format!("{:.0}%", g.peak_utilization * 100.0);

    let (engine, title, rationale, action, editor) = match kind {
        "hedge_policy_missing" => (
            DecisionEngineEnum::DecisionEngineHedge,
            format!("{scope_label} has no hedge policy"),
            format!(
                "{n} risk evaluations on {scope_label} found no hedge policy graph to walk — \
                 not even a firm-wide one — so the risk was warehoused by default. Peak \
                 utilisation over those evaluations was {peak} of the warehouse cap."
            ),
            "Author a hedge rule for this scope (or a firm-wide fallback) in the Hedging \
             rule builder, so a breach resolves to a real exit action instead of an \
             implicit hold."
                .to_owned(),
            "hedging",
        ),
        "hedge_policy_broken" => (
            DecisionEngineEnum::DecisionEngineHedge,
            format!("{scope_label}'s hedge policy is structurally broken"),
            format!(
                "{n} evaluations on {scope_label} could not resolve the installed hedge \
                 graph and degraded to a warehouse hold. A graph that fails to resolve \
                 never fires, whatever the band."
            ),
            "Open the hedge rule builder for this scope and re-validate the graph — an \
             edge points at a node that no longer exists, or the rules form a cycle."
                .to_owned(),
            "hedging",
        ),
        "desk_halted" => (
            DecisionEngineEnum::DecisionEngineHedge,
            format!("Hedging is halted for {scope_label}"),
            format!(
                "{n} evaluations on {scope_label} were suppressed before the policy graph \
                 ran because the desk kill-switch is engaged or the desk is disabled. \
                 Nothing this desk warehouses can be hedged while that holds."
            ),
            "If the halt was not intended, clear the kill-switch / re-enable the desk in \
             the hedge engine configuration."
                .to_owned(),
            "hedge_config",
        ),
        "hedge_warehouses_hot_book" => (
            DecisionEngineEnum::DecisionEngineHedge,
            format!("{scope_label} warehouses risk while already at the warning band"),
            format!(
                "{n} evaluations on {scope_label} walked the trader's own hedge graph and \
                 landed on a WAREHOUSE leaf while the book was in the amber/red band \
                 (peak utilisation {peak} of cap). The rule is running — it is choosing \
                 to hold."
            ),
            "Check the branch conditions that route to the WAREHOUSE leaf: the band or \
             utilisation threshold guarding your exit action is probably set above where \
             this book actually trades. The audit rows below show the exact path walked."
                .to_owned(),
            "hedging",
        ),
        "hedge_guard_suppressing" => (
            DecisionEngineEnum::DecisionEngineHedge,
            format!("Rate / notional guards are downgrading {scope_label} hedges to advisory"),
            format!(
                "{n} hedge decisions on {scope_label} resolved a live external action and \
                 were then downgraded to an ADVISORY (nothing traded) because a \
                 max-hedges-per-interval or daily-external-notional guard was already \
                 spent."
            ),
            "Either raise the guard in the hedge engine configuration, or reduce the \
             per-fire clip so the same budget covers more fires. Leaving it as-is means \
             the policy fires on paper and never on the street."
                .to_owned(),
            "hedge_config",
        ),
        "hedge_vehicle_unresolved" => (
            DecisionEngineEnum::DecisionEngineHedge,
            format!("The hedge vehicle for {scope_label} does not resolve"),
            format!(
                "{n} hedge decisions on {scope_label} named a hedge vehicle that could not \
                 be resolved to a tradeable instrument, so they fell back to selling the \
                 position's own security back."
            ),
            "Open the Hedge Vehicle registry and repair the mapping — a delivery month \
             has rolled past cessation, or the product code names no live contract."
                .to_owned(),
            "hedge_vehicles",
        ),
        "acceptance_hold_recurring" => (
            DecisionEngineEnum::DecisionEngineAcceptance,
            format!("Lifts from {cp} keep landing in manual review"),
            format!(
                "{n} inbound lifts from {cp} resolved to HOLD-FOR-REVIEW rather than a \
                 straight accept or reject, so each one waited on a human."
            ),
            "If these are routinely accepted by hand, narrow the holding condition (or \
             add an earlier accept branch for this name) so the desk stops queueing work \
             it always approves. If they are routinely refused, make it a reject."
                .to_owned(),
            "acceptance",
        ),
        "acceptance_reject_recurring" => (
            DecisionEngineEnum::DecisionEngineAcceptance,
            format!("Lifts from {cp} are being rejected repeatedly"),
            format!(
                "{n} inbound lifts from {cp} were rejected by the acceptance policy. Every \
                 one of them was a quote this desk published and then refused to honour."
            ),
            "Either widen/withdraw the pricing this name sees so it stops lifting quotes \
             we will not fill, or relax the acceptance condition if the rejections are \
             collateral damage from a rule aimed elsewhere."
                .to_owned(),
            "acceptance",
        ),
        // The only remaining registered kind.
        _ => (
            DecisionEngineEnum::DecisionEngineRiskRouting,
            format!("Fills on {scope_label} are booking unrouted"),
            format!(
                "{n} fills were booked without a risk book because no routing graph was \
                 installed. Unrouted risk is invisible to every book-scoped hedge policy \
                 and threshold."
            ),
            "Author a risk-routing graph so each fill lands in a real book; a catch-all \
             leaf is enough to start."
                .to_owned(),
            "riskrouting",
        ),
    };

    RuleAdvice {
        advice_id: format!(
            "{kind}:{}:{}:{}",
            g.book,
            g.instrument,
            g.counterparty.as_deref().unwrap_or("")
        ),
        kind: kind.to_owned(),
        engine: engine as i32,
        title,
        rationale,
        recommended_action: action,
        scope_book: g.book.clone(),
        scope_instrument: g.instrument.clone(),
        scope_counterparty: g.counterparty.clone(),
        occurrences: n,
        first_seen: g.first_seen,
        last_seen: g.last_seen,
        evidence_seqs: g.seqs.clone(),
        editor: editor.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::decision_journal::decision;

    fn hedge_idle(
        seq: u64,
        at: i64,
        book: &str,
        band: &str,
        util: f64,
        reason: &str,
    ) -> DecisionRecord {
        DecisionRecord {
            seq,
            book: book.to_owned(),
            instrument: "OIS".to_owned(),
            band: band.to_owned(),
            utilization: util,
            ..decision(
                DecisionEngineEnum::DecisionEngineHedge,
                DecisionOutcomeEnum::DecisionOutcomeNoAction,
                "WAREHOUSE",
                reason,
                at,
            )
        }
    }

    fn acceptance(seq: u64, at: i64, cp: &str, label: &str) -> DecisionRecord {
        DecisionRecord {
            seq,
            counterparty: Some(cp.to_owned()),
            ..decision(
                DecisionEngineEnum::DecisionEngineAcceptance,
                DecisionOutcomeEnum::DecisionOutcomeNoAction,
                label,
                "acceptance policy resolved a non-accept leaf",
                at,
            )
        }
    }

    /// One event is not a pattern — the table already shows single rows.
    #[test]
    fn a_pattern_below_the_floor_produces_no_advice() {
        let rows = vec![
            hedge_idle(
                1,
                10,
                "B",
                "red",
                0.95,
                "red · no hedge policy configured for scope",
            ),
            hedge_idle(
                2,
                20,
                "B",
                "red",
                0.95,
                "red · no hedge policy configured for scope",
            ),
        ];
        let (advice, considered) = derive(&rows, None);
        assert!(
            advice.is_empty(),
            "2 rows is under the {MIN_OCCURRENCES} floor"
        );
        assert_eq!(considered, 2);
    }

    #[test]
    fn a_book_with_no_hedge_policy_is_suggested_and_cites_its_rows() {
        let rows: Vec<DecisionRecord> = (1..=4)
            .map(|i| {
                hedge_idle(
                    i,
                    i as i64 * 100,
                    "rates-usd",
                    "red",
                    0.9 + f64::from(u32::try_from(i).unwrap()) / 100.0,
                    "red · no hedge policy configured for scope",
                )
            })
            .collect();
        let (advice, considered) = derive(&rows, None);
        assert_eq!(considered, 4);
        let a = advice
            .iter()
            .find(|a| a.kind == "hedge_policy_missing")
            .expect("the missing-policy pattern is derived");
        assert_eq!(a.occurrences, 4);
        assert_eq!(a.scope_book, "rates-usd");
        assert_eq!(a.first_seen, 100);
        assert_eq!(a.last_seen, 400);
        assert_eq!(a.evidence_seqs, vec![1, 2, 3, 4], "every row is cited");
        assert_eq!(a.editor, "hedging");
        assert!(
            a.rationale.contains('4'),
            "the rationale states the real count: {}",
            a.rationale
        );
    }

    /// A GREEN-band warehouse hold is the policy working correctly and must NOT be
    /// advertised as a problem; the same hold at amber/red is the real signal.
    #[test]
    fn a_green_band_warehouse_hold_is_not_advice_but_a_hot_one_is() {
        let green: Vec<DecisionRecord> = (1..=6)
            .map(|i| hedge_idle(i, i as i64, "B", "green", 0.2, "green · WAREHOUSE"))
            .collect();
        let (advice, _) = derive(&green, None);
        assert!(
            !advice.iter().any(|a| a.kind == "hedge_warehouses_hot_book"),
            "a green-band hold is correct behaviour, not advice"
        );

        let hot: Vec<DecisionRecord> = (1..=6)
            .map(|i| hedge_idle(i, i as i64, "B", "red", 0.97, "red · WAREHOUSE"))
            .collect();
        let (advice, _) = derive(&hot, None);
        let a = advice
            .iter()
            .find(|a| a.kind == "hedge_warehouses_hot_book")
            .expect("holding at the red band is advice-worthy");
        assert_eq!(a.occurrences, 6);
        assert!(
            a.rationale.contains("97%"),
            "peak utilisation is real: {}",
            a.rationale
        );
    }

    #[test]
    fn recurring_holds_group_by_counterparty() {
        let mut rows: Vec<DecisionRecord> = (1..=3)
            .map(|i| acceptance(i, i as i64, "cp-a", "hold"))
            .collect();
        rows.extend((4..=5).map(|i| acceptance(i, i as i64, "cp-b", "hold")));
        let (advice, _) = derive(&rows, None);
        let held: Vec<&RuleAdvice> = advice
            .iter()
            .filter(|a| a.kind == "acceptance_hold_recurring")
            .collect();
        assert_eq!(held.len(), 1, "only cp-a clears the floor");
        assert_eq!(held[0].scope_counterparty.as_deref(), Some("cp-a"));
        assert_eq!(held[0].occurrences, 3);
        assert_eq!(held[0].editor, "acceptance");
    }

    #[test]
    fn the_book_filter_narrows_the_derivation_and_the_denominator() {
        let mut rows: Vec<DecisionRecord> = (1..=4)
            .map(|i| {
                hedge_idle(
                    i,
                    i as i64,
                    "rates-usd",
                    "red",
                    0.9,
                    "red · no hedge policy",
                )
            })
            .collect();
        rows.extend((5..=8).map(|i| {
            hedge_idle(
                i,
                i as i64,
                "rates-eur",
                "red",
                0.9,
                "red · no hedge policy",
            )
        }));
        let (advice, considered) = derive(&rows, Some("rates-usd"));
        assert_eq!(considered, 4, "only the named book's rows are read");
        assert_eq!(advice.len(), 1);
        assert_eq!(advice[0].scope_book, "rates-usd");
    }

    #[test]
    fn suggestions_are_ordered_strongest_evidence_first() {
        let mut rows: Vec<DecisionRecord> = (1..=3)
            .map(|i| hedge_idle(i, i as i64, "b-small", "red", 0.9, "red · no hedge policy"))
            .collect();
        rows.extend(
            (10..=20)
                .map(|i| hedge_idle(i, i as i64, "b-large", "red", 0.9, "red · no hedge policy")),
        );
        let (advice, _) = derive(&rows, None);
        assert_eq!(advice[0].scope_book, "b-large");
        assert!(advice[0].occurrences > advice[1].occurrences);
    }

    #[test]
    fn evidence_citations_are_capped_but_the_count_stays_true() {
        let rows: Vec<DecisionRecord> = (1..=40)
            .map(|i| hedge_idle(i, i as i64, "B", "red", 0.9, "red · no hedge policy"))
            .collect();
        let (advice, _) = derive(&rows, None);
        assert_eq!(advice[0].occurrences, 40, "the true count is reported");
        assert_eq!(
            advice[0].evidence_seqs.len(),
            EVIDENCE_SAMPLE,
            "the citation list is sampled"
        );
    }

    #[test]
    fn a_guard_downgrade_is_derived_from_a_fired_row() {
        let rows: Vec<DecisionRecord> = (1..=3)
            .map(|i| DecisionRecord {
                seq: i,
                book: "rates-usd".to_owned(),
                instrument: "OIS".to_owned(),
                advisory: true,
                ..decision(
                    DecisionEngineEnum::DecisionEngineHedge,
                    DecisionOutcomeEnum::DecisionOutcomeFired,
                    "SUBMIT_MARKET_ORDER",
                    "red · SUBMIT_MARKET_ORDER · rate cap: max hedges per interval reached → advisory",
                    i as i64,
                )
            })
            .collect();
        let (advice, _) = derive(&rows, None);
        let a = advice
            .iter()
            .find(|a| a.kind == "hedge_guard_suppressing")
            .expect("a repeated advisory downgrade is derived");
        assert_eq!(a.occurrences, 3);
        assert_eq!(a.editor, "hedge_config");
    }

    #[test]
    fn an_empty_journal_derives_nothing_and_says_so() {
        let (advice, considered) = derive(&[], None);
        assert!(advice.is_empty());
        assert_eq!(considered, 0);
    }
}
