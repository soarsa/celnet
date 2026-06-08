//! Property sweep for the multi-dealer ranking invariants over random ladders.
//!
//! For a random panel of in-process [`LadderSource`]s (random bids, offers,
//! epochs, validity windows), the engine's [`RankedPanel`](celnet_rfq::RankedPanel)
//! must satisfy — against an **independent** brute-force oracle computed from the
//! same injected ladders (NOT the engine's algebra):
//!
//! * `lp_count` == the number of responders (here: all sources answer in time);
//! * every `lp_won_*` references a real responder row (consistency invariant);
//! * `best_bid` == the max bid over *liftable* (not last-look-stale) responders,
//!   and the winner is the documented `(−bid, epoch, lp_id)`-minimal LP;
//! * `best_offer` == the min offer over liftable responders, winner
//!   `(offer, epoch, lp_id)`-minimal;
//! * when no responder is liftable, both winners are `None`.

mod harness;

use std::time::Duration;

use celnet_rfq::{MultiDealerEngine, QuoteSource, RfqRequest};
use celnet_types::{Ccy, CcyPair, OptionType, Tenor};
use proptest::prelude::*;

use harness::LadderSource;

/// One injected ladder row the oracle reasons over directly.
#[derive(Debug, Clone)]
struct Row {
    lp_id: String,
    bid: f64,
    offer: f64,
    epoch: u64,
    valid_until: u64,
}

/// The independent brute-force oracle: pick the bid-side and offer-side winners
/// straight from the injected rows using the documented total order, considering
/// only rows liftable at `now`. This does NOT call the engine — it is the ground
/// truth the engine is checked against.
fn oracle_winner(rows: &[Row], now: u64, bid_side: bool) -> Option<String> {
    rows.iter()
        .filter(|r| r.valid_until >= now)
        .min_by(|a, b| {
            let ka = if bid_side { -a.bid } else { a.offer };
            let kb = if bid_side { -b.bid } else { b.offer };
            ka.partial_cmp(&kb)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.epoch.cmp(&b.epoch))
                .then_with(|| a.lp_id.cmp(&b.lp_id))
        })
        .map(|r| r.lp_id.clone())
}

fn req() -> RfqRequest {
    RfqRequest::new(
        "REQ-PROP",
        CcyPair::new(Ccy::EUR, Ccy::USD),
        OptionType::Call,
        1.10,
        Tenor::Months(3),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn ranking_matches_independent_oracle(
        // 3..=8 LPs; each a (bid, offer, epoch, valid_until) ladder. Bids/offers
        // are small premia; epochs/validity span a band around `now`.
        raw in proptest::collection::vec(
            (0.0001f64..0.02, 0.0001f64..0.02, 0u64..1000, 0u64..2000),
            3..=8,
        ),
        now in 0u64..2000,
    ) {
        // Build rows with stable, distinct lp_ids.
        let rows: Vec<Row> = raw
            .iter()
            .enumerate()
            .map(|(i, &(bid, offer, epoch, valid_until))| Row {
                lp_id: format!("LP-{i:02}"),
                bid,
                offer,
                epoch,
                valid_until,
            })
            .collect();

        let sources: Vec<Box<dyn QuoteSource>> = rows
            .iter()
            .map(|r| {
                let src = LadderSource::firm(&r.lp_id, r.bid, r.offer, r.epoch)
                    .valid_until(r.valid_until);
                Box::new(src) as Box<dyn QuoteSource>
            })
            .collect();

        let engine = MultiDealerEngine::new(sources);
        let panel = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                engine
                    .request(&req(), Duration::from_secs(5), now)
                    .await
                    .unwrap()
            });

        // All sources answer in time ⇒ lp_count == panel size.
        prop_assert_eq!(panel.lp_count, rows.len());

        // Consistency invariant: every winner is a real responder row.
        if let Some(w) = &panel.lp_won_bid {
            prop_assert!(panel.rows.iter().any(|r| &r.lp_id == w));
        }
        if let Some(w) = &panel.lp_won_offer {
            prop_assert!(panel.rows.iter().any(|r| &r.lp_id == w));
        }

        // Winners match the independent oracle.
        let exp_bid = oracle_winner(&rows, now, true);
        let exp_offer = oracle_winner(&rows, now, false);
        prop_assert_eq!(&panel.lp_won_bid, &exp_bid);
        prop_assert_eq!(&panel.lp_won_offer, &exp_offer);

        // best_bid / best_offer match the winner's quoted side.
        match &exp_bid {
            Some(w) => {
                let r = rows.iter().find(|r| &r.lp_id == w).unwrap();
                prop_assert!((panel.best_bid.unwrap() - r.bid).abs() < 1e-12);
            }
            None => prop_assert!(panel.best_bid.is_none()),
        }
        match &exp_offer {
            Some(w) => {
                let r = rows.iter().find(|r| &r.lp_id == w).unwrap();
                prop_assert!((panel.best_offer.unwrap() - r.offer).abs() < 1e-12);
            }
            None => prop_assert!(panel.best_offer.is_none()),
        }
    }
}
