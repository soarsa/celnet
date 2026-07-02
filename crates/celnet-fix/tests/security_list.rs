//! Security-list download: a `SecurityListRequest(x)` → `SecurityList(y)` cycle
//! driven by the REAL acceptor against a REAL initiator session over a loopback
//! TCP socket — no fakes. The venue answers with the exact tradable-securities
//! universe its [`QuoteSource`] projects, so a client can enumerate what the
//! desk can quote before ever sending an RFQ.
//!
//! The whole path is real framing: the request is encoded through the session's
//! `send_app` and the response is parsed with [`FrameCursor`], not string
//! matched.

use std::time::Duration;

use celnet_fix::acceptor::{Acceptor, QuoteSource};
use celnet_fix::dialect_fx::{
    self, MarketSnapshot, SecurityDef, SecurityListRequestParams, VanillaPricer,
};
use celnet_fix::framing::FrameCursor;
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig, SessionState};
use celnet_fix::transport::{FrameReader, write_frame};
use celnet_types::Tenor;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

const T: &[u8] = b"20260530-12:00:00.000";
const DEADLINE: Duration = Duration::from_secs(5);
const REQ_ID: &[u8] = b"SLR-1";

fn acc_cfg() -> SessionConfig {
    SessionConfig {
        sender: b"VENUE".to_vec(),
        target: b"TAKER".to_vec(),
        heart_bt_int: 30,
        role: Role::Acceptor,
    }
}
fn init_cfg() -> SessionConfig {
    SessionConfig {
        sender: b"TAKER".to_vec(),
        target: b"VENUE".to_vec(),
        heart_bt_int: 30,
        role: Role::Initiator,
    }
}

/// A [`QuoteSource`] whose only job here is to project a KNOWN tradable-securities
/// universe. It carries real (never used) pricing inputs so it satisfies the
/// whole trait, but the test only exercises `securities()`.
struct UniverseSource {
    universe: Vec<SecurityDef>,
}

impl QuoteSource for UniverseSource {
    fn snapshot_for(&self, _frame: &FrameCursor<'_>) -> (MarketSnapshot, Tenor) {
        (
            MarketSnapshot {
                spot: 1.10,
                vol: 0.10,
                t: 0.25,
                r_dom: 0.03,
                r_for: 0.01,
            },
            Tenor::Months(3),
        )
    }
    fn half_spread(&self) -> f64 {
        0.0
    }
    fn validity_ticks(&self) -> u64 {
        1_000
    }
    fn pricer(&self) -> VanillaPricer {
        celnet_vanilla::price
    }
    fn securities(&self) -> Vec<SecurityDef> {
        self.universe.clone()
    }
}

#[tokio::test]
async fn security_list_round_trips_the_universe() {
    let body = async {
        // The venue's known tradable universe: two deliverable FX vanilla pairs.
        let universe = vec![
            SecurityDef::new(b"EURUSD", dialect_fx::SEC_TYPE_FXVO, b"USD"),
            SecurityDef::new(b"GBPUSD", dialect_fx::SEC_TYPE_FXVO, b"USD"),
        ];
        let expected_count = universe.len();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // --- Acceptor task: real Acceptor answering the SecurityListRequest. ---
        let acceptor = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let session = Session::new(acc_cfg(), InMemoryStore::new());
            let mut acc = Acceptor::new(session, UniverseSource { universe });
            acc.run(stream, T.to_vec()).await.ok();
        });

        // --- Initiator side: real client session over the socket. ---
        let stream = TcpStream::connect(addr).await.unwrap();
        let (rd, mut wr) = tokio::io::split(stream);
        let mut reader = FrameReader::new(rd);
        let mut sess = Session::new(init_cfg(), InMemoryStore::new());

        // 1. Logon and consume the acceptor's logon mirror.
        let logon = sess.start_logon(T, false);
        write_frame(&mut wr, &logon).await.unwrap();
        let mirror = reader.next_frame().await.unwrap().unwrap();
        sess.on_inbound(&mirror, T).unwrap();
        assert_eq!(sess.state(), SessionState::Active);

        // 2. Send a SecurityListRequest(x) for the whole (currency) universe.
        let req = sess.send_app(T, |h, e| {
            let p = SecurityListRequestParams {
                security_req_id: REQ_ID,
                product: Some(dialect_fx::PRODUCT_CURRENCY),
                currency: None,
            };
            dialect_fx::build_security_list_request(h, &p, e)
        });
        write_frame(&mut wr, &req).await.unwrap();

        // 3. Read and parse the returned SecurityList(y) via the real framer.
        let resp = reader.next_frame().await.unwrap().unwrap();
        let frame = FrameCursor::parse(&resp).unwrap();

        // MsgType is y.
        assert_eq!(frame.msg_type(), b"y");
        // SecurityReqID(320) echoes the request.
        assert_eq!(frame.get(320), Some(REQ_ID));
        // SecurityRequestResult(560) = 0 (valid, list follows).
        assert_eq!(frame.get(560), Some(&b"0"[..]));
        // TotNoRelatedSym(393) and NoRelatedSym(146) equal the universe count.
        assert_eq!(frame.get(393), Some(expected_count.to_string().as_bytes()));
        assert_eq!(frame.get(146), Some(expected_count.to_string().as_bytes()));
        // LastFragment(893) = Y — the whole universe fits one fragment.
        assert_eq!(frame.get(893), Some(&b"Y"[..]));

        // Every Symbol(55) in the universe is present in the group.
        let symbols: Vec<&[u8]> = frame
            .fields()
            .filter(|f| f.tag == 55)
            .map(|f| f.value)
            .collect();
        assert_eq!(symbols.len(), expected_count);
        assert!(symbols.iter().any(|s| *s == b"EURUSD"));
        assert!(symbols.iter().any(|s| *s == b"GBPUSD"));

        // Close the client side; let the acceptor task drain and finish.
        drop(wr);
        drop(reader);
        drop(sess);
        timeout(DEADLINE, acceptor).await.unwrap().unwrap();
    };
    timeout(DEADLINE, body).await.expect("test timed out");
}
