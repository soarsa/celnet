//! End-to-end golden-source + lifecycle tests (§7.3, §7.5, §8).
//!
//! Seeds the store from the deterministic OSS govvie source, then drives the announce → confirm →
//! apply lifecycle on real corporate-action events and asserts the two payoffs the doc requires: a
//! confirmed call collapses the instrument's schedule (pricing re-derives off fewer flows) and a
//! partial call scales the held position; a coupon on its pay date pays income and leaves the future
//! stream. Also covers bitemporal reads, reversal-by-supersession, and durable recovery.

use celnet_corpactions::{
    CaDates, CaEvent, CaStatus, CaTerms, Caev, Camv, CivilDate, PositionEffect,
};
use celnet_refstore::lifecycle;
use celnet_refstore::{
    GoldenSourceStore, GovvieSource, InMemoryPositionBook, Provenance, RefDataSource, SourceRef,
    StoredCorpAction,
};

const RECORDED: CivilDate = CivilDate::new(2026, 4, 16);

fn prov(valid_from: CivilDate) -> Provenance {
    Provenance {
        source_priority: 200,
        source: SourceRef("test".to_string()),
        valid_from,
        recorded_at: RECORDED,
        quality: 100,
    }
}

/// Open a store seeded with the full curated govvie universe, and return it plus a position book.
fn seeded() -> (GoldenSourceStore, InMemoryPositionBook, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = GoldenSourceStore::open(dir.path().join("golden.jrnl")).expect("open");
    for m in GovvieSource::curated().masters(RECORDED).expect("masters") {
        store.upsert_instrument(m).expect("seed");
    }
    (store, InMemoryPositionBook::new(), dir)
}

/// Pick a fixed-coupon govvie with several remaining coupons, returning (instrument_id, isin).
fn a_coupon_bond(store: &GoldenSourceStore) -> (String, String) {
    for id in store.instrument_ids() {
        let m = store.latest_instrument(&id).unwrap();
        let coupons = m
            .schedule
            .flows()
            .iter()
            .filter(|f| f.principal == 0.0)
            .count();
        if coupons >= 3
            && let Some(isin) = m.isin()
        {
            return (id.clone(), isin.to_string());
        }
    }
    panic!("no multi-coupon govvie in the curated universe");
}

fn call_event(
    isin: &str,
    terms: CaTerms,
    payment: CivilDate,
    caev: Caev,
    camv: Camv,
) -> StoredCorpAction {
    StoredCorpAction {
        ca_id: format!("{isin}:{}:{}", caev.code(), payment.year),
        event: CaEvent {
            isin: isin.to_string(),
            caev,
            camv,
            dates: CaDates {
                announcement: RECORDED,
                record: payment,
                ex: payment,
                response_deadline: None,
                payment,
            },
            terms,
            status: CaStatus::Announced,
            source_ref: "MT564/test".to_string(),
        },
        provenance: prov(payment),
    }
}

#[test]
fn full_call_collapses_schedule_and_realises_position() {
    let (mut store, mut book, _dir) = seeded();
    let (instrument_id, isin) = a_coupon_bond(&store);
    book.set_face(&instrument_id, 1_000_000.0);
    let flows_before = store
        .latest_instrument(&instrument_id)
        .unwrap()
        .schedule
        .len();
    assert!(flows_before > 1);

    // A full mandatory call at 101 on a future date.
    let ca = call_event(
        &isin,
        CaTerms::partial(1.0, 101.0),
        CivilDate::new(2027, 3, 15),
        Caev::Mcal,
        Camv::Mand,
    );
    let ca_id = ca.ca_id.clone();
    lifecycle::announce(&mut store, ca).expect("announce");
    lifecycle::confirm(&mut store, &ca_id, prov(CivilDate::new(2027, 3, 15))).expect("confirm");
    let outcome = lifecycle::apply(
        &mut store,
        &mut book,
        &ca_id,
        1_000_000.0,
        CivilDate::new(2027, 3, 15),
        RECORDED,
    )
    .expect("apply");

    // Schedule SHORTENS (collapses to empty) — pricing now re-derives off zero remaining flows.
    assert_eq!(outcome.remaining_flows, 0);
    assert!(
        store
            .latest_instrument(&instrument_id)
            .unwrap()
            .schedule
            .is_empty()
    );
    // Position REALISED to zero.
    assert!(matches!(outcome.effect, PositionEffect::Realise { .. }));
    assert!((book.holding(&instrument_id).unwrap().face).abs() < 1e-6);
    // CA is Applied.
    assert_eq!(
        store.latest_corp_action(&ca_id).unwrap().event.status,
        CaStatus::Applied
    );
}

#[test]
fn partial_call_scales_position_and_schedule_amounts() {
    let (mut store, mut book, _dir) = seeded();
    let (instrument_id, isin) = a_coupon_bond(&store);
    book.set_face(&instrument_id, 1_000_000.0);
    let coupon_before = store
        .latest_instrument(&instrument_id)
        .unwrap()
        .schedule
        .flows()[0]
        .coupon;

    // A partial call of 40% at 101.
    let ca = call_event(
        &isin,
        CaTerms::partial(0.40, 101.0),
        CivilDate::new(2027, 6, 15),
        Caev::Pcal,
        Camv::Mand,
    );
    let ca_id = ca.ca_id.clone();
    lifecycle::announce(&mut store, ca).expect("announce");
    lifecycle::confirm(&mut store, &ca_id, prov(CivilDate::new(2027, 6, 15))).expect("confirm");
    let outcome = lifecycle::apply(
        &mut store,
        &mut book,
        &ca_id,
        1_000_000.0,
        CivilDate::new(2027, 6, 15),
        RECORDED,
    )
    .expect("apply");

    // Position SCALES: face 1,000,000 → 600,000, cash 400,000 at 101 = 404,000.
    assert!(matches!(outcome.effect, PositionEffect::Scale { .. }));
    let h = book.holding(&instrument_id).unwrap();
    assert!((h.face - 600_000.0).abs() < 1e-6, "face {}", h.face);
    assert!((h.cash - 404_000.0).abs() < 1e-6, "cash {}", h.cash);
    // Schedule amounts scale by the 0.60 retained fraction (pool factor + coupon).
    let post = store.latest_instrument(&instrument_id).unwrap();
    assert!((post.schedule.pool_factor() - 0.60).abs() < 1e-9);
    assert!((post.schedule.flows()[0].coupon - coupon_before * 0.60).abs() < 1e-9);
}

#[test]
fn coupon_on_pay_date_is_income_and_leaves_future_stream() {
    let (mut store, mut book, _dir) = seeded();
    let (instrument_id, isin) = a_coupon_bond(&store);
    book.set_face(&instrument_id, 1_000_000.0);
    let m = store.latest_instrument(&instrument_id).unwrap();
    // The first FUTURE coupon-only flow.
    let first = m
        .schedule
        .flows()
        .iter()
        .find(|f| f.principal == 0.0 && f.date > RECORDED)
        .copied()
        .expect("a future coupon");
    let flows_before = m.schedule.len();

    let ca = call_event(
        &isin,
        CaTerms::coupon(0.0),
        first.date,
        Caev::Intr,
        Camv::Mand,
    );
    let ca_id = ca.ca_id.clone();
    lifecycle::announce(&mut store, ca).expect("announce");
    lifecycle::confirm(&mut store, &ca_id, prov(first.date)).expect("confirm");
    let outcome = lifecycle::apply(
        &mut store,
        &mut book,
        &ca_id,
        1_000_000.0,
        first.date,
        RECORDED,
    )
    .expect("apply");

    // Income = the coupon on a 1mm holding; face unchanged; exactly one flow leaves the schedule.
    match outcome.effect {
        PositionEffect::Income { cash_per_100 } => {
            assert!((cash_per_100 - first.coupon).abs() < 1e-9)
        }
        other => panic!("expected Income, got {other:?}"),
    }
    let h = book.holding(&instrument_id).unwrap();
    assert!((h.face - 1_000_000.0).abs() < 1e-6);
    assert!((h.cash - first.coupon * 10_000.0).abs() < 1e-3); // coupon per 100 * (1mm/100)
    assert_eq!(
        store
            .latest_instrument(&instrument_id)
            .unwrap()
            .schedule
            .len(),
        flows_before - 1
    );
}

#[test]
fn reversal_supersedes_restores_schedule_and_unbooks_position() {
    let (mut store, mut book, _dir) = seeded();
    let (instrument_id, isin) = a_coupon_bond(&store);
    book.set_face(&instrument_id, 1_000_000.0);
    let flows_before = store
        .latest_instrument(&instrument_id)
        .unwrap()
        .schedule
        .len();

    let ca = call_event(
        &isin,
        CaTerms::partial(0.25, 100.0),
        CivilDate::new(2027, 9, 15),
        Caev::Pred,
        Camv::Mand,
    );
    let ca_id = ca.ca_id.clone();
    lifecycle::announce(&mut store, ca).expect("announce");
    lifecycle::confirm(&mut store, &ca_id, prov(CivilDate::new(2027, 9, 15))).expect("confirm");
    lifecycle::apply(
        &mut store,
        &mut book,
        &ca_id,
        1_000_000.0,
        CivilDate::new(2027, 9, 15),
        RECORDED,
    )
    .expect("apply");
    assert!((book.holding(&instrument_id).unwrap().face - 750_000.0).abs() < 1e-6);

    // Reverse (seev.037): position back to 1mm, cash back to 0, schedule restored.
    lifecycle::reverse(
        &mut store,
        &mut book,
        &ca_id,
        1_000_000.0,
        CivilDate::new(2027, 9, 15),
        RECORDED,
    )
    .expect("reverse");
    let h = book.holding(&instrument_id).unwrap();
    assert!((h.face - 1_000_000.0).abs() < 1e-6, "face {}", h.face);
    assert!(h.cash.abs() < 1e-6, "cash {}", h.cash);
    let post = store.latest_instrument(&instrument_id).unwrap();
    assert_eq!(post.schedule.len(), flows_before);
    assert!((post.schedule.pool_factor() - 1.0).abs() < 1e-9);
    // Nothing deleted: the full lifecycle history is retained (announced..applied..reversed).
    let history = store.corp_action_history(&ca_id);
    assert!(history.len() >= 4);
    assert_eq!(history.last().unwrap().event.status, CaStatus::Reversed);
}

#[test]
fn bitemporal_read_resolves_the_effective_version() {
    let (mut store, _book, _dir) = seeded();
    let (instrument_id, isin) = a_coupon_bond(&store);

    // Apply a partial redemption effective 2028-01-15 → a new version.
    let ca = call_event(
        &isin,
        CaTerms::partial(0.5, 100.0),
        CivilDate::new(2028, 1, 15),
        Caev::Pred,
        Camv::Mand,
    );
    let ca_id = ca.ca_id.clone();
    let mut book = InMemoryPositionBook::new();
    book.set_face(&instrument_id, 1_000_000.0);
    lifecycle::announce(&mut store, ca).expect("announce");
    lifecycle::confirm(&mut store, &ca_id, prov(CivilDate::new(2028, 1, 15))).expect("confirm");
    lifecycle::apply(
        &mut store,
        &mut book,
        &ca_id,
        1_000_000.0,
        CivilDate::new(2028, 1, 15),
        CivilDate::new(2028, 1, 15),
    )
    .expect("apply");

    // As-of a valuation BEFORE the event's effective date → the original full-pool version.
    let before = store
        .effective_instrument(
            &instrument_id,
            CivilDate::new(2027, 1, 1),
            CivilDate::new(2028, 6, 1),
        )
        .expect("effective before");
    assert!((before.schedule.pool_factor() - 1.0).abs() < 1e-9);
    // As-of a valuation AFTER the effective date → the scaled version.
    let after = store
        .effective_instrument(
            &instrument_id,
            CivilDate::new(2028, 6, 1),
            CivilDate::new(2028, 6, 1),
        )
        .expect("effective after");
    assert!((after.schedule.pool_factor() - 0.5).abs() < 1e-9);
}

#[test]
fn store_recovers_from_the_durable_journal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("golden.jrnl");
    let (instrument_id, n) = {
        let mut store = GoldenSourceStore::open(&path).expect("open");
        for m in GovvieSource::curated().masters(RECORDED).expect("masters") {
            store.upsert_instrument(m).expect("seed");
        }
        let id = store.instrument_ids()[0].clone();
        (id, store.instrument_ids().len())
    };
    // Reopen: the append-only log rebuilds the projections identically.
    let reopened = GoldenSourceStore::open(&path).expect("reopen");
    assert_eq!(reopened.instrument_ids().len(), n);
    assert!(reopened.latest_instrument(&instrument_id).is_some());
}
