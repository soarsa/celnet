//! Cross-product **client-flow analytics** rollup (Analytics phase 2, server
//! side — `docs/ANALYTICS-REQUIREMENTS.md` §11.1a/§11.3/§4.2/§4.3).
//!
//! This module is the thin server layer that turns Celnet's **already-captured**
//! quote/deal history into the pure [`celnet_analytics`] rollup. It owns no
//! market model and re-prices nothing: each *source* (an FXO quote edge, the FI
//! desk edge, …) maps its own retained records into neutral
//! [`FlowRecord`](celnet_analytics::FlowRecord)s — asset-tagged and time-filtered
//! at the source under its own store lock — and this module folds the union into
//! per-key [`ClientFlowMetrics`](celnet_analytics::ClientFlowMetrics) via the
//! crate's `group_by_*` helpers.
//!
//! # Off the hot path (guardrail 11)
//!
//! Collection is **fold-on-query**: nothing runs on the pinned zero-alloc pricing
//! thread. A source's [`ClientFlowSource::flow_records`] is called only when the
//! `ListClientFlowMetrics` RPC fires, reads the source's existing history store
//! (already `Mutex`/`RwLock`-guarded and only touched at the RFQ request/accept
//! edge, never across a price call), and allocates freely on that async edge. No
//! new capture is added to the streaming core.
//!
//! # Honesty (guardrail 2)
//!
//! Every economic quantity is read from what is actually stored. Where a datum
//! genuinely does not exist upstream — `markout`/`hedge_cost` (no post-trade
//! mark pass yet), the FI desk path's per-feature margin (it isn't priced through
//! `celnet-tiering`, so `pricing_provenance` is `None` at the source), or a
//! single-dealer path's cover (no panel) — the record carries `None`/`0`, never a
//! fabricated value. The rollup's divide-by-zero guards then report the derived
//! ratio as absent rather than inventing one.

pub mod lp;

use std::collections::BTreeMap;
use std::sync::Arc;

use celnet_analytics::{
    ClientFlowMetrics, FlowRecord, group_by_asset, group_by_client, group_by_counterparty,
    group_by_instrument,
};
use celnet_proto::{ClientFlowMetricsDesc, FlowGroupBy};

/// A provider of already-captured flow the rollup folds over. Object-safe (via
/// `tonic::async_trait`) so the server holds a heterogeneous
/// `Vec<Arc<dyn ClientFlowSource>>` — one per asset class / edge. The method is
/// **async** because a source (the FXO quote edge) guards its history behind a
/// tokio `Mutex`; the await happens on the query edge, never on the hot path.
#[tonic::async_trait]
pub trait ClientFlowSource: Send + Sync {
    /// This source's [`FlowRecord`]s whose event time falls in `[from, to)` (each
    /// bound optional/open), already `asset`-tagged. Called on-query, off the hot
    /// path, under the source's own store lock.
    async fn flow_records(&self, from: Option<i64>, to: Option<i64>) -> Vec<FlowRecord>;
}

/// `true` iff `t` (epoch nanos) is inside the half-open `[from, to)` window; an
/// absent bound is open on that side.
#[must_use]
pub fn in_window(t: i64, from: Option<i64>, to: Option<i64>) -> bool {
    from.is_none_or(|f| t >= f) && to.is_none_or(|u| t < u)
}

/// Map the wire grouping enum to a domain choice; the proto3 default (0 / CLIENT)
/// and any unknown discriminant both resolve to per-client (the primary lens).
#[must_use]
fn resolve_group_by(raw: i32) -> FlowGroupBy {
    FlowGroupBy::try_from(raw).unwrap_or(FlowGroupBy::Client)
}

/// Collect + union every source's window-filtered records (async — awaits each
/// source's store). Kept separate from [`fold`] so the fold stays a pure,
/// synchronously-testable function.
pub async fn collect(
    sources: &[Arc<dyn ClientFlowSource>],
    from: Option<i64>,
    to: Option<i64>,
) -> Vec<FlowRecord> {
    let mut records: Vec<FlowRecord> = Vec::new();
    for source in sources {
        records.extend(source.flow_records(from, to).await);
    }
    records
}

/// Fold a record union into the per-group-key metrics via the requested
/// `celnet_analytics` grouping. **Pure** and deterministic (`BTreeMap` key order)
/// — the whole numerical surface lives in the oracle-tested crate; this only
/// selects the dimension.
#[must_use]
pub fn fold(records: &[FlowRecord], group_by: i32) -> BTreeMap<String, ClientFlowMetrics> {
    match resolve_group_by(group_by) {
        FlowGroupBy::Client => group_by_client(records),
        FlowGroupBy::Counterparty => group_by_counterparty(records),
        FlowGroupBy::Instrument => group_by_instrument(records),
        FlowGroupBy::Asset => group_by_asset(records),
    }
}

/// Project one pure [`ClientFlowMetrics`] onto its wire mirror. `Option<f64>` ⇒
/// proto3 `optional double` (absent = JSON null on the WS wire).
#[must_use]
pub fn metrics_to_wire(m: &ClientFlowMetrics) -> ClientFlowMetricsDesc {
    ClientFlowMetricsDesc {
        label: m.label.clone(),
        quote_count: m.quote_count,
        traded_count: m.traded_count,
        traded_notional: m.traded_notional,
        gross_pnl: m.gross_pnl,
        total_markout: m.total_markout,
        total_hedge_cost: m.total_hedge_cost,
        net_pnl: m.net_pnl,
        dpm_gross: m.dpm_gross,
        dpm_net: m.dpm_net,
        captured_vs_offered: m.captured_vs_offered,
        mean_cover_distance: m.mean_cover_distance,
        breakeven_spread: m.breakeven_spread,
        quote_to_trade_ratio: m.quote_to_trade_ratio,
        hit_rate: m.hit_rate,
        fishing_score: m.fishing_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_analytics::Side;

    fn rec(client: &str, asset: &str, traded: bool, notional: f64, margin: f64) -> FlowRecord {
        FlowRecord {
            client: client.to_owned(),
            counterparty: "CP".to_owned(),
            instrument: "X".to_owned(),
            asset: asset.to_owned(),
            notional,
            side: Side::Buy,
            was_quoted: true,
            was_traded: traded,
            margin,
            quoted_spread: 0.0,
            cover_distance: None,
            markout: None,
            hedge_cost: None,
        }
    }

    #[test]
    fn window_is_half_open() {
        assert!(in_window(10, None, None));
        assert!(in_window(10, Some(10), Some(20))); // inclusive lower
        assert!(!in_window(20, Some(10), Some(20))); // exclusive upper
        assert!(!in_window(9, Some(10), None));
    }

    #[test]
    fn unknown_group_by_falls_back_to_client() {
        // 99 is not a valid FlowGroupBy discriminant → per-client.
        let recs = vec![
            rec("ACME", "fxo", true, 5_000_000.0, 250.0),
            rec("ACME", "fxo", false, 0.0, 0.0),
        ];
        let out = fold(&recs, 99);
        assert_eq!(out.len(), 1);
        assert_eq!(out["ACME"].traded_count, 1);
        assert_eq!(out["ACME"].quote_count, 2);
        assert_eq!(out["ACME"].dpm_gross, Some(50.0));
    }

    #[test]
    fn group_by_asset_partitions_fi_and_fxo() {
        // A fisher on FXO (5 quotes, 0 trades) and a converter on FI (all trade).
        let mut recs = vec![
            rec("HFUND", "fi", true, 10_000_000.0, 0.0),
            rec("HFUND", "fi", true, 20_000_000.0, 0.0),
        ];
        for _ in 0..5 {
            recs.push(rec("FISH", "fxo", false, 0.0, 0.0));
        }
        // group_by = 3 (FLOW_GROUP_BY_ASSET).
        let by_asset = fold(&recs, FlowGroupBy::Asset as i32);
        assert_eq!(by_asset.len(), 2);
        assert_eq!(by_asset["fi"].traded_notional, 30_000_000.0);
        assert_eq!(by_asset["fxo"].traded_count, 0);
        // The fxo group is a pure fisher: max fishing score; the FI converter low.
        assert_eq!(by_asset["fxo"].fishing_score, 1.0);
        assert_eq!(by_asset["fi"].fishing_score, 0.0);

        // group_by = 0 (CLIENT) splits the two clients.
        let by_client = fold(&recs, FlowGroupBy::Client as i32);
        assert_eq!(by_client.len(), 2);
        assert_eq!(by_client["HFUND"].traded_count, 2);
        assert_eq!(by_client["FISH"].fishing_score, 1.0);

        // The wire projection preserves the guarded ratios (dpm present on FXO
        // fisher? no trades ⇒ None; FI converter ⇒ Some).
        let wire = metrics_to_wire(&by_asset["fi"]);
        assert_eq!(wire.traded_count, 2);
        assert_eq!(wire.dpm_gross, Some(0.0));
        let wire_fish = metrics_to_wire(&by_asset["fxo"]);
        assert_eq!(wire_fish.dpm_gross, None);
        assert_eq!(wire_fish.fishing_score, 1.0);
    }
}
