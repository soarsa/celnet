//! `fix_rfq_client` — a minimal, real FIX 4.4 price-taker for driving the celnet
//! FIX acceptor from the command line.
//!
//! It opens a TCP session to a running `celnet-server` FIX edge (the acceptor
//! bound when `CELNET_FIX_ADDR` is set — see `crates/celnet-server/examples/
//! demo_edge.rs`), logs on, sends ONE single-leg `QuoteRequest(R)` built from CLI
//! arguments, and prints the returned market. Two asset classes share the one
//! session machinery, selected by `--asset` (default **fi**, fixed income):
//!
//!   * **fi** — a fixed-income OIS rates RFQ (the [`celnet_fix::dialect_rates`]
//!     vocabulary). Rates are priced by the human desk asynchronously, so the
//!     client only ever observes: it submits the RFQ and, if no price comes back
//!     on the reply, reports that the ticket is queued with the rates desk (a
//!     success — the trader will price it out of band). If a price does return it
//!     is printed like any two-way quote.
//!   * **fx** — a single-leg FX-option RFQ (the [`celnet_fix::dialect_fx`]
//!     vocabulary). Prints the returned two-way `Quote(S)` and — when asked to
//!     trade — lifts it with a `NewOrderSingle(D)` and prints the
//!     `ExecutionReport(8)`.
//!
//! This is not a stub: it drives the SAME [`celnet_fix::initiator::Initiator`] the
//! integration tests use over a loopback socket, so it exercises the live RFQ →
//! Quote → (FX) lift path exactly as an external counterparty would.
//!
//! ## Usage
//!
//! ```text
//! cargo run -p celnet-fix --example fix_rfq_client -- [--key value ...]
//!
//!   --asset fi|fx        asset class (fi=rates OIS, fx=FX option)   (default fi)
//!   --addr HOST:PORT     FIX acceptor address           (default 127.0.0.1:9099)
//!
//!   Fixed income (--asset fi):
//!   --curve SYMBOL       curve symbol (e.g. USD-OIS)     (default USD-OIS)
//!   --tenor YEARS        OIS tenor in whole years (>= 1) (default 5)
//!   --notional AMOUNT    notional (> 0)                  (default 10000000)
//!   --side pay|receive|two-way  fixed-leg intent; absent ⇒ two-way market
//!                                                         (default two-way)
//!
//!   FX option (--asset fx):
//!   --pair PAIR          6-letter currency pair          (default EURUSD)
//!   --type call|put      option type                     (default call)
//!   --strike PRICE       strike (quote ccy per base)     (default 1.10)
//!   --expiry-years YEARS vol-time to expiry in years     (default 1.0)
//!   --side observe|buy|sell  observe = RFQ only; buy lifts the offer, sell hits the bid
//!                                                         (default observe)
//!   --settlement deliverable|ndf                          (default deliverable)
//!   --exercise european|american                          (default european)
//!
//!   Common:
//!   --sender COMP_ID     our SenderCompID                (default CELNET-CPTY)
//!   --target COMP_ID     venue TargetCompID              (default CELNET)
//!   --req-id ID          QuoteReqID                      (default RFQ-CLI)
//! ```
//!
//! The CompID defaults match the demo edge's expected counterparty/venue; override
//! with `--sender`/`--target` (or the edge's `CELNET_FIX_TARGET`/`CELNET_FIX_SENDER`).
//! `--side` means different things per asset (a lift direction for fx, a fixed-leg
//! direction for fi), so it is interpreted once `--asset` is known — flags may
//! appear in any order.

use std::time::{SystemTime, UNIX_EPOCH};

use celnet_fix::dialect_fx::{self, ExerciseStyle, QuoteRequestParams};
use celnet_fix::dialect_rates::{self, RatesQuoteRequestParams, RatesSide, SubscriptionRequest};
use celnet_fix::initiator::{Initiator, LiftPolicy};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};
use celnet_types::{OptionType, Settlement};
use tokio::net::TcpStream;

/// The asset class the client drives — one session, two dialects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetClass {
    /// A fixed-income OIS rates RFQ (the [`dialect_rates`] vocabulary).
    FixedIncome,
    /// A single-leg FX-option RFQ (the [`dialect_fx`] vocabulary).
    FxOption,
}

/// The parsed client request — owned so the backing bytes outlive the async send.
struct Args {
    addr: String,
    asset: AssetClass,
    // FX-option fields (used when `asset == FxOption`).
    pair: String,
    strike_ccy: String,
    option_type: OptionType,
    strike: f64,
    expiry_years: f64,
    settlement: Settlement,
    exercise: ExerciseStyle,
    // Fixed-income fields (used when `asset == FixedIncome`).
    curve: String,
    tenor_years: u32,
    notional: f64,
    rates_side: RatesSide,
    // The FX lift policy. Fixed income always observes (the desk prices async), so
    // this is `Observe` there and only ever lifts on the FX path.
    policy: LiftPolicy,
    // Common session fields.
    sender: String,
    target: String,
    req_id: String,
}

fn usage_and_exit(msg: &str) -> ! {
    eprintln!("fix_rfq_client: {msg}");
    eprintln!("try: cargo run -p celnet-fix --example fix_rfq_client -- --help");
    std::process::exit(2);
}

fn print_help() -> ! {
    // The module-level doc block carries the full reference; echo the essentials.
    println!(
        "fix_rfq_client — FIX 4.4 RFQ price-taker (rates OIS or FX option)\n\n\
         Flags (all optional; sensible defaults dial the demo edge):\n  \
         --asset fi|fx (fi)   --addr HOST:PORT (127.0.0.1:9099)\n  \
         fi:  --curve USD-OIS  --tenor 5  --notional 10000000  --side pay|receive|two-way\n  \
         fx:  --pair EURUSD  --type call|put  --strike 1.10  --expiry-years 1.0\n       \
         --side observe|buy|sell  --settlement deliverable|ndf  --exercise european|american\n  \
         common: --sender CELNET-CPTY  --target CELNET  --req-id RFQ-CLI"
    );
    std::process::exit(0);
}

fn parse_args() -> Args {
    let mut addr = String::from("127.0.0.1:9099");
    let mut asset = AssetClass::FixedIncome;
    // FX-option defaults.
    let mut pair = String::from("EURUSD");
    let mut option_type = OptionType::Call;
    let mut strike = 1.10_f64;
    let mut expiry_years = 1.0_f64;
    let mut settlement = Settlement::Deliverable;
    let mut exercise = ExerciseStyle::European;
    // Fixed-income defaults.
    let mut curve = String::from("USD-OIS");
    let mut tenor_years = 5_u32;
    let mut notional = 10_000_000.0_f64;
    // Common defaults.
    let mut sender = String::from("CELNET-CPTY");
    let mut target = String::from("CELNET");
    let mut req_id = String::from("RFQ-CLI");
    // `--side` means different things per asset and flags arrive in any order, so
    // capture it raw and interpret it after the loop once `--asset` is known.
    let mut side_raw: Option<String> = None;

    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "-h" || flag == "--help" {
            print_help();
        }
        let val = it
            .next()
            .unwrap_or_else(|| usage_and_exit(&format!("flag `{flag}` needs a value")));
        match flag.as_str() {
            "--addr" => addr = val,
            "--asset" => {
                asset = match val.to_lowercase().as_str() {
                    "fi" | "rates" | "fixedincome" | "fixed-income" => AssetClass::FixedIncome,
                    "fx" | "options" | "fxo" | "fxoption" => AssetClass::FxOption,
                    other => usage_and_exit(&format!("--asset must be fi|fx, got `{other}`")),
                }
            }
            "--curve" => curve = val.to_uppercase(),
            "--tenor" => {
                tenor_years = val
                    .parse()
                    .unwrap_or_else(|_| usage_and_exit("--tenor must be a whole number of years"))
            }
            "--notional" => {
                notional = val
                    .parse()
                    .unwrap_or_else(|_| usage_and_exit("--notional must be a number"))
            }
            "--pair" => pair = val.to_uppercase(),
            "--type" => {
                option_type = match val.to_lowercase().as_str() {
                    "call" | "c" => OptionType::Call,
                    "put" | "p" => OptionType::Put,
                    other => usage_and_exit(&format!("--type must be call|put, got `{other}`")),
                }
            }
            "--strike" => {
                strike = val
                    .parse()
                    .unwrap_or_else(|_| usage_and_exit("--strike must be a number"))
            }
            "--expiry-years" => {
                expiry_years = val
                    .parse()
                    .unwrap_or_else(|_| usage_and_exit("--expiry-years must be a number"))
            }
            "--side" => side_raw = Some(val),
            "--settlement" => {
                settlement = match val.to_lowercase().as_str() {
                    "deliverable" | "fxvo" => Settlement::Deliverable,
                    "ndf" | "fxno" | "nondeliverable" => Settlement::NonDeliverable,
                    other => usage_and_exit(&format!(
                        "--settlement must be deliverable|ndf, got `{other}`"
                    )),
                }
            }
            "--exercise" => {
                exercise = match val.to_lowercase().as_str() {
                    "european" | "euro" => ExerciseStyle::European,
                    "american" => ExerciseStyle::American,
                    other => usage_and_exit(&format!(
                        "--exercise must be european|american, got `{other}`"
                    )),
                }
            }
            "--sender" => sender = val,
            "--target" => target = val,
            "--req-id" => req_id = val,
            other => usage_and_exit(&format!("unknown flag `{other}`")),
        }
    }

    // Interpret `--side` per asset (default: fi ⇒ two-way market, fx ⇒ observe).
    // Fixed income never lifts (the desk prices async), so its lift policy is
    // always `Observe`; the fixed-leg direction rides `rates_side` instead.
    let mut rates_side = RatesSide::TwoWay;
    let mut policy = LiftPolicy::Observe;
    match asset {
        AssetClass::FixedIncome => {
            rates_side = match side_raw.as_deref() {
                None => RatesSide::TwoWay,
                Some(s) => match s.to_lowercase().as_str() {
                    "pay" | "payfixed" | "pay-fixed" => RatesSide::PayFixed,
                    "receive" | "rec" | "receivefixed" | "receive-fixed" => RatesSide::ReceiveFixed,
                    "two-way" | "twoway" | "2way" | "rfq" => RatesSide::TwoWay,
                    other => usage_and_exit(&format!(
                        "--side (fi) must be pay|receive|two-way, got `{other}`"
                    )),
                },
            };
        }
        AssetClass::FxOption => {
            policy = match side_raw.as_deref() {
                None => LiftPolicy::Observe,
                Some(s) => match s.to_lowercase().as_str() {
                    "observe" | "rfq" => LiftPolicy::Observe,
                    "buy" | "lift" => LiftPolicy::LiftOffer,
                    "sell" | "hit" => LiftPolicy::HitBid,
                    other => usage_and_exit(&format!(
                        "--side (fx) must be observe|buy|sell, got `{other}`"
                    )),
                },
            };
        }
    }

    // Validate per asset, so an fi run never trips FX-pair rules and vice versa.
    match asset {
        AssetClass::FixedIncome => {
            if curve.trim().is_empty() {
                usage_and_exit("--curve must be a non-empty curve symbol (e.g. USD-OIS)");
            }
            if tenor_years < 1 {
                usage_and_exit("--tenor must be a whole number of years >= 1");
            }
            if !(notional > 0.0 && notional.is_finite()) {
                usage_and_exit("--notional must be positive");
            }
        }
        AssetClass::FxOption => {
            if pair.len() != 6 {
                usage_and_exit(&format!(
                    "--pair must be a 6-letter pair (e.g. EURUSD), got `{pair}`"
                ));
            }
            if !(strike > 0.0 && strike.is_finite()) {
                usage_and_exit("--strike must be positive");
            }
            if !(expiry_years > 0.0 && expiry_years.is_finite()) {
                usage_and_exit("--expiry-years must be positive");
            }
        }
    }
    // The strike currency is the pair's quote (domestic) leg — the last three
    // letters. Only the FX path reads it; guard the slice so a fixed-income run
    // with a non-6-letter `--pair` override cannot panic here.
    let strike_ccy = if pair.len() == 6 {
        pair[3..6].to_string()
    } else {
        String::new()
    };

    Args {
        addr,
        asset,
        pair,
        strike_ccy,
        option_type,
        strike,
        expiry_years,
        settlement,
        exercise,
        curve,
        tenor_years,
        notional,
        rates_side,
        policy,
        sender,
        target,
        req_id,
    }
}

/// A FIX `UTCTimestamp` ("YYYYMMDD-HH:MM:SS.sss") for `SendingTime(52)`. The session
/// FSM is clock-free (the caller supplies this for outbound frames; the venue stamps
/// its own on replies), so this only needs to be a well-formed UTC stamp.
fn fix_utc_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let millis = now.subsec_millis();
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let (hh, mm, ss) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}{month:02}{day:02}-{hh:02}:{mm:02}:{ss:02}.{millis:03}")
}

/// Howard Hinnant's `civil_from_days`: days since the Unix epoch → (year, month, day).
/// Exact integer date arithmetic — no calendar dependency.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Print a returned two-way market (shared by both asset classes when a quote comes
/// back on the reply).
fn print_quote(quote_id: Option<&[u8]>, bid: f64, offer: f64) {
    let qid = quote_id
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_else(|| "<none>".to_string());
    let mid = 0.5 * (bid + offer);
    println!("✓ Quote received");
    println!("    QuoteID   {qid}");
    println!("    bid       {bid:.8}");
    println!("    offer     {offer:.8}");
    println!("    mid       {mid:.8}  (spread {:.8})", offer - bid);
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = parse_args();

    println!("── celnet FIX RFQ client ─────────────────────────────");
    println!(
        "  connect   {} as {} → {}",
        args.addr, args.sender, args.target
    );
    match args.asset {
        AssetClass::FixedIncome => {
            let side_label = match args.rates_side {
                RatesSide::PayFixed => "pay fixed",
                RatesSide::ReceiveFixed => "receive fixed",
                RatesSide::TwoWay => "two-way",
            };
            println!(
                "  RFQ (fi)  {} OIS {}y · notional {:.2} · {}",
                args.curve, args.tenor_years, args.notional, side_label
            );
        }
        AssetClass::FxOption => {
            let side_label = match args.policy {
                LiftPolicy::Observe => "observe (RFQ only)",
                LiftPolicy::LiftOffer => "BUY — lift the offer",
                LiftPolicy::HitBid => "SELL — hit the bid",
            };
            let type_label = match args.option_type {
                OptionType::Call => "Call",
                OptionType::Put => "Put",
            };
            println!(
                "  RFQ (fx)  {} {} strike {} · {:.4}y · {}",
                args.pair, type_label, args.strike, args.expiry_years, side_label
            );
        }
    }
    println!("──────────────────────────────────────────────────────");

    let stream = match TcpStream::connect(&args.addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "✗ could not connect to {} — is the edge up with CELNET_FIX_ADDR set? ({e})",
                args.addr
            );
            std::process::exit(1);
        }
    };
    stream.set_nodelay(true).ok();

    let cfg = SessionConfig {
        sender: args.sender.clone().into_bytes(),
        target: args.target.clone().into_bytes(),
        heart_bt_int: 30,
        role: Role::Initiator,
    };
    let session = Session::new(cfg, InMemoryStore::new());
    let mut initiator = Initiator::new(session, args.policy);

    match args.asset {
        AssetClass::FixedIncome => {
            // Backing bytes for the request must outlive the (single) async send.
            let req_id = args.req_id.clone().into_bytes();
            let symbol = args.curve.clone().into_bytes();
            let params = RatesQuoteRequestParams {
                quote_req_id: &req_id,
                symbol: &symbol,
                tenor_years: args.tenor_years,
                notional: args.notional,
                side: args.rates_side,
                subscription: SubscriptionRequest::Snapshot, // one-shot RFQ
            };

            let sending_time = fix_utc_timestamp().into_bytes();
            let result = initiator
                .request_and_lift(stream, sending_time, |hdr, enc| {
                    dialect_rates::build_rates_quote_request(hdr, &params, enc)
                })
                .await?;

            match (result.bid, result.offer) {
                (Some(bid), Some(offer)) => {
                    print_quote(result.quote_id.as_deref(), bid, offer);
                }
                _ => {
                    // Rates are priced by the human desk out of band, so no quote on
                    // the reply is the expected happy path for an OIS RFQ, not a
                    // failure: the ticket is now queued with the rates desk.
                    println!("✓ RFQ submitted to the rates desk (awaiting a trader's price)");
                }
            }
        }
        AssetClass::FxOption => {
            // Backing bytes for the request must outlive the (single) async send.
            let req_id = args.req_id.clone().into_bytes();
            let symbol = args.pair.clone().into_bytes();
            let strike_ccy = args.strike_ccy.clone().into_bytes();
            let params = QuoteRequestParams {
                quote_req_id: &req_id,
                symbol: &symbol,
                option_type: args.option_type,
                strike: args.strike,
                expiry_years: args.expiry_years,
                settlement: args.settlement,
                exercise: args.exercise,
                strike_ccy: &strike_ccy,
            };

            let sending_time = fix_utc_timestamp().into_bytes();
            let result = initiator
                .request_and_lift(stream, sending_time, |hdr, enc| {
                    dialect_fx::build_quote_request(hdr, &params, enc)
                })
                .await?;

            match (result.bid, result.offer) {
                (Some(bid), Some(offer)) => {
                    print_quote(result.quote_id.as_deref(), bid, offer);
                }
                _ => {
                    eprintln!("✗ no quote returned (the venue declined or the session closed)");
                    std::process::exit(1);
                }
            }

            if !matches!(args.policy, LiftPolicy::Observe) {
                if result.filled {
                    let px = result.fill_px.unwrap_or(f64::NAN);
                    println!("✓ FILLED @ {px:.8}");
                } else {
                    println!("✗ NOT FILLED (last-look declined or quote expired)");
                }
            }
        }
    }

    Ok(())
}
