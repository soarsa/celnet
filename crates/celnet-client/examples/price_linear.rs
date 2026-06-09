//! SDK quickstart — price the FX *linear book* via the typed vocabulary builders.
//!
//! One-shot-prices the three linear (non-option) products the SDK exposes through
//! ergonomic [`InstrumentSpec`] builders, against a running edge:
//!
//! * an **FX outright forward** (deliverable EURUSD) — a closed-form discounted
//!   cashflow priced by the dedicated linear book (exact ⇒ no Monte-Carlo standard
//!   error), shown alongside the *fair-forward* case whose PV is ~0,
//! * an **FX swap** (near leg spot-settling + far leg at the forward tenor, opposite
//!   sides) — the PV is the net of the two legs, and
//! * a **non-deliverable forward (NDF)** on a restricted pair (USDBRL, BRL PTAX
//!   fixing) — cash-settled in the convertible currency; the risk-neutral PV equals a
//!   deliverable forward of equal terms (only the fixing IDENTITY is in-repo — the
//!   live fixing VALUE is an estate-gated feed, never sourced here).
//!
//! Each is built purely from the SDK's typed domain vocabulary — no raw proto — and
//! priced over the real edge, surfacing the maker's deterministic `celnet-core`
//! numbers verbatim.
//!
//! # Run it
//!
//! In one terminal, boot a seeded demo edge (binds gRPC on `127.0.0.1:50551`):
//! ```text
//! cargo run -p celnet-server --example demo_edge
//! ```
//! In another:
//! ```text
//! cargo run -p celnet-client --example price_linear
//! ```
//!
//! It exits non-zero unless each product returns a finite priced result with the
//! linear structural invariants (fair-forward PV ≈ 0, exact ⇒ no std-error, NDF PV ==
//! the equal-terms deliverable forward) — so a CI lane that boots the edge and runs
//! the example asserts a real priced linear book, not just a clean compile.

use std::error::Error;
use std::process::ExitCode;

use celnet_client::{
    Client, Conventions, FixingSource, ForwardSide, ForwardTerms, InstrumentSpec, MarketContext,
    NdfTerms, Quantity, SwapTerms,
};
use celnet_types::{CcyPair, Tenor};

/// The gRPC endpoint, overridable. The default matches `demo_edge`.
fn endpoint() -> String {
    std::env::var("CELNET_GRPC_ADDR")
        .map(|a| {
            if a.starts_with("http") {
                a
            } else {
                format!("http://{a}")
            }
        })
        .unwrap_or_else(|_| "http://127.0.0.1:50551".to_owned())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("price_linear failed: {e}");
            eprintln!(
                "is a demo edge running? start one with: \
                 cargo run -p celnet-server --example demo_edge"
            );
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let endpoint = endpoint();
    println!("connecting to {endpoint}");
    let client = Client::connect(endpoint).await?;
    let conventions = Conventions::major_default();

    // ---- FX outright forward, deliverable EURUSD, 1Y -----------------------------
    let eurusd = CcyPair::parse("EURUSD").expect("EURUSD parses");
    // Mild EURUSD market: spot 1.10, 2%/1% rates. The 1Y fair forward is
    // F = S·e^{(r_dom − r_for)·t} = 1.10·e^{0.01}.
    let fx_market = MarketContext {
        spot: 1.10,
        vol: 0.10,
        r_dom: 0.02,
        r_for: 0.01,
    };
    let fair_forward = 1.10 * f64::exp(0.01);

    // A forward struck at the fair forward has PV ≈ 0 — the cleanest structural gate
    // (the discounted cashflow nets to zero when the contract rate IS the fair rate).
    let at_fair = InstrumentSpec::fx_forward(
        eurusd,
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        ForwardTerms::new(fair_forward, 1_000_000.0, ForwardSide::Buy),
    );
    let priced_fair = client.price(&at_fair, fx_market, conventions).await?;
    println!(
        "EURUSD 1Y forward @ fair {fair_forward:.8}: PV {pv:.6}  delta {d:.4}  std_err {se:?}",
        pv = priced_fair.greeks.price,
        d = priced_fair.greeks.delta_spot,
        se = priced_fair.price_std_error,
    );
    // Structural: a fair-struck forward has PV ≈ 0, and the linear PV is EXACT so it
    // carries no Monte-Carlo standard error.
    if !priced_fair.greeks.price.is_finite() || priced_fair.greeks.price.abs() > 1.0 {
        return Err(format!(
            "fair-struck forward PV should be ~0, got {}",
            priced_fair.greeks.price
        )
        .into());
    }
    if priced_fair.price_std_error.is_some() {
        return Err("a closed-form forward must carry no MC standard error".into());
    }

    // An in-the-money long forward (struck below the fair forward) has positive PV.
    let itm = InstrumentSpec::fx_forward(
        eurusd,
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        ForwardTerms::new(1.08, 1_000_000.0, ForwardSide::Buy),
    );
    let priced_itm = client.price(&itm, fx_market, conventions).await?;
    println!(
        "EURUSD 1Y forward @ 1.08 (long): PV {pv:.4}",
        pv = priced_itm.greeks.price
    );
    if !(priced_itm.greeks.price.is_finite() && priced_itm.greeks.price > 0.0) {
        return Err(format!(
            "a long forward struck below fair should be ITM (PV>0), got {}",
            priced_itm.greeks.price
        )
        .into());
    }

    // ---- FX swap, EURUSD, near (spot) BUY + far (1Y) opposite ---------------------
    let swap = InstrumentSpec::fx_swap(
        eurusd,
        Tenor::Years(1),
        1.0,
        Quantity::base(1_000_000.0),
        SwapTerms::new(ForwardTerms::new(1.10, 1_000_000.0, ForwardSide::Buy)),
    );
    let priced_swap = client.price(&swap, fx_market, conventions).await?;
    println!(
        "EURUSD 1Y swap (near BUY @ 1.10 / far SELL): PV {pv:.4}  std_err {se:?}",
        pv = priced_swap.greeks.price,
        se = priced_swap.price_std_error,
    );
    if !priced_swap.greeks.price.is_finite() || priced_swap.price_std_error.is_some() {
        return Err("a closed-form swap must carry a finite PV and no MC std-error".into());
    }

    // ---- NDF, USDBRL (non-deliverable), 6M, BRL PTAX fixing -----------------------
    let usdbrl = CcyPair::parse("USDBRL").expect("USDBRL parses");
    // A USDBRL market: spot 5.00, 10%/5% rates.
    let ndf_market = MarketContext {
        spot: 5.00,
        vol: 0.10,
        r_dom: 0.10,
        r_for: 0.05,
    };
    let ndf = InstrumentSpec::ndf(
        usdbrl,
        Tenor::Months(6),
        0.5,
        Quantity::base(1_000_000.0),
        NdfTerms::new(
            5.10,
            1_000_000.0,
            ForwardSide::Buy,
            FixingSource::BrlPtax,
            "USD",
        ),
    );
    let priced_ndf = client.price(&ndf, ndf_market, conventions).await?;

    // The NDF PV must equal a DELIVERABLE forward of equal terms (non-deliverability
    // changes only the settlement mechanics, not the risk-neutral PV) — an
    // independent structural identity. We can't price a deliverable forward on USDBRL
    // (the server rejects a deliverable product on a non-deliverable pair), so we
    // re-derive the equal-terms PV from the hand-written two-bond discounted cashflow:
    //   PV = side · N · e^{−r_dom·t} · (S·e^{(r_dom−r_for)·t} − K).
    let t = 0.5_f64;
    let notional = 1_000_000.0_f64;
    let fwd = ndf_market.spot * f64::exp((ndf_market.r_dom - ndf_market.r_for) * t);
    let expected_ndf = notional * f64::exp(-ndf_market.r_dom * t) * (fwd - 5.10);
    println!(
        "USDBRL 6M NDF (BRL PTAX, settle USD) @ 5.10: PV {pv:.8}  (hand-derived {exp:.8})",
        pv = priced_ndf.greeks.price,
        exp = expected_ndf,
    );
    if priced_ndf.price_std_error.is_some() {
        return Err("a closed-form NDF must carry no MC standard error".into());
    }
    let scale = priced_ndf
        .greeks
        .price
        .abs()
        .max(expected_ndf.abs())
        .max(1.0);
    if (priced_ndf.greeks.price - expected_ndf).abs() > 1e-9 * scale {
        return Err(format!(
            "NDF PV {} disagrees with the equal-terms deliverable-forward PV {expected_ndf}",
            priced_ndf.greeks.price
        )
        .into());
    }

    println!("done: priced an FX forward, an FX swap, and an NDF via the SDK linear builders.");
    Ok(())
}
