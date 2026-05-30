//! Real crash-recovery integration test (`docs/ARCHITECTURE.md` §5).
//!
//! Exercises the production durable-recovery flow end to end against a real
//! on-disk journal in a temp directory:
//!
//! 1. Build a durable book, mark an accepted market state, book several option
//!    lines (all `fsync`'d to the journal), and capture each line's full Greek set.
//! 2. **Simulate a crash** by `drop`ping the [`DurableBook`] (and the journal
//!    handle it owns) without any clean shutdown — exactly what a `SIGKILL` leaves
//!    behind: the durable log on disk, no in-memory state.
//! 3. Open a **fresh** engine that [`celnet_engine::recover`]s the same journal,
//!    and assert the rebuilt [`BookState`] is **byte-identical** (`to_bits` over
//!    every field) to the pre-crash book, AND that it **reprices every captured
//!    line bit-identically** (the full Greek set, to the bit).
//!
//! The whole test is bounded by an in-process watchdog thread so a hang (e.g. a
//! never-terminating replay) fails fast instead of stalling the suite.

use std::sync::mpsc;
use std::time::Duration;

use celnet_engine::core::{PriceRequest, PricingCore};
use celnet_engine::rt::{BookEntry, BookState};
use celnet_engine::{DurableBook, recover};
use celnet_types::{CcyPair, Greeks, OptionType, Tenor};

/// A unique temp journal path for this test run (PID + nanos), so concurrent
/// nextest binaries never collide on the same file.
fn temp_journal_path() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    p.push(format!(
        "celnet-engine-recovery-{}-{nanos}.journal",
        std::process::id()
    ));
    p
}

/// Build a genuinely calibrated EURUSD market state (same path the engine uses).
fn market(spot: f64) -> celnet_engine::rt::MarketState {
    let conv =
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    celnet_engine::testing::market_state(spot, 0.105, 0.015, 0.0035, conv)
}

/// Every `f64` field of a `Greeks` compared by raw bit pattern — the strictest
/// possible "repriced identically" assertion (no tolerance).
fn greeks_bits(g: &Greeks) -> Vec<u64> {
    // Destructure exhaustively so the *entire* Greek set is compared to the bit,
    // and so adding a new Greek to `Greeks` forces this test to be updated (a
    // missing field is a compile error, not a silently-unchecked sensitivity).
    let Greeks {
        price,
        delta_spot,
        delta_forward,
        gamma,
        vega,
        theta,
        rho_dom,
        rho_for,
        vanna,
        volga,
        charm,
        speed,
        zomma,
        color,
    } = *g;
    vec![
        price.to_bits(),
        delta_spot.to_bits(),
        delta_forward.to_bits(),
        gamma.to_bits(),
        vega.to_bits(),
        theta.to_bits(),
        rho_dom.to_bits(),
        rho_for.to_bits(),
        vanna.to_bits(),
        volga.to_bits(),
        charm.to_bits(),
        speed.to_bits(),
        zomma.to_bits(),
        color.to_bits(),
    ]
}

#[test]
fn crash_recovery_rebuilds_byte_identical_book_and_reprices_bit_identically() {
    // ---- bounded watchdog: fail fast on any hang -------------------------------
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        // The whole flow is local fs + arithmetic; 60 s is orders of magnitude of
        // headroom. A hang trips this and aborts the test process loudly.
        if done_rx.recv_timeout(Duration::from_secs(60)).is_err() {
            eprintln!("crash-recovery test exceeded its deadline — aborting (hang)");
            std::process::abort();
        }
    });

    let path = temp_journal_path();

    // The booked lines we will recover and reprice.
    let lines = [
        BookEntry {
            id: 101,
            option_type: OptionType::Call,
            strike: 1.12,
            notional: 1_000_000.0,
        },
        BookEntry {
            id: 102,
            option_type: OptionType::Put,
            strike: 1.05,
            notional: -2_500_000.0,
        },
        BookEntry {
            id: 103,
            option_type: OptionType::Call,
            strike: 1.20,
            notional: 750_000.0,
        },
        BookEntry {
            id: 104,
            option_type: OptionType::Put,
            strike: 1.08,
            notional: 3_000_000.0,
        },
    ];

    // ---- phase 1: live engine, journal everything, capture prices --------------
    let pre_crash_book: BookState;
    let captured_greeks: Vec<Vec<u64>>;
    {
        let mut db = DurableBook::open(&path).expect("open fresh durable book");

        // Accept a market state (durable mark) — note an earlier mark then a
        // superseding one, so recovery's last-mark-wins is exercised.
        db.mark(market(1.05)).expect("durable mark #1");
        db.mark(market(1.10))
            .expect("durable mark #2 (the live one)");

        // Book every line (each fsync'd).
        for &line in &lines {
            db.book(line).expect("durable book line");
        }

        pre_crash_book = db.book_state().clone();

        // Capture the full priced Greek set of the live book.
        let mut core = PricingCore::new(db.market_state().expect("a marked state").clone());
        captured_greeks = db
            .book_state()
            .entries
            .iter()
            .map(|e| {
                let resp = core.price(PriceRequest::new(e.id, e.option_type, e.strike));
                greeks_bits(&resp.greeks)
            })
            .collect();

        // ---- phase 2: SIMULATE A CRASH ----------------------------------------
        // Drop the DurableBook (and its journal handle) with no clean shutdown.
        // Everything acknowledged above is already fsync'd to `path`.
        drop(db);
    }

    // ---- phase 3: FRESH engine replays the journal -----------------------------
    let (recovered_market, recovered_book) = recover(&path).expect("recover from journal");

    // (a) The rebuilt book is BYTE-IDENTICAL to the pre-crash book.
    assert_eq!(
        recovered_book.len(),
        pre_crash_book.len(),
        "recovered book line count must match"
    );
    for (rec, orig) in recovered_book
        .entries
        .iter()
        .zip(pre_crash_book.entries.iter())
    {
        assert_eq!(rec.id, orig.id, "line id byte-identical");
        assert_eq!(
            rec.option_type, orig.option_type,
            "option type byte-identical"
        );
        assert_eq!(
            rec.strike.to_bits(),
            orig.strike.to_bits(),
            "strike bit-identical"
        );
        assert_eq!(
            rec.notional.to_bits(),
            orig.notional.to_bits(),
            "notional bit-identical"
        );
    }
    // And as a whole-struct equality (PartialEq over the Vec<BookEntry>).
    assert_eq!(
        recovered_book, pre_crash_book,
        "recovered BookState must equal the pre-crash BookState"
    );

    // The recovered market state is the LAST accepted mark (spot 1.10), bit-exact.
    assert_eq!(
        recovered_market.spot.to_bits(),
        1.10_f64.to_bits(),
        "recovered market state must be the last accepted mark"
    );

    // (b) The fresh engine REPRICES every captured line BIT-IDENTICALLY.
    let mut fresh_core = PricingCore::new(recovered_market);
    for (e, want) in recovered_book.entries.iter().zip(captured_greeks.iter()) {
        let resp = fresh_core.price(PriceRequest::new(e.id, e.option_type, e.strike));
        let got = greeks_bits(&resp.greeks);
        assert_eq!(
            &got, want,
            "recovered line {} must reprice to the identical bits",
            e.id
        );
    }

    // Cleanup the temp journal; signal the watchdog we finished in time.
    let _ = std::fs::remove_file(&path);
    let _ = done_tx.send(());
    watchdog.join().expect("watchdog joins");
}

/// A second, stronger recovery property: a process that crashed mid-stream and is
/// recovered, then *continues* booking on top of the recovered log, replays the
/// whole history (old + new) on the next recovery — proving the durable log is the
/// single source of truth across multiple crash/restart cycles.
#[test]
fn recovery_then_continued_booking_replays_full_history() {
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(Duration::from_secs(60)).is_err() {
            eprintln!("continued-booking recovery test exceeded its deadline — aborting");
            std::process::abort();
        }
    });

    let path = temp_journal_path();

    // Cycle 1: mark + two lines, then "crash".
    {
        let mut db = DurableBook::open(&path).expect("open #1");
        db.mark(market(1.10)).expect("mark #1");
        db.book(BookEntry {
            id: 1,
            option_type: OptionType::Call,
            strike: 1.12,
            notional: 1_000_000.0,
        })
        .expect("book #1");
        db.book(BookEntry {
            id: 2,
            option_type: OptionType::Put,
            strike: 1.05,
            notional: -1.0,
        })
        .expect("book #2");
        drop(db); // crash
    }

    // Cycle 2: reopen (recovers the two lines), book one more, then "crash".
    {
        let mut db = DurableBook::open(&path).expect("reopen #2 recovers prior state");
        assert_eq!(
            db.book_state().len(),
            2,
            "reopen recovered both prior lines"
        );
        db.book(BookEntry {
            id: 3,
            option_type: OptionType::Call,
            strike: 1.20,
            notional: 500_000.0,
        })
        .expect("book #3");
        drop(db); // crash
    }

    // Final recovery sees the full history (3 lines) and the marked state.
    let (m, b) = recover(&path).expect("final recover");
    assert_eq!(b.len(), 3, "full history replayed across both cycles");
    assert_eq!(m.spot.to_bits(), 1.10_f64.to_bits());

    let _ = std::fs::remove_file(&path);
    let _ = done_tx.send(());
    watchdog.join().expect("watchdog joins");
}
