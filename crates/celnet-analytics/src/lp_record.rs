//! The neutral input record for **street-side / LP liquidity analytics** — one
//! LP quote-update or one LP RFQ-panel outcome's contribution.
//!
//! This is the LP-keyed analogue of [`crate::FlowRecord`] (which is client-keyed):
//! where `FlowRecord` grades *our clients'* flow, an [`LpFlowRecord`] grades *our
//! liquidity providers'* street-side behaviour — how fast each LP ticks, how often
//! it wins the deals it is shown, how often it is on the panel but loses, and how
//! often it reneges on last-look.
//!
//! Like the client-flow record it is **server-, wire-, and market-data-free**: the
//! server layer populates it from its own already-captured history (the aggregation
//! hub's per-LP ingest count, the RFQ panel rows pinned on each `QuoteRecord`, the
//! multi-dealer engine's last-look outcome), then this crate folds a
//! `&[LpFlowRecord]` into per-LP metrics. Every quantity is an **input** here —
//! nothing is re-priced — so the fold stays a pure, deterministic function of its
//! inputs (guardrail 5, oracle-testable).

/// One street-side event's contribution to an LP's liquidity metrics.
///
/// A record represents **either** a single quote-update *tick* (`tick == true`,
/// carrying no panel outcome) **or** one LP row on a resolved RFQ panel (`tick ==
/// false`, carrying the `was_quoted` / `was_won` / `was_missed` /
/// `was_last_look_reject` outcome flags). The two shapes are folded by the same
/// accumulator, so a source is free to emit whichever it retains.
///
/// # Flag semantics (locked, mutually consistent)
///
/// - `was_quoted` — the LP responded on the panel (its quote was present). This is
///   the **response-presence** signal and the denominator of `win_rate`.
/// - `was_won` — the LP won the deal (`booked_lp_id == lp_id`). A win implies the
///   LP was on the panel, so a well-formed won record also has `was_quoted == true`.
/// - `was_missed` — the LP was quoted **and a deal was done that it did not win**
///   (it was on the panel but lost). Disjoint from `was_won`.
/// - `was_last_look_reject` — the LP's quote was excluded at ranking time because
///   its `valid_until_nanos` had lapsed (last-look). An LP can be rejected on
///   last-look on an RFQ it also responded to, so this may co-occur with
///   `was_quoted`.
/// - `notional` — the **non-negative** deal notional magnitude. Meaningful on a
///   `was_won` record (it sums into `won_notional`); `0.0` otherwise.
/// - `cover_distance` — how far this LP's price was from the winner when this LP was
///   the **cover** (the runner-up we beat, or that beat the field), as a
///   non-negative price/premium distance. `None` when this LP was not the cover on
///   this outcome (the vast majority of rows) — never fabricated.
#[derive(Debug, Clone, PartialEq)]
pub struct LpFlowRecord {
    /// The liquidity provider identity (the panel `lp_id` / the pushing venue
    /// name) — the primary and only rollup key.
    pub lp_id: String,
    /// Instrument identifier (symbol / ISIN) this event was on — carried for
    /// context and future per-instrument LP slicing; not itself a metric.
    pub instrument: String,
    /// `true` if the LP responded on the panel (its quote was present). The
    /// response-presence signal and the `win_rate` denominator.
    pub was_quoted: bool,
    /// `true` if the LP won the booked deal (`booked_lp_id == lp_id`).
    pub was_won: bool,
    /// `true` if the LP was quoted, a deal was done, and it did **not** win.
    pub was_missed: bool,
    /// `true` if the LP's quote was rejected at ranking time on last-look.
    pub was_last_look_reject: bool,
    /// Non-negative deal notional magnitude (summed into `won_notional` on wins).
    pub notional: f64,
    /// `true` if this record is a single quote-update tick (counts toward the
    /// LP's tick rate; carries no panel outcome).
    pub tick: bool,
    /// This LP's price distance from the winner when it was the cover (runner-up).
    /// `None` when the LP was not the cover on this outcome.
    pub cover_distance: Option<f64>,
}

impl LpFlowRecord {
    /// A blank panel-outcome record for `lp_id` on `instrument`: not quoted, not
    /// won/missed/rejected, no tick, zero notional. Sources override only the
    /// fields an event actually sets (struct-update syntax), keeping every call
    /// site a readable one-liner rather than a wide positional constructor.
    #[must_use]
    pub fn blank(lp_id: impl Into<String>, instrument: impl Into<String>) -> Self {
        Self {
            lp_id: lp_id.into(),
            instrument: instrument.into(),
            was_quoted: false,
            was_won: false,
            was_missed: false,
            was_last_look_reject: false,
            notional: 0.0,
            tick: false,
            cover_distance: None,
        }
    }

    /// A single quote-update tick for `lp_id` on `instrument` (no panel outcome).
    #[must_use]
    pub fn tick(lp_id: impl Into<String>, instrument: impl Into<String>) -> Self {
        Self {
            tick: true,
            ..Self::blank(lp_id, instrument)
        }
    }
}
