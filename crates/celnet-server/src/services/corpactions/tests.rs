//! Corporate-actions edge tests — the confirm→apply lifecycle drives the effective
//! schedule + rates-book realisation, verified against the `celnet-corpactions` oracle,
//! plus the `refdata` capability gate.

use std::sync::Arc;

use celnet_corpactions::{
    BondSchedule, CaDates, CaEvent, CaStatus, CaTerms, Caev, Camv, CivilDate, apply_event,
    position_delta,
};
use celnet_entitlements::{Action, AssetClass, Capability};
use celnet_proto::corporate_actions_service_server::CorporateActionsService;
use celnet_proto::{
    ApplyCorporateActionRequest, ConfirmCorporateActionRequest, CorpActionStatus,
    ListCorporateActionsRequest, ListInstrumentScheduleRequest, Side, rates_instrument,
};
use celnet_refstore::{
    CouponType, ExternalIds, GoldenSourceStore, InstrumentMaster, InstrumentTerms, Provenance,
    SourceRef, StoredCorpAction,
};
use tonic::{Code, Request};

use super::CorporateActionsEdge;
use crate::clock::Clock;
use crate::config::identity::{Role, default_trader_bundle};
use crate::readiness::ReadinessGate;
use crate::services::rates_book::RatesPositionStore;
use crate::services::sessions::{AuthenticatedUser, SessionRegistry};

const ISIN: &str = "TESTISIN0001";
const INSTR: &str = "test-callable";
const HELD_FACE: f64 = 1_000_000.0;

/// A 2y semi-annual 4% bond master, dated 2033-06-15, matures 2035-06-15 → 4 flows.
fn master() -> InstrumentMaster {
    let schedule = BondSchedule::fixed_coupon(
        CivilDate::new(2033, 6, 15),
        CivilDate::new(2035, 6, 15),
        0.04,
        2,
        100.0,
    )
    .expect("schedule");
    InstrumentMaster {
        instrument_id: INSTR.to_string(),
        external_ids: ExternalIds {
            figi: None,
            isin: Some(ISIN.to_string()),
            lei_issuer: None,
            cusip: None,
        },
        terms: InstrumentTerms {
            name: "Test Callable 4% 2035".to_string(),
            issuer: "Test Issuer".to_string(),
            currency: "USD".to_string(),
            coupon_type: CouponType::Fixed,
            coupon_rate: 0.04,
            coupons_per_year: 2,
            day_count: "act_act".to_string(),
            dated_date: Some(CivilDate::new(2033, 6, 15)),
            maturity_date: CivilDate::new(2035, 6, 15),
            redemption: 100.0,
            calendars: vec!["us".to_string()],
        },
        schedule,
        provenance: Provenance {
            source_priority: 200,
            source: SourceRef("test".to_string()),
            valid_from: CivilDate::new(2026, 1, 1),
            recorded_at: CivilDate::new(2026, 1, 1),
            quality: 100,
        },
    }
}

fn dates(payment: CivilDate) -> CaDates {
    CaDates {
        announcement: CivilDate::new(2034, 1, 1),
        record: payment,
        ex: payment,
        response_deadline: None,
        payment,
    }
}

fn ca(ca_id: &str, caev: Caev, terms: CaTerms, payment: CivilDate) -> StoredCorpAction {
    StoredCorpAction {
        ca_id: ca_id.to_string(),
        event: CaEvent {
            isin: ISIN.to_string(),
            caev,
            camv: Camv::Mand,
            dates: dates(payment),
            terms,
            status: CaStatus::Announced,
            source_ref: "test".to_string(),
        },
        provenance: Provenance {
            source_priority: 200,
            source: SourceRef("test".to_string()),
            valid_from: payment,
            recorded_at: payment,
            quality: 100,
        },
    }
}

struct Fixture {
    edge: Arc<CorporateActionsEdge>,
    rates: Arc<RatesPositionStore>,
    sessions: Arc<SessionRegistry>,
}

fn fixture() -> Fixture {
    let clock = Clock::manual(0);
    let sessions = Arc::new(SessionRegistry::new(clock));
    let gate = Arc::new(ReadinessGate::new());
    gate.mark_ready();
    let rates = Arc::new(RatesPositionStore::new());
    let tmp = tempfile::NamedTempFile::new().expect("temp journal");
    let store = GoldenSourceStore::open(tmp.path()).expect("open store");
    // Keep the tempfile alive for the edge's lifetime.
    std::mem::forget(tmp);
    let edge = Arc::new(CorporateActionsEdge::new(
        Arc::clone(&sessions),
        gate,
        Arc::clone(&rates),
        store,
    ));
    edge.upsert_master(master()).expect("seed master");
    Fixture {
        edge,
        rates,
        sessions,
    }
}

/// Issue a session for a user with the given role + explicit per-user grants.
fn token(sessions: &SessionRegistry, role: Role, grants: Vec<Capability>) -> String {
    let user = AuthenticatedUser {
        user_id: "u1".to_string(),
        email: "u1@celnet.com".to_string(),
        display_name: "U One".to_string(),
        role,
        desk_ids: Vec::new(),
        all_desks: true,
        role_caps: default_trader_bundle(),
        cap_grants: grants,
        cap_denies: Vec::new(),
    };
    sessions.issue(user).expect("issue session").token
}

/// A plain trader token (no `refdata`): the read floor only.
fn trader_token(sessions: &SessionRegistry) -> String {
    token(sessions, Role::Trader, Vec::new())
}

/// A trader token holding `refdata·fixed_income` (the CA confirm/apply authority).
fn refdata_token(sessions: &SessionRegistry) -> String {
    token(
        sessions,
        Role::Trader,
        vec![Capability::new(Action::Refdata, AssetClass::FixedIncome)],
    )
}

#[tokio::test]
async fn confirmed_applied_full_call_shortens_schedule_and_realises_position() {
    let fx = fixture();
    let payment = CivilDate::new(2034, 9, 15);
    let ca_id = "test-callable:MCAL:20340915";
    // A full mandatory call at 101 between the Jun/Dec 2034 coupons.
    let event = ca(ca_id, Caev::Mcal, CaTerms::partial(1.0, 101.0), payment);
    fx.edge.announce(event.clone()).expect("announce");
    let tok = refdata_token(&fx.sessions);

    // Confirm then apply.
    fx.edge
        .confirm_corporate_action(Request::new(ConfirmCorporateActionRequest {
            session_token: tok.clone(),
            ca_id: ca_id.to_string(),
            correlation_id: None,
        }))
        .await
        .expect("confirm");
    let applied = fx
        .edge
        .apply_corporate_action(Request::new(ApplyCorporateActionRequest {
            session_token: tok.clone(),
            ca_id: ca_id.to_string(),
            held_face: HELD_FACE,
            correlation_id: None,
        }))
        .await
        .expect("apply")
        .into_inner();

    // Oracle: the pure effect on the pre-event schedule.
    let oracle = apply_event(&master().schedule, &event.event).expect("oracle");
    let oracle_delta = position_delta(&oracle.effect, HELD_FACE);
    assert!((applied.face_delta - oracle_delta.face_delta).abs() < 1e-6);
    assert!((applied.cash - oracle_delta.cash).abs() < 1e-6);
    assert!((applied.face_delta + HELD_FACE).abs() < 1e-6, "realised");
    assert!(
        (applied.cash - 1_010_000.0).abs() < 1e-6,
        "101 per 100 face"
    );
    assert_eq!(
        applied.remaining_flows, 0,
        "schedule emptied by a full call"
    );
    assert_eq!(
        applied.action.expect("action").status,
        CorpActionStatus::Applied as i32
    );

    // The effective schedule read now shows an emptied schedule.
    let sched = fx
        .edge
        .list_instrument_schedule(Request::new(ListInstrumentScheduleRequest {
            session_token: tok,
            instrument_id: INSTR.to_string(),
            correlation_id: None,
        }))
        .await
        .expect("list schedule")
        .into_inner();
    assert!(sched.flows.is_empty(), "post-call schedule is empty");

    // A realising SELL bond leg sized by the redeemed face was booked into the rates book.
    let positions = fx.rates.snapshot();
    let bond = positions
        .iter()
        .find_map(|p| match p.instrument.as_ref()?.instrument.as_ref()? {
            rates_instrument::Instrument::Bond(b) => Some(b),
            _ => None,
        })
        .expect("a realising bond leg was booked");
    assert_eq!(bond.side, Side::Sell as i32);
    assert!((bond.redemption - HELD_FACE).abs() < 1e-6);
}

#[tokio::test]
async fn coupon_on_pay_date_pays_income_without_a_face_change() {
    let fx = fixture();
    let payment = CivilDate::new(2033, 12, 15); // the first coupon date
    let ca_id = "test-callable:INTR:20331215";
    // The 4%/2 = 2.0-per-100 coupon on the first pay date.
    let event = ca(ca_id, Caev::Intr, CaTerms::coupon(2.0), payment);
    fx.edge.announce(event.clone()).expect("announce");
    let tok = refdata_token(&fx.sessions);

    fx.edge
        .confirm_corporate_action(Request::new(ConfirmCorporateActionRequest {
            session_token: tok.clone(),
            ca_id: ca_id.to_string(),
            correlation_id: None,
        }))
        .await
        .expect("confirm");
    let applied = fx
        .edge
        .apply_corporate_action(Request::new(ApplyCorporateActionRequest {
            session_token: tok.clone(),
            ca_id: ca_id.to_string(),
            held_face: HELD_FACE,
            correlation_id: None,
        }))
        .await
        .expect("apply")
        .into_inner();

    // Income: cash, no face change (2.0 per 100 × 1mm face = 20_000).
    assert!(
        applied.face_delta.abs() < 1e-9,
        "no face change on a coupon"
    );
    assert!((applied.cash - 20_000.0).abs() < 1e-6);
    // The coupon-only flow on/before the pay date is settled out of the future schedule.
    let sched = fx
        .edge
        .list_instrument_schedule(Request::new(ListInstrumentScheduleRequest {
            session_token: tok,
            instrument_id: INSTR.to_string(),
            correlation_id: None,
        }))
        .await
        .expect("list schedule")
        .into_inner();
    assert_eq!(
        sched.flows.len(),
        3,
        "one coupon flow settled, three remain"
    );

    // No rates-book leg is booked for a pure income event.
    assert_eq!(fx.rates.len(), 0, "income books no position leg");
}

#[tokio::test]
async fn refdata_gate_denies_confirm_and_apply_for_a_non_refdata_caller() {
    let fx = fixture();
    let payment = CivilDate::new(2034, 9, 15);
    let ca_id = "test-callable:MCAL:20340915";
    fx.edge
        .announce(ca(ca_id, Caev::Mcal, CaTerms::partial(1.0, 101.0), payment))
        .expect("announce");
    let plain = trader_token(&fx.sessions);

    // A plain trader (holding the default bundle, which excludes `refdata`) is denied both
    // the confirm and the apply.
    let confirm = fx
        .edge
        .confirm_corporate_action(Request::new(ConfirmCorporateActionRequest {
            session_token: plain.clone(),
            ca_id: ca_id.to_string(),
            correlation_id: None,
        }))
        .await;
    assert_eq!(confirm.unwrap_err().code(), Code::PermissionDenied);

    let apply = fx
        .edge
        .apply_corporate_action(Request::new(ApplyCorporateActionRequest {
            session_token: plain.clone(),
            ca_id: ca_id.to_string(),
            held_face: HELD_FACE,
            correlation_id: None,
        }))
        .await;
    assert_eq!(apply.unwrap_err().code(), Code::PermissionDenied);

    // But the same trader CAN read the CA inbox (the `view` floor).
    let inbox = fx
        .edge
        .list_corporate_actions(Request::new(ListCorporateActionsRequest {
            session_token: plain,
            isin: None,
            correlation_id: None,
        }))
        .await
        .expect("read inbox")
        .into_inner();
    assert_eq!(inbox.actions.len(), 1);

    // A refdata-capable caller confirms successfully.
    let tok = refdata_token(&fx.sessions);
    fx.edge
        .confirm_corporate_action(Request::new(ConfirmCorporateActionRequest {
            session_token: tok,
            ca_id: ca_id.to_string(),
            correlation_id: None,
        }))
        .await
        .expect("refdata caller confirms");
}
