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
//!   * **esp** — the streaming ESP lifecycle ([`run_esp`]): connect to the server's
//!     reference-data service (`AuthService` over gRPC, `--grpc-addr`), download the top-N
//!     most-liquid/relevant instruments (`--esp-instruments`, default 15), then open ONE
//!     FIX session and stream **bond RFS** on them — which the venue prices off the
//!     aggregated-book composite, tiered by the connection's pricing group — while randomly
//!     LIFTING some (seeded by `--seed`) to book live streaming deals into the blotter,
//!     stamping a rotated counterparty per request. This is where the composite-based +
//!     tiered outbound stream and the "streaming deals" show up end-to-end.
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
//!   --notional AMOUNT|mix  notional (> 0), or `mix` to rotate 100k…30m per stream
//!                          request (one-shot uses the fixed value)   (default 10000000)
//!   --side pay|receive|two-way|mix  fixed-leg intent; `mix` (and absent) rotate
//!                          pay/receive per stream request → a BUY/SELL booked-deal mix;
//!                          a one-shot uses the fixed side       (stream default: mix)
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
use celnet_proto::auth_service_client::AuthServiceClient;
use celnet_proto::{
    AccrualBasis, BrokenDate, ListInstrumentsRequest, LoginRequest, PaymentFrequency, Side,
    instrument_def_desc,
};
use celnet_types::{OptionType, Settlement};
use tokio::net::TcpStream;

/// The asset class the client drives — one session, several dialects/lifecycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetClass {
    /// A fixed-income OIS rates RFQ (the [`dialect_rates`] vocabulary).
    FixedIncome,
    /// A single-leg FX-option RFQ (the [`dialect_fx`] vocabulary).
    FxOption,
    /// The **streaming ESP** lifecycle: connect, download the top-N reference-data
    /// instruments from the server's reference-data service, then subscribe/stream bond
    /// RFS prices on them (which the venue prices off the aggregated-book composite,
    /// tiered by the connection's pricing group) AND randomly lift some — booking live
    /// streaming deals. See [`run_esp`].
    Esp,
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
    // When set, a stream (`--repeat != 1`) rotates the notional per request over the
    // deterministic `sim::rates_notional_for` ladder (100k … 30m) instead of using the
    // fixed `notional`; a one-shot still uses the explicit `notional`. Selected by
    // `--notional mix`. Gives booked deals (and their DV01) a realistic size spread.
    notional_mix: bool,
    rates_side: RatesSide,
    // When set, a stream (`--repeat != 1`) rotates the fixed-leg side per request over the
    // deterministic `sim::rates_side_for` rotation (pay/receive) instead of using the fixed
    // `rates_side`; a one-shot still uses the explicit `rates_side`. Selected by `--side mix`
    // (and the default when no `--side` is given). Gives the blotter a BUY/SELL deal mix.
    rates_side_mix: bool,
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
    // ESP-mode (`--asset esp`) controls.
    /// The gRPC endpoint of the server's reference-data service (`AuthService`), used to
    /// log on and download the top-N instruments to stream. Default `http://127.0.0.1:50051`.
    grpc_addr: String,
    /// How many of the most-liquid/relevant reference-data instruments to download and
    /// stream in ESP mode (`--asset esp`). Default [`DEFAULT_ESP_INSTRUMENTS`].
    esp_instruments: usize,
    /// Service login email/password for the ESP reference-data download (`AuthService.Login`).
    /// Default the seeded admin so it authenticates out-of-the-box on a fresh box.
    user: String,
    password: String,
    /// Deterministic seed for the ESP random-trade selection (which streamed instruments
    /// get lifted) — the same seed replays the same trade decisions.
    seed: u64,
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
         --asset fi|fx|esp (fi)   --addr HOST:PORT (127.0.0.1:9099)\n  \
         fi:  --curve USD-OIS  --tenor 5  --notional 10000000|mix  --side pay|receive|two-way|mix\n  \
         fx:  --pair EURUSD  --type call|put  --strike 1.10  --expiry-years 1.0\n       \
         --side observe|buy|sell  --settlement deliverable|ndf  --exercise european|american\n  \
         esp: --grpc-addr http://127.0.0.1:50051  --esp-instruments 15  --user admin@celnet.com  --password ****  --seed 0x5EED1234\n  \
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

/// Default number of top reference-data instruments the ESP client downloads + streams.
const DEFAULT_ESP_INSTRUMENTS: usize = 15;

/// Default ESP login (the seeded admin), so the reference-data download authenticates
/// out-of-the-box on a fresh box. Override with `--user` / `--password`.
const DEFAULT_ESP_USER: &str = "admin@celnet.com";
const DEFAULT_ESP_PASSWORD: &str = "password";

/// Default deterministic seed for the ESP random-trade selection.
const DEFAULT_ESP_SEED: u64 = 0x5EED_1234;

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
    // `--notional mix` selects the per-request notional rotation on a stream (backward
    // compatible: a number keeps the fixed notional; a one-shot always uses the fixed value).
    let mut notional_mix = false;
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
    let mut grpc_addr = String::from("http://127.0.0.1:50051");
    let mut esp_instruments = DEFAULT_ESP_INSTRUMENTS;
    let mut user = String::from(DEFAULT_ESP_USER);
    let mut password = String::from(DEFAULT_ESP_PASSWORD);
    let mut seed = DEFAULT_ESP_SEED;
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
                    "esp" | "stream" | "streaming" => AssetClass::Esp,
                    other => usage_and_exit(&format!("--asset must be fi|fx|esp, got `{other}`")),
                }
            }
            "--grpc-addr" => grpc_addr = val,
            "--esp-instruments" => {
                esp_instruments = val.parse().unwrap_or_else(|_| {
                    usage_and_exit("--esp-instruments must be a whole number (>= 1)")
                })
            }
            "--user" => user = val,
            "--password" => password = val,
            "--seed" => {
                seed = val
                    .parse()
                    .unwrap_or_else(|_| usage_and_exit("--seed must be a whole number"))
            }
            "--curve" => curve = val.to_uppercase(),
            "--tenor" => {
                tenor_years = val
                    .parse()
                    .unwrap_or_else(|_| usage_and_exit("--tenor must be a whole number of years"))
            }
            "--notional" => match val.to_lowercase().as_str() {
                // A "mix" sentinel selects the deterministic per-request notional rotation
                // (stream only); the numeric `notional` stays at its default as the one-shot
                // fallback. Consistent with `--side mix`.
                "mix" | "rotate" | "mixed" | "random" => notional_mix = true,
                _ => {
                    notional = val
                        .parse()
                        .unwrap_or_else(|_| usage_and_exit("--notional must be a number or `mix`"))
                }
            },
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
    let mut rates_side_mix = false;
    let mut policy = LiftPolicy::Observe;
    match asset {
        AssetClass::FixedIncome => {
            // A stream (`--repeat != 1`) rotates the side per request when in "mix" mode,
            // giving booked deals a realistic BUY/SELL spread; an explicit `pay`/`receive`/
            // `two-way` is honoured exactly (backward compatible), and a one-shot always uses
            // the fixed `rates_side`. No `--side` at all defaults to the mix (the common case:
            // a bare `--asset fi` stream should show a mixture, not one direction). The base
            // `rates_side` doubles as the one-shot fallback when mix is selected (two-way).
            rates_side = match side_raw.as_deref() {
                None => {
                    rates_side_mix = true;
                    RatesSide::TwoWay
                }
                Some(s) => match s.to_lowercase().as_str() {
                    "pay" | "payfixed" | "pay-fixed" => RatesSide::PayFixed,
                    "receive" | "rec" | "receivefixed" | "receive-fixed" => RatesSide::ReceiveFixed,
                    "two-way" | "twoway" | "2way" | "rfq" => RatesSide::TwoWay,
                    "mix" | "mixed" | "rotate" | "both" => {
                        rates_side_mix = true;
                        RatesSide::TwoWay
                    }
                    other => usage_and_exit(&format!(
                        "--side (fi) must be pay|receive|two-way|mix, got `{other}`"
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
        // ESP mode drives its own lifecycle (`run_esp`) — `--side` does not apply.
        AssetClass::Esp => {}
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
        AssetClass::Esp => {
            if esp_instruments < 1 {
                usage_and_exit("--esp-instruments must be >= 1");
            }
            if !(notional > 0.0 && notional.is_finite()) {
                usage_and_exit("--notional must be positive");
            }
            if grpc_addr.trim().is_empty() {
                usage_and_exit("--grpc-addr must be a non-empty gRPC endpoint");
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
        notional_mix,
        rates_side,
        rates_side_mix,
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
        grpc_addr,
        esp_instruments,
        user,
        password,
        seed,
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

    // ESP mode drives its own connect → download-refdata → stream + random-trade lifecycle.
    if args.asset == AssetClass::Esp {
        return run_esp(&args).await;
    }

    println!("── celnet FIX RFQ client ─────────────────────────────");
    println!(
        "  connect   {} as {} → {}",
        args.addr, args.sender, args.target
    );
    match args.asset {
        AssetClass::FixedIncome => {
            let side_label = if args.rates_side_mix {
                "mix (rotating pay/receive → BUY/SELL)"
            } else {
                match args.rates_side {
                    RatesSide::PayFixed => "pay fixed",
                    RatesSide::ReceiveFixed => "receive fixed",
                    RatesSide::TwoWay => "two-way",
                }
            };
            let notional_label = if args.notional_mix {
                "mix (100k…30m)".to_string()
            } else {
                format!("{:.2}", args.notional)
            };
            println!(
                "  RFQ (fi)  {} OIS {}y · notional {} · {}",
                args.curve, args.tenor_years, notional_label, side_label
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
        // ESP returns early (`run_esp`) before this header prints.
        AssetClass::Esp => unreachable!("ESP mode is handled by run_esp"),
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
                // Lift (execute + book) every `--lift-every`-th LIFTABLE (auto-quoted,
                // non-manual) request — keyed off the auto-quote ORDINAL (cycle index minus
                // the desk-routed manual requests seen so far), NOT the raw cycle index. A
                // manual (desk-routed) tenor has no quote to lift, so it is never lifted.
                // Keying off `i` made `!is_manual && (i+1)%N == 0` an unsatisfiable
                // contradiction whenever `--lift-every` and `--manual-every` shared a period
                // (e.g. both 3 — the launcher default): the desk then quoted but booked ZERO
                // deals. The ordinal decouples the two cadences so lifts always occur.
                let manual_seen = (i + 1).checked_div(args.manual_every).unwrap_or(0);
                let auto_ordinal = (i + 1) - manual_seen;
                let should_lift = !is_manual
                    && args.lift_every > 0
                    && auto_ordinal.is_multiple_of(args.lift_every);

                // In a stream (`--repeat != 1`) rotate the fixed-leg side and the notional
                // per request when in "mix" mode, so booked OIS deals show a realistic
                // BUY/SELL direction spread and a varied size/DV01 on the risk dashboard; a
                // one-shot honours the explicit `--side` / `--notional` exactly (backward
                // compatible). The booked `Deal.side` is carried by the RFQ side field
                // (server `rates_side_to_side`): PayFixed → BUY, ReceiveFixed → SELL.
                let effective_side = if args.repeat != 1 && args.rates_side_mix {
                    sim::rates_side_for(i)
                } else {
                    args.rates_side
                };
                let effective_notional = if args.repeat != 1 && args.notional_mix {
                    sim::rates_notional_for(i)
                } else {
                    args.notional
                };

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
                        notional: effective_notional,
                        side: effective_side,
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
                        notional: effective_notional,
                        side: effective_side,
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
                // Lift (execute + book) every `--lift-every`-th LIFTABLE (auto-quoted,
                // non-manual) request — keyed off the auto-quote ORDINAL (cycle index minus
                // the desk-routed manual legs injected so far), NOT the raw cycle index; a
                // desk-routed manual leg has no quote to lift, so it is never lifted. Keying
                // off `i` made `!is_manual && (i+1)%N == 0` unsatisfiable whenever
                // `--lift-every` and `--manual-every` shared a period, booking ZERO deals.
                let manual_seen = (i + 1).checked_div(args.manual_every).unwrap_or(0);
                let auto_ordinal = (i + 1) - manual_seen;
                let should_lift = !is_manual
                    && args.lift_every > 0
                    && auto_ordinal.is_multiple_of(args.lift_every);
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
            // ESP returns early (`run_esp`) before this loop is reached.
            AssetClass::Esp => unreachable!("ESP mode is handled by run_esp"),
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

// ===========================================================================
// ESP streaming lifecycle (`--asset esp`)
// ===========================================================================

/// A tiny deterministic PRNG (SplitMix64) — seeded, reproducible, no external dependency.
/// The ESP random-trade selection draws from this so a run with a given `--seed` replays
/// the same lift/observe decisions (gate-testable), matching this module's determinism
/// ethos while giving genuinely varied, non-periodic trade timing.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A bounded roll in `[0, n)` (`0` when `n == 0`).
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
}

/// One downloaded bond the ESP client streams — the reference-data fields the FIX bond
/// dialect needs on the wire, resolved from the server's [`instrument_def_desc::Definition`].
struct EspBond {
    instrument_id: String,
    name: String,
    coupon_rate: f64,
    coupon_frequency: PaymentFrequency,
    day_count: AccrualBasis,
    maturity: BrokenDate,
    redemption: f64,
}

/// Map a reference-data `coupon_frequency` label onto the FIX dialect's [`PaymentFrequency`].
/// A blank/`zero` label (zero-coupon) defaults to semi-annual, which the wire encodes as the
/// standard 2/yr — the coupon itself is `0`, so the frequency is inert for a zero.
fn esp_frequency_from_label(label: &str) -> PaymentFrequency {
    match label.trim().to_ascii_lowercase().as_str() {
        "annual" => PaymentFrequency::Annual,
        "quarterly" => PaymentFrequency::Quarterly,
        _ => PaymentFrequency::SemiAnnual,
    }
}

/// Map a reference-data `day_count` label onto the FIX dialect's [`AccrualBasis`]. The bond
/// dialect wire supports Act/360, Act/365F and 30/360; an `act_act` govvie basis (not on the
/// wire enum) maps to 30/360 — the RFQ still prices and streams, and the composite two-way
/// (what ESP demonstrates) is resolved by symbol, independent of the accrual basis.
fn esp_day_count_from_label(label: &str) -> AccrualBasis {
    match label.trim().to_ascii_lowercase().as_str() {
        "act_360" | "act360" => AccrualBasis::Act360,
        "act_365_fixed" | "act365fixed" | "act_365f" | "act365f" => AccrualBasis::Act365Fixed,
        _ => AccrualBasis::Thirty360BondBasis,
    }
}

/// Download the top-N streamable instruments from the server's reference-data service:
/// authenticate (`AuthService.Login`), `ListInstruments`, and keep the bond-family
/// definitions (with a resolvable maturity) in registry order — the curated benchmark
/// universe the platform actually makes markets in and the LP feed streams, i.e. the most
/// liquid/relevant set, NOT a hardcoded client list — capped at `--esp-instruments`.
async fn download_top_bonds(args: &Args) -> Result<Vec<EspBond>, String> {
    let channel = tonic::transport::Channel::from_shared(args.grpc_addr.clone())
        .map_err(|e| format!("bad --grpc-addr `{}`: {e}", args.grpc_addr))?
        .connect()
        .await
        .map_err(|e| format!("connect {}: {e}", args.grpc_addr))?;
    let mut auth = AuthServiceClient::new(channel);
    let token = auth
        .login(LoginRequest {
            email: args.user.clone(),
            password: args.password.clone(),
            correlation_id: None,
        })
        .await
        .map_err(|e| format!("login as {}: {}", args.user, e.message()))?
        .into_inner()
        .session_token;
    let resp = auth
        .list_instruments(ListInstrumentsRequest {
            session_token: token,
            correlation_id: None,
        })
        .await
        .map_err(|e| format!("list_instruments: {}", e.message()))?
        .into_inner();

    let mut out: Vec<EspBond> = Vec::new();
    for desc in resp.instruments {
        let Some(instrument_def_desc::Definition::Bond(bond)) = desc.definition else {
            continue;
        };
        let Some(maturity) = bond.maturity_date else {
            continue;
        };
        let name = if desc.name.trim().is_empty() {
            desc.instrument_id.clone()
        } else {
            desc.name.clone()
        };
        out.push(EspBond {
            instrument_id: desc.instrument_id,
            name,
            coupon_rate: bond.coupon_rate,
            coupon_frequency: esp_frequency_from_label(&bond.coupon_frequency),
            day_count: esp_day_count_from_label(&bond.day_count),
            maturity,
            redemption: if bond.redemption > 0.0 {
                bond.redemption
            } else {
                100.0
            },
        });
        if out.len() >= args.esp_instruments {
            break;
        }
    }
    Ok(out)
}

/// The streaming **ESP** lifecycle (`--asset esp`): connect to the server's reference-data
/// service, download the top-N most-liquid/relevant instruments, then open ONE FIX session
/// and stream bond RFS on them — which the venue prices off the aggregated-book composite,
/// tiered by this connection's pricing group (the composite+tiered outbound seam) — while
/// randomly LIFTING some to book live streaming deals into the blotter. Deterministic under
/// `--seed`: the same seed replays the same trade decisions and instrument rotation.
async fn run_esp(args: &Args) -> std::io::Result<()> {
    println!("── celnet ESP streaming client ───────────────────────");
    println!("  refdata   {} as {}", args.grpc_addr, args.user);
    println!(
        "  stream    FIX {} as {} → {}",
        args.addr, args.sender, args.target
    );
    println!(
        "  top-N     {}  ·  cadence {}ms  ·  hold {}ms  ·  seed {:#x}",
        args.esp_instruments, args.interval_ms, args.stream_hold_ms, args.seed
    );

    // 1) Download the top-N instruments from the reference-data service.
    let bonds = match download_top_bonds(args).await {
        Ok(b) if !b.is_empty() => b,
        Ok(_) => {
            eprintln!(
                "✗ reference-data at {} returned no streamable bonds",
                args.grpc_addr
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("✗ reference-data download failed: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "  loaded    {} instrument(s): {}",
        bonds.len(),
        bonds
            .iter()
            .map(|b| b.instrument_id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("──────────────────────────────────────────────────────");

    // 2) Open the FIX session ONCE and stream RFS over it (no logon/logout churn).
    let tcp = match TcpStream::connect(&args.addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "✗ could not connect FIX {} — is the edge up with a fixed-income STREAM \
                 acceptor? ({e})",
                args.addr
            );
            std::process::exit(1);
        }
    };
    tcp.set_nodelay(true).ok();
    let cfg = SessionConfig {
        sender: args.sender.clone().into_bytes(),
        target: args.target.clone().into_bytes(),
        heart_bt_int: 30,
        role: Role::Initiator,
    };
    let session = Session::new(cfg, InMemoryStore::new());
    let mut initiator = Initiator::new(session, LiftPolicy::Observe);
    let mut sess = initiator.open(tcp, fix_utc_timestamp().as_bytes()).await?;

    // 3) Stream + randomly trade. Each cycle streams one instrument's RFS (round-robin over
    // the downloaded set, so all N build up live server-side subscriptions) and lifts ~1 in 3
    // cycles (seeded) — a lift books a streaming deal into the blotter via the venue's bond
    // RFS lift path. A stable per-instrument QuoteReqID REPLACES that instrument's one live
    // stream on each re-subscribe (keyed by QuoteReqID) instead of piling up.
    let mut rng = SplitMix64::new(args.seed);
    let hold = std::time::Duration::from_millis(args.stream_hold_ms.max(1));
    let forever = args.repeat == 0;
    let mut i: u64 = 0;
    loop {
        let bond = &bonds[(i as usize) % bonds.len()];
        let counterparty = sim::counterparty_for(i);
        // In "mix" notional mode rotate the streamed size per request (ESP always streams),
        // so booked streaming bond deals — and their DV01 — show a realistic size spread
        // instead of one repeated clip; otherwise use the fixed `--notional`.
        let esp_notional = if args.notional_mix {
            sim::rates_notional_for(i)
        } else {
            args.notional
        };
        // Randomly (seeded) LIFT ~1 in 3 cycles; a lift executes + books a streaming deal.
        let should_lift = rng.below(3) == 0;
        // Vary the lift DIRECTION deterministically per index so booked streaming deals show
        // a BUY/SELL mix (LiftOffer books BUY off the offer leg, HitBid books SELL off the
        // bid leg), mirroring the RFQ side rotation; the two-way bond RFS mints both legs so
        // either direction books symmetrically.
        let lift_policy = if should_lift {
            match sim::rates_side_for(i) {
                RatesSide::PayFixed => LiftPolicy::LiftOffer,
                RatesSide::ReceiveFixed | RatesSide::TwoWay => LiftPolicy::HitBid,
            }
        } else {
            LiftPolicy::Observe
        };
        sess.set_policy(lift_policy);

        let stream_req_id = format!("{}-ESP-{}", args.req_id, bond.instrument_id).into_bytes();
        let symbol = bond.instrument_id.clone().into_bytes();
        let params = dialect_rates::BondQuoteRequestParams {
            quote_req_id: &stream_req_id,
            symbol: &symbol,
            coupon_rate: bond.coupon_rate,
            coupon_frequency: bond.coupon_frequency,
            day_count: bond.day_count,
            maturity: bond.maturity,
            redemption: bond.redemption,
            notional: esp_notional,
            side: Side::TwoWay,
            subscription: SubscriptionRequest::Subscribe,
        };
        let party = counterparty.as_bytes();
        let sending_time = fix_utc_timestamp().into_bytes();
        let lift_after = should_lift.then(|| hold / 2);
        let outcome = sess
            .stream(
                &sending_time,
                |hdr, enc| {
                    dialect_rates::build_bond_quote_request_with_party(
                        hdr,
                        &params,
                        Some(party),
                        enc,
                    )
                },
                hold,
                lift_after,
            )
            .await?;
        println!(
            "[{i}] {counterparty} · {} ({}) ESP — streamed {} update(s)",
            bond.name, bond.instrument_id, outcome.updates
        );
        if let (Some(bid), Some(offer)) = (outcome.result.bid, outcome.result.offer) {
            print_quote(outcome.result.quote_id.as_deref(), bid, offer);
        }
        if should_lift {
            if outcome.result.filled {
                let px = outcome.result.fill_px.unwrap_or(f64::NAN);
                println!("[{i}] ✓ streamed quote LIFTED & FILLED @ {px:.8} — deal booked");
            } else {
                println!("[{i}] ✗ stream lift NOT filled (last-look declined / no auto-quote yet)");
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The seeded PRNG is deterministic (same seed ⇒ same sequence) and its bounded roll
    /// stays in range — so the ESP random-trade selection replays identically under a seed.
    #[test]
    fn splitmix64_is_deterministic_and_bounded() {
        let mut a = SplitMix64::new(0x5EED_1234);
        let mut b = SplitMix64::new(0x5EED_1234);
        for _ in 0..256 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut r = SplitMix64::new(7);
        for _ in 0..1_000 {
            assert!(r.below(3) < 3);
        }
        // A different seed yields a different stream (with overwhelming probability).
        assert_ne!(SplitMix64::new(1).next_u64(), SplitMix64::new(2).next_u64());
    }

    /// The reference-data label mappers resolve the govvie/registry labels onto the FIX
    /// bond dialect enums, defaulting unknown/blank labels to the standard USD conventions
    /// (semi-annual, 30/360) so a downloaded bond always encodes to a valid wire descriptor.
    #[test]
    fn esp_label_mappers_cover_the_registry_labels() {
        assert_eq!(esp_frequency_from_label("annual"), PaymentFrequency::Annual);
        assert_eq!(
            esp_frequency_from_label("semi_annual"),
            PaymentFrequency::SemiAnnual
        );
        assert_eq!(
            esp_frequency_from_label("quarterly"),
            PaymentFrequency::Quarterly
        );
        assert_eq!(esp_frequency_from_label(""), PaymentFrequency::SemiAnnual);

        assert_eq!(
            esp_day_count_from_label("act_365_fixed"),
            AccrualBasis::Act365Fixed
        );
        assert_eq!(esp_day_count_from_label("act_360"), AccrualBasis::Act360);
        // An act/act govvie basis (not on the wire enum) maps to 30/360; blanks likewise.
        assert_eq!(
            esp_day_count_from_label("act_act"),
            AccrualBasis::Thirty360BondBasis
        );
        assert_eq!(
            esp_day_count_from_label(""),
            AccrualBasis::Thirty360BondBasis
        );
    }
}
