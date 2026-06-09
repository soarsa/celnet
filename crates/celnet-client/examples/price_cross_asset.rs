//! SDK quickstart — price **cross-asset** vanilla options via the typed vocabulary
//! builders.
//!
//! One unversioned contract names every asset class: this one-shot-prices a vanilla
//! European CALL on three different underlyings against a running edge, each built
//! purely from the SDK's typed domain vocabulary (no raw proto):
//!
//! * an **equity** vanilla (`AAPL` on `XNAS`, USD) — [`InstrumentSpec::equity_vanilla`],
//! * a **commodity** vanilla (`BRENT`, USD) — [`InstrumentSpec::commodity_vanilla`], and
//! * a **digital-asset (crypto)** vanilla (`BTCUSDT`) — [`InstrumentSpec::crypto_vanilla`].
//!
//! Under the linear (quote-margined) settlement style — the default — the option
//! payoff is the asset-class-agnostic generalized-BSM / Garman-Kohlhagen closed form
//! over the carry-producing market (ADR-0008): the underlying travels as contract
//! identity, and `r_for` is the asset's carry yield (an equity dividend yield, a
//! commodity cost-of-carry, a crypto funding rate). So a cross-asset vanilla on a
//! given market prices identically to an FX vanilla on that market — which is exactly
//! the structural identity this example gates on.
//!
//! # Run it
//!
//! In one terminal, boot a seeded demo edge (binds gRPC on `127.0.0.1:50551`):
//! ```text
//! cargo run -p celnet-server --example demo_edge
//! ```
//! In another:
//! ```text
//! cargo run -p celnet-client --example price_cross_asset
//! ```
//!
//! It exits non-zero unless each cross-asset price is finite, matches the independent
//! generalized-BSM closed form, and is byte-identical to the FX baseline on the same
//! market — so a CI lane that boots the edge and runs the example asserts a real
//! priced cross-asset book, not just a clean compile.

use std::error::Error;
use std::process::ExitCode;

use celnet_client::{
    Ccy, Client, Conventions, InstrumentSpec, MarketContext, Quantity, Side, StrikeSpec, Underlying,
};
use celnet_types::{CcyPair, OptionType, Tenor, VanillaInputs};

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
            eprintln!("price_cross_asset failed: {e}");
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

    // A non-degenerate 1Y market shared across asset classes. `r_for` is the asset's
    // carry yield (dividend / cost-of-carry / funding), `r_dom` the discount rate.
    let (spot, strike, vol, t, r_dom, r_for) = (100.0_f64, 105.0_f64, 0.22, 1.0, 0.03, 0.012);
    let market = MarketContext {
        spot,
        vol,
        r_dom,
        r_for,
    };
    let tenor = Tenor::Years(1);
    let qty = Quantity::base(1.0);

    // The independent oracle: the generalized-BSM / GK closed form for this market —
    // a code path the wire/server never touches in this example.
    let oracle = celnet_vanilla::greeks(
        OptionType::Call,
        &VanillaInputs::new(spot, strike, vol, t, r_dom, r_for),
    );

    // The FX baseline on the identical market — the byte-identity reference.
    let fx = InstrumentSpec::vanilla_on(
        Underlying::Fx(CcyPair::parse("EURUSD").expect("EURUSD parses")),
        tenor,
        t,
        qty,
        Side::TwoWay,
        OptionType::Call,
        StrikeSpec::Absolute(strike),
    );
    let fx_line = client.price(&fx, market, conventions).await?;
    println!(
        "FX baseline (EURUSD 1Y 105 call): price {p:.10}  vega {v:.10}",
        p = fx_line.greeks.price,
        v = fx_line.greeks.vega,
    );

    // The three cross-asset underlyings, each a 1Y CALL struck at 105 on the market.
    let specs: [(&str, InstrumentSpec); 3] = [
        (
            "equity AAPL.XNAS (USD)",
            InstrumentSpec::equity_vanilla(
                "AAPL",
                "XNAS",
                Ccy::USD,
                tenor,
                t,
                qty,
                Side::TwoWay,
                OptionType::Call,
                StrikeSpec::Absolute(strike),
            ),
        ),
        (
            "commodity BRENT (USD)",
            InstrumentSpec::commodity_vanilla(
                "BRENT",
                "",
                Ccy::USD,
                tenor,
                t,
                qty,
                Side::TwoWay,
                OptionType::Call,
                StrikeSpec::Absolute(strike),
            ),
        ),
        (
            "crypto BTCUSDT",
            InstrumentSpec::crypto_vanilla(
                "BTC",
                "USDT",
                tenor,
                t,
                qty,
                Side::TwoWay,
                OptionType::Call,
                StrikeSpec::Absolute(strike),
            ),
        ),
    ];

    for (label, spec) in &specs {
        let line = client.price(spec, market, conventions).await?;
        println!(
            "{label}: price {p:.10}  delta {d:.6}  vega {v:.10}",
            p = line.greeks.price,
            d = line.greeks.delta_spot,
            v = line.greeks.vega,
        );

        // SDK == independent generalized-BSM oracle.
        if !line.greeks.price.is_finite() || (line.greeks.price - oracle.price).abs() > 1e-10 {
            return Err(format!(
                "{label} price {} disagrees with the independent oracle {}",
                line.greeks.price, oracle.price
            )
            .into());
        }
        // SDK == server == FX baseline, bit-for-bit: the asset class is identity only
        // and must not perturb the linear (quote-margined) payoff.
        if line.greeks.price.to_bits() != fx_line.greeks.price.to_bits()
            || line.greeks.vega.to_bits() != fx_line.greeks.vega.to_bits()
        {
            return Err(
                format!("{label} price/vega must be byte-identical to the FX baseline").into(),
            );
        }
    }

    println!(
        "done: priced an equity, a commodity, and a crypto vanilla via the SDK \
         cross-asset builders (each == the FX baseline, == the independent oracle)."
    );
    Ok(())
}
