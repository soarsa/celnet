//! `lp-sim` — the runnable **LP-SIM liquidity-provider feed**.
//!
//! Loads the bundled US-Treasury reference universe, stands up the named `LP-SIM`
//! member panel over the selected bonds, and — on an interval, like the FIX
//! simulator — consolidates each bond through the REAL
//! [`celnet_aggregation::ConsolidatedBook`] engine and emits the composite a
//! subscriber to an FI Aggregated Book sees: the bond identity (name + ISIN +
//! CUSIP), the composite best bid/offer + size + confidence, and every
//! contributing `LP-SIM…` connection's price.
//!
//! It is self-contained (the universe is embedded), needs no data file on the host,
//! and is supervised by `deploy/start-lp-sim.sh` / the `lpsimctl` wrapper exactly as
//! the FIX quote simulator is. The CLI mirrors the fix-sim client's flag style.
//!
//! ## Scope note
//!
//! This binary runs the LP fleet **and** the consolidation in one process and emits
//! the composite locally — a complete, runnable demonstration of the LP-SIM feed
//! pushing live Treasury two-ways into an aggregated book. Wiring the same fleet as
//! a network client that pushes `LpQuote`s to a server-side ingest RPC (so the
//! composite surfaces in the GUI) is the P2 server follow-on; the per-LP wire view
//! this binary already computes ([`celnet_lp_sim::LpQuoteSnapshot`]) is exactly the
//! payload that ingest consumes.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use celnet_lp_sim::{
    BookFeedOptions, FaultSchedule, LoginCredentials, LpSimConfig, QuotedLine, bond_lines,
    build_fleet, composite_for, futures_lines, into_feeds, load_futures_universe,
    load_government_universe, run_book_aware_feed,
};
use celnet_types::BrokenDate;
use clap::Parser;

/// The runnable LP-SIM Treasury liquidity feed.
#[derive(Debug, Parser)]
#[command(
    name = "lp-sim",
    about = "LP-SIM — 5-LP, book-aware synthetic Treasury liquidity-provider feed",
    long_about = "LP-SIM — a fleet of 5 synthetic bond liquidity providers \
(LP-SIM-01…LP-SIM-05), each with a distinct seeded pricing character, that push \
oracle-anchored two-way Treasury quotes into a running celnet server's LpFeed ingest.\n\n\
DEPLOYED (book-aware) MODE — the default when --addr is set:\n  \
  lp-sim --addr http://127.0.0.1:50051 --members 5 --book-poll 5 \\\n         \
     --user admin@celnet.com --password ****\n  \
  The feed authenticates (AuthService.Login), polls the enabled aggregated books \
(ListAggregatedBooks) every --book-poll seconds, and for each book resolves which \
LP-SIM-0N members it should impersonate and which instruments to quote \
(all-members-quote ⇒ the full Treasury universe; explicit ⇒ the listed instrument ids). \
It streams exactly those (member × instrument) two-ways, and starts/stops pricing bonds \
automatically as a user creates or edits a book. Books with no LP-SIM member are ignored.\n\n\
BROADCAST FALLBACK — --no-book-poll (or an explicit --instruments list):\n  \
  Streams the whole (or listed) universe as the N LPs without polling books — for when \
no books exist yet.\n\n\
LOCAL DEMO — no --addr (or --local):\n  \
  Consolidates the fleet in-process and prints the composite each interval."
)]
struct Args {
    /// The LP connection name advertised as the price contributor (venue id).
    #[arg(long, default_value = "LP-SIM")]
    lp_name: String,

    /// The aggregated-book id this feed streams into (shown in the header; the
    /// server-side book selects which members it consolidates).
    #[arg(long, default_value = "ust-composite")]
    book: String,

    /// Number of decorrelated LP member connections (≥ 1). The default 5 stands up
    /// `LP-SIM-01`…`LP-SIM-05`, each with a distinct seeded pricing character
    /// (half-spread, size, refresh cadence, quality) so the server's
    /// best-bid=max / best-offer=min consolidation across them is meaningful. ≥ 3
    /// makes the consolidator's divergence gating decidable.
    #[arg(long, default_value_t = 5)]
    members: usize,

    /// Seconds between composite emissions / per-member quote pushes.
    #[arg(long, default_value_t = 2)]
    interval: u64,

    /// Sub-second override of `--interval`, in **milliseconds** (`0` ⇒ use `--interval`
    /// seconds). Lets the LP feed refresh the composite faster than 1s for a livelier
    /// book (the deploy sets it via `LPSIM_PERIOD_MS`).
    #[arg(long, default_value_t = 0)]
    interval_ms: u64,

    /// Which instruments to quote: `all`, or a comma-separated list of ISINs and/or
    /// CUSIPs.
    #[arg(long, default_value = "all")]
    instruments: String,

    /// Cap on the number of instruments streamed (keeps the emitted view readable).
    #[arg(long, default_value_t = 12)]
    max_instruments: usize,

    /// Root seed — the same seed reproduces the feed byte-for-byte.
    #[arg(long, default_value_t = 0x1234_5678)]
    seed: u64,

    /// Settlement/valuation date (YYYY-MM-DD) used to build each bond's reference
    /// schedule and invert its reference yield.
    #[arg(long, default_value = "2026-04-16")]
    settlement: String,

    /// Also include zero-coupon Bills (priced off the reference mid where the yield
    /// solver can bracket them); by default only coupon Notes/Bonds are streamed.
    #[arg(long, default_value_t = false)]
    include_bills: bool,

    /// Do NOT quote the listed Treasury futures complex (2Y/5Y/10Y/Ultra-10Y/Bond/
    /// Ultra-Bond). Futures are quoted by DEFAULT: the server seeds the same
    /// contracts into its tradeable reference registry, and an instrument that is
    /// tradeable but not quotable never reaches an aggregated book — so every
    /// DV01-ratio hedge routed to it would backstop to the synthetic COMPOSITE
    /// venue. Only pass this to isolate the cash feed.
    #[arg(long, default_value_t = false)]
    exclude_futures: bool,

    /// Emit a single round and exit (default: stream forever).
    #[arg(long, default_value_t = false)]
    once: bool,

    /// Network feed mode: the gRPC endpoint of a running celnet server (e.g.
    /// `http://127.0.0.1:50051`). When set, the feed connects and streams `LpQuote`s
    /// to the server's `LiquidityFeedService.LpFeed` ingest — so the consolidated
    /// composite surfaces to the server's GUI subscribers — instead of printing the
    /// composite locally. Absent ⇒ the default in-process local mode.
    #[arg(long)]
    addr: Option<String>,

    /// Force the in-process local mode even if `--addr` is given (prints the
    /// composite locally; the default when `--addr` is absent).
    #[arg(long, default_value_t = false)]
    local: bool,

    /// Book-aware poll cadence in seconds: how often the daemon re-reads the enabled
    /// aggregated books (`ListAggregatedBooks`) and starts/stops instrument×LP streams
    /// as books change. Only used in the network book-aware mode.
    #[arg(long, default_value_t = 5)]
    book_poll: u64,

    /// Disable book polling: stream the selected universe as the N LPs regardless of
    /// any server-side books (the broadcast fallback). Also implied by passing an
    /// explicit `--instruments` list rather than `all`.
    #[arg(long, default_value_t = false)]
    no_book_poll: bool,

    /// Service login email for the book poll (`AuthService.Login`). Falls back to the
    /// `LPSIM_USER` env var, then to the seeded admin (`admin@celnet.com`) so the
    /// daemon authenticates out-of-the-box on a fresh box.
    #[arg(long)]
    user: Option<String>,

    /// Service login password. Falls back to `LPSIM_PASSWORD`, then the seeded admin
    /// password.
    #[arg(long)]
    password: Option<String>,

    /// Disable the occasional injected staleness/outlier faults (network modes emit,
    /// by default, an occasional divergent or stale print on at most one member per
    /// round to exercise the server's MAD gate and staleness decay).
    #[arg(long, default_value_t = false)]
    no_faults: bool,
}

impl Args {
    /// The effective quote/composite cadence: `--interval-ms` when set (sub-second
    /// capable), else `--interval` whole seconds; floored to 1ms so the loop always
    /// advances.
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

    let settlement = match parse_iso_date(&args.settlement) {
        Some(d) => d,
        None => {
            eprintln!(
                "[lp-sim] ERROR: --settlement must be YYYY-MM-DD, got {:?}",
                args.settlement
            );
            return std::process::ExitCode::from(2);
        }
    };

    let cfg = LpSimConfig {
        lp_name: args.lp_name.clone(),
        members: args.members.max(1),
        seed: args.seed,
        settlement,
        ..LpSimConfig::default()
    };

    // Load the full government reference universe (US Treasuries + curated non-US
    // govvies), keep only the bonds the analytics leaf can model at this settlement,
    // and — unless opted out — extend it with the listed Treasury futures complex
    // anchored to that same cash curve. This is the sim's full QUOTABLE set, and it
    // is deliberately the same universe the server seeds its TRADEABLE registry
    // from: anything tradeable but unquotable never reaches an aggregated book.
    let universe = load_government_universe(args.include_bills);
    let mut priceable = bond_lines(
        &universe,
        cfg.settlement,
        cfg.reversion_per_sec,
        cfg.perturbation,
    );
    let futures_count = if args.exclude_futures {
        0
    } else {
        let contracts = load_futures_universe(&universe, cfg.settlement);
        let lines = futures_lines(
            &contracts,
            cfg.half_spread,
            cfg.skew_step,
            cfg.reversion_per_sec,
            cfg.perturbation,
        );
        let n = lines.len();
        priceable.extend(lines);
        n
    };
    eprintln!(
        "[lp-sim] quotable set: {} cash bond(s) + {futures_count} Treasury future(s)",
        priceable.len() - futures_count,
    );
    if priceable.is_empty() {
        eprintln!(
            "[lp-sim] ERROR: no modellable instruments at settlement {}",
            args.settlement
        );
        return std::process::ExitCode::from(1);
    }

    // Mode selection: book-aware daemon is the default when --addr is set (and books
    // are not explicitly opted out of); an explicit instrument list or --no-book-poll
    // is the broadcast fallback; no --addr (or --local) is the in-process demo.
    let network = args.addr.is_some() && !args.local;
    let instruments_is_all = args.instruments.trim().eq_ignore_ascii_case("all");
    let book_aware = network && !args.no_book_poll && instruments_is_all;

    eprintln!(
        "[lp-sim] feed '{}' : {} members ({}), interval {}s, seed {:#x}",
        cfg.lp_name,
        cfg.members,
        member_names(&cfg).join(", "),
        args.interval,
        cfg.seed,
    );

    // ---- Book-aware network daemon (the deployed default when --addr is set) -------
    if book_aware {
        let addr = args.addr.as_deref().expect("network implies --addr");
        let email = args
            .user
            .clone()
            .or_else(|| std::env::var("LPSIM_USER").ok())
            .unwrap_or_else(|| celnet_lp_sim::net::DEFAULT_SERVICE_EMAIL.to_string());
        let password = args
            .password
            .clone()
            .or_else(|| std::env::var("LPSIM_PASSWORD").ok())
            .unwrap_or_else(|| celnet_lp_sim::net::DEFAULT_SERVICE_PASSWORD.to_string());
        let faults = FaultSchedule {
            enabled: !args.no_faults,
            seed: args.seed ^ 0x00FA_0175_0000_0000,
            ..FaultSchedule::default()
        };
        let opts = BookFeedOptions {
            book_poll: Duration::from_secs(args.book_poll.max(1)),
            quote_interval: args.quote_interval(),
            credentials: LoginCredentials {
                email: email.clone(),
                password,
            },
            faults,
            once: args.once,
        };
        eprintln!(
            "[lp-sim] book-aware mode → {addr} : login {email}, book-poll {}s, {} priceable instrument(s)",
            args.book_poll.max(1),
            priceable.len(),
        );
        return match run_book_aware_feed(&cfg, &priceable, addr, &opts) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("[lp-sim] ERROR: book-aware feed failed: {e}");
                std::process::ExitCode::from(1)
            }
        };
    }

    // ---- Broadcast / local: select the instruments to quote (filter + cap) --------
    let mut selection = filter_instruments(priceable, &args.instruments);
    if selection.len() > args.max_instruments {
        selection.truncate(args.max_instruments);
    }
    if selection.is_empty() {
        eprintln!(
            "[lp-sim] ERROR: no modellable instruments match --instruments {:?} at settlement {}",
            args.instruments, args.settlement
        );
        return std::process::ExitCode::from(1);
    }
    eprintln!("[lp-sim] {} instrument(s) selected", selection.len());

    // Broadcast network fallback: stream the selected universe as the N LPs, no books.
    if network {
        let addr = args.addr.as_deref().expect("network implies --addr");
        eprintln!(
            "[lp-sim] broadcast network mode → pushing LpFeed to {addr} (book '{}')",
            args.book
        );
        return match celnet_lp_sim::net::run_network_feed(
            &cfg,
            &selection,
            addr,
            args.interval,
            args.once,
        ) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("[lp-sim] ERROR: network feed failed: {e}");
                std::process::ExitCode::from(1)
            }
        };
    }

    // Local in-process mode (default): consolidate + print the composite here.
    let feeds = into_feeds(build_fleet(&cfg, &selection));
    let ccfg = cfg.consolidation();

    let start = SystemTime::now();
    let mut round: u64 = 0;
    loop {
        let now_nanos = epoch_nanos(start);
        println!(
            "--- {} composite @ {} (round {round}) ---",
            args.book,
            wall_clock()
        );
        for line in &selection {
            match composite_for(&feeds, line, now_nanos, &ccfg) {
                Ok(c) => println!("  {}", c.rendered),
                Err(e) => println!(
                    "  {} [{}] no composite: {e}",
                    line.display_name, line.identity
                ),
            }
        }

        round += 1;
        if args.once {
            return std::process::ExitCode::SUCCESS;
        }
        std::thread::sleep(args.quote_interval());
    }
}

/// Filter a loaded universe by the `--instruments` selector (`all`, or a CSV of
/// ISINs/CUSIPs; matching is case-insensitive on either identifier).
fn filter_instruments(universe: Vec<QuotedLine>, selector: &str) -> Vec<QuotedLine> {
    let sel = selector.trim();
    if sel.eq_ignore_ascii_case("all") {
        return universe;
    }
    let wanted: Vec<String> = sel
        .split(',')
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    universe
        .into_iter()
        .filter(|line| {
            // Match the canonical id (CUSIP / slug / contract code) or any of the
            // line's cross-reference ids (a bond's `ISIN / CUSIP`).
            let id = line.instrument_id.to_ascii_uppercase();
            wanted.contains(&id)
                || line
                    .identity
                    .split('/')
                    .map(|x| x.trim().to_ascii_uppercase())
                    .any(|x| !x.is_empty() && wanted.contains(&x))
        })
        .collect()
}

/// The member venue names for the configured panel (for the startup banner).
fn member_names(cfg: &LpSimConfig) -> Vec<String> {
    (0..cfg.members.max(1))
        .map(|i| cfg.member_venue(i))
        .collect()
}

/// Epoch nanoseconds of `now`, from a monotonic-ish `start` reference plus elapsed
/// wall time (saturating; the valuation clock only needs to advance).
fn epoch_nanos(start: SystemTime) -> i64 {
    let d = start
        .elapsed()
        .unwrap_or_default()
        .saturating_add(start.duration_since(UNIX_EPOCH).unwrap_or_default());
    i64::try_from(d.as_nanos()).unwrap_or(i64::MAX)
}

/// A wall-clock instant string for the round header (epoch seconds — enough to
/// correlate rounds in a `tail -f`'d log).
fn wall_clock() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("epoch+{secs}s")
}

/// Parse a `YYYY-MM-DD` date into a [`BrokenDate`], validated against the calendar.
fn parse_iso_date(s: &str) -> Option<BrokenDate> {
    celnet_lp_sim::universe::parse_civil_date(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default parse (no `--interval-ms`) keeps the whole-second cadence; an
    /// `--interval-ms` override takes precedence and is sub-second capable, so a redeploy
    /// with `LPSIM_PERIOD_MS` refreshes the composite faster than 1s.
    #[test]
    fn interval_ms_overrides_seconds_and_is_sub_second_capable() {
        let base = Args::parse_from(["lp-sim", "--interval", "2"]);
        assert_eq!(base.quote_interval(), Duration::from_secs(2));

        let fast = Args::parse_from(["lp-sim", "--interval", "2", "--interval-ms", "250"]);
        assert_eq!(fast.quote_interval(), Duration::from_millis(250));

        // A zero override falls back to the whole-second interval; both floor to ≥1 unit.
        let zero = Args::parse_from(["lp-sim", "--interval", "0", "--interval-ms", "0"]);
        assert_eq!(zero.quote_interval(), Duration::from_secs(1));
    }
}
