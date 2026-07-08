//! Deterministic RFQ rotation for the FIX quote simulator (the runnable
//! [`fix_rfq_client`](../../examples/fix_rfq_client.rs) example and the deploy
//! `start-fix-sim.sh` daemon).
//!
//! The rotation is a **pure function of the stream iteration index** — no RNG, no
//! wall-clock — so a replayed stream is byte-reproducible and gate-testable. The FX
//! arm rotates a realistic grid of major deliverable pairs, near-the-money strikes,
//! short-dated expiries and call/put, injecting a deliberately **unpriceable** RFQ
//! every `manual_every`-th iteration so the operator can watch both the auto-quote
//! path and the manual-intervention (desk-routed) path fill up.
//!
//! ## Why these two manual variants
//!
//! The FX-options venue prices an inbound `QuoteRequest(R)` off its live surface and
//! returns nothing (no `Quote(S)`) when it cannot — the taker then reports the ticket
//! as routed to the FX desk for manual pricing. Two request shapes are declined
//! *deterministically by the server's own rules*, so they are the reliable
//! manual-intervention triggers:
//!
//!   * **American exercise** — the FIX edge prices European vanillas only (American FX
//!     vanillas are worked by voice/desk), so the venue declines a `1194=1` request.
//!   * **Non-deliverable request on a deliverable major** — the dialect cross-checks
//!     the requested `SecurityType` (FXVO/FXNO) against the pair's resolved settlement
//!     convention; an NDF (`FXNO`) request on a G10 deliverable major is a settlement
//!     mismatch and is declined.
//!
//! An "unknown pair" is intentionally **not** used: any three ASCII letters parse to a
//! valid currency and resolve a default (deliverable) convention, so a bogus pair would
//! actually price — it is not a reliable decline.

use celnet_types::{OptionType, Settlement};

use crate::dialect_fx::ExerciseStyle;

/// Major **deliverable** currency pairs the demo venue auto-quotes. The demo server
/// prices every pair off its calibrated EUR/USD 1Y fixture (a single global surface at
/// spot ~1.10), so the pair label varies the blotter while the strike grid is held near
/// that level for lively, non-degenerate two-way premiums.
pub const FX_PAIRS: &[&str] = &["EURUSD", "USDJPY", "GBPUSD", "USDCHF", "AUDUSD", "EURGBP"];

/// Near-the-money strike grid bracketing the demo spot (~1.10): ITM / ATM / OTM for both
/// calls and puts, so every auto RFQ returns a healthy two-way off the flat demo surface.
pub const FX_STRIKES: &[f64] = &[1.00, 1.05, 1.10, 1.15, 1.20];

/// Standard short-dated vol-times to expiry in years — 1W, 1M, 3M, 6M, 1Y (ACT/365 for
/// the sub-month points) — the market-standard grid. Any positive vol-time prices on the
/// flat demo surface; these are the labels a trader would recognise.
pub const FX_EXPIRIES_YEARS: &[f64] = &[
    7.0 / 365.0, // 1W
    1.0 / 12.0,  // 1M
    0.25,        // 3M
    0.5,         // 6M
    1.0,         // 1Y
];

/// How the venue is expected to handle a rotated leg — an auto-quote, or one of the two
/// deterministic desk-routed (declined) variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxLegKind {
    /// A priceable European deliverable vanilla the venue auto-quotes off the surface.
    AutoQuote,
    /// American exercise — the vanilla edge prices European only ⇒ desk-routed.
    ManualAmerican,
    /// A non-deliverable request on a deliverable major ⇒ settlement mismatch, desk-routed.
    ManualNonDeliverable,
}

impl FxLegKind {
    /// A short operator label for the log line describing why a manual leg is desk-routed.
    #[must_use]
    pub fn manual_reason(self) -> Option<&'static str> {
        match self {
            FxLegKind::AutoQuote => None,
            FxLegKind::ManualAmerican => Some("American exercise"),
            FxLegKind::ManualNonDeliverable => Some("non-deliverable settlement"),
        }
    }
}

/// One rotated FX vanilla-option RFQ the simulator emits on stream iteration `i`.
#[derive(Debug, Clone, PartialEq)]
pub struct FxLeg {
    /// The 6-letter currency pair (`Symbol(55)`), e.g. `EURUSD`.
    pub pair: &'static str,
    /// Call or put (`PutOrCall(201)`).
    pub option_type: OptionType,
    /// The absolute strike in quote ccy per base (`StrikePrice(202)`).
    pub strike: f64,
    /// Vol-time to expiry in years (the dialect's custom expiry tag).
    pub expiry_years: f64,
    /// Deliverable (FXVO) or non-deliverable (FXNO) — the `SecurityType(167)` block.
    pub settlement: Settlement,
    /// European or American exercise (`ExerciseStyle(1194)`).
    pub exercise: ExerciseStyle,
    /// The expected venue disposition of this leg.
    pub kind: FxLegKind,
}

/// Select the FX vanilla-option RFQ for stream iteration `i`, injecting a manual
/// (unpriceable, desk-routed) leg every `manual_every`-th iteration (`0` ⇒ never manual).
///
/// Deterministic: a pure function of `(i, manual_every)`. The two manual variants
/// alternate by the manual occurrence ordinal so the desk inbox shows both an American
/// and an NDF exception over time.
#[must_use]
pub fn fx_leg(i: u64, manual_every: u64) -> FxLeg {
    let pair = FX_PAIRS[(i as usize) % FX_PAIRS.len()];
    let option_type = if i.is_multiple_of(2) {
        OptionType::Call
    } else {
        OptionType::Put
    };
    let strike = FX_STRIKES[(i as usize) % FX_STRIKES.len()];
    let expiry_years = FX_EXPIRIES_YEARS[(i as usize) % FX_EXPIRIES_YEARS.len()];

    let is_manual = manual_every > 0 && (i + 1).is_multiple_of(manual_every);
    // Alternate the two manual variants deterministically by the manual occurrence
    // ordinal (no RNG): even ordinal ⇒ American, odd ⇒ non-deliverable.
    let manual_american = is_manual && ((i + 1) / manual_every).is_multiple_of(2);

    let (settlement, exercise, kind) = if is_manual && manual_american {
        (
            Settlement::Deliverable,
            ExerciseStyle::American,
            FxLegKind::ManualAmerican,
        )
    } else if is_manual {
        (
            Settlement::NonDeliverable,
            ExerciseStyle::European,
            FxLegKind::ManualNonDeliverable,
        )
    } else {
        (
            Settlement::Deliverable,
            ExerciseStyle::European,
            FxLegKind::AutoQuote,
        )
    };

    FxLeg {
        pair,
        option_type,
        strike,
        expiry_years,
        settlement,
        exercise,
        kind,
    }
}

/// The strike currency of a 6-letter pair is its quote (domestic) leg — the last three
/// letters. The dialect requires `StrikeCurrency(947)` to equal the pair's quote ccy.
#[must_use]
pub fn strike_ccy_of(pair: &str) -> &str {
    if pair.len() == 6 { &pair[3..6] } else { "" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect_fx::{DialectError, SEC_TYPE_FXNO, SEC_TYPE_FXVO, decode_option};
    use crate::framing::{FrameCursor, FrameEncoder};
    use celnet_types::Tenor;

    /// The auto legs the simulator emits are all priceable European deliverable vanillas
    /// with valid, finite, positive fields on a 6-letter deliverable major.
    #[test]
    fn auto_leg_is_priceable_european_deliverable_with_valid_fields() {
        // manual_every = 3 ⇒ i = 0, 1 are auto (i+1 = 1, 2 not multiples of 3).
        let leg = fx_leg(0, 3);
        assert_eq!(leg.kind, FxLegKind::AutoQuote);
        assert_eq!(leg.exercise, ExerciseStyle::European);
        assert_eq!(leg.settlement, Settlement::Deliverable);
        assert!(leg.strike > 0.0 && leg.strike.is_finite());
        assert!(leg.expiry_years > 0.0 && leg.expiry_years.is_finite());
        assert_eq!(leg.pair.len(), 6);
        assert_eq!(leg.kind.manual_reason(), None);
    }

    /// The rotation depends only on the iteration index (no RNG / no wall-clock) and
    /// genuinely varies the pair, so the same replay yields the same stream.
    #[test]
    fn rotation_is_deterministic_and_varies_the_pair() {
        for i in 0..64 {
            assert_eq!(fx_leg(i, 3), fx_leg(i, 3));
        }
        // Distinct consecutive pairs, and every configured pair is visited over a cycle.
        assert_ne!(fx_leg(0, 0).pair, fx_leg(1, 0).pair);
        let visited: std::collections::BTreeSet<&str> = (0..FX_PAIRS.len() as u64)
            .map(|i| fx_leg(i, 0).pair)
            .collect();
        assert_eq!(visited.len(), FX_PAIRS.len());
    }

    /// Every `manual_every`-th leg is one of the two deterministic desk-routed variants,
    /// alternating American ↔ non-deliverable; `manual_every = 0` disables manual legs.
    #[test]
    fn manual_cadence_alternates_two_unpriceable_variants() {
        // manual when (i+1) % 3 == 0: i = 2 (ordinal 1, odd ⇒ NDF), i = 5 (ordinal 2,
        // even ⇒ American), i = 8 (ordinal 3, odd ⇒ NDF), i = 11 (ordinal 4, American).
        assert_eq!(fx_leg(2, 3).kind, FxLegKind::ManualNonDeliverable);
        assert_eq!(fx_leg(2, 3).settlement, Settlement::NonDeliverable);
        assert_eq!(fx_leg(5, 3).kind, FxLegKind::ManualAmerican);
        assert_eq!(fx_leg(5, 3).exercise, ExerciseStyle::American);
        assert_eq!(fx_leg(8, 3).kind, FxLegKind::ManualNonDeliverable);
        assert_eq!(fx_leg(11, 3).kind, FxLegKind::ManualAmerican);
        // No manual legs when the cadence is 0.
        for i in 0..12 {
            assert_eq!(fx_leg(i, 0).kind, FxLegKind::AutoQuote);
        }
    }

    /// Encode a leg as the venue would see it on the wire and assert it decodes exactly
    /// as intended: an auto leg to a European deliverable descriptor, an NDF leg to the
    /// settlement-mismatch decline, and an American leg to the American descriptor the
    /// server then declines. This proves the constructed FIX fields are valid.
    fn frame_for(leg: &FxLeg) -> Vec<u8> {
        let mut e = FrameEncoder::new();
        e.push(35, b"R");
        e.push(131, b"REQ");
        e.push(55, leg.pair.as_bytes());
        e.push(460, b"4");
        e.push(
            167,
            match leg.settlement {
                Settlement::Deliverable => SEC_TYPE_FXVO,
                Settlement::NonDeliverable => SEC_TYPE_FXNO,
            },
        );
        e.push(
            201,
            match leg.option_type {
                OptionType::Call => b"1",
                OptionType::Put => b"0",
            },
        );
        e.push(202, format!("{}", leg.strike).as_bytes());
        e.push(947, strike_ccy_of(leg.pair).as_bytes());
        e.push(
            1194,
            match leg.exercise {
                ExerciseStyle::European => b"0",
                ExerciseStyle::American => b"1",
            },
        );
        e.finish()
    }

    #[test]
    fn auto_leg_round_trips_to_a_european_deliverable_descriptor() {
        let leg = fx_leg(0, 3);
        let raw = frame_for(&leg);
        let frame = FrameCursor::parse(&raw).unwrap();
        let desc = decode_option(&frame, Tenor::Months(3)).expect("auto leg must decode");
        assert_eq!(desc.exercise, ExerciseStyle::European);
        assert_eq!(desc.settlement, Settlement::Deliverable);
        assert_eq!(desc.option_type, leg.option_type);
        assert!((desc.strike - leg.strike).abs() < 1e-12);
    }

    #[test]
    fn ndf_manual_leg_is_a_settlement_mismatch_on_a_deliverable_major() {
        let leg = fx_leg(2, 3); // NDF on the deliverable major at index 2.
        assert_eq!(leg.kind, FxLegKind::ManualNonDeliverable);
        let raw = frame_for(&leg);
        let frame = FrameCursor::parse(&raw).unwrap();
        assert_eq!(
            decode_option(&frame, Tenor::Months(3)),
            Err(DialectError::SettlementMismatch),
        );
    }

    #[test]
    fn american_manual_leg_decodes_american_for_the_server_to_decline() {
        let leg = fx_leg(5, 3);
        assert_eq!(leg.kind, FxLegKind::ManualAmerican);
        let raw = frame_for(&leg);
        let frame = FrameCursor::parse(&raw).unwrap();
        let desc = decode_option(&frame, Tenor::Months(3)).expect("American leg decodes");
        // The dialect accepts American; the vanilla-GK edge (price_request) then declines
        // it because it prices European only.
        assert_eq!(desc.exercise, ExerciseStyle::American);
    }
}
