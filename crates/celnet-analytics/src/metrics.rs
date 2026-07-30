//! The per-client rollup: a pure fold of `&FlowRecord`s into
//! [`ClientFlowMetrics`].
//!
//! Every field is a deterministic function of the input records — no clock, no
//! rng, no I/O — so the whole rollup is oracle-testable against a hand-computed
//! truth table (guardrail 5). Divide-by-zero is guarded everywhere: a ratio with
//! a zero denominator is reported as `None`, never `NaN`/`inf`.

use crate::fishing::fishing_score;
use crate::record::FlowRecord;

/// Notional units in one "million" — the $/mm denominator scale.
pub const NOTIONAL_PER_MILLION: f64 = 1_000_000.0;

/// Per-client (or per-counterparty / per-instrument) flow & P&L-attribution
/// metrics — the output of the fold.
///
/// Ratios that would divide by zero are `None` (documented per field). Cash
/// sums are in the input records' settlement currency; `$/mm` values are that
/// cash normalised per USD 1mm of traded notional.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientFlowMetrics {
    /// The rollup key this metric block belongs to (client id, counterparty, or
    /// instrument depending on the grouping helper used).
    pub label: String,

    // ── volume / counts ────────────────────────────────────────────────────
    /// Number of quote/RFQ responses issued to this key.
    pub quote_count: u64,
    /// Number of fills done with this key.
    pub traded_count: u64,
    /// Sum of traded notional magnitude.
    pub traded_notional: f64,

    // ── P&L attribution ────────────────────────────────────────────────────
    /// Gross margin captured on fills (Σ `margin` over traded records).
    pub gross_pnl: f64,
    /// Total adverse-selection cost (Σ `markout`, positive = cost to us).
    pub total_markout: f64,
    /// Total hedging/warehousing cost (Σ `hedge_cost`, positive = cost).
    pub total_hedge_cost: f64,
    /// Net P&L = `gross_pnl − total_markout − total_hedge_cost`.
    pub net_pnl: f64,

    // ── dollar-per-million ($/mm) ──────────────────────────────────────────
    /// Gross margin per USD 1mm traded = `gross_pnl / (traded_notional / 1e6)`.
    /// `None` when no notional traded (divide-by-zero guard).
    pub dpm_gross: Option<f64>,
    /// Net P&L per USD 1mm traded = `net_pnl / (traded_notional / 1e6)`.
    /// `None` when no notional traded.
    pub dpm_net: Option<f64>,

    // ── spread economics ───────────────────────────────────────────────────
    /// Realised margin as a fraction of the spread we quoted =
    /// `gross_pnl / Σ quoted_spread` over fills. `None` when we quoted no
    /// spread. In `[0, 1]` when we capture within the spread; `> 1` would mean
    /// we captured more than we showed.
    pub captured_vs_offered: Option<f64>,
    /// Mean of the present `cover_distance` inputs (positive = we tended to
    /// beat the cover). `None` when no record carried a cover.
    pub mean_cover_distance: Option<f64>,
    /// The offered spread, in `$/mm`, at which net $/mm would be zero given the
    /// realised markout + hedging cost and the realised capture fraction:
    /// `((markout + hedge)/mm) / captured_vs_offered`. Below this offered
    /// spread the client is a net loser. `None` when it is undefined (no
    /// trades, or a zero/absent capture fraction).
    pub breakeven_spread: Option<f64>,

    // ── quote-fishing ──────────────────────────────────────────────────────
    /// Quotes ÷ trades. `None` when no trades (an unbounded ratio — the
    /// zero-trade fisher case is handled by `fishing_score`, which does not
    /// divide).
    pub quote_to_trade_ratio: Option<f64>,
    /// Trades ÷ quotes (the RFQ hit-rate). `None` when no quotes.
    pub hit_rate: Option<f64>,
    /// Bounded `[0, 1]` fishing score (see [`crate::fishing::fishing_score`]).
    pub fishing_score: f64,
}

/// Single-pass accumulator over the records for one key.
#[derive(Debug, Clone, Copy, Default)]
struct Acc {
    quote_count: u64,
    traded_count: u64,
    traded_notional: f64,
    gross_pnl: f64,
    total_markout: f64,
    total_hedge_cost: f64,
    quoted_spread_traded: f64,
    cover_sum: f64,
    cover_n: u64,
}

impl Acc {
    fn ingest(&mut self, r: &FlowRecord) {
        if r.was_quoted {
            self.quote_count += 1;
        }
        if let Some(cd) = r.cover_distance {
            self.cover_sum += cd;
            self.cover_n += 1;
        }
        if r.was_traded {
            self.traded_count += 1;
            self.traded_notional += r.notional;
            self.gross_pnl += r.margin;
            self.quoted_spread_traded += r.quoted_spread;
            self.total_markout += r.markout.unwrap_or(0.0);
            self.total_hedge_cost += r.hedge_cost.unwrap_or(0.0);
        }
    }

    fn finish(self, label: String) -> ClientFlowMetrics {
        let has_volume = self.traded_notional > 0.0;
        let traded_mm = self.traded_notional / NOTIONAL_PER_MILLION;
        let net_pnl = self.gross_pnl - self.total_markout - self.total_hedge_cost;

        let dpm_gross = has_volume.then(|| self.gross_pnl / traded_mm);
        let dpm_net = has_volume.then(|| net_pnl / traded_mm);

        let captured_vs_offered =
            (self.quoted_spread_traded > 0.0).then(|| self.gross_pnl / self.quoted_spread_traded);

        let mean_cover_distance = (self.cover_n > 0).then(|| self.cover_sum / self.cover_n as f64);

        // Breakeven offered spread ($/mm): the cost per mm we must out-earn,
        // grossed up by the fraction of the offered spread we actually keep.
        let breakeven_spread = match (has_volume, captured_vs_offered) {
            (true, Some(r)) if r > 0.0 => {
                let cost_per_mm = (self.total_markout + self.total_hedge_cost) / traded_mm;
                Some(cost_per_mm / r)
            }
            _ => None,
        };

        let quote_to_trade_ratio =
            (self.traded_count > 0).then(|| self.quote_count as f64 / self.traded_count as f64);
        let hit_rate =
            (self.quote_count > 0).then(|| self.traded_count as f64 / self.quote_count as f64);

        let fishing_score = fishing_score(hit_rate, dpm_net, self.quote_count);

        ClientFlowMetrics {
            label,
            quote_count: self.quote_count,
            traded_count: self.traded_count,
            traded_notional: self.traded_notional,
            gross_pnl: self.gross_pnl,
            total_markout: self.total_markout,
            total_hedge_cost: self.total_hedge_cost,
            net_pnl,
            dpm_gross,
            dpm_net,
            captured_vs_offered,
            mean_cover_distance,
            breakeven_spread,
            quote_to_trade_ratio,
            hit_rate,
            fishing_score,
        }
    }
}

/// Fold an iterator of records (assumed to belong to one `label`) into a
/// [`ClientFlowMetrics`]. Pure and O(records) with bounded memory.
pub fn metrics_from<'a, I>(label: impl Into<String>, records: I) -> ClientFlowMetrics
where
    I: IntoIterator<Item = &'a FlowRecord>,
{
    let mut acc = Acc::default();
    for r in records {
        acc.ingest(r);
    }
    acc.finish(label.into())
}
