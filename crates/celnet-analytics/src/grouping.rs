//! Grouping helpers: partition a `&[FlowRecord]` by a rollup dimension and fold
//! each group into [`ClientFlowMetrics`].
//!
//! The partition is **exhaustive and disjoint**: every record lands in exactly
//! one group keyed by the chosen dimension, so summing a per-group extensive
//! quantity (traded notional, counts, gross P&L) reproduces the whole-slice
//! total exactly. A [`std::collections::BTreeMap`] keeps the output ordered and
//! deterministic regardless of input order.

use std::collections::BTreeMap;

use crate::metrics::{ClientFlowMetrics, metrics_from};
use crate::record::FlowRecord;

/// Partition `records` by the string key `key(record)` and fold each group.
///
/// Preserves input order *within* a group (so any order-sensitive downstream
/// reading is stable) and orders the groups by key.
fn group_by<K>(records: &[FlowRecord], key: K) -> BTreeMap<String, ClientFlowMetrics>
where
    K: Fn(&FlowRecord) -> &str,
{
    let mut buckets: BTreeMap<&str, Vec<&FlowRecord>> = BTreeMap::new();
    for r in records {
        buckets.entry(key(r)).or_default().push(r);
    }
    buckets
        .into_iter()
        .map(|(k, group)| (k.to_owned(), metrics_from(k, group)))
        .collect()
}

/// Roll up per **client** (`FlowRecord::client`).
#[must_use]
pub fn group_by_client(records: &[FlowRecord]) -> BTreeMap<String, ClientFlowMetrics> {
    group_by(records, |r| r.client.as_str())
}

/// Roll up per **counterparty** (`FlowRecord::counterparty`).
#[must_use]
pub fn group_by_counterparty(records: &[FlowRecord]) -> BTreeMap<String, ClientFlowMetrics> {
    group_by(records, |r| r.counterparty.as_str())
}

/// Roll up per **instrument** (`FlowRecord::instrument`).
#[must_use]
pub fn group_by_instrument(records: &[FlowRecord]) -> BTreeMap<String, ClientFlowMetrics> {
    group_by(records, |r| r.instrument.as_str())
}

/// Roll up per **asset / product** (`FlowRecord::asset`) — the cross-product
/// dimension a global Analytics surface slices FI vs FXO by.
#[must_use]
pub fn group_by_asset(records: &[FlowRecord]) -> BTreeMap<String, ClientFlowMetrics> {
    group_by(records, |r| r.asset.as_str())
}
