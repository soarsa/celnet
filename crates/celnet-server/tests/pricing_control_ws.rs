//! End-to-end firm-wide **pricing kill-switch** WS test (server-only control plane).
//!
//! Proves the control plane the trader GUI drives:
//!
//! 1. On WS connect, every client is handed the CURRENT `pricing_control` state
//!    (both-enabled on a fresh edge) without subscribing to anything.
//! 2. `SetPricingControl` over the WS mirror, carrying an admin session token, applies
//!    the halt, replies the applied state, AND fans a `pricing_control` push frame out
//!    to the connected client with the bumped version.
//! 3. An unprivileged caller (no `ManageLiquidity·FixedIncome`) is refused
//!    `permission_denied`.
//!
//! Every await is hard wall-clock bounded, so a regression surfaces fast.

mod common;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use celnet_proto::auth_service_client::AuthServiceClient;
use celnet_proto::{CreateUserRequest, LoginRequest, UserRole};

use common::{STEP_DEADLINE, login_seed_admin, start_ready_edge};

/// Pull the next JSON text frame within the step deadline (skipping ping/pong).
async fn next_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    loop {
        let msg = tokio::time::timeout(STEP_DEADLINE, ws.next())
            .await
            .expect("a WS frame arrives before the deadline")
            .expect("the socket stays open")
            .expect("the frame is well-formed");
        match msg {
            WsMessage::Text(t) => return serde_json::from_str(&t).expect("frame is valid JSON"),
            WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
            other => panic!("expected a text frame, got: {other:?}"),
        }
    }
}

#[tokio::test]
async fn ws_pricing_control_snapshot_then_broadcast_on_change() {
    let (edge, addr, _tmp) = start_ready_edge().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{}", edge.ws_addr());
    let token = login_seed_admin(&base).await;

    let (mut ws, _resp) = tokio::time::timeout(STEP_DEADLINE, connect_async(ws_url))
        .await
        .expect("WS connects in time")
        .expect("WS mirror accepts the connection");

    // (1) On connect, the client is handed the current control state — both-enabled on
    // a fresh edge (the persisted default), unsolicited.
    let initial = loop {
        let v = next_json(&mut ws).await;
        if v.get("type").and_then(Value::as_str) == Some("pricing_control") {
            break v;
        }
    };
    assert_eq!(
        initial.get("outbound_enabled"),
        Some(&Value::Bool(true)),
        "connect snapshot: outbound enabled by default"
    );
    assert_eq!(initial.get("inbound_enabled"), Some(&Value::Bool(true)));
    let initial_version = initial
        .get("version")
        .and_then(Value::as_u64)
        .expect("version present");
    assert!(initial_version >= 1);

    // (2) Halt outbound pricing ("Stop all pricing") as the authorized admin.
    let set = json!({
        "type": "set_pricing_control",
        "session_token": token,
        "outbound_enabled": false,
        "inbound_enabled": true,
        "correlation_id": "ks-1",
    });
    tokio::time::timeout(STEP_DEADLINE, ws.send(WsMessage::Text(set.to_string())))
        .await
        .expect("set sends in time")
        .expect("set sends");

    // Both the direct reply and the fan-out push carry the applied state; find a
    // `pricing_control` push frame with the halt applied and a bumped version.
    let pushed = loop {
        let v = next_json(&mut ws).await;
        match v.get("type").and_then(Value::as_str) {
            Some("error") => panic!("set_pricing_control was refused for the admin: {v}"),
            Some("pricing_control") => break v,
            // The direct RPC reply arrives too; assert it echoes the applied state.
            Some("set_pricing_control_response") => {
                assert_eq!(v.get("outbound_enabled"), Some(&Value::Bool(false)));
                assert_eq!(v.get("inbound_enabled"), Some(&Value::Bool(true)));
                assert_eq!(v.get("correlation_id"), Some(&Value::String("ks-1".into())));
            }
            _ => {}
        }
    };
    assert_eq!(
        pushed.get("outbound_enabled"),
        Some(&Value::Bool(false)),
        "the halt fans out: outbound now disabled"
    );
    assert_eq!(pushed.get("inbound_enabled"), Some(&Value::Bool(true)));
    let pushed_version = pushed
        .get("version")
        .and_then(Value::as_u64)
        .expect("version present");
    assert!(
        pushed_version > initial_version,
        "the version bumps on change ({pushed_version} > {initial_version})"
    );
}

#[tokio::test]
async fn ws_set_pricing_control_denies_unprivileged_caller() {
    let (edge, addr, _tmp) = start_ready_edge().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{}", edge.ws_addr());
    let admin = login_seed_admin(&base).await;

    // Mint a plain trader with NO liquidity-management capability, then log in as them.
    let mut auth = AuthServiceClient::connect(base.clone())
        .await
        .expect("auth client connects");
    auth.create_user(CreateUserRequest {
        session_token: admin,
        email: "trader@celnet.test".to_owned(),
        display_name: "Plain Trader".to_owned(),
        role: UserRole::Trader as i32,
        desk_ids: vec![],
        password: "password-123".to_owned(),
        correlation_id: None,
        all_desks: true,
    })
    .await
    .expect("create_user succeeds");
    let trader_token = auth
        .login(LoginRequest {
            email: "trader@celnet.test".to_owned(),
            password: "password-123".to_owned(),
            correlation_id: None,
        })
        .await
        .expect("trader login succeeds")
        .into_inner()
        .session_token;

    let (mut ws, _resp) = tokio::time::timeout(STEP_DEADLINE, connect_async(ws_url))
        .await
        .expect("WS connects in time")
        .expect("WS mirror accepts the connection");
    // Drain the unsolicited connect-time snapshot.
    let _ = loop {
        let v = next_json(&mut ws).await;
        if v.get("type").and_then(Value::as_str) == Some("pricing_control") {
            break v;
        }
    };

    let set = json!({
        "type": "set_pricing_control",
        "session_token": trader_token,
        "outbound_enabled": false,
        "inbound_enabled": false,
        "correlation_id": "ks-deny",
    });
    tokio::time::timeout(STEP_DEADLINE, ws.send(WsMessage::Text(set.to_string())))
        .await
        .expect("set sends in time")
        .expect("set sends");

    // The unprivileged caller is refused: an `error` frame, and NO further
    // `pricing_control` change frame follows (the runtime control never flipped).
    let mut saw_error = false;
    for _ in 0..4 {
        let v = next_json(&mut ws).await;
        match v.get("type").and_then(Value::as_str) {
            Some("error") => {
                saw_error = true;
                break;
            }
            Some("pricing_control") => {
                panic!("an unprivileged caller must not flip the control: {v}")
            }
            _ => {}
        }
    }
    assert!(saw_error, "the unprivileged set_pricing_control is refused");
}
