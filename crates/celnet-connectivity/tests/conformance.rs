//! Loopback conformance: the initiator [`run_cert_cycle`] driver runs a full
//! certification cycle against an in-process `celnet-fix` **acceptor**
//! counterparty over a real TCP socket, and the Order-role app-flow
//! (`NewOrderSingle → ExecutionReport`) auto-passes the certification ledger's
//! Logon + order checks.
//!
//! Every frame is a real `celnet-fix` FIX 4.4 message with correct sequence
//! numbers, CompIDs and checksums — there is no mock counterparty, only the real
//! engine in acceptor role. This is the initiator-side analogue of `celnet-fix`'s
//! own `desk_gateway` / `session_loopback` tests.

use celnet_connectivity::{
    AdapterRegistry, CHECK_LOGON, CHECK_ORDER_ER, CertLedger, CheckStatus, VenueEndpoint,
    run_cert_cycle,
};

use celnet_fix::MsgType;
use celnet_fix::framing::FrameCursor;
use celnet_fix::messages::{
    EXEC_FILLED, ExecReportParams, NewOrderParams, build_execution_report, build_new_order_single,
};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig, SessionState};
use celnet_fix::transport::{FrameReader, write_frame};

use tokio::net::{TcpListener, TcpStream};

const SENDING_TIME: &[u8] = b"20260702-12:00:00.000";

/// A minimal acceptor counterparty driven by a real `celnet-fix` `Session`:
/// mirrors the Logon, answers a `NewOrderSingle` with a filled
/// `ExecutionReport`, and mirrors the Logout. The `Session` supplies correct
/// sequence numbers, CompID swapping and framing.
async fn run_counterparty(stream: TcpStream) -> std::io::Result<()> {
    let cfg = SessionConfig {
        sender: b"VENUE".to_vec(),
        target: b"CELNET".to_vec(),
        heart_bt_int: 30,
        role: Role::Acceptor,
    };
    let mut sess = Session::new(cfg, InMemoryStore::new());
    let (rd, mut wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rd);

    while let Some(raw) = reader.next_frame().await? {
        // Capture the order fields before handing the frame to the FSM.
        let (cl_ord_id, symbol, side) = match FrameCursor::parse(&raw) {
            Ok(c) => (
                c.get(11).unwrap_or(b"").to_vec(),
                c.get(55).unwrap_or(b"EURUSD").to_vec(),
                c.get(54).and_then(|s| s.first().copied()).unwrap_or(b'1'),
            ),
            Err(_) => (Vec::new(), b"EURUSD".to_vec(), b'1'),
        };

        let action = match sess.on_inbound(&raw, SENDING_TIME) {
            Ok(a) => a,
            Err(_) => break,
        };
        for out in &action.outbound {
            write_frame(&mut wr, out).await?;
        }

        if let Some(MsgType::NewOrderSingle) = action.deliver {
            let er = sess.send_app(SENDING_TIME, |h, e| {
                build_execution_report(
                    h,
                    &ExecReportParams {
                        order_id: b"OID-1",
                        exec_id: b"EID-1",
                        cl_ord_id: &cl_ord_id,
                        exec_type: EXEC_FILLED,
                        ord_status: EXEC_FILLED,
                        symbol: &symbol,
                        side,
                        last_qty: 1_000_000.0,
                        last_px: 0.010_5,
                        multileg_type: None,
                        text: None,
                    },
                    e,
                )
            });
            write_frame(&mut wr, &er).await?;
        }

        if sess.state() == SessionState::Disconnected {
            break;
        }
    }
    Ok(())
}

#[tokio::test]
async fn order_adapter_cert_cycle_over_loopback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let counterparty = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        run_counterparty(stream).await
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let endpoint = VenueEndpoint {
        sender_comp_id: b"CELNET".to_vec(),
        target_comp_id: b"VENUE".to_vec(),
        heartbeat_secs: 30,
        sending_time: SENDING_TIME.to_vec(),
    };

    // Seed the ledger from a real seeded ORDER-role adapter.
    let reg = AdapterRegistry::new();
    let spec = reg
        .find("rabofx_esp_order")
        .expect("seeded order adapter present");
    assert_eq!(spec.role.as_str(), "ORDER");
    let mut ledger = CertLedger::seed(spec);

    let report = run_cert_cycle(&endpoint, stream, &mut ledger, "D", |h, e| {
        build_new_order_single(
            h,
            &NewOrderParams {
                cl_ord_id: b"ORD-1",
                quote_id: b"",
                symbol: b"EURUSD",
                side: b'1',
                qty: 1_000_000.0,
                transact_time: SENDING_TIME,
            },
            e,
        )
    })
    .await
    .expect("cert cycle completes");

    counterparty.await.unwrap().expect("counterparty ran clean");

    let status = |k: &str| {
        ledger
            .checks()
            .iter()
            .find(|c| c.check_key == k)
            .map(|c| c.status)
    };

    assert_eq!(
        status(CHECK_LOGON),
        Some(CheckStatus::Passed),
        "Logon should auto-pass; flipped={:?}",
        report.flipped
    );
    assert_eq!(
        status(CHECK_ORDER_ER),
        Some(CheckStatus::Passed),
        "NewOrderSingle→ExecutionReport should auto-pass; flipped={:?}",
        report.flipped
    );
    assert!(
        report.frames_observed >= 2,
        "should observe at least the Logon ack + the ExecutionReport, got {}",
        report.frames_observed
    );
    // The manual resend/soak gate is still pending, so PROD promotion is blocked.
    assert!(
        !ledger.ready_for_prod(true),
        "manual required checks still pending"
    );
}
