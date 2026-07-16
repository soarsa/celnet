//! The streamed two-way + size model for a fixed-income rates line.
//!
//! A live streaming rates line ([`crate::services::stream`]) is a *tradeable*
//! dealer market, not just an indicative mid: it carries a **bid** and an
//! **offer** either side of the fair mid (the par rate for an OIS/IRS/FRA, the
//! clean price for a cash bond) plus an available **size** per side. This module
//! derives that two-way + size from the already-priced line, so the streaming
//! edge never re-runs pricing — it enriches the landed [`price_rates`] result.
//!
//! # The half-spread convention (why it widens with tenor/DV01)
//!
//! A dealer's quoted half-spread compensates the inventory (warehousing) risk of
//! taking the position onto the book. That risk is proportional to the position's
//! rate sensitivity — its **modified duration** (DV01 per unit notional): a 30y
//! swap moves many multiples of a 1y swap's PV for the same rate shock, so it must
//! be quoted wider. Liquid short OIS trades sub-basis-point; longer / less-liquid
//! lines widen. We therefore model the half-spread as an affine function of the
//! line's per-notional DV01 (its modified duration):
//!
//! ```text
//! half_spread_bp = BASE_HALF_SPREAD_BP + DURATION_SLOPE_BP_PER_YEAR * duration
//! ```
//!
//! where `duration = |dv01| / (notional * 1bp)` is the modified duration in years
//! (notional-independent, so a 100mm and a 10mm 5y swap quote the *same* spread —
//! the market convention: bid/offer widens with maturity/risk, not with ticket
//! size). This is the standard inventory-risk-proportional maker convention (see
//! e.g. Stoll, "The Supply of Dealer Services", J. Finance 1978 — the dealer
//! spread compensates the risk of the inventory held), specialised to the linear
//! rate risk a swap/bond desk warehouses. The single-homed indicative RFQ
//! constants ([`super::RATES_RFQ_HALF_SPREAD`] / [`super::BOND_RFQ_HALF_SPREAD`])
//! are recovered as this model's zero-duration floor for a rate / price market
//! respectively, so the streamed line and the RFQ line are one consistent family.
//!
//! # The size convention (why it shrinks with tenor/DV01)
//!
//! A dealer shows deeper size on a line it can hedge cheaply (short, liquid) and
//! thinner size where each unit of notional carries more risk. Streamed available
//! notional therefore *decays* with duration:
//!
//! ```text
//! size = max(MIN_STREAM_NOTIONAL, BASE_STREAM_NOTIONAL / (1 + duration / DURATION_REF_YEARS))
//! ```
//!
//! monotonically non-increasing in duration, floored strictly positive. Both sides
//! stream the same available size (a symmetric two-way line).

use celnet_proto::RatesPricingResult;

/// One basis point as an absolute decimal rate (`0.0001`).
const ONE_BP: f64 = 0.000_1;

/// The zero-duration half-spread floor for a **rate** market, in basis points
/// (`0.5bp` each side ⇒ a 1bp-wide market at the front end). Equals
/// [`super::RATES_RFQ_HALF_SPREAD`] expressed in bp, so the streamed line's floor
/// coincides with the indicative RFQ line's flat half-spread.
const RATE_BASE_HALF_SPREAD_BP: f64 = 0.5;

/// The zero-duration half-spread floor for a **clean-price** market, in price
/// points per 100 face (`0.05` ⇒ a 10-cent-wide market). Equals
/// [`super::BOND_RFQ_HALF_SPREAD`].
const PRICE_BASE_HALF_SPREAD_PTS: f64 = 0.05;

/// The half-spread widening per year of modified duration, in basis points for a
/// **rate** market. A 10y line (duration ≈ 9y) quotes ≈ `0.5 + 0.15*9 ≈ 1.85bp`
/// each side; a 30y line (duration ≈ 22y) ≈ `3.8bp` — the wider quote long / less
/// liquid rates markets carry.
const RATE_DURATION_SLOPE_BP_PER_YEAR: f64 = 0.15;

/// The half-spread widening per year of duration for a **clean-price** market, in
/// price points per 100 face. Bond markets quote in price, and a longer bond's
/// price moves more per bp, so its quoted price band widens with duration.
const PRICE_DURATION_SLOPE_PTS_PER_YEAR: f64 = 0.01;

/// The base streamed available notional on the most liquid (zero-duration) line.
const BASE_STREAM_NOTIONAL: f64 = 100_000_000.0;

/// The duration (years) at which streamed size halves — the liquidity reference.
const DURATION_REF_YEARS: f64 = 10.0;

/// The strictly-positive floor on streamed size (a dealer always shows *some*
/// tradeable size, however thin the line).
const MIN_STREAM_NOTIONAL: f64 = 1_000_000.0;

/// The modified duration (per-bp rate sensitivity per unit notional, in years)
/// implied by a line's DV01 and notional. Notional-independent and non-negative.
///
/// `duration = |dv01| / (notional * 1bp)`. A non-positive or non-finite notional
/// (a bond redemption edge, say) falls back to a zero duration — the front-end
/// floor spread — rather than dividing by zero.
fn modified_duration(dv01: f64, notional: f64) -> f64 {
    if notional > 0.0 && notional.is_finite() {
        (dv01.abs() / (notional * ONE_BP)).max(0.0)
    } else {
        0.0
    }
}

/// The symmetric streamed available notional for a line of the given duration —
/// non-increasing in duration, floored strictly positive.
fn stream_size(duration: f64) -> f64 {
    (BASE_STREAM_NOTIONAL / (1.0 + duration / DURATION_REF_YEARS)).max(MIN_STREAM_NOTIONAL)
}

/// The absolute-rate half-spread for a swap/FRA line of the given duration.
fn rate_half_spread(duration: f64) -> f64 {
    (RATE_BASE_HALF_SPREAD_BP + RATE_DURATION_SLOPE_BP_PER_YEAR * duration) * ONE_BP
}

/// The price-point half-spread for a cash-bond line of the given duration.
fn price_half_spread(duration: f64) -> f64 {
    PRICE_BASE_HALF_SPREAD_PTS + PRICE_DURATION_SLOPE_PTS_PER_YEAR * duration
}

/// The streamed two-way + size a rates line carries: a `(bid, offer)` struck
/// around the fair mid and a symmetric available `size` (same both sides).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamTwoWay {
    /// The bid — the fair mid less the half-spread. `bid <= mid`.
    pub bid: f64,
    /// The offer — the fair mid plus the half-spread. `offer >= mid`, `> bid`.
    pub offer: f64,
    /// The available notional on the bid side (strictly positive).
    pub bid_size: f64,
    /// The available notional on the offer side (strictly positive, `== bid_size`).
    pub offer_size: f64,
}

/// Derive the streamed two-way + size for a **rate** market (OIS / IRS / FRA) from
/// the priced result and the line's notional. The two-way is struck around the
/// fair `mid` par rate; the half-spread and size scale with the line's modified
/// duration (see the module doc). `bid <= mid <= offer`, `bid < offer`, sizes
/// strictly positive.
pub fn two_way_rate(mid: f64, result: &RatesPricingResult, notional: f64) -> StreamTwoWay {
    let duration = modified_duration(result.dv01, notional);
    let half = rate_half_spread(duration);
    let size = stream_size(duration);
    StreamTwoWay {
        bid: mid - half,
        offer: mid + half,
        bid_size: size,
        offer_size: size,
    }
}

/// Derive the streamed two-way + size for a **clean-price** market (cash bond)
/// from the priced result and the bond notional (face). The two-way is struck
/// around the fair `mid` clean price; the half-spread and size scale with the
/// bond's modified duration. `bid <= mid <= offer`, `bid < offer`, sizes strictly
/// positive.
pub fn two_way_price(mid: f64, result: &RatesPricingResult, notional: f64) -> StreamTwoWay {
    let duration = modified_duration(result.dv01, notional);
    let half = price_half_spread(duration);
    let size = stream_size(duration);
    StreamTwoWay {
        bid: mid - half,
        offer: mid + half,
        bid_size: size,
        offer_size: size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result_with_dv01(dv01: f64) -> RatesPricingResult {
        RatesPricingResult {
            pv: 0.0,
            par_rate: 0.04,
            pv01: dv01,
            dv01,
            key_rate_ladder: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn rate_two_way_brackets_the_mid() {
        let notional = 100_000_000.0;
        // A 5y-ish line: DV01 ≈ notional * 5y * 1bp = 50_000.
        let r = result_with_dv01(50_000.0);
        let tw = two_way_rate(0.0405, &r, notional);
        assert!(tw.bid <= 0.0405, "bid {} must be <= mid", tw.bid);
        assert!(tw.offer >= 0.0405, "offer {} must be >= mid", tw.offer);
        assert!(
            tw.bid < tw.offer,
            "bid {} must be < offer {}",
            tw.bid,
            tw.offer
        );
        // Symmetric around the mid.
        let mid_recovered = 0.5 * (tw.bid + tw.offer);
        assert!((mid_recovered - 0.0405).abs() < 1e-12);
    }

    #[test]
    fn rate_sizes_are_strictly_positive() {
        let r = result_with_dv01(50_000.0);
        let tw = two_way_rate(0.0405, &r, 100_000_000.0);
        assert!(tw.bid_size > 0.0);
        assert!(tw.offer_size > 0.0);
        assert_eq!(tw.bid_size, tw.offer_size);
    }

    #[test]
    fn rate_half_spread_widens_with_duration() {
        let notional = 100_000_000.0;
        // 2y line (DV01 ≈ 20_000) vs 30y line (DV01 ≈ 600_000) at the same notional.
        let short = two_way_rate(0.04, &result_with_dv01(20_000.0), notional);
        let long = two_way_rate(0.04, &result_with_dv01(600_000.0), notional);
        let short_half = 0.5 * (short.offer - short.bid);
        let long_half = 0.5 * (long.offer - long.bid);
        assert!(
            long_half > short_half,
            "30y half-spread {long_half} must exceed 2y half-spread {short_half}",
        );
    }

    #[test]
    fn rate_size_shrinks_with_duration() {
        let notional = 100_000_000.0;
        let short = two_way_rate(0.04, &result_with_dv01(20_000.0), notional);
        let long = two_way_rate(0.04, &result_with_dv01(600_000.0), notional);
        assert!(
            long.bid_size < short.bid_size,
            "30y size {} must be thinner than 2y size {}",
            long.bid_size,
            short.bid_size,
        );
    }

    #[test]
    fn front_end_half_spread_equals_the_rfq_floor() {
        // A zero-duration (spot) line recovers the indicative RATES_RFQ_HALF_SPREAD
        // floor (0.5bp) exactly, so streamed and RFQ lines are one family.
        let r = result_with_dv01(0.0);
        let tw = two_way_rate(0.04, &r, 100_000_000.0);
        let half = 0.5 * (tw.offer - tw.bid);
        assert!((half - super::super::RATES_RFQ_HALF_SPREAD).abs() < 1e-15);
    }

    #[test]
    fn bond_two_way_brackets_the_mid_and_widens() {
        let notional = 100.0; // per 100 face
        let short = two_way_price(99.5, &result_with_dv01(2.0), notional);
        let long = two_way_price(99.5, &result_with_dv01(18.0), notional);
        assert!(short.bid <= 99.5 && short.offer >= 99.5 && short.bid < short.offer);
        let short_half = 0.5 * (short.offer - short.bid);
        let long_half = 0.5 * (long.offer - long.bid);
        assert!(long_half > short_half, "longer bond must quote wider");
        assert!(short.bid_size > 0.0 && long.bid_size > 0.0);
    }

    #[test]
    fn bond_front_end_half_spread_equals_the_rfq_floor() {
        let r = result_with_dv01(0.0);
        let tw = two_way_price(99.5, &r, 100.0);
        let half = 0.5 * (tw.offer - tw.bid);
        // Price points sit near 100, so the half-spread recovery carries the
        // O(1e-14) rounding of subtracting two ~100 magnitudes — a price-scale
        // tolerance, not the 1e-15 a near-zero rate quote affords.
        assert!((half - super::super::BOND_RFQ_HALF_SPREAD).abs() < 1e-12);
    }

    #[test]
    fn size_is_floored_positive_even_for_extreme_duration() {
        let tw = two_way_rate(0.04, &result_with_dv01(1.0e12), 100_000_000.0);
        assert!(tw.bid_size >= MIN_STREAM_NOTIONAL);
        assert!(tw.bid_size > 0.0);
    }

    #[test]
    fn non_positive_notional_falls_back_to_the_floor_not_a_divide_by_zero() {
        let tw = two_way_rate(0.04, &result_with_dv01(50_000.0), 0.0);
        let half = 0.5 * (tw.offer - tw.bid);
        assert!(half.is_finite());
        assert!((half - super::super::RATES_RFQ_HALF_SPREAD).abs() < 1e-15);
        assert!(tw.bid_size > 0.0);
    }
}
