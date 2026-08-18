//! `cme-sim` — the runnable **listed Treasury-futures venue**.
//!
//! Loads the committed listed-futures universe, anchors every contract to the real
//! cash Treasury curve, and either publishes whole-contract tick-aligned two-ways
//! into a running celnet server's `LpFeed` ingest (network mode) or prints them
//! locally (demo mode). In network mode it simultaneously binds a FIX order
//! acceptor, so a taker can actually trade against the prices it shows.
//!
//! It is a first-class peer of `lp-sim`: same flag style, same self-contained
//! embedded universe, supervised by `deploy/start-cme-sim.sh` / the `cmesimctl`
//! control script exactly as the OTC simulator is by `lpsimctl`.

use std::time::Duration;

use celnet_cme_sim::{
    FuturesFeedOptions, VENUE_ID, as_of, build_order_venue, build_venue_feed, contract_lot_size,
    futures_lines, load_futures_universe, publish_markets, resolve_contract, run_futures_feed,
    venue,
};
use celnet_lp_sim::VenueFeed;
use clap::Parser;

/// The runnable listed Treasury-futures venue.
#[derive(Debug, Parser)]
#[command(
    name = "cme-sim",
    about = "cme-sim — simulated listed Treasury-futures venue (quotes + accepts orders)",
    long_about = "cme-sim — a single simulated listed venue (connection id `cme-sim`) that \
quotes the Treasury futures complex (ZT/ZF/ZN/TN/ZB/UB) from the reference registry's REAL \
contract terms — face value, minimum price increment, tick value, notional coupon, delivery \
dates, derived DV01 per contract — anchored to the real cash Treasury curve, and ACCEPTS \
ORDERS against those quotes over FIX, answering each with a real ExecutionReport(8).\n\n\
DEPLOYED MODE:\n  \
  cme-sim --addr http://127.0.0.1:50051 --order-port 5710 --interval 2\n  \
  Publishes into LiquidityFeedService.LpFeed under the `cme-sim` connection id and binds \
the FIX order acceptor. An aggregated book that carries Treasury futures MUST list \
`cme-sim` in its member_connection_ids.\n\n\
LOCAL DEMO — no --addr:\n  \
  Prints the venue's two-way for every contract each interval.\n\n\
SYMBOL RESOLUTION — --resolve <SYMBOL>:\n  \
  Print which contract a symbol resolves to. A PRODUCT symbol (`ZF`) rolls to its front \
delivery month; an explicit delivery month (`ZFU26`) is never re-pointed."
)]
struct Args {
    /// Seconds between quote rounds.
    #[arg(long, default_value_t = 2)]
    interval: u64,

    /// Sub-second override of `--interval`, in **milliseconds** (`0` ⇒ use
    /// `--interval` seconds).
    #[arg(long, default_value_t = 0)]
    interval_ms: u64,

    /// Root seed — the same seed reproduces the venue byte-for-byte.
    #[arg(long, default_value_t = 0x1234_5678)]
    seed: u64,

    /// Settlement/valuation date: the instant the listed universe is filtered and a
    /// product symbol is rolled against, so a contract that has stopped trading is never
    /// quoted. `today` (the default) tracks the current UTC date; pin a fixed
    /// `YYYY-MM-DD` for a byte-reproducible replay.
    ///
    /// A pinned default is what let this venue keep quoting an expired delivery month —
    /// the roll is evaluated against THIS date, not the wall clock. See
    /// `universe::resolve_settlement`.
    #[arg(long, default_value = celnet_lp_sim::universe::SETTLEMENT_TODAY)]
    settlement: String,

    /// Network mode: the gRPC endpoint of a running celnet server. When set, the
    /// venue publishes `LpQuote`s into `LiquidityFeedService.LpFeed` instead of
    /// printing locally.
    #[arg(long)]
    addr: Option<String>,

    /// The `host:port` the FIX order acceptor binds. Without it the venue publishes
    /// prices it cannot be hit on, which is a price display rather than a venue —
    /// the deployed configuration always sets this.
    #[arg(long)]
    order_port: Option<String>,

    /// Publish/print a single round and exit.
    #[arg(long, default_value_t = false)]
    once: bool,

    /// Resolve a symbol to a contract and exit — a product symbol (`ZF`) rolls to
    /// its front delivery month, an explicit month (`ZFU26`) is matched verbatim.
    #[arg(long)]
    resolve: Option<String>,
}

impl Args {
    /// The effective quote cadence: `--interval-ms` when set, else `--interval`
    /// whole seconds; floored to 1 ms so the loop always advances.
    fn quote_interval(&self) -> Duration {
        if self.interval_ms > 0 {
            Duration::from_millis(self.interval_ms.max(1))
        } else {
            Duration::from_secs(self.interval.max(1))
        }
    }
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();

    let Some(settlement) = celnet_lp_sim::universe::resolve_settlement(&args.settlement) else {
        eprintln!(
            "[cme-sim] ERROR: --settlement must be YYYY-MM-DD, got {:?}",
            args.settlement
        );
        return std::process::ExitCode::from(2);
    };

    // The cash curve the notional deliverables are anchored to, then the listed
    // contracts themselves. A contract with no usable anchor is DROPPED, never
    // given a fabricated level.
    let bonds = celnet_lp_sim::load_government_universe(false);
    let contracts = load_futures_universe(&bonds, settlement);
    if contracts.is_empty() {
        eprintln!(
            "[cme-sim] ERROR: no listed contract is quotable at settlement {} \
             (no cash curve to anchor to)",
            args.settlement
        );
        return std::process::ExitCode::from(1);
    }

    // --- Symbol resolution probe -------------------------------------------------
    if let Some(symbol) = &args.resolve {
        return match resolve_contract(&contracts, symbol, as_of(settlement)) {
            Some((c, how)) => {
                let lot = contract_lot_size(c).unwrap_or_default();
                println!(
                    "{symbol} -> {} ({:?}); face {lot}, tick {}, DV01/contract {:.2}",
                    c.instrument_id(),
                    how,
                    c.spec.terms.tick_size_points,
                    c.dv01_per_contract().unwrap_or_default(),
                );
                std::process::ExitCode::SUCCESS
            }
            None => {
                eprintln!(
                    "[cme-sim] {symbol} resolves to no live contract at {} \
                     (not a listed product, and not a trading delivery month)",
                    args.settlement
                );
                std::process::ExitCode::from(1)
            }
        };
    }

    let lines = futures_lines(
        &contracts,
        venue::BASE_HALF_SPREAD,
        venue::BASE_SKEW_STEP,
        REVERSION_PER_SEC,
        PERTURBATION,
    );
    eprintln!(
        "[cme-sim] venue '{VENUE_ID}': {} listed contract(s) quotable at {}, seed {:#x}",
        lines.len(),
        args.settlement,
        args.seed,
    );
    if lines.len() < contracts.len() {
        eprintln!(
            "[cme-sim] WARNING: {} contract(s) loaded but not quotable — they are NOT advertised",
            contracts.len() - lines.len()
        );
    }

    // --- Network mode: publish + accept orders -----------------------------------
    if let Some(addr) = &args.addr {
        let opts = FuturesFeedOptions {
            quote_interval: args.quote_interval(),
            order_bind: args.order_port.clone(),
            seed: args.seed,
            once: args.once,
        };
        eprintln!(
            "[cme-sim] network mode -> {addr} (orders: {})",
            args.order_port.as_deref().unwrap_or("DISABLED")
        );
        return match run_futures_feed(&lines, &contracts, addr, &opts) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("[cme-sim] ERROR: feed failed: {e}");
                std::process::ExitCode::from(1)
            }
        };
    }

    // --- Local demo: print the venue's two-way each round ------------------------
    let feed = build_venue_feed(&lines, args.seed);
    let markets = celnet_lp_sim::orders::LiveMarkets::default();
    let order_venue = build_order_venue(
        celnet_lp_sim::orders::LiveMarkets::clone(&markets),
        args.seed,
    );
    let mut round: u64 = 0;
    loop {
        let now = celnet_lp_sim::orders::wall_nanos();
        publish_markets(&feed, &order_venue, &lines, &contracts, now);
        println!("--- {VENUE_ID} listed market (round {round}) ---");
        for line in &lines {
            let Some(q) = feed.top_of_book(&line.instrument, now) else {
                println!("  {} [{}] no market", line.display_name, line.identity);
                continue;
            };
            let lot = contracts
                .iter()
                .find(|c| c.instrument_id() == line.instrument_id)
                .and_then(contract_lot_size)
                .unwrap_or(1.0);
            println!(
                "  {name} [{id}]  bid {bid:.6} x{bc:.0}ct  offer {offer:.6} x{oc:.0}ct  \
                 tick {tick}",
                name = line.display_name,
                id = line.instrument_id,
                bid = q.bid,
                bc = q.bid_size / lot,
                offer = q.offer,
                oc = q.offer_size / lot,
                tick = line.tick.unwrap_or_default(),
            );
        }
        round += 1;
        if args.once {
            return std::process::ExitCode::SUCCESS;
        }
        std::thread::sleep(args.quote_interval());
    }
}

/// Mean-reversion speed of the anchor yield process, per second. The same value the
/// OTC simulator's cash feed uses, so the futures strip and the cash strip move
/// consistently rather than on two unrelated clocks.
const REVERSION_PER_SEC: f64 = 0.02;
/// Yield jitter amplitude (decimal yield) — ±3 bp, matching the cash feed.
const PERTURBATION: f64 = 3.0e-4;

#[cfg(test)]
mod tests {
    use super::*;

    /// The cadence override is sub-second capable and always floors to a positive
    /// interval, so the publish loop can never spin at zero.
    #[test]
    fn the_quote_cadence_is_sub_second_capable_and_never_zero() {
        let base = Args::parse_from(["cme-sim", "--interval", "3"]);
        assert_eq!(base.quote_interval(), Duration::from_secs(3));

        let fast = Args::parse_from(["cme-sim", "--interval", "3", "--interval-ms", "150"]);
        assert_eq!(fast.quote_interval(), Duration::from_millis(150));

        let zero = Args::parse_from(["cme-sim", "--interval", "0", "--interval-ms", "0"]);
        assert_eq!(zero.quote_interval(), Duration::from_secs(1));
    }

    /// The order acceptor is OPT-IN on the command line but the deployed launcher
    /// always sets it; absent, the binary warns rather than silently publishing
    /// prices nobody can trade on.
    #[test]
    fn the_order_port_is_absent_by_default_and_parsed_when_given() {
        assert_eq!(Args::parse_from(["cme-sim"]).order_port, None);
        assert_eq!(
            Args::parse_from(["cme-sim", "--order-port", "0.0.0.0:5710"]).order_port,
            Some("0.0.0.0:5710".to_owned())
        );
    }
}
