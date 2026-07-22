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

use std::time::{SystemTime, UNIX_EPOCH};

use celnet_lp_sim::{
    LpSimConfig, TreasuryBond, build_fleet, composite_for, into_feeds, load_coupon_universe,
    load_universe,
};
use celnet_types::BrokenDate;
use clap::Parser;

/// The runnable LP-SIM Treasury liquidity feed.
#[derive(Debug, Parser)]
#[command(
    name = "lp-sim",
    about = "LP-SIM — synthetic Treasury liquidity-provider feed"
)]
struct Args {
    /// The LP connection name advertised as the price contributor (venue id).
    #[arg(long, default_value = "LP-SIM")]
    lp_name: String,

    /// The aggregated-book id this feed streams into (shown in the header; the
    /// server-side book selects which members it consolidates).
    #[arg(long, default_value = "ust-composite")]
    book: String,

    /// Number of decorrelated LP member connections (≥ 1). ≥ 3 makes the
    /// consolidator's divergence gating decidable.
    #[arg(long, default_value_t = 4)]
    members: usize,

    /// Seconds between composite emissions.
    #[arg(long, default_value_t = 2)]
    interval: u64,

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

    /// Emit a single round and exit (default: stream forever).
    #[arg(long, default_value_t = false)]
    once: bool,
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

    // Load and select the instruments to quote.
    let universe = if args.include_bills {
        load_universe()
    } else {
        load_coupon_universe()
    };
    let mut selection = filter_instruments(universe, &args.instruments);
    // Keep only bonds the analytics leaf can model at this settlement.
    selection.retain(|b| {
        b.yield_model(cfg.settlement, cfg.reversion_per_sec, cfg.perturbation)
            .is_some()
    });
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

    let feeds = into_feeds(build_fleet(&cfg, &selection));
    let ccfg = cfg.consolidation();

    eprintln!(
        "[lp-sim] feed '{}' → book '{}' : {} members, {} instrument(s), interval {}s, seed {:#x}",
        cfg.lp_name,
        args.book,
        cfg.members,
        selection.len(),
        args.interval,
        cfg.seed,
    );
    eprintln!("[lp-sim] members: {}", member_names(&cfg).join(", "));

    let start = SystemTime::now();
    let mut round: u64 = 0;
    loop {
        let now_nanos = epoch_nanos(start);
        println!(
            "--- {} composite @ {} (round {round}) ---",
            args.book,
            wall_clock()
        );
        for bond in &selection {
            match composite_for(&feeds, bond, now_nanos, &ccfg) {
                Ok(c) => println!("  {}", c.rendered),
                Err(e) => println!(
                    "  {} [{}] no composite: {e}",
                    bond.display_name(),
                    bond.cusip
                ),
            }
        }

        round += 1;
        if args.once {
            return std::process::ExitCode::SUCCESS;
        }
        std::thread::sleep(std::time::Duration::from_secs(args.interval.max(1)));
    }
}

/// Filter a loaded universe by the `--instruments` selector (`all`, or a CSV of
/// ISINs/CUSIPs; matching is case-insensitive on either identifier).
fn filter_instruments(universe: Vec<TreasuryBond>, selector: &str) -> Vec<TreasuryBond> {
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
        .filter(|b| {
            let cusip = b.cusip.to_ascii_uppercase();
            let isin = b.isin.to_ascii_uppercase();
            wanted.iter().any(|w| *w == cusip || *w == isin)
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
