//! Grouping helper: partition a `&[LpFlowRecord]` by LP and fold each group into
//! [`LpFlowMetrics`].
//!
//! The partition is **exhaustive and disjoint**: every record lands in exactly one
//! group keyed by `lp_id`, so summing a per-group extensive quantity (tick count,
//! quotes, wins, missed, rejects, won notional) reproduces the whole-slice total
//! exactly. A [`BTreeMap`] keeps the output ordered and deterministic regardless of
//! input order.

use std::collections::BTreeMap;

use crate::lp_metrics::{LpFlowMetrics, lp_metrics_from};
use crate::lp_record::LpFlowRecord;

/// Roll up per **LP** (`LpFlowRecord::lp_id`).
///
/// Preserves input order *within* a group and orders the groups by `lp_id`.
#[must_use]
pub fn group_by_lp(records: &[LpFlowRecord]) -> BTreeMap<String, LpFlowMetrics> {
    let mut buckets: BTreeMap<&str, Vec<&LpFlowRecord>> = BTreeMap::new();
    for r in records {
        buckets.entry(r.lp_id.as_str()).or_default().push(r);
    }
    buckets
        .into_iter()
        .map(|(lp, group)| (lp.to_owned(), lp_metrics_from(lp, group)))
        .collect()
}
