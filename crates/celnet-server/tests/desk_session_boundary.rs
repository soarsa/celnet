//! **Desk-session boundary suite** (lane J of the integration program;
//! `docs/RISK-HIERARCHY.md` §3 desk narrowing): under the PRODUCTION deny-by-default
//! [`AccessMode::Enforce`] posture, a desk-bound trader session is confined to its
//! **own** desk — proven through the real gRPC [`RiskService`] handler chain
//! (`resolve_caller` → `authorize_caller` → `RiskEdge::effective_principal`), driven
//! by a **server-validated session token**, not an asserted body principal.
//!
//! # The gap this closes
//!
//! Existing coverage proves the narrowing arithmetic but never on the handler edge
//! with a real session:
//!
//! * the unit test `desk_narrowing_equals_explicit_desk_scope_and_is_narrower_than_admin`
//!   (in `services::risk::mod`) mints sessions but calls `aggregate_risk_impl`
//!   **directly**, bypassing the `tonic` trait method's `resolve_caller` /
//!   `authorize_caller` layer;
//! * `tests/entitlements_boundary.rs` drives the gRPC trait methods but only with
//!   **asserted** `EntitlementPrincipal`s and **no** session (`session_token: None`),
//!   so the session-derived desk narrowing never runs.
//!
//! Here the request carries a real `session_token` minted into the SAME
//! [`SessionRegistry`] the edge validates against (the finding-#3 lesson: an edge must
//! validate tokens against the registry that minted them), and the call goes through
//! the published gRPC handler, end to end.
//!
//! # What is proven, under `Enforce`
//!
//! 1. **AggregateRisk** — an EM-desk trader's firm apex reflects only the EM leg, an
//!    admin sees both legs, a G10 trader only the G10 leg; the EM aggregate is strictly
//!    smaller than admin's and (additively) excludes the G10 leg;
//! 2. **DrillRisk** — the EM trader drilling its own desk node sees only its book +
//!    position; drilling the *G10* desk node as the EM trader yields the empty set;
//! 3. **ListPositions** — the EM trader sees exactly its desk's position, the admin all;
//! 4. **Cross-desk attempt** — the EM trader asserting an explicit `Desk = g10` body
//!    principal gets an EMPTY result (the intersection `Desk=em-vol ∧ Desk=g10` admits
//!    no fact), NOT a permission error — narrowing intersects, it does not widen.
//!
//! The KILL-TRIGGER (called out at its assertion) is in (1): delete
//! [`RiskEdge::effective_principal`]'s narrowing and the desk trader sees the whole
//! firm, failing the `em_count == 1` and `em_count < admin_count` asserts.

use std::sync::Arc;

use celnet_entitlements::AccessMode;
use celnet_proto::risk_service_server::RiskService;
use celnet_proto::{
    AggregateRiskRequest, AttributionRecord, BookId as WireBookId, DrillRiskRequest,
    EntitlementPrincipal, EntitlementRule, ListPositionsRequest, NumeraireRate, Owner,
    ReportingNumeraire, RiskDimension, RiskScope, owner,
};
use celnet_server::ReadinessGate;
use celnet_server::clock::Clock;
use celnet_server::config::identity::{Role, default_trader_bundle};
use celnet_server::services::risk::RiskEdge;
use celnet_server::services::risk::store::{BookedPosition, PositionStore};
use celnet_server::services::sessions::{AuthenticatedUser, SessionRegistry};
use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};
use tonic::Request;

// ---------------------------------------------------------------------------
// fixtures: a two-desk live book under the Enforce posture, three real sessions
// ---------------------------------------------------------------------------

/// The wire position id of the EM-VOL leg (booked first, into `EM-VOL-1`). The store
/// round-trips the wire id verbatim, so it stays stable across the read RPCs.
const EM_POSITION_ID: u64 = 1;

fn usd_numeraire() -> ReportingNumeraire {
    ReportingNumeraire {
        numeraire: "USD".to_owned(),
        rates: vec![NumeraireRate {
            ccy: "EUR".to_owned(),
            rate: 1.10,
        }],
    }
}

/// An attribution chain whose **holder** book is `book`, held by `trader` — the live
/// book line a desk's risk rolls up from (mirrors `tests/entitlements_boundary.rs`).
fn attribution(book: &str, trader: &str) -> AttributionRecord {
    AttributionRecord {
        quoted_by: Some(WireBookId {
            book: "AUTO-MM".to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::AutoPricer("celnet-auto-pricer".to_owned())),
            }),
        }),
        held_by: Some(WireBookId {
            book: book.to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::Trader(trader.to_owned())),
            }),
        }),
        won: Some(true),
        lp_count: Some(2),
    }
}

fn booked(id: u64, notional: f64) -> BookedPosition {
    BookedPosition {
        position_id: id,
        pair: CcyPair::new(Ccy::EUR, Ccy::USD),
        option: OptionType::Call,
        notional_base: notional,
        inputs: VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        quoted_delta: DeltaConvention::SpotUnadjusted,
        premium_style: PremiumStyle::DomesticPips,
        surface_version: 1,
    }
}

/// An [`AuthenticatedUser`] of `role`, optionally bound to desk slug `desk` — the
/// session identity the server (never the client) decides the caller is.
fn user(id: &str, role: Role, desk: Option<&str>) -> AuthenticatedUser {
    AuthenticatedUser {
        user_id: id.to_owned(),
        email: format!("{id}@celnet.com"),
        display_name: id.to_owned(),
        role,
        desk_id: desk.map(str::to_owned),
        role_caps: default_trader_bundle(),
        cap_grants: Vec::new(),
        cap_denies: Vec::new(),
    }
}

/// A ready two-desk edge under the production `Enforce` posture, with three real
/// session tokens minted into the edge's shared registry.
struct DeskFixture {
    edge: RiskEdge,
    em_token: String,
    admin_token: String,
    g10_token: String,
    /// Canonical numeric desk ids (the interned slugs the desk bridge populated).
    em_desk: u32,
    g10_desk: u32,
}

/// Two desks (`em-vol` ← `EM-VOL-1`, `g10` ← `G10-1`), one seeded position each, the
/// production deny-by-default access mode, and three sessions: an EM-desk trader, an
/// admin (no desk), and a G10-desk trader. The session registry the tokens are minted
/// into is the SAME one the edge validates against (`with_sessions`).
fn two_desk_enforced_edge() -> DeskFixture {
    let gate = Arc::new(ReadinessGate::new());
    gate.mark_ready();
    let store = Arc::new(PositionStore::new());

    // Two desks, distinct holder books, one live line each (EM 10mm, G10 7mm — distinct
    // magnitudes so the per-leg premiums are separable). Book first, then bridge the
    // desk→book ownership via the boot-time `configure_desk` path (idempotent interning
    // agrees with the lazy book interning the booking just did).
    store
        .book_from_attribution(booked(1, 10_000_000.0), &attribution("EM-VOL-1", "ezhao"))
        .expect("book the EM-VOL leg");
    store
        .book_from_attribution(booked(2, 7_000_000.0), &attribution("G10-1", "gjones"))
        .expect("book the G10 leg");
    store.configure_desk("em-vol", &["EM-VOL-1".to_owned()]);
    store.configure_desk("g10", &["G10-1".to_owned()]);

    // The PRODUCTION posture: deny-by-default. It is also the construction default;
    // we set + assert it explicitly so the test is anchored to the production edge,
    // not to a default that could drift.
    store.set_access_mode(AccessMode::Enforce);
    assert_eq!(
        store.access_mode(),
        AccessMode::Enforce,
        "this suite must run under the production deny-by-default posture"
    );

    // The canonical numeric desk ids (idempotent — same handles `configure_desk` and
    // `effective_principal` resolve the slugs to within this store).
    let em_desk = store.intern("em-vol");
    let g10_desk = store.intern("g10");

    // The edge-wide session registry the tokens live in AND the edge validates against.
    let sessions = Arc::new(SessionRegistry::new(Clock::manual(0)));
    let em_token = sessions
        .issue(user("emtrader", Role::Trader, Some("em-vol")))
        .expect("issue the EM-desk trader session")
        .token;
    let admin_token = sessions
        .issue(user("boss", Role::Admin, None))
        .expect("issue the admin session")
        .token;
    let g10_token = sessions
        .issue(user("g10trader", Role::Trader, Some("g10")))
        .expect("issue the G10-desk trader session")
        .token;

    let edge = RiskEdge::new(store, gate).with_sessions(Arc::clone(&sessions));
    DeskFixture {
        edge,
        em_token,
        admin_token,
        g10_token,
        em_desk,
        g10_desk,
    }
}

fn firm_aggregate_request(
    token: &str,
    principal: Option<EntitlementPrincipal>,
) -> AggregateRiskRequest {
    AggregateRiskRequest {
        dimension: RiskDimension::Firm as i32,
        numeraire: Some(usd_numeraire()),
        principal,
        scope: None,
        vega_pillars: vec![],
        var_spot_shocks: vec![],
        var_alpha: 0.0,
        curvature_risk_weight: 0.0,
        correlation_id: Some(7),
        session_token: Some(token.to_owned()),
    }
}

fn list_request(token: &str, principal: Option<EntitlementPrincipal>) -> ListPositionsRequest {
    ListPositionsRequest {
        scope: None,
        principal,
        correlation_id: Some(7),
        session_token: Some(token.to_owned()),
    }
}

/// A drill of the `desk_value` desk node, broken out by its child books, with the
/// contributing positions — carried by `token`'s session.
fn drill_desk_request(token: &str, desk_value: u64) -> DrillRiskRequest {
    DrillRiskRequest {
        node: Some(RiskScope {
            dimension: RiskDimension::Desk as i32,
            value: desk_value,
        }),
        child_dimension: RiskDimension::Book as i32,
        numeraire: Some(usd_numeraire()),
        principal: None,
        vega_pillars: vec![],
        include_children: true,
        include_positions: true,
        correlation_id: Some(7),
        session_token: Some(token.to_owned()),
    }
}

/// A scoped body principal asserting visibility over exactly one desk subtree.
fn desk_principal(desk_value: u64) -> EntitlementPrincipal {
    EntitlementPrincipal {
        grant_all: false,
        grants: vec![EntitlementRule {
            scopes: vec![RiskScope {
                dimension: RiskDimension::Desk as i32,
                value: desk_value,
            }],
        }],
        denies: vec![],
    }
}

/// Drive the gRPC `AggregateRisk` handler for `token` (the full
/// `resolve_caller` → `authorize_caller` → `effective_principal` chain) and read the
/// firm apex node's `(position_count, premium_numeraire)`.
async fn firm_node(
    edge: &RiskEdge,
    token: &str,
    principal: Option<EntitlementPrincipal>,
) -> (u32, f64) {
    let resp =
        RiskService::aggregate_risk(edge, Request::new(firm_aggregate_request(token, principal)))
            .await
            .expect("a valid session is authorized for a ReadAny RPC under Enforce")
            .into_inner();
    let node = resp
        .nodes
        .first()
        .expect("a non-empty book yields a firm apex node");
    let premium = node
        .additive
        .as_ref()
        .expect("the apex node carries additive measures")
        .premium_numeraire;
    (node.position_count, premium)
}

// ---------------------------------------------------------------------------
// A. AggregateRisk: a desk trader's firm view is exactly its own desk
// ---------------------------------------------------------------------------

#[tokio::test]
async fn aggregate_risk_narrows_a_desk_trader_to_their_own_desk() {
    let fx = two_desk_enforced_edge();

    // All three drive the SAME firm-dimension RPC; only the session token differs.
    let (em_count, em_prem) = firm_node(&fx.edge, &fx.em_token, None).await;
    let (admin_count, admin_prem) = firm_node(&fx.edge, &fx.admin_token, None).await;
    let (g10_count, g10_prem) = firm_node(&fx.edge, &fx.g10_token, None).await;

    // KILL-TRIGGER. The EM trader asserts NO body principal (grant-all default); the
    // session narrowing in `RiskEdge::effective_principal` pins it to Desk=em-vol, so
    // the firm apex reflects ONLY the EM leg. Delete that narrowing and the EM session
    // sees the whole firm (em_count == 2 == admin_count) — failing both this assert and
    // the strict-less-than below.
    assert_eq!(
        em_count, 1,
        "the EM trader's firm apex is exactly its one desk leg (narrowed, not firm-wide)"
    );
    assert!(
        em_count < admin_count,
        "a desk-bound trader ({em_count}) sees strictly fewer positions than admin ({admin_count})"
    );

    // The admin (no desk) is unnarrowed — the whole firm; the G10 trader its one leg.
    assert_eq!(admin_count, 2, "the admin sees both desks' legs");
    assert_eq!(g10_count, 1, "the G10 trader sees only its own desk leg");

    // The EM aggregate EXCLUDES the G10 leg: with long calls all premiums are positive,
    // and the firm total is additively EM + G10 over the two distinct legs — so the EM
    // number is its own leg alone, not the firm sum and not a double count.
    assert!(
        em_prem > 0.0 && g10_prem > 0.0 && admin_prem > 0.0,
        "long calls produce positive premiums"
    );
    let tol = 1e-6 * admin_prem.abs();
    assert!(
        (admin_prem - em_prem - g10_prem).abs() <= tol,
        "firm premium {admin_prem} must equal EM {em_prem} + G10 {g10_prem} (no leakage, no double count)"
    );
    assert!(
        (em_prem - g10_prem).abs() > tol,
        "the two desks carry distinct magnitudes — the EM view is specifically the EM leg, not the G10 one"
    );
}

// ---------------------------------------------------------------------------
// B. DrillRisk: a desk trader is confined to its own node; a foreign node is empty
// ---------------------------------------------------------------------------

#[tokio::test]
async fn drill_risk_confines_a_desk_trader_to_its_own_node() {
    let fx = two_desk_enforced_edge();

    // The EM trader drilling its OWN desk node → exactly its one book + position.
    let own = RiskService::drill_risk(
        &fx.edge,
        Request::new(drill_desk_request(&fx.em_token, u64::from(fx.em_desk))),
    )
    .await
    .expect("the EM trader drills its own desk under Enforce")
    .into_inner();
    assert_eq!(
        own.children.len(),
        1,
        "the EM desk node breaks out into exactly one visible child book"
    );
    assert_eq!(own.positions.len(), 1, "exactly the EM leg position");
    assert_eq!(own.positions[0].position_id, EM_POSITION_ID);
    assert!(
        own.children
            .iter()
            .all(|c| c.dimension == RiskDimension::Book as i32),
        "the child sub-nodes are books"
    );
    // The single visible book is EM-VOL-1 (holder attribution round-trips).
    let book = own.positions[0]
        .attribution
        .as_ref()
        .and_then(|a| a.held_by.as_ref())
        .map(|b| b.book.as_str());
    assert_eq!(book, Some("EM-VOL-1"), "the EM trader sees only its own book");

    // The EM trader drilling the G10 desk node → narrowed to Desk=em-vol, the node
    // scope is Desk=g10, the intersection is empty: no children, no positions — and it
    // is an OK empty result, NOT a permission error (narrowing intersects, never widens
    // nor denies).
    let cross = RiskService::drill_risk(
        &fx.edge,
        Request::new(drill_desk_request(&fx.em_token, u64::from(fx.g10_desk))),
    )
    .await
    .expect("drilling a foreign desk is an empty intersection, never an error")
    .into_inner();
    assert!(
        cross.children.is_empty(),
        "a desk trader sees no foreign-desk child books"
    );
    assert!(
        cross.positions.is_empty(),
        "a desk trader sees no foreign-desk positions"
    );
}

// ---------------------------------------------------------------------------
// C. ListPositions: a desk trader lists only its desk; the admin lists all
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_positions_shows_a_desk_trader_only_its_desk() {
    let fx = two_desk_enforced_edge();

    let em = RiskService::list_positions(&fx.edge, Request::new(list_request(&fx.em_token, None)))
        .await
        .expect("the EM trader lists positions under Enforce")
        .into_inner();
    assert_eq!(
        em.positions.len(),
        1,
        "the EM trader lists exactly its own desk's position"
    );
    assert_eq!(em.positions[0].position_id, EM_POSITION_ID);

    let admin =
        RiskService::list_positions(&fx.edge, Request::new(list_request(&fx.admin_token, None)))
            .await
            .expect("the admin lists positions under Enforce")
            .into_inner();
    assert_eq!(
        admin.positions.len(),
        2,
        "the admin (no desk narrowing) lists every desk's position"
    );
}

// ---------------------------------------------------------------------------
// D. Cross-desk assertion intersects to the empty set, never a permission error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cross_desk_assertion_intersects_to_empty_not_an_error() {
    let fx = two_desk_enforced_edge();

    // The EM trader explicitly asserts a `Desk = g10` body principal. The session
    // narrowing conjoins each grant with Desk=em-vol (`Rule::and`), so the rule becomes
    // `Desk=g10 ∧ Desk=em-vol` — no fact has two desks, so the admitted set is EMPTY.
    // The contract returns an OK empty result (a desk trader simply cannot widen across
    // desks), NOT a `permission_denied` — the `.expect` below proves it is not an error.
    let resp = RiskService::list_positions(
        &fx.edge,
        Request::new(list_request(&fx.em_token, Some(desk_principal(u64::from(fx.g10_desk))))),
    )
    .await
    .expect("a cross-desk assertion is an empty intersection, never a permission error")
    .into_inner();
    assert!(
        resp.positions.is_empty(),
        "Desk=em-vol ∧ Desk=g10 admits no fact — the desk trader cannot widen across desks"
    );
}
