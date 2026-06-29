//! The Celnet FIX↔gRPC quoting gateway binary.
//!
//! Runs a FIX 4.4 acceptor that bridges an external counterparty's rates quoting
//! flow onto the Celnet dealer desk: it authenticates to the running edge via
//! `AuthService.Login`, then maps every accepted FIX session's
//! `QuoteRequest`/lift lifecycle onto `RfqDeskService`
//! (`SubmitDeskRequest`/`AcceptDeskQuote`) — see [`celnet_fix::gateway`].
//!
//! Live counterparty-venue connectivity (the inbound FIX socket from a real
//! external bank) is the deploy-tier cutover; against a local edge + the bundled
//! `celnet fix` test client (or any FIX 4.4 initiator) the whole path runs today.
//!
//! ```text
//! celnet-fix-gateway \
//!   --listen 127.0.0.1:9880 \
//!   --edge http://127.0.0.1:50551 \
//!   --email admin@celnet.com --password password \
//!   --desk RATES --sender-comp-id CELNET-FIX --target-comp-id CPARTY \
//!   --curve-ref 2026-06-29 --pillar 1:0.0432 --pillar 5:0.0405 --pillar 10:0.0400
//! ```

use std::process::ExitCode;
use std::sync::Arc;

use celnet_fix::backend::{GrpcDeskBackend, usd_sofr_curve};
use celnet_fix::gateway::{DeskGateway, GatewayConfig};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};
use celnet_proto::CurveSet;
use clap::Parser;
use tokio::net::TcpListener;

/// CLI configuration for the gateway.
#[derive(Debug, Parser)]
#[command(
    name = "celnet-fix-gateway",
    about = "FIX 4.4 acceptor bridging counterparty rates quoting onto the Celnet dealer desk."
)]
struct Args {
    /// The `host:port` the FIX acceptor binds (the counterparty connects here).
    #[arg(long, default_value = "127.0.0.1:9880")]
    listen: String,
    /// The gRPC endpoint of the Celnet edge.
    #[arg(long, default_value = "http://127.0.0.1:50551")]
    edge: String,
    /// The login email for `AuthService.Login` (the desk-entitled gateway user).
    #[arg(long, default_value = "admin@celnet.com")]
    email: String,
    /// The login password.
    #[arg(long, default_value = "password")]
    password: String,
    /// The desk inbound requests route to (entitlement + notification scope).
    #[arg(long, default_value = "RATES")]
    desk: String,
    /// Our `SenderCompID` (the gateway identity the counterparty targets).
    #[arg(long, default_value = "CELNET-FIX")]
    sender_comp_id: String,
    /// The counterparty `TargetCompID` (the peer identity + counterparty label).
    #[arg(long, default_value = "CPARTY")]
    target_comp_id: String,
    /// The session heartbeat interval in seconds.
    #[arg(long, default_value_t = 30)]
    heartbeat: u32,
    /// The desk curve reference (spot-anchor) date, `YYYY-MM-DD`.
    #[arg(long, default_value = "2026-06-29")]
    curve_ref: String,
    /// A calibrating curve pillar `TENOR_YEARS:PAR_RATE` (repeatable). Defaults to
    /// a representative USD-SOFR curve when none are given.
    #[arg(long = "pillar", value_parser = parse_pillar)]
    pillars: Vec<(u32, f64)>,
}

/// Parse a `TENOR_YEARS:PAR_RATE` pillar (e.g. `5:0.0405`).
fn parse_pillar(s: &str) -> Result<(u32, f64), String> {
    let (t, r) = s
        .split_once(':')
        .ok_or_else(|| format!("expected TENOR:RATE, got `{s}`"))?;
    let tenor: u32 = t
        .parse()
        .map_err(|_| format!("invalid tenor years in `{s}`"))?;
    let rate: f64 = r
        .parse()
        .map_err(|_| format!("invalid par rate in `{s}`"))?;
    if tenor < 1 {
        return Err(format!("tenor must be >= 1 year in `{s}`"));
    }
    if !(rate.is_finite()) {
        return Err(format!("par rate must be finite in `{s}`"));
    }
    Ok((tenor, rate))
}

/// Parse a `YYYY-MM-DD` reference date into `(year, month, day)`.
fn parse_ref_date(s: &str) -> Result<(i32, u32, u32), String> {
    let mut it = s.split('-');
    let y = it.next().and_then(|v| v.parse().ok());
    let m = it.next().and_then(|v| v.parse().ok());
    let d = it.next().and_then(|v| v.parse().ok());
    match (y, m, d, it.next()) {
        (Some(y), Some(m), Some(d), None) => Ok((y, m, d)),
        _ => Err(format!("expected YYYY-MM-DD, got `{s}`")),
    }
}

/// A representative USD-SOFR curve used when no `--pillar` flags are supplied.
const DEFAULT_PILLARS: &[(u32, f64)] = &[
    (1, 0.0432),
    (2, 0.0418),
    (3, 0.0410),
    (5, 0.0405),
    (7, 0.0402),
    (10, 0.0400),
];

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    let reference = match parse_ref_date(&args.curve_ref) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: --curve-ref: {e}");
            return ExitCode::FAILURE;
        }
    };
    let pillars: Vec<(u32, f64)> = if args.pillars.is_empty() {
        DEFAULT_PILLARS.to_vec()
    } else {
        args.pillars.clone()
    };
    let curve: CurveSet = usd_sofr_curve(reference, &pillars);

    // Authenticate once; the bearer token + channel are shared across every
    // accepted FIX session (a desk-entitled gateway user).
    let backend = match GrpcDeskBackend::login(
        args.edge.clone(),
        args.email.clone(),
        args.password.clone(),
        args.desk.clone(),
        curve,
    )
    .await
    {
        Ok(b) => Arc::new(b),
        Err(e) => {
            eprintln!(
                "error: could not authenticate to the edge at {}: {e}",
                args.edge
            );
            return ExitCode::FAILURE;
        }
    };

    let listener = match TcpListener::bind(&args.listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: could not bind FIX acceptor on {}: {e}", args.listen);
            return ExitCode::FAILURE;
        }
    };
    println!(
        "celnet-fix-gateway: FIX 4.4 acceptor on {} → desk `{}` on edge {} (SenderCompID={}, expecting TargetCompID={})",
        args.listen, args.desk, args.edge, args.sender_comp_id, args.target_comp_id
    );

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("warning: accept failed: {e}");
                continue;
            }
        };
        let backend = Arc::clone(&backend);
        let cfg = SessionConfig {
            sender: args.sender_comp_id.clone().into_bytes(),
            target: args.target_comp_id.clone().into_bytes(),
            heart_bt_int: args.heartbeat,
            role: Role::Acceptor,
        };
        let gw_cfg = GatewayConfig {
            desk: args.desk.clone(),
            counterparty: args.target_comp_id.clone(),
        };
        tokio::spawn(async move {
            let session = Session::new(cfg, InMemoryStore::new());
            let mut gateway = DeskGateway::new(session, backend, gw_cfg);
            if let Err(e) = gateway.run(stream).await {
                eprintln!("warning: session with {peer} ended: {e}");
            }
        });
    }
}
