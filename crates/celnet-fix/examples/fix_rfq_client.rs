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
//!   * **fx** — a single-leg FX vanilla-option RFQ (the [`celnet_fix::dialect_fx`]
//!     vocabulary). A one-shot (`--repeat 1`) sends the RFQ built from the explicit
//!     CLI flags, prints the returned two-way `Quote(S)` and — when asked to trade —
//!     lifts it with a `NewOrderSingle(D)` and prints the `ExecutionReport(8)`. A
//!     stream (`--repeat != 1`) rotates the deterministic, seed-free
//!     [`celnet_fix::sim::fx_leg`] grid — major deliverable pairs, near-the-money
//!     strikes, short-dated expiries, call/put — auto-quoting most, LIFTING (→ booked
//!     deal) every `--lift-every`-th, and injecting a deliberately UNPRICEABLE leg
//!     every `--manual-every`-th (American exercise, or an NDF request on a deliverable
//!     major) so the venue routes it to the FX desk for manual pricing (no `Quote(S)`).
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
use celnet_fix::sim;
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
    // Persistent-session loop controls (see the local defaults in `parse_args`).
    repeat: u64,
    /// Cadence between streamed RFQs, in milliseconds (sub-second capable). Set by
    /// `--interval-ms` directly, or `--interval` (whole seconds ×1000); default
    /// [`DEFAULT_INTERVAL_MS`].
    interval_ms: u64,
    manual_every: u64,
    manual_tenor: u32,
    // The bogus curve symbol used for the "unknown security" manual variant: half the
    // manual RFQs name this (unrecognised) `Symbol(55)` on a valid on-the-run tenor, so
    // the venue routes them to the desk as an UNKNOWN_SECURITY manual intervention. The
    // other half use `manual_tenor` (an unconfigured tenor on the valid curve).
    manual_security: String,
    // Every Nth successful auto-quote is LIFTED (executed → booked deal); 0 = never
    // (pure observe). Lets one RFQ/RFS stream show both quoted-only and booked-deal rows.
    lift_every: u64,
    // Run the fixed-income venue as an RFS *stream* (Subscribe → continuous re-priced
    // quotes) instead of a one-shot RFQ (Snapshot); each cycle holds the stream.
    stream: bool,
    // The RFS hold per cycle, in milliseconds: how long to read streamed updates before
    // the next cycle / lift. Set by `--stream-hold-ms` directly, or `--stream-hold` (whole
    // seconds ×1000); default [`DEFAULT_STREAM_HOLD_MS`]. Only used in `--intent rfs` mode.
    stream_hold_ms: u64,
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
         loop: --repeat 0  --interval-ms 750 (or --interval SECS)  --stream-hold-ms 2000  --manual-every 3  --manual-tenor 15  --manual-security XXX-UNKNOWN  --lift-every 3\n  \
         common: --sender CELNET-CPTY  --target CELNET  --req-id RFQ-CLI"
    );
    std::process::exit(0);
}

/// Default cadence between streamed RFQs, in **milliseconds**, when neither `--interval`
/// nor `--interval-ms` is given — a deliberately **fast** sub-second default (750ms) so a
/// stream builds up deals / positions / per-client flow / portfolio risk quickly for a
/// lively demo, bounded so it never floods the venue. The deploy simulator overrides it
/// via `--interval-ms` (`FIXSIM_PERIOD_MS`) or `--interval` (`FIXSIM_PERIOD`, whole
/// seconds); never a bare magic number at the call site.
const DEFAULT_INTERVAL_MS: u64 = 750;

/// Default RFS hold per cycle, in **milliseconds**, when neither `--stream-hold` nor
/// `--stream-hold-ms` is given — how long an RFS cycle reads streamed updates before the
/// next cycle / lift. Kept short (2s) so the fast cadence turns over many cycles.
const DEFAULT_STREAM_HOLD_MS: u64 = 2_000;

/// Default `--manual-every`: 1 in 3 streamed RFQs is a manual (desk-routed) one, giving a
/// ~2/3 auto-quoted : ~1/3 manual mix.
const DEFAULT_MANUAL_EVERY: u64 = 3;

/// Default bogus `Symbol(55)` for the unknown-security manual variant (see [`Args`]).
const DEFAULT_MANUAL_SECURITY: &str = "XXX-UNKNOWN";

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
    // Persistent-session loop controls: with `--repeat != 1` the client logs on ONCE
    // and streams that many RFQs (0 = forever) over the SAME session, `--interval`
    // seconds apart — no logon/logout churn per request. For fixed income the loop
    // rotates the tenor: mostly on-the-run (auto-quoted), every `--manual-every`-th a
    // `--manual-tenor` request the venue routes to a human desk.
    let mut repeat = 1_u64;
    let mut interval_ms = DEFAULT_INTERVAL_MS;
    let mut manual_every = DEFAULT_MANUAL_EVERY;
    let mut manual_tenor = 15_u32;
    let mut manual_security = String::from(DEFAULT_MANUAL_SECURITY);
    let mut lift_every = 0_u64;
    let mut stream = false;
    let mut stream_hold_ms = DEFAULT_STREAM_HOLD_MS;
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
            "--repeat" => {
                repeat = val.parse().unwrap_or_else(|_| {
                    usage_and_exit("--repeat must be a whole number (0 = forever)")
                })
            }
            "--interval" => {
                let secs: u64 = val.parse().unwrap_or_else(|_| {
                    usage_and_exit("--interval must be a whole number of seconds")
                });
                interval_ms = secs.saturating_mul(1_000);
            }
            "--interval-ms" => {
                interval_ms = val.parse().unwrap_or_else(|_| {
                    usage_and_exit("--interval-ms must be a whole number of milliseconds")
                })
            }
            "--manual-every" => {
                manual_every = val.parse().unwrap_or_else(|_| {
                    usage_and_exit("--manual-every must be a whole number (0 = never)")
                })
            }
            "--manual-tenor" => {
                manual_tenor = val.parse().unwrap_or_else(|_| {
                    usage_and_exit("--manual-tenor must be a whole number of years")
                })
            }
            "--manual-security" => manual_security = val.to_uppercase(),
            "--lift-every" => {
                lift_every = val.parse().unwrap_or_else(|_| {
                    usage_and_exit("--lift-every must be a whole number (0 = never)")
                })
            }
            "--intent" => {
                stream = match val.to_lowercase().as_str() {
                    "rfs" | "stream" | "subscribe" => true,
                    "rfq" | "snapshot" => false,
                    other => usage_and_exit(&format!("--intent must be rfq|rfs, got `{other}`")),
                }
            }
            "--stream-hold" => {
                let secs: u64 = val.parse().unwrap_or_else(|_| {
                    usage_and_exit("--stream-hold must be a whole number of seconds")
                });
                stream_hold_ms = secs.saturating_mul(1_000);
            }
            "--stream-hold-ms" => {
                stream_hold_ms = val.parse().unwrap_or_else(|_| {
                    usage_and_exit("--stream-hold-ms must be a whole number of milliseconds")
                })
            }
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
    Args {
        addr,
        asset,
        pair,
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
        repeat,
        interval_ms,
        manual_every,
        manual_tenor,
        manual_security,
        lift_every,
        stream,
        stream_hold_ms,
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

    // On-the-run tenors the venue auto-quotes; the loop rotates through these and every
    // `--manual-every`-th request injects the non-standard `--manual-tenor` (desk-routed).
    const AUTO_TENORS: &[u32] = &[2, 3, 5, 7, 10];

    // Log on ONCE and stream requests over the SAME session — no logon/logout per RFQ.
    let mut sess = initiator
        .open(stream, fix_utc_timestamp().as_bytes())
        .await?;

    let forever = args.repeat == 0;
    let mut i: u64 = 0;
    loop {
        // A unique request id per RFQ (stable base for a one-shot, base+counter in a stream).
        let this_req = if args.repeat == 1 {
            args.req_id.clone()
        } else {
            format!("{}-{i}", args.req_id)
        };
        let req_id = this_req.into_bytes();
        let sending_time = fix_utc_timestamp().into_bytes();

        // The simulated counterparty on whose behalf this RFQ is entered — a deterministic
        // rotation over the shared pool (`sim::counterparty_for`), stamped into the RFQ's
        // `PartyID(448)` so the venue's desk blotter shows a varied, realistic counterparty
        // per request over this single FIX session (one SenderCompID). A one-shot
        // (`--repeat 1`) names no party (`None`) — the venue then falls back to the
        // authenticated CompID, keeping the explicit single-shot behaviour unchanged.
        let counterparty = sim::counterparty_for(i);
        let party_id: Option<&[u8]> = (args.repeat != 1).then_some(counterparty.as_bytes());
        // Operator log prefix: name the rotated counterparty on a stream; empty on a
        // one-shot (which sends no party), so single-shot output stays byte-identical.
        let cpty_tag = if party_id.is_some() {
            format!("{counterparty} · ")
        } else {
            String::new()
        };

        let result = match args.asset {
            AssetClass::FixedIncome => {
                // A one-shot honours --tenor; a stream rotates on-the-run tenors and
                // injects a manual (desk-routed) tenor every `--manual-every`-th request.
                let is_manual = args.repeat != 1
                    && args.manual_every > 0
                    && (i + 1).is_multiple_of(args.manual_every);
                // Alternate the two manual variants deterministically by the manual
                // occurrence ordinal (no RNG — varies by iteration index): odd ⇒
                // unconfigured tenor (valid curve on an off-the-run tenor → the venue
                // routes it as UNCONFIGURED_TENOR); even ⇒ unknown security (a bogus curve
                // symbol on a valid on-the-run tenor → routed as UNKNOWN_SECURITY).
                let manual_unknown_security =
                    is_manual && ((i + 1) / args.manual_every).is_multiple_of(2);
                let (symbol_str, tenor) = if args.repeat == 1 {
                    (args.curve.as_str(), args.tenor_years)
                } else if manual_unknown_security {
                    (
                        args.manual_security.as_str(),
                        AUTO_TENORS[(i as usize) % AUTO_TENORS.len()],
                    )
                } else if is_manual {
                    (args.curve.as_str(), args.manual_tenor)
                } else {
                    (
                        args.curve.as_str(),
                        AUTO_TENORS[(i as usize) % AUTO_TENORS.len()],
                    )
                };
                let symbol = symbol_str.as_bytes().to_vec();
                // Lift (execute + book) an auto-quote every `--lift-every`-th cycle; a
                // manual (desk-routed) tenor has no quote to lift, so it is never lifted.
                let should_lift =
                    !is_manual && args.lift_every > 0 && (i + 1).is_multiple_of(args.lift_every);

                if args.stream {
                    // RFS: Subscribe → the venue streams continuous re-priced quotes; hold
                    // the session reading updates, and lift one mid-hold when due (executing
                    // a streaming deal that books). Use a STABLE QuoteReqID across cycles so
                    // each re-subscribe REPLACES the one live stream on the venue (keyed by
                    // QuoteReqID) instead of piling up a fresh subscription every cycle.
                    let stream_req_id = format!("{}-RFS", args.req_id).into_bytes();
                    let params = RatesQuoteRequestParams {
                        quote_req_id: &stream_req_id,
                        symbol: &symbol,
                        tenor_years: tenor,
                        notional: args.notional,
                        side: args.rates_side,
                        subscription: SubscriptionRequest::Subscribe,
                    };
                    let hold = std::time::Duration::from_millis(args.stream_hold_ms.max(1));
                    let lift_after = should_lift.then(|| hold / 2);
                    let outcome = sess
                        .stream(
                            &sending_time,
                            |hdr, enc| {
                                dialect_rates::build_rates_quote_request_with_party(
                                    hdr, &params, party_id, enc,
                                )
                            },
                            hold,
                            lift_after,
                        )
                        .await?;
                    println!(
                        "[{i}] {tenor}y OIS RFS — streamed {} update(s)",
                        outcome.updates
                    );
                    if let (Some(bid), Some(offer)) = (outcome.result.bid, outcome.result.offer) {
                        print_quote(outcome.result.quote_id.as_deref(), bid, offer);
                    }
                    if should_lift {
                        if outcome.result.filled {
                            let px = outcome.result.fill_px.unwrap_or(f64::NAN);
                            println!(
                                "[{i}] ✓ streamed quote LIFTED & FILLED @ {px:.8} — deal booked"
                            );
                        } else {
                            println!(
                                "[{i}] ✗ stream lift NOT filled (last-look declined / expired)"
                            );
                        }
                    }
                    outcome.result
                } else {
                    // RFQ snapshot: auto-quoted or desk-routed. Lift some auto-quotes so they
                    // execute and book as completed deals; observe the rest (quoted only).
                    sess.set_policy(if should_lift {
                        LiftPolicy::LiftOffer
                    } else {
                        LiftPolicy::Observe
                    });
                    let params = RatesQuoteRequestParams {
                        quote_req_id: &req_id,
                        symbol: &symbol,
                        tenor_years: tenor,
                        notional: args.notional,
                        side: args.rates_side,
                        subscription: SubscriptionRequest::Snapshot,
                    };
                    let r = sess
                        .request(&sending_time, |hdr, enc| {
                            dialect_rates::build_rates_quote_request_with_party(
                                hdr, &params, party_id, enc,
                            )
                        })
                        .await?;
                    match (r.bid, r.offer) {
                        (Some(bid), Some(offer)) => {
                            println!("[{i}] {cpty_tag}{tenor}y OIS — auto-quoted:");
                            print_quote(r.quote_id.as_deref(), bid, offer);
                        }
                        // No quote = the venue routed this RFQ to a human desk (expected
                        // for a manual variant): either an unknown security or an
                        // unconfigured tenor.
                        _ => println!(
                            "[{i}] {cpty_tag}{symbol_str} {tenor}y OIS — ✓ submitted to the rates desk (manual: {})",
                            if manual_unknown_security {
                                "unknown security"
                            } else {
                                "unconfigured tenor"
                            }
                        ),
                    }
                    if should_lift {
                        if r.filled {
                            let px = r.fill_px.unwrap_or(f64::NAN);
                            println!("[{i}] ✓ auto-quote LIFTED & FILLED @ {px:.8} — deal booked");
                        } else if r.bid.is_some() {
                            println!("[{i}] ✗ lift NOT filled (last-look declined / expired)");
                        }
                    }
                    r
                }
            }
            AssetClass::FxOption => {
                // A one-shot (`--repeat 1`) honours the explicit CLI flags exactly
                // (backward compatible). A stream rotates a realistic grid of major
                // deliverable pairs / near-the-money strikes / short-dated expiries /
                // call-put, injecting a deliberately UNPRICEABLE (desk-routed) leg every
                // `--manual-every`-th request and lifting an auto-quote every
                // `--lift-every`-th (an executed → booked FX deal). The rotation is the
                // pure, seed-free `sim::fx_leg` — the same iteration index always yields
                // the same RFQ (see the gated unit tests in `celnet_fix::sim`).
                let (pair, option_type, strike, expiry_years, settlement, exercise, kind) =
                    if args.repeat == 1 {
                        (
                            args.pair.clone(),
                            args.option_type,
                            args.strike,
                            args.expiry_years,
                            args.settlement,
                            args.exercise,
                            None,
                        )
                    } else {
                        let leg = sim::fx_leg(i, args.manual_every);
                        (
                            leg.pair.to_string(),
                            leg.option_type,
                            leg.strike,
                            leg.expiry_years,
                            leg.settlement,
                            leg.exercise,
                            Some(leg.kind),
                        )
                    };
                let is_manual = matches!(
                    kind,
                    Some(sim::FxLegKind::ManualAmerican)
                        | Some(sim::FxLegKind::ManualNonDeliverable)
                );
                // Lift (execute + book) an auto-quote every `--lift-every`-th cycle; a
                // desk-routed manual leg has no quote to lift, so it is never lifted.
                let should_lift =
                    !is_manual && args.lift_every > 0 && (i + 1).is_multiple_of(args.lift_every);
                // Only override the session policy on the stream path; a one-shot keeps
                // the policy the `--side` flag constructed the initiator with.
                if args.repeat != 1 {
                    sess.set_policy(if should_lift {
                        LiftPolicy::LiftOffer
                    } else {
                        LiftPolicy::Observe
                    });
                }

                let symbol = pair.clone().into_bytes();
                let strike_ccy = sim::strike_ccy_of(&pair).as_bytes().to_vec();
                let params = QuoteRequestParams {
                    quote_req_id: &req_id,
                    symbol: &symbol,
                    option_type,
                    strike,
                    expiry_years,
                    settlement,
                    exercise,
                    strike_ccy: &strike_ccy,
                };
                let r = sess
                    .request(&sending_time, |hdr, enc| {
                        dialect_fx::build_quote_request_with_party(hdr, &params, party_id, enc)
                    })
                    .await?;
                let type_label = match option_type {
                    OptionType::Call => "call",
                    OptionType::Put => "put",
                };
                match (r.bid, r.offer) {
                    (Some(bid), Some(offer)) => {
                        println!(
                            "[{i}] {cpty_tag}FX {pair} {type_label} K={strike} {expiry_years:.4}y — auto-quoted:"
                        );
                        print_quote(r.quote_id.as_deref(), bid, offer);
                    }
                    // A one-shot with no quote is an error; a stream notes the disposition.
                    _ if args.repeat == 1 => {
                        eprintln!("✗ no quote returned (the venue declined or the session closed)");
                        std::process::exit(1);
                    }
                    // No quote = the venue could not auto-price and routed this RFQ to the
                    // FX desk (expected for a manual variant): an American exercise or a
                    // non-deliverable request on a deliverable major.
                    _ => match kind.and_then(sim::FxLegKind::manual_reason) {
                        Some(reason) => println!(
                            "[{i}] {cpty_tag}FX {pair} {type_label} — ✓ submitted to the FX desk (manual: {reason})"
                        ),
                        None => {
                            println!("[{i}] FX {pair} {type_label} — no quote (venue declined)")
                        }
                    },
                }
                if should_lift {
                    if r.filled {
                        let px = r.fill_px.unwrap_or(f64::NAN);
                        println!("[{i}] ✓ FX auto-quote LIFTED & FILLED @ {px:.8} — deal booked");
                    } else if r.bid.is_some() {
                        println!("[{i}] ✗ FX lift NOT filled (last-look declined / expired)");
                    }
                }
                r
            }
        };

        // Auto-accept/trade visibility: when the lift policy trades, report the fill.
        if !matches!(args.policy, LiftPolicy::Observe) && result.bid.is_some() {
            if result.filled {
                let px = result.fill_px.unwrap_or(f64::NAN);
                println!("[{i}] ✓ auto-accepted & FILLED @ {px:.8}");
            } else {
                println!("[{i}] ✗ NOT FILLED (last-look declined or quote expired)");
            }
        }

        i += 1;
        if !forever && i >= args.repeat {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(args.interval_ms)).await;
    }

    Ok(())
}
