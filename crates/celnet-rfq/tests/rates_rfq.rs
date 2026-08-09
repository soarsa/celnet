//! Multi-dealer **fixed-income** RFQ oracle — the rates analogue of `oracle.rs`.
//!
//! Proves the [`MultiDealerEngine`] ranks a linear-rates (fixed-income) RFQ over
//! ≥ 2 competing dealer lines and books the best on each side, over the SAME
//! ranking / tie-break / booking seam the FX-option panel uses (ADR-0021
//! uniform-asset-class — the engine ranks `TwoWay` + timestamps, class-agnostic).
//! The panel mixes:
//!
//! * a native in-process [`InternalPricerSource`] (its mid injected by the edge —
//!   it ignores the leg, so it prices a rates RFQ unchanged);
//! * a **real** loopback [`FixLpAdapter`](celnet_rfq::FixLpAdapter) that translates
//!   the rates [`RfqLeg`](celnet_rfq::RfqLeg) into a rates FIX `QuoteRequest(R)`
//!   via the `celnet-fix` rates dialect and runs a genuine FIX 4.4 session over a
//!   socket (no mock — the synthetic LP decodes the request as an OIS/bond RFQ and
//!   replies an injected two-way);
//! * a deterministic in-process [`LadderSource`] with a KNOWN injected ladder.
//!
//! The oracle computes the expected touch winners from the injected two-ways (its
//! ground truth), NOT from the engine, and every body is hard wall-clock bounded.
//!
//! # Honest boundary (verbatim)
//!
//! Live LP-panel connectivity (real bank sessions over WAN FIX) is ENV — designed,
//! seamed and ADR'd in-repo, validated at deploy, NEVER claimed in-repo. In-repo
//! this suite proves the aggregation / ranking / booking ALGORITHM plus the rates
//! FIX framing / dialect round-trip over a **loopback** socket only.

mod harness;

use std::time::Duration;

use celnet_proto::{
    AccrualBasis, BondInstrument, BrokenDate, OisInstrument, PaymentFrequency, RatesInstrument,
    Side, rates_instrument,
};
use celnet_rfq::{InternalPricerSource, MultiDealerEngine, RfqRequest, TwoWay};

use harness::{LadderSource, spawn_fix_rates_lp};

const DEADLINE: Duration = Duration::from_secs(5);
const PANEL_DEADLINE: Duration = Duration::from_millis(500);

/// A 5y USD-SOFR OIS `RatesInstrument` (receive-fixed), the P0 arm.
fn ois_5y() -> RatesInstrument {
    RatesInstrument {
        instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
            tenor_years: 5,
            fixed_rate: 0.0,
            notional: 100_000_000.0,
            side: Side::Sell as i32,
        })),
    }
}

/// A fixed-coupon cash-bond `RatesInstrument` (long).
fn bond_5y() -> RatesInstrument {
    RatesInstrument {
        instrument: Some(rates_instrument::Instrument::Bond(BondInstrument {
            coupon_rate: 0.05,
            coupon_frequency: PaymentFrequency::SemiAnnual as i32,
            day_count: AccrualBasis::Thirty360BondBasis as i32,
            maturity_date: Some(BrokenDate {
                year: 2031,
                month: 6,
                day: 25,
            }),
            redemption: 100.0,
            side: Side::Buy as i32,
            ..Default::default()
        })),
    }
}

/// Gate — a multi-dealer FI RFQ ranks ≥ 2 dealer lines and books the best on each
/// side, with a REAL loopback FIX rates LP in the panel. The engine names the
/// injected best-bid / best-offer dealer; the oracle knows the winners by
/// construction. Mirrors the options `best_bid_offer_match_injected_extremum`.
#[tokio::test]
async fn fi_multi_dealer_ranks_and_books_best_with_real_fix_lp() {
    tokio::time::timeout(DEADLINE, async {
        let epoch = 1_000_u64;
        // A rates RFQ leg: a 5y OIS, two-way requested, `USD-OIS` symbol.
        let req = RfqRequest::new_rates(
            "FI-RFQ-1",
            b"USD-OIS".to_vec(),
            ois_5y(),
            100_000_000.0,
            Side::TwoWay,
        );

        // Native in-process dealer: mid 0.0405 ± 1bp ⇒ bid ≈0.0404 / offer ≈0.0406
        // (the injected offer is the panel touch — the tightest offer).
        let native = InternalPricerSource::new("NATIVE", 0.0405, 0.0001, epoch, 1_000_000);
        let native_tw = native.two_way();
        assert!((native_tw.bid - 0.0404).abs() < 1e-12 && (native_tw.offer - 0.0406).abs() < 1e-12);

        // A REAL loopback FIX rates LP quoting the strongest bid (0.0405) — it must
        // win the bid side, proving the engine ranks across the genuine FIX leg.
        let fix_lp = spawn_fix_rates_lp(
            "FIX-RATES",
            TwoWay {
                bid: 0.0405,
                offer: 0.0408,
            },
            epoch,
            1_000_000,
        )
        .await;

        // A wide in-process ladder LP (neither touch): bid 0.0403 / offer 0.0407.
        let ladder = LadderSource::firm("LADDER", 0.0403, 0.0407, epoch);

        let engine =
            MultiDealerEngine::new(vec![Box::new(native), Box::new(fix_lp), Box::new(ladder)]);
        let panel = engine
            .request(&req, PANEL_DEADLINE, epoch)
            .await
            .expect("panel ranks without a consistency fault");

        // All three responded (the real FIX leg included) — ≥ 2 dealer lines.
        assert_eq!(panel.lp_count, 3, "three dealers quoted (incl. the FIX LP)");
        assert!(panel.rows.iter().any(|r| r.lp_id == "FIX-RATES"));

        // Books the best: FIX-RATES holds the top bid 0.0405 (wire-exact from the
        // real FIX leg); NATIVE the tightest offer ≈0.0406 — the injected extrema,
        // known here by construction.
        assert_eq!(panel.lp_won_bid.as_deref(), Some("FIX-RATES"));
        assert_eq!(panel.lp_won_offer.as_deref(), Some("NATIVE"));
        assert_eq!(panel.best_bid, Some(0.0405));
        assert!((panel.best_offer.unwrap() - 0.0406).abs() < 1e-12);
    })
    .await
    .expect("test must not hang");
}

/// The [`FixLpAdapter`] cash-bond arm: a bond `RfqLeg` is encoded as a rates FIX
/// `QuoteRequest(R)` with `SecurityType(167)=BOND` and round-trips a clean-price
/// two-way over a real loopback FIX session (the bond-dialect encode path).
#[tokio::test]
async fn fi_bond_arm_round_trips_over_real_fix() {
    tokio::time::timeout(DEADLINE, async {
        let epoch = 2_000_u64;
        let req = RfqRequest::new_rates(
            "FI-BOND-1",
            b"US-TSY-5Y".to_vec(),
            bond_5y(),
            25_000_000.0,
            Side::Buy,
        );

        // The synthetic LP quotes an injected clean-price two-way (8-dp clean values).
        let fix_lp = spawn_fix_rates_lp(
            "FIX-BOND",
            TwoWay {
                bid: 98.50,
                offer: 98.70,
            },
            epoch,
            1_000_000,
        )
        .await;

        let engine = MultiDealerEngine::new(vec![Box::new(fix_lp)]);
        let panel = engine
            .request(&req, PANEL_DEADLINE, epoch)
            .await
            .expect("bond panel ranks");

        // The bond RFQ was accepted as a valid bond-dialect frame and quoted.
        assert_eq!(panel.lp_count, 1, "the FIX bond LP quoted");
        assert_eq!(panel.best_bid, Some(98.50));
        assert_eq!(panel.best_offer, Some(98.70));
        assert_eq!(panel.lp_won_bid.as_deref(), Some("FIX-BOND"));
    })
    .await
    .expect("test must not hang");
}
