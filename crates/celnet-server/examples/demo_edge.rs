//! A runnable, seeded Celnet edge for end-to-end add-in / GUI verification.
//!
//! This example starts the **real** [`Edge`] — the four `celnet-proto` gRPC
//! services **and** the WebSocket JSON mirror of that same single contract — over
//! the engine's calibrated EURUSD fixture, exactly as `crates/.../tests/ws_mirror.rs`
//! does, but bound to a **fixed** localhost port and left running until killed so an
//! out-of-process client (the Excel add-in's `excel/e2e/verify.mjs` harness, the
//! React GUI, or a raw WS client) can drive the live mirror.
//!
//! It additionally **pre-marks the [`SurfaceBook`]** with one calibrated EURUSD 1Y
//! smile under a fresh `surface_version`, so a client can pin that version on a
//! price *immediately* (without first issuing its own `MarkSurface`) and reproduce
//! the marked surface to the bit — the determinism / reproducibility guarantee of
//! `docs/EXCEL-INTEGRATION.md` §5. The pre-marked version is printed alongside the
//! WS address.
//!
//! Ports (env-overridable so the same example serves any local layout):
//!   * `CELNET_WS_ADDR`   — WebSocket-mirror bind address (default `127.0.0.1:8081`).
//!   * `CELNET_GRPC_ADDR` — gRPC bind address           (default `127.0.0.1:50551`).
//!   * `CELNET_FIX_ADDR`  — **optional** live FIX 4.4 acceptor bind address. Absent ⇒
//!     no FIX listener is started (the edge is byte-identical to today); present ⇒ a
//!     real FIX acceptor binds there and serves RFQ→Quote→lift→ExecutionReport over the
//!     same pricing + click-to-trade token path as gRPC/WS. The bound address is
//!     printed alongside the WS address when set.
//!   * `CELNET_DEMO_LPS`  — how many **deterministic synthetic** demo/test dealers
//!     join the native maker on the multi-dealer RFQ panel (default **3** for this
//!     demo edge, so the live e2e suites exercise the ranked panel; `0` ⇒ the
//!     byte-identical single-dealer edge). Honest boundary: live LP connectivity is
//!     ENV — the in-repo panel is labeled synthetic dealers quoting around the same
//!     edge mid, never faked external fills.
//!
//! The WS mirror binds on its own ephemeral port *inside* [`Edge::start`]; to expose
//! it on a fixed port we bind the WS listener address from `CELNET_WS_ADDR`. The
//! edge prints the resolved `ws://HOST:PORT` so a harness can read it from stderr
//! even when a `:0` ephemeral port is requested.
//!
//! Run (always under a wrapping timeout; kill the PID when done):
//! ```text
//! cargo run -p celnet-server --example demo_edge
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use celnet_conventions::ConventionRecord;
use celnet_engine::testing::make_state;
use celnet_server::{Clock, CoreLink, Edge, LpPanelConfig, SpreadModel};
use celnet_surface::{
    MarketContext as SurfaceContext, MarketQuotes, SmileModel, build_model_smile,
};
use celnet_types::{
    AtmConvention, Carry, CcyPair, Cut, DayCount, DeltaConvention, PremiumStyle, Settlement, Tenor,
};

/// Parse a socket address from an env var, falling back to `default`.
fn addr_from_env(key: &str, default: &str) -> SocketAddr {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| default.parse().expect("valid default socket address"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let grpc_addr = addr_from_env("CELNET_GRPC_ADDR", "127.0.0.1:50551");
    let ws_addr = addr_from_env("CELNET_WS_ADDR", "127.0.0.1:8081");

    // Bootstrap market state: the engine's calibrated EURUSD 1Y fixture — the same
    // fixture the gRPC/WS integration tests price against, so a verify harness can
    // form a first-principles reference from the same numbers.
    let conv =
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    let initial = make_state(1.10, conv);

    // Start the pinned pricing core + async bridge.
    let link = CoreLink::start(initial, None);

    // Bring up the edge with the WS mirror bound to the fixed `ws_addr`. The demo
    // edge defaults to a 3-LP multi-dealer panel (native maker + 3 deterministic
    // synthetic demo dealers), env-overridable via `CELNET_DEMO_LPS`, so live e2e
    // suites exercise the ranked-panel → accept-with-lp_id path out of the box.
    let panel = LpPanelConfig::from_env_or(3);
    let edge = Edge::start_on_with_panel(
        grpc_addr,
        ws_addr,
        Arc::clone(&link),
        SpreadModel::default(),
        Clock::system(),
        panel,
    )
    .await?;
    edge.gate().mark_ready();

    // Pre-mark the surface book with one calibrated EURUSD 1Y smile under a fresh
    // version, so a client can pin it on a price straight away (the §5
    // reproducibility guarantee) without first issuing its own MarkSurface.
    let pre_marked_version = pre_mark_eurusd_1y(&edge);

    // If a FIX acceptor was started (CELNET_FIX_ADDR set), surface its bound address so
    // an external FIX counterparty can dial it.
    let fix_note = match edge.fix_addr() {
        Some(addr) => format!(" | FIX-acceptor {addr}"),
        None => String::new(),
    };
    eprintln!(
        "celnet-server demo edge ready — gRPC {} | WS-mirror ws://{}{} | \
         pre-marked surface_version={} | LP panel: native maker + {} synthetic demo dealer(s)",
        edge.grpc_addr(),
        edge.ws_addr(),
        fix_note,
        pre_marked_version,
        panel.synthetic_lps
    );

    // Run until Ctrl-C / SIGTERM, then drain gracefully.
    tokio::signal::ctrl_c().await?;
    eprintln!("celnet-server demo edge draining for graceful shutdown…");
    edge.shutdown(Duration::from_secs(5)).await;
    link.stop();
    Ok(())
}

/// Calibrate one EURUSD 1Y Vanna-Volga smile from mild broker quotes and deposit it
/// into the edge's shared [`SurfaceBook`] under a fresh `surface_version`, returning
/// that version. Mirrors the server's own `MarkSurface` calibration path.
fn pre_mark_eurusd_1y(edge: &Edge) -> u64 {
    // The fixture's rates/spot/tenor and the canonical EURUSD 1Y convention.
    let spot = 1.10;
    let r_dom = 0.02;
    let r_for = 0.01;
    let t = 1.0;
    let record = ConventionRecord::new(
        DeltaConvention::SpotUnadjusted,
        AtmConvention::AtmForward,
        PremiumStyle::DomesticPips,
        Cut::NewYork1000,
        DayCount::Act365Fixed,
        DayCount::Act365Fixed,
        DayCount::Act365Fixed,
        Settlement::Deliverable,
    );
    let ctx = SurfaceContext::new(spot, Carry::FxRates { r_dom, r_for }, t, record);
    // Mild EURUSD-like skew (ATM 10.5%, 25Δ RR +1.5%, 25Δ BF 0.35%) — the same
    // quotes the engine fixture uses, so the marked smile is representative.
    let quotes = MarketQuotes::three_point(0.105, 0.015, 0.0035);
    let smile = build_model_smile(SmileModel::MarketHedge, &ctx, &quotes)
        .expect("EURUSD 1Y smile calibrates");
    let forward = smile.forward();

    let book = edge.surface_book();
    let version = book.next_version();
    book.deposit(version, "EUR", "USD", t, forward, smile);
    version
}
