//! The venue-feed abstraction: subscribe to instruments and receive a stream of
//! [`VenueQuote`]s.
//!
//! The trait is intentionally **object-safe** (`&dyn VenueFeed`) and pull-based:
//! the consolidator holds a heterogeneous panel of `Box<dyn VenueFeed>` and asks
//! each for its current top-of-book at a logical valuation instant. A "stream" is
//! then the sequence of top-of-book snapshots as the logical clock advances,
//! realised deterministically by [`VenueStream`] over a fixed tick grid — no
//! async runtime, fully reproducible, and directly testable. A real network feed
//! (a later lane) implements the same trait by returning its most recent tick.

use crate::instrument::{Instrument, VenueId, VenueQuote};

/// A market-data venue: answers "what is your current top-of-book for this
/// instrument, as of this logical instant?".
///
/// Object-safe so a panel can mix a [`crate::sim::SimVenue`] with any future
/// real-feed adapter behind one `Box<dyn VenueFeed>`. Implementations are pure
/// functions of `now_nanos` (given their own configuration/state) so a
/// consolidation is reproducible: re-asking at the same instant yields the same
/// quote.
pub trait VenueFeed {
    /// This venue's stable identity.
    fn venue(&self) -> &VenueId;

    /// The venue's top-of-book two-way for `instrument` as of logical
    /// `now_nanos` (epoch nanoseconds), or `None` if the venue makes no market in
    /// that instrument.
    fn top_of_book(&self, instrument: &Instrument, now_nanos: i64) -> Option<VenueQuote>;
}

/// A deterministic top-of-book **stream** over one [`VenueFeed`]: it samples the
/// feed's top-of-book for a set of instruments on a fixed tick grid, yielding one
/// [`VenueQuote`] per `(tick, instrument)` in that order (instruments the venue
/// does not make a market in are skipped).
///
/// This is the concrete realisation of "subscribe to instruments → a stream of
/// [`VenueQuote`]". It borrows the feed as `&dyn VenueFeed` (keeping the trait
/// object-safe) and is a plain [`Iterator`], so it is deterministic and needs no
/// async runtime. Advancing the grid is exact integer arithmetic
/// (`start + k · tick`), so two streams built with identical parameters over a
/// seeded feed yield byte-identical quote sequences.
pub struct VenueStream<'a> {
    feed: &'a dyn VenueFeed,
    instruments: &'a [Instrument],
    start_nanos: i64,
    tick_nanos: i64,
    ticks: u64,
    // Cursor: which tick (0..ticks) and which instrument within the tick.
    tick: u64,
    instr_idx: usize,
}

impl<'a> VenueStream<'a> {
    /// Subscribe: build a stream that samples `feed` for `instruments` at
    /// `start_nanos`, `start_nanos + tick_nanos`, … for `ticks` grid points.
    ///
    /// `tick_nanos` must be positive; a zero or negative tick would not advance
    /// the logical clock, so it is clamped to `1` ns to keep the grid strictly
    /// monotone (a stream can never stall on a degenerate tick).
    #[must_use]
    pub fn new(
        feed: &'a dyn VenueFeed,
        instruments: &'a [Instrument],
        start_nanos: i64,
        tick_nanos: i64,
        ticks: u64,
    ) -> Self {
        Self {
            feed,
            instruments,
            start_nanos,
            tick_nanos: tick_nanos.max(1),
            ticks,
            tick: 0,
            instr_idx: 0,
        }
    }
}

impl Iterator for VenueStream<'_> {
    type Item = VenueQuote;

    fn next(&mut self) -> Option<Self::Item> {
        while self.tick < self.ticks {
            // Exhausted this tick's instruments → advance to the next tick.
            if self.instr_idx >= self.instruments.len() {
                self.tick += 1;
                self.instr_idx = 0;
                continue;
            }
            let instrument = &self.instruments[self.instr_idx];
            self.instr_idx += 1;
            let now = self
                .start_nanos
                .saturating_add((self.tick as i64).saturating_mul(self.tick_nanos));
            if let Some(quote) = self.feed.top_of_book(instrument, now) {
                return Some(quote);
            }
            // Venue makes no market in this instrument at this tick → skip it and
            // continue scanning (never yields a phantom quote).
        }
        None
    }
}
