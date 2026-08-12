//! **Simulator service-identity suite** — proves that a simulator can do its job with
//! an EMPTY effective capability set, so it never needs to authenticate as an admin.
//!
//! # What this exists to protect
//!
//! The LP and FIX simulators used to log in as `admin@celnet.com`, with the password on
//! the command line. Removing that rests on one load-bearing claim:
//!
//! > the only authenticated call each simulator makes is gated on **authentication
//! > alone**, not on any capability — so a zero-capability service account suffices.
//!
//! That claim is not obvious from the call site and would break silently if either RPC
//! were later re-gated behind `require_capability` / `require_admin`: the simulators
//! would start failing with `permission_denied` only in deployment. This suite pins the
//! claim in CI, through the real `AuthService` handler chain with a server-minted
//! session token.
//!
//! # What is proven
//!
//! 1. **Lock-down resolves empty** — a `Role::Trader` service account carrying a deny
//!    for every capability the trader bundle confers resolves to an effective set that
//!    is exactly EMPTY (deny wins over the role bundle).
//! 2. **It can still authenticate** — an empty capability set does not prevent login.
//! 3. **`ListAggregatedBooks` succeeds** for it — the one call `lp-sim` makes.
//! 4. **`ListInstruments` succeeds** for it — the one call `fix_rfq_client --asset esp`
//!    makes.
//! 5. **Admin-gated RPCs are refused** — `ListUsers` / `CreateUser` come back
//!    `permission_denied`, proving the account genuinely holds no administrative
//!    authority rather than merely being labelled a service account.
//!
//! The KILL-TRIGGER: re-gate `list_aggregated_books` or `list_instruments` on any
//! capability and assertions (3)/(4) fail — which is the intended alarm, because that
//! change would silently break every deployed simulator.
//!
//! See `docs/SIMULATOR-SERVICE-IDENTITIES.md` for the full rationale.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use celnet_proto::auth_service_server::AuthService;
use celnet_proto::{
    CapabilityDesc, CreateUserRequest, GetUserCapabilitiesRequest, ListAggregatedBooksRequest,
    ListInstrumentsRequest, ListUsersRequest, LoginRequest, SetUserCapabilitiesRequest, UserRole,
};
use celnet_server::ReadinessGate;
use celnet_server::clock::Clock;
use celnet_server::config::identity::{IdentityStore, default_trader_bundle};
use celnet_server::services::auth::AuthEdge;
use celnet_server::services::sessions::SessionRegistry;
use tonic::{Code, Request};

/// The seeded administrator the store ensures on first boot (the fixture's admin).
const SEED_ADMIN_EMAIL: &str = "admin@celnet.com";
const SEED_ADMIN_PASSWORD: &str = "password";

/// The service identity under test — the same address `lp-sim` defaults to.
const SERVICE_EMAIL: &str = "lp-sim@svc.celnet.local";
/// A generated-strength stand-in; the real one is minted per environment and never
/// committed (`deploy/provision-sim-identities.mjs`).
const SERVICE_PASSWORD: &str = "test-only-service-secret-not-a-real-credential";

/// Build an `AuthEdge` over a throwaway identity file seeded with the admin account.
fn edge(tag: &str) -> (AuthEdge, PathBuf) {
    let dir = std::env::temp_dir().join("celnet-sim-identity");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("identity-{}-{tag}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut store = IdentityStore::default();
    store.ensure_seed_admin().unwrap();
    store.save(&path).unwrap();
    let identity = Arc::new(Mutex::new(store));
    let clock = Clock::system();
    let sessions = Arc::new(SessionRegistry::new(clock.clone()));
    let gate = Arc::new(ReadinessGate::new());
    gate.mark_ready();
    (
        AuthEdge::new(identity, path.clone(), sessions, gate, clock),
        path,
    )
}

/// Log in and return the session token.
async fn login(edge: &AuthEdge, email: &str, password: &str) -> Result<String, tonic::Status> {
    Ok(edge
        .login(Request::new(LoginRequest {
            email: email.into(),
            password: password.into(),
            correlation_id: None,
        }))
        .await?
        .into_inner()
        .session_token)
}

/// A stable `action|asset` key so capability sets compare independent of ordering.
fn key(c: &CapabilityDesc) -> String {
    format!("{}|{}", c.action, c.asset)
}

/// The full end-to-end service-identity contract.
#[tokio::test]
async fn zero_capability_service_account_can_do_exactly_what_a_simulator_needs() {
    let (edge, path) = edge("zero-cap");

    // --- The admin provisions the service account ---------------------------
    let admin = login(&edge, SEED_ADMIN_EMAIL, SEED_ADMIN_PASSWORD)
        .await
        .expect("seeded admin logs in");

    let created = edge
        .create_user(Request::new(CreateUserRequest {
            session_token: admin.clone(),
            email: SERVICE_EMAIL.into(),
            display_name: "LP Simulator (service)".into(),
            // Role::Admin is grant-all and NEVER narrowable, so a service account
            // must be a trader whose authority is then denied away.
            role: UserRole::Trader as i32,
            // Deskless: no desk's inbound flow is visible to it.
            desk_ids: Vec::new(),
            all_desks: false,
            password: SERVICE_PASSWORD.into(),
            correlation_id: None,
        }))
        .await
        .expect("service account is created")
        .into_inner();
    let service_id = created.user.expect("created user is returned").id;

    // Before lock-down it inherits the whole trader bundle — the authority we are
    // about to remove. Asserting this first makes the later "empty" meaningful:
    // without it, an empty set could just mean the bundle was empty all along.
    let before = edge
        .get_user_capabilities(Request::new(GetUserCapabilitiesRequest {
            session_token: admin.clone(),
            id: service_id.clone(),
            correlation_id: None,
        }))
        .await
        .expect("admin reads the overlay")
        .into_inner();
    assert!(
        !before.effective.is_empty(),
        "a fresh trader must inherit the role bundle, else this test proves nothing"
    );

    // --- Lock it down: deny every capability the role bundle confers ---------
    let denies: Vec<CapabilityDesc> = default_trader_bundle()
        .into_iter()
        .map(|c| CapabilityDesc {
            action: c.action.label().to_string(),
            asset: c.asset.label().to_string(),
        })
        .collect();
    assert!(!denies.is_empty(), "the trader bundle must be non-empty");

    let after = edge
        .set_user_capabilities(Request::new(SetUserCapabilitiesRequest {
            session_token: admin.clone(),
            id: service_id.clone(),
            grants: Vec::new(),
            denies: denies.clone(),
            correlation_id: None,
        }))
        .await
        .expect("admin replaces the overlay")
        .into_inner();

    // (1) The lock-down resolves EMPTY — deny wins over the role bundle.
    assert!(
        after.effective.is_empty(),
        "effective set must be empty after denying the bundle, got: {:?}",
        after.effective.iter().map(key).collect::<Vec<_>>()
    );

    // (2) An empty capability set does not prevent authentication.
    let service = login(&edge, SERVICE_EMAIL, SERVICE_PASSWORD)
        .await
        .expect("a zero-capability service account can still log in");

    // (3) The one call `lp-sim` makes: ListAggregatedBooks.
    edge.list_aggregated_books(Request::new(ListAggregatedBooksRequest {
        session_token: service.clone(),
        correlation_id: None,
    }))
    .await
    .expect(
        "lp-sim's ListAggregatedBooks must succeed with NO capability — if this fails, \
         the RPC was re-gated and every deployed LP simulator is broken",
    );

    // (4) The one call `fix_rfq_client --asset esp` makes: ListInstruments.
    edge.list_instruments(Request::new(ListInstrumentsRequest {
        session_token: service.clone(),
        correlation_id: None,
    }))
    .await
    .expect(
        "the FIX sim's ListInstruments must succeed with NO capability — if this fails, \
         the RPC was re-gated and the deployed ESP leg is broken",
    );

    // (5) It genuinely holds no administrative authority.
    let listed = edge
        .list_users(Request::new(ListUsersRequest {
            session_token: service.clone(),
            correlation_id: None,
        }))
        .await;
    assert_eq!(
        listed.err().map(|s| s.code()),
        Some(Code::PermissionDenied),
        "the service account must NOT be able to enumerate users"
    );

    let escalation = edge
        .create_user(Request::new(CreateUserRequest {
            session_token: service.clone(),
            email: "attacker@example.test".into(),
            display_name: "Attacker".into(),
            role: UserRole::Admin as i32,
            desk_ids: Vec::new(),
            all_desks: false,
            password: "irrelevant-because-this-must-fail".into(),
            correlation_id: None,
        }))
        .await;
    assert_eq!(
        escalation.err().map(|s| s.code()),
        Some(Code::PermissionDenied),
        "the service account must NOT be able to mint an admin (privilege escalation)"
    );

    let _ = std::fs::remove_file(&path);
}

/// The lock-down must be **idempotent**: re-applying the same deny set leaves the
/// effective set empty and the account still able to authenticate. This mirrors what
/// `deploy/provision-sim-identities.mjs` does on every re-run, and pins the property
/// that a second provisioning pass cannot accidentally widen the account back open.
#[tokio::test]
async fn re_applying_the_lockdown_is_idempotent() {
    let (edge, path) = edge("idempotent");
    let admin = login(&edge, SEED_ADMIN_EMAIL, SEED_ADMIN_PASSWORD)
        .await
        .unwrap();

    let created = edge
        .create_user(Request::new(CreateUserRequest {
            session_token: admin.clone(),
            email: SERVICE_EMAIL.into(),
            display_name: "LP Simulator (service)".into(),
            role: UserRole::Trader as i32,
            desk_ids: Vec::new(),
            all_desks: false,
            password: SERVICE_PASSWORD.into(),
            correlation_id: None,
        }))
        .await
        .unwrap()
        .into_inner();
    let id = created.user.unwrap().id;

    let denies: Vec<CapabilityDesc> = default_trader_bundle()
        .into_iter()
        .map(|c| CapabilityDesc {
            action: c.action.label().to_string(),
            asset: c.asset.label().to_string(),
        })
        .collect();

    let mut seen: Option<Vec<String>> = None;
    for pass in 0..3 {
        let resp = edge
            .set_user_capabilities(Request::new(SetUserCapabilitiesRequest {
                session_token: admin.clone(),
                id: id.clone(),
                grants: Vec::new(),
                denies: denies.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(
            resp.effective.is_empty(),
            "pass {pass}: effective set must stay empty"
        );

        let mut applied: Vec<String> = resp.denies.iter().map(key).collect();
        applied.sort();
        match &seen {
            None => seen = Some(applied),
            Some(first) => assert_eq!(
                first, &applied,
                "pass {pass}: the deny set must converge, not drift"
            ),
        }

        // Still usable after every pass — the whole point of the account.
        let token = login(&edge, SERVICE_EMAIL, SERVICE_PASSWORD)
            .await
            .unwrap_or_else(|e| panic!("pass {pass}: service account must still log in: {e}"));
        edge.list_aggregated_books(Request::new(ListAggregatedBooksRequest {
            session_token: token,
            correlation_id: None,
        }))
        .await
        .unwrap_or_else(|e| panic!("pass {pass}: ListAggregatedBooks must still succeed: {e}"));
    }

    let _ = std::fs::remove_file(&path);
}
