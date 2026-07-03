//! The **one class-parametric additive fan-in seam** — the single additive
//! risk-aggregation path every asset class shares (ADR-0021,
//! `docs/RISK-HIERARCHY.md` §2.5/§3.4).
//!
//! # What this is (the additive-side complement of C2c)
//!
//! C2c ([`celnet_risk_cube::fi`]) unified the **non-additive** cube: one
//! [`celnet_core::tail_var_es`] reduction consumes the summed per-scenario P&L of
//! *every* risk class (options spot/vol legs AND linear-FI rate legs), so there is
//! ONE joint tail engine rather than a separate per-class VaR. This module is its
//! **additive** complement: ONE fold driver ([`fan_in_additive_seq`]) reduces a
//! deterministically-ordered sequence of per-shard **additive aggregates** into the
//! firm aggregate, for *any* asset class. The options net-Greeks / vega ladder and
//! the fixed-income net-DV01 / PV01 / key-rate ladder join the **same** fan-in
//! through the [`AdditiveAggregate`] seam, instead of two parallel reducers.
//!
//! # The seam is the driver, NOT a flattened merge rule (surfaced deliberately)
//!
//! The two asset classes combine their additive aggregates with **different, and
//! individually load-bearing, exact-reconciliation contracts** — and the seam keeps
//! both, because collapsing them into one merge rule would *break* one of them:
//!
//! - **Options** ([`celnet_risk_cube::NodeAggregate`]) combine by a naive net-sum of
//!   the [`celnet_risk_cube::NodeAggregate::merge_additive`] fields. Floating-point
//!   addition is commutative but not associative, so this reconciles to the
//!   single-node [`celnet_risk_cube::Cube::firm_aggregate`] **up to summation
//!   order**, which the reducer's fixed ascending-[`celnet_router::ReplicaId`] shard
//!   order pins (the `celnet-risk-fleet` additive tests).
//! - **Fixed income** ([`crate::RatesFirmRollup`]) combine by a **fixed
//!   ascending-`(ccy, entity, tenor)` re-fold** ([`crate::RatesFirmRollup`]'s
//!   `merge`), a pure function of the facts and *not* of the shard layout, so the
//!   fan-in equals the single-node [`crate::firm_aggregate_rates`] **bit-for-bit**
//!   under *any* sharding — the F5 headline invariant.
//!
//! The **unification** is therefore the single fold *driver* (partition → per-shard
//! local roll-up → fold in deterministic order), not a single merge *rule*. Each
//! class supplies its own [`AdditiveAggregate::combine`] — its exact-reconciliation
//! contract — and both flow through the one driver. Every existing result is
//! `to_bits`-unchanged: the combine operation each class uses is exactly the one it
//! used before, folded in exactly the same order.

/// An asset class's **additive** risk aggregate — a per-shard partial that combines
/// associatively across shards into the firm aggregate.
///
/// This is the single seam the one additive fan-in driver
/// ([`fan_in_additive_seq`]) folds through: options net-Greeks / vega-ladder
/// aggregates and fixed-income net-DV01 / PV01 / key-rate-ladder aggregates both
/// implement it, so they roll up the **same** way rather than through two parallel
/// reducers (ADR-0021).
///
/// [`combine`](Self::combine) carries the implementing class's **own** exact
/// reconciliation contract (the options naive net-sum; the FI fixed
/// ascending-`(ccy, entity, tenor)` re-fold). The trait deliberately does not
/// prescribe one merge rule — it prescribes that a class *has* an associative
/// combine, so the driver can fold any class uniformly (see the module note).
pub trait AdditiveAggregate {
    /// Combine another shard's aggregate into this one, preserving the class's exact
    /// reconciliation semantics. Consumes `self` and returns the combined aggregate,
    /// so the driver folds left-to-right uniformly across classes.
    #[must_use]
    fn combine(self, other: &Self) -> Self;
}

/// The **one additive fan-in driver**: fold a deterministically-ordered sequence of
/// per-shard additive aggregates into the firm aggregate via
/// [`AdditiveAggregate::combine`], seeding an empty input with `empty()`.
///
/// This is the single additive-aggregation path both
/// [`crate::FleetReducer::fan_in_additive`] (options) and
/// [`crate::RatesFleetReducer::fan_in_additive`] (fixed income) delegate to. It is a
/// pure left fold in the iterator's order, so — given the reducers' fixed
/// ascending-replica shard order — it reproduces each class's pre-unification
/// aggregate **bit-for-bit** (the combine operation and its order are unchanged).
///
/// `shards` yields each shard's already-built local aggregate (the per-class
/// roll-up, which may need class-specific context such as a vega-pillar map, is
/// performed at the call site so the driver stays context-free); `empty` supplies
/// the identity firm aggregate for a book that fanned across zero shards.
pub fn fan_in_additive_seq<A, I>(shards: I, empty: impl FnOnce() -> A) -> A
where
    A: AdditiveAggregate,
    I: IntoIterator<Item = A>,
{
    let mut iter = shards.into_iter();
    match iter.next() {
        // Empty book / zero shards — the identity firm aggregate.
        None => empty(),
        // Left fold in the (deterministic) shard order: acc = acc.combine(next).
        Some(first) => iter.fold(first, |acc, next| acc.combine(&next)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal order-recording aggregate: `combine` concatenates, so the driver's
    /// fold order is directly observable in the result.
    #[derive(Clone, Debug, PartialEq)]
    struct Trace(Vec<i64>);

    impl AdditiveAggregate for Trace {
        fn combine(mut self, other: &Self) -> Self {
            self.0.extend_from_slice(&other.0);
            self
        }
    }

    /// The driver folds **left-to-right in iterator order** (the shard order the
    /// reducer fixes), seeding the accumulator with the first element.
    #[test]
    fn folds_left_to_right_in_order() {
        let got = fan_in_additive_seq([Trace(vec![1]), Trace(vec![2]), Trace(vec![3])], || {
            Trace(Vec::new())
        });
        assert_eq!(got, Trace(vec![1, 2, 3]));
    }

    /// An empty sequence yields exactly `empty()` (never touches `combine`).
    #[test]
    fn empty_sequence_yields_identity() {
        let got = fan_in_additive_seq(std::iter::empty::<Trace>(), || Trace(vec![-1]));
        assert_eq!(got, Trace(vec![-1]));
    }

    /// A single element is returned as-is — the accumulator seed with no combine.
    #[test]
    fn single_element_is_the_seed() {
        let got = fan_in_additive_seq([Trace(vec![7])], || Trace(Vec::new()));
        assert_eq!(got, Trace(vec![7]));
    }
}
