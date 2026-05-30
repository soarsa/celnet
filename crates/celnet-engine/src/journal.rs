//! Durable crash-recovery for the engine's **control plane** (`docs/ARCHITECTURE.md`
//! §5; closes the durable-log wiring gap on top of `celnet-journal`).
//!
//! This module wires the standalone [`celnet_journal::Journal`] into the engine's
//! booking / accepted-market-state path — the same control-plane path on which the
//! blue-green [`crate::handoff`] serialization and the audit sink already live. It
//! is the durability seam that lets a freshly-started process **rebuild the running
//! book and the marked surface state and reprice bit-identically** after a crash.
//!
//! # Where this lives — and where it does **not**
//!
//! Journaling does IO and `fsync`. It is therefore a strict **control-plane / edge**
//! concern and is invoked **only** when a durable fact is accepted:
//!
//! * a booked option line ([`DurableBook::book`]), and
//! * an accepted [`MarketState`] / surface mark ([`DurableBook::mark`]).
//!
//! It is **never** called from the pinned, zero-alloc, busy-poll hot pricing loop
//! ([`crate::core::PricingCore::price`] / [`crate::core::PricingCore::drain`]): that
//! loop performs no IO, no `fsync`, no log, no lock and no allocation. Recovery
//! rebuild happens at **startup** ([`recover`]), not on the hot path. The existing
//! zero-allocation proof (`tests/zero_alloc.rs`) is extended to assert that pricing
//! a request that flows through a journalled book acquires no memory.
//!
//! # Single current byte contract (no fork)
//!
//! Each durable event is framed as `tag: u8 || payload`, where the payload is
//! produced by the **same** [`crate::handoff`] encoders used for blue-green
//! handoff — there is exactly one current byte format (guardrail #9):
//!
//! * [`EventTag::BookLine`] — one [`crate::rt::BookEntry`] in the identical
//!   fixed-width little-endian form a book entry takes inside a handoff buffer
//!   (`celnet-engine` `handoff::write_book_entry`).
//! * [`EventTag::MarkState`] — a full [`serialize_state`] buffer of the accepted
//!   `MarketState` paired with an *empty* book; [`restore_state`] reconstructs the
//!   `MarketState` on replay (the empty book section is discarded — the book is
//!   rebuilt from the `BookLine` events). Reusing `serialize_state` means the smile
//!   pillars, conventions and market scalars all round-trip through the one
//!   audited, bit-exact codec — no second serializer to drift.
//!
//! Determinism (libm math via `celnet_core::math`, IEEE-754 bit-pattern f64
//! encoding, counter-based RNG elsewhere) guarantees that the rebuilt state
//! reprices to the **same bits** as before the crash — the recovery contract.
//!
//! # Ordering semantics
//!
//! Events replay in append order. A later [`EventTag::MarkState`] supersedes an
//! earlier one (the live marked state is the *last* accepted mark), exactly as the
//! running engine would observe successive market ticks / recalibrations. Booked
//! lines accumulate in insertion order, reproducing the live [`BookState`].

use std::path::Path;

use celnet_journal::{Journal, JournalError};

use crate::handoff::{self, HandoffError};
use crate::rt::{BookEntry, BookState, MarketState};

/// The one-byte discriminant prefixing each durable event's payload.
///
/// A namespacing content tag (not a version — guardrail #9): a byte outside the
/// known set is rejected as corruption on replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum EventTag {
    /// A booked option line; payload is one handoff-form book entry.
    BookLine = 0,
    /// An accepted market state / surface mark; payload is a `serialize_state`
    /// buffer (market + empty book).
    MarkState = 1,
}

impl EventTag {
    fn from_byte(b: u8) -> Result<Self, RecoveryError> {
        match b {
            0 => Ok(EventTag::BookLine),
            1 => Ok(EventTag::MarkState),
            other => Err(RecoveryError::UnknownEventTag(other)),
        }
    }
}

/// Errors from the durable control-plane path.
#[derive(Debug)]
pub enum RecoveryError {
    /// An underlying journal (IO / framing) error.
    Journal(JournalError),
    /// A recovered event payload failed to decode against the current handoff
    /// contract.
    Decode(HandoffError),
    /// A recovered event carried an unknown event-tag byte.
    UnknownEventTag(u8),
    /// A recovered event payload was empty (no tag byte) — corruption.
    EmptyEvent,
    /// The journal replayed cleanly but contained no [`EventTag::MarkState`], so
    /// there is no marked state to price against. A consumer must `mark` an
    /// accepted state before any book line can be priced; recovering a log with
    /// booked lines but no mark is a genuine inconsistency.
    NoMarkedState,
}

impl core::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RecoveryError::Journal(e) => write!(f, "journal error: {e}"),
            RecoveryError::Decode(e) => write!(f, "durable event decode error: {e}"),
            RecoveryError::UnknownEventTag(b) => {
                write!(f, "unknown durable event tag byte: {b}")
            }
            RecoveryError::EmptyEvent => f.write_str("durable event payload was empty"),
            RecoveryError::NoMarkedState => {
                f.write_str("recovered journal has no accepted market state")
            }
        }
    }
}

impl std::error::Error for RecoveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RecoveryError::Journal(e) => Some(e),
            RecoveryError::Decode(e) => Some(e),
            _ => None,
        }
    }
}

impl From<JournalError> for RecoveryError {
    fn from(e: JournalError) -> Self {
        RecoveryError::Journal(e)
    }
}

impl From<HandoffError> for RecoveryError {
    fn from(e: HandoffError) -> Self {
        RecoveryError::Decode(e)
    }
}

/// Encode a [`EventTag::BookLine`] event: tag byte + one handoff-form book entry.
fn encode_book_line(entry: &BookEntry) -> Vec<u8> {
    // 1 tag byte + the fixed-width entry (8 + 1 + 8 + 8 = 25 bytes).
    let mut w = handoff::Writer::with_capacity(1 + 25);
    w.u8(EventTag::BookLine as u8);
    handoff::write_book_entry(&mut w, entry);
    w.into_bytes()
}

/// Encode a [`EventTag::MarkState`] event: tag byte + a `serialize_state` buffer of
/// `(market, empty book)`. Reuses the single current handoff codec (no fork).
fn encode_mark_state(market: &MarketState) -> Vec<u8> {
    let body = handoff::serialize_state(market, &BookState::new());
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(EventTag::MarkState as u8);
    out.extend_from_slice(&body);
    out
}

/// The control-plane durable writer: it owns the live [`BookState`] and the last
/// accepted [`MarketState`], and **journals every durable mutation** before
/// acknowledging it.
///
/// Construct with [`DurableBook::open`] (fresh or recovering). Each [`book`] /
/// [`mark`] appends an `fsync`'d event to the journal **and** updates the in-memory
/// state, so the durable log and the live state never diverge: once the call
/// returns `Ok`, the mutation survives a crash and will be replayed by [`recover`].
///
/// [`book`]: DurableBook::book
/// [`mark`]: DurableBook::mark
#[derive(Debug)]
pub struct DurableBook {
    journal: Journal,
    book: BookState,
    market: Option<MarketState>,
}

impl DurableBook {
    /// Open (creating if absent) the durable book backed by the journal at `path`,
    /// replaying any existing log to rebuild the in-memory state.
    ///
    /// On a fresh path this yields an empty book with no marked state. On an
    /// existing log it rebuilds the book and the last accepted market state exactly
    /// as [`recover`] would, then continues appending after the recovered tail.
    ///
    /// # Errors
    ///
    /// - [`RecoveryError::Journal`] on IO / framing failure.
    /// - [`RecoveryError::Decode`] / [`RecoveryError::UnknownEventTag`] /
    ///   [`RecoveryError::EmptyEvent`] if a recovered event is malformed.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RecoveryError> {
        let journal = Journal::open(path)?;
        let Recovered { book, market } = replay_into_state(&journal)?;
        Ok(Self {
            journal,
            book,
            market,
        })
    }

    /// Book an option line: append it durably, then add it to the live book.
    ///
    /// The journal `append` `fsync`s before returning, so the line is durable the
    /// instant this call returns `Ok`. In-memory state is updated only after the
    /// durable write succeeds, so a crash mid-append never leaves the live book
    /// ahead of the log.
    ///
    /// # Errors
    ///
    /// [`RecoveryError::Journal`] if the durable append fails.
    pub fn book(&mut self, entry: BookEntry) -> Result<(), RecoveryError> {
        let bytes = encode_book_line(&entry);
        self.journal.append(&bytes)?;
        self.book.push(entry);
        Ok(())
    }

    /// Accept a market state / surface mark: append it durably, then make it the
    /// live marked state.
    ///
    /// Supersedes any previous mark (the live marked state is the last accepted
    /// one). `fsync`'d before returning, like [`book`](Self::book).
    ///
    /// # Errors
    ///
    /// [`RecoveryError::Journal`] if the durable append fails.
    pub fn mark(&mut self, market: MarketState) -> Result<(), RecoveryError> {
        let bytes = encode_mark_state(&market);
        self.journal.append(&bytes)?;
        self.market = Some(market);
        Ok(())
    }

    /// The live book (every booked line, in insertion order).
    #[must_use]
    pub fn book_state(&self) -> &BookState {
        &self.book
    }

    /// The last accepted market state, or `None` if none has been marked yet.
    #[must_use]
    pub fn market_state(&self) -> Option<&MarketState> {
        self.market.as_ref()
    }

    /// The sequence number the next durable append will assign (diagnostics).
    #[must_use]
    pub fn next_sequence(&self) -> u64 {
        self.journal.next_sequence()
    }
}

/// The rebuilt state recovered from a journal.
struct Recovered {
    book: BookState,
    market: Option<MarketState>,
}

/// Replay `journal` into a fresh [`Recovered`] state.
fn replay_into_state(journal: &Journal) -> Result<Recovered, RecoveryError> {
    let mut book = BookState::new();
    let mut market: Option<MarketState> = None;
    // `replay` cannot return our `RecoveryError` from its callback, so we capture a
    // decode error out-of-band and surface it after the (infallible-callback) walk.
    let mut decode_err: Option<RecoveryError> = None;

    journal.replay(|rec| {
        if decode_err.is_some() {
            return; // already failed; ignore the rest
        }
        match decode_event(&rec.payload) {
            Ok(DurableEvent::BookLine(entry)) => book.push(entry),
            Ok(DurableEvent::MarkState(m)) => market = Some(m),
            Err(e) => decode_err = Some(e),
        }
    })?;

    if let Some(e) = decode_err {
        return Err(e);
    }
    Ok(Recovered { book, market })
}

/// A decoded durable event.
enum DurableEvent {
    BookLine(BookEntry),
    MarkState(MarketState),
}

/// Decode one framed event payload (`tag || body`).
fn decode_event(payload: &[u8]) -> Result<DurableEvent, RecoveryError> {
    let (&tag, body) = payload.split_first().ok_or(RecoveryError::EmptyEvent)?;
    match EventTag::from_byte(tag)? {
        EventTag::BookLine => {
            let mut r = handoff::Reader::new(body);
            let entry = handoff::read_book_entry(&mut r)?;
            r.finish()?;
            Ok(DurableEvent::BookLine(entry))
        }
        EventTag::MarkState => {
            // `restore_state` reconstructs `(market, empty book)`; the book section
            // is empty by construction (we encoded an empty book) and is discarded.
            let (market, _empty) = handoff::restore_state(body)?;
            Ok(DurableEvent::MarkState(market))
        }
    }
}

/// Rebuild the engine's durable state from the journal at `path` **at startup**.
///
/// Opens (read-only walk) the journal, replays every accepted event in order, and
/// returns the rebuilt `(MarketState, BookState)` — a state that reprices
/// **bit-identically** to the pre-crash engine (determinism guarantee). This is the
/// post-crash entry point: a fresh process calls it before serving any request.
///
/// # Errors
///
/// - [`RecoveryError::Journal`] on IO / framing failure.
/// - [`RecoveryError::Decode`] / [`RecoveryError::UnknownEventTag`] /
///   [`RecoveryError::EmptyEvent`] if a recovered event is malformed.
/// - [`RecoveryError::NoMarkedState`] if the log holds no accepted market state
///   (nothing to price against).
pub fn recover(path: impl AsRef<Path>) -> Result<(MarketState, BookState), RecoveryError> {
    let journal = Journal::open(path)?;
    let Recovered { book, market } = replay_into_state(&journal)?;
    let market = market.ok_or(RecoveryError::NoMarkedState)?;
    Ok((market, book))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{PriceRequest, PricingCore};
    use crate::testing::market_state;
    use celnet_conventions::ConventionRecord;
    use celnet_types::{CcyPair, OptionType, Tenor};

    fn conv() -> ConventionRecord {
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record
    }

    fn tmp_journal() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!(
            "celnet-engine-journal-unit-{}-{nanos}.log",
            std::process::id()
        ));
        p
    }

    #[test]
    fn book_and_mark_round_trip_through_recover() {
        let path = tmp_journal();
        {
            let mut db = DurableBook::open(&path).unwrap();
            db.mark(market_state(1.10, 0.105, 0.015, 0.0035, conv()))
                .unwrap();
            db.book(BookEntry {
                id: 1,
                option_type: OptionType::Call,
                strike: 1.12,
                notional: 1_000_000.0,
            })
            .unwrap();
            db.book(BookEntry {
                id: 2,
                option_type: OptionType::Put,
                strike: 1.05,
                notional: -500_000.0,
            })
            .unwrap();
        }
        let (m, b) = recover(&path).unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!(m.spot.to_bits(), 1.10_f64.to_bits());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn last_mark_wins() {
        let path = tmp_journal();
        {
            let mut db = DurableBook::open(&path).unwrap();
            db.mark(market_state(1.10, 0.105, 0.015, 0.0035, conv()))
                .unwrap();
            db.mark(market_state(1.25, 0.105, 0.015, 0.0035, conv()))
                .unwrap();
        }
        let (m, _b) = recover(&path).unwrap();
        assert_eq!(m.spot.to_bits(), 1.25_f64.to_bits());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn recover_without_mark_errors() {
        let path = tmp_journal();
        {
            let mut db = DurableBook::open(&path).unwrap();
            db.book(BookEntry {
                id: 1,
                option_type: OptionType::Call,
                strike: 1.12,
                notional: 1.0,
            })
            .unwrap();
        }
        assert!(matches!(recover(&path), Err(RecoveryError::NoMarkedState)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_tag_is_rejected() {
        // A foreign payload (bad tag byte) must be surfaced, not silently dropped.
        assert!(matches!(
            decode_event(&[0xFE, 0x00]),
            Err(RecoveryError::UnknownEventTag(0xFE))
        ));
        assert!(matches!(decode_event(&[]), Err(RecoveryError::EmptyEvent)));
    }

    #[test]
    fn rebuilt_state_reprices_bit_identically() {
        let path = tmp_journal();
        let m0 = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let lines = [
            (1u64, OptionType::Call, 1.12_f64),
            (2, OptionType::Put, 1.05),
            (3, OptionType::Call, 1.20),
        ];
        let captured: Vec<u64> = {
            let mut db = DurableBook::open(&path).unwrap();
            db.mark(m0.clone()).unwrap();
            for (id, ot, k) in lines {
                db.book(BookEntry {
                    id,
                    option_type: ot,
                    strike: k,
                    notional: 1_000_000.0,
                })
                .unwrap();
            }
            // Price the live book before the "crash".
            let mut core = PricingCore::new(db.market_state().unwrap().clone());
            db.book_state()
                .entries
                .iter()
                .map(|e| {
                    core.price(PriceRequest::new(e.id, e.option_type, e.strike))
                        .greeks
                        .price
                        .to_bits()
                })
                .collect()
        };

        let (m, b) = recover(&path).unwrap();
        let mut core = PricingCore::new(m);
        for (e, &want) in b.entries.iter().zip(captured.iter()) {
            let got = core
                .price(PriceRequest::new(e.id, e.option_type, e.strike))
                .greeks
                .price
                .to_bits();
            assert_eq!(
                got, want,
                "rebuilt line {} must reprice to the same bits",
                e.id
            );
        }
        let _ = std::fs::remove_file(&path);
    }
}
