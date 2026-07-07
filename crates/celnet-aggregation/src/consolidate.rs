//! The consolidated book: reconcile N venue top-of-books for one [`Instrument`]
//! into a continuous best-bid/best-offer with depth, a fair composite mid, and a
//! confidence measure — excluding stale and divergent venues.
//!
//! # Algorithm (adapted from `celnet_integration::aggregate` / `divergence`)
//!
//! For a set of [`VenueQuote`]s (one current top-of-book per venue) and a
//! valuation instant `now_nanos`:
//!
//! 1. **Non-finite fold** — a venue quoting a non-finite price is excluded
//!    ([`ExclusionReason::NonFinite`]); it can never contribute or win (mirrors
//!    the RFQ panel's non-finite handling).
//! 2. **Staleness decay + cutoff** — each venue is weighted by
//!    `2^{−age/τ}` (`age = max(0, now − ts)` seconds, `τ` the half-life), so a
//!    fresh feed dominates a laggy one without a hard edge. A venue older than a
//!    hard `staleness_cutoff_secs` is fully excluded ([`ExclusionReason::Stale`]).
//! 3. **Divergent-source exclusion** — the robust consensus mid is the
//!    **median** of the surviving venue mids; a venue whose mid sits beyond the
//!    tolerance *and* beyond a MAD-scaled bound `k · 1.4826 · MAD` is excluded
//!    ([`ExclusionReason::Divergent`]) so one mispriced feed cannot poison the
//!    fair value. Gating needs at least [`MIN_SOURCES_FOR_GATING`] fresh venues to
//!    be decidable; below that the median is the midpoint and cannot say which
//!    feed is wrong, so gating is suppressed and `gating_undecidable` is set
//!    (deviations are still reported).
//! 4. **Composite mid** — the staleness-weighted mean of the surviving mids (a
//!    convex combination, so it always lies within the surviving-source spread).
//! 5. **Consolidated BBO + depth** — `best_bid` is the **max** surviving bid,
//!    `best_offer` the **min** surviving offer, each with a deterministic
//!    `(price, ts, venue)` total-order tie-break (mirrors the RFQ panel). Sizes
//!    stack by price level.
//! 6. **Confidence** — `coverage · freshness · agreement ∈ [0, 1]`: the fraction
//!    of venues that survived, their weighted freshness, and how tightly they
//!    agree relative to the tolerance.
//!
//! Method provenance (median-consensus + MAD outlier rule, exponential recency
//! kernel) is documented in `celnet_integration`; identifiers here are
//! purpose-named and vendor/research-neutral.

use crate::feed::VenueFeed;
use crate::instrument::{Instrument, VenueId, VenueQuote};

/// Minimum number of fresh venues for median-consensus divergence gating to be
/// meaningful. Below this the median is the midpoint of the surviving mids and
/// cannot discriminate the outlier, so gating is suppressed (see the module
/// docs). Mirrors `celnet_integration::divergence::MIN_SOURCES_FOR_GATING`.
pub const MIN_SOURCES_FOR_GATING: usize = 3;

/// Consistency factor making the median-absolute-deviation a consistent estimator
/// of the standard deviation under normality (`1 / Φ⁻¹(0.75)`).
const MAD_TO_SIGMA: f64 = 1.482_602_218_505_602;

/// Customary robust outlier cutoff in MAD-scaled sigmas: a venue is excluded as
/// divergent only when its deviation also exceeds `MAD_K · σ̂_MAD`.
const MAD_K: f64 = 3.0;

/// Configuration for a consolidation: the staleness half-life, the hard staleness
/// cutoff, and the divergence tolerance (all in the natural units of the
/// instrument's mid).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConsolidationConfig {
    /// Staleness half-life in seconds: a venue this old contributes exactly half
    /// the weight of a just-observed one. Must be positive.
    pub staleness_half_life_secs: f64,
    /// Hard staleness cutoff in seconds: a venue older than this is fully
    /// excluded (never contributes and never sets the BBO).
    pub staleness_cutoff_secs: f64,
    /// Divergence tolerance in **mid units** (absolute price/yield): a venue whose
    /// mid sits more than this from the robust consensus (and beyond the
    /// MAD-scaled bound) is excluded.
    pub divergence_tolerance: f64,
}

impl ConsolidationConfig {
    /// A sensible default: 30-second half-life, 5-minute hard cutoff, and a
    /// divergence tolerance the caller must set for the instrument's price scale
    /// (defaulted here to `5e-3`, i.e. ~50 bp on a unit-scale mid).
    #[must_use]
    pub fn new() -> Self {
        Self {
            staleness_half_life_secs: 30.0,
            staleness_cutoff_secs: 300.0,
            divergence_tolerance: 5.0e-3,
        }
    }

    /// The exponential staleness weight `2^{−age/half}` for a venue observed at
    /// `ts` relative to `now` (both epoch nanoseconds); `age = max(0, now − ts)`.
    #[must_use]
    pub fn staleness_weight(&self, ts: i64, now: i64) -> f64 {
        let age_secs = age_secs(ts, now);
        // 2^{−age/half} = e^{−ln2·age/half}.
        libm::exp(-core::f64::consts::LN_2 * age_secs / self.staleness_half_life_secs)
    }
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Age in seconds of a venue observed at `ts` relative to valuation `now`
/// (clamped at zero — a future-dated tick is treated as just-observed).
fn age_secs(ts: i64, now: i64) -> f64 {
    (now - ts).max(0) as f64 * 1e-9
}

/// Why a venue was excluded from the consolidated mid / BBO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionReason {
    /// The venue quoted a non-finite (NaN/±∞) price.
    NonFinite,
    /// The venue's last tick is older than the hard staleness cutoff.
    Stale,
    /// The venue's mid diverges from the robust consensus beyond tolerance.
    Divergent,
}

/// One venue's contribution to (or exclusion from) the consolidated book.
#[derive(Debug, Clone, PartialEq)]
pub struct VenueContribution {
    /// The venue.
    pub venue: VenueId,
    /// The venue's mid at the valuation instant.
    pub mid: f64,
    /// Age of the venue's tick in seconds at the valuation instant.
    pub age_secs: f64,
    /// Exponential staleness weight (`∈ (0, 1]`).
    pub staleness_weight: f64,
    /// Absolute deviation of the venue mid from the robust consensus (mid units);
    /// `NaN` for a non-finite venue that never entered the consensus.
    pub deviation: f64,
    /// Why the venue was excluded, or `None` if it contributed.
    pub excluded: Option<ExclusionReason>,
    /// Final normalized weight in the composite mid (`0` for excluded venues; the
    /// surviving weights sum to `1`).
    pub weight: f64,
}

impl VenueContribution {
    /// Whether this venue contributed to the mid (was not excluded).
    #[must_use]
    pub fn contributed(&self) -> bool {
        self.excluded.is_none()
    }
}

/// One price level in the stacked depth ladder: a price, the total firm size
/// available at it across venues, and the venues that quote it.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthLevel {
    /// The price of this level.
    pub price: f64,
    /// Total firm size across all venues at this price.
    pub size: f64,
    /// The venues quoting this price level (sorted for determinism).
    pub venues: Vec<VenueId>,
}

/// The consolidated book for one [`Instrument`]: the continuous BBO, the fair
/// composite mid, stacked depth, a confidence measure, and the per-venue
/// contribution report.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsolidatedBook {
    /// The instrument consolidated.
    pub instrument: Instrument,
    /// Consolidated best bid (max surviving bid).
    pub best_bid: f64,
    /// Consolidated best offer (min surviving offer).
    pub best_offer: f64,
    /// Total firm size at the consolidated best bid.
    pub best_bid_size: f64,
    /// Total firm size at the consolidated best offer.
    pub best_offer_size: f64,
    /// Staleness-weighted composite mid over the surviving venues.
    pub composite_mid: f64,
    /// Stacked bid depth, best (highest) price first.
    pub bid_depth: Vec<DepthLevel>,
    /// Stacked offer depth, best (lowest) price first.
    pub offer_depth: Vec<DepthLevel>,
    /// Confidence in the consolidated mid, `∈ [0, 1]`.
    pub confidence: f64,
    /// Per-venue contribution / exclusion report.
    pub contributions: Vec<VenueContribution>,
    /// Whether divergence gating was suppressed for lack of fresh venues.
    pub gating_undecidable: bool,
}

impl ConsolidatedBook {
    /// Number of venues that contributed to the composite mid.
    #[must_use]
    pub fn contributing(&self) -> usize {
        self.contributions
            .iter()
            .filter(|c| c.contributed())
            .count()
    }

    /// The consolidated two-way spread `best_offer − best_bid` (may be negative
    /// when venues overlap — a locked/crossed consolidated book).
    #[must_use]
    pub fn spread(&self) -> f64 {
        self.best_offer - self.best_bid
    }
}

/// A failure to consolidate. Mirrors the shape of
/// `celnet_integration::aggregate::BlendError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsolidationError {
    /// No venue quotes were supplied.
    NoQuotes,
    /// Quotes for more than one distinct instrument were supplied (the
    /// consolidator reconciles one instrument at a time).
    MixedInstruments,
    /// Every venue was excluded (all stale, non-finite, or divergent) — there is
    /// no surviving source to form a mid.
    AllExcluded,
}

impl core::fmt::Display for ConsolidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ConsolidationError::NoQuotes => f.write_str("no venue quotes supplied"),
            ConsolidationError::MixedInstruments => {
                f.write_str("quotes span more than one instrument")
            }
            ConsolidationError::AllExcluded => {
                f.write_str("every venue was excluded (stale / non-finite / divergent)")
            }
        }
    }
}

impl std::error::Error for ConsolidationError {}

impl ConsolidatedBook {
    /// Pull the current top-of-book from each venue in `venues` for `instrument`
    /// at `now_nanos`, then consolidate. Venues that make no market in the
    /// instrument are simply absent from the panel.
    ///
    /// # Errors
    /// Returns [`ConsolidationError::NoQuotes`] if no venue quotes the instrument,
    /// or [`ConsolidationError::AllExcluded`] if every quote is excluded.
    pub fn consolidate(
        venues: &[Box<dyn VenueFeed>],
        instrument: &Instrument,
        now_nanos: i64,
        config: &ConsolidationConfig,
    ) -> Result<Self, ConsolidationError> {
        let quotes: Vec<VenueQuote> = venues
            .iter()
            .filter_map(|v| v.top_of_book(instrument, now_nanos))
            .collect();
        Self::from_quotes(&quotes, now_nanos, config)
    }

    /// Consolidate an explicit set of venue quotes (the pure core; every input is
    /// one venue's current top-of-book for the same instrument).
    ///
    /// # Errors
    /// - [`ConsolidationError::NoQuotes`] if `quotes` is empty.
    /// - [`ConsolidationError::MixedInstruments`] if the quotes are not all for
    ///   the same instrument.
    /// - [`ConsolidationError::AllExcluded`] if every venue is excluded.
    pub fn from_quotes(
        quotes: &[VenueQuote],
        now_nanos: i64,
        config: &ConsolidationConfig,
    ) -> Result<Self, ConsolidationError> {
        let first = quotes.first().ok_or(ConsolidationError::NoQuotes)?;
        let instrument = first.instrument.clone();
        if quotes.iter().any(|q| q.instrument != instrument) {
            return Err(ConsolidationError::MixedInstruments);
        }

        // Stage 1+2: classify each venue by finiteness and staleness, compute the
        // staleness weight and mid. Non-finite and over-cutoff venues are excluded
        // up front and never enter the consensus.
        struct Scored<'a> {
            q: &'a VenueQuote,
            mid: f64,
            age: f64,
            sw: f64,
            excluded: Option<ExclusionReason>,
        }
        let mut scored: Vec<Scored<'_>> = quotes
            .iter()
            .map(|q| {
                let age = age_secs(q.ts, now_nanos);
                let sw = config.staleness_weight(q.ts, now_nanos);
                let excluded = if !q.is_finite() {
                    Some(ExclusionReason::NonFinite)
                } else if age > config.staleness_cutoff_secs {
                    Some(ExclusionReason::Stale)
                } else {
                    None
                };
                Scored {
                    q,
                    mid: q.mid(),
                    age,
                    sw,
                    excluded,
                }
            })
            .collect();

        // Stage 3: divergence gating over the fresh, finite venues.
        let fresh_mids: Vec<f64> = scored
            .iter()
            .filter(|s| s.excluded.is_none())
            .map(|s| s.mid)
            .collect();
        let consensus = median(&fresh_mids);
        let gating_undecidable = fresh_mids.len() < MIN_SOURCES_FOR_GATING;
        let mad_bound = if gating_undecidable {
            f64::INFINITY
        } else {
            let devs: Vec<f64> = fresh_mids.iter().map(|m| (m - consensus).abs()).collect();
            MAD_K * MAD_TO_SIGMA * median(&devs)
        };
        for s in &mut scored {
            if s.excluded.is_none() {
                let dev = (s.mid - consensus).abs();
                if !gating_undecidable && dev > config.divergence_tolerance && dev > mad_bound {
                    s.excluded = Some(ExclusionReason::Divergent);
                }
            }
        }

        // Stage 4: normalized staleness weights over survivors → composite mid.
        let survivor_sw: f64 = scored
            .iter()
            .filter(|s| s.excluded.is_none())
            .map(|s| s.sw)
            .sum();
        if survivor_sw <= 0.0 || !scored.iter().any(|s| s.excluded.is_none()) {
            return Err(ConsolidationError::AllExcluded);
        }
        let composite_mid: f64 = scored
            .iter()
            .filter(|s| s.excluded.is_none())
            .map(|s| (s.sw / survivor_sw) * s.mid)
            .sum();

        // Stage 5: consolidated BBO with deterministic (price, ts, venue)
        // tie-break, and stacked depth, over the survivors.
        let survivors: Vec<&VenueQuote> = scored
            .iter()
            .filter(|s| s.excluded.is_none())
            .map(|s| s.q)
            .collect();
        let best_bid = best_price(&survivors, Side::Bid);
        let best_offer = best_price(&survivors, Side::Offer);
        let best_bid_size = size_at(&survivors, Side::Bid, best_bid);
        let best_offer_size = size_at(&survivors, Side::Offer, best_offer);
        let bid_depth = depth(&survivors, Side::Bid);
        let offer_depth = depth(&survivors, Side::Offer);

        // Stage 6: confidence = coverage · freshness · agreement.
        let total = scored.len() as f64;
        let survivor_count = survivors.len() as f64;
        let coverage = survivor_count / total;
        // freshness = Σ w_i·sw_i = Σ sw_i² / Σ sw_i  ∈ (0, 1].
        let freshness: f64 = scored
            .iter()
            .filter(|s| s.excluded.is_none())
            .map(|s| (s.sw / survivor_sw) * s.sw)
            .sum();
        let dispersion = survivors
            .iter()
            .map(|q| (q.mid() - composite_mid).abs())
            .fold(0.0_f64, f64::max);
        let agreement = if config.divergence_tolerance > 0.0 {
            1.0 - (dispersion / config.divergence_tolerance).min(1.0)
        } else if dispersion == 0.0 {
            1.0
        } else {
            0.0
        };
        let confidence = (coverage * freshness * agreement).clamp(0.0, 1.0);

        // Build the per-venue report.
        let contributions = scored
            .iter()
            .map(|s| VenueContribution {
                venue: s.q.venue.clone(),
                mid: s.mid,
                age_secs: s.age,
                staleness_weight: s.sw,
                deviation: if matches!(s.excluded, Some(ExclusionReason::NonFinite)) {
                    f64::NAN
                } else {
                    (s.mid - consensus).abs()
                },
                excluded: s.excluded,
                weight: if s.excluded.is_none() {
                    s.sw / survivor_sw
                } else {
                    0.0
                },
            })
            .collect();

        Ok(ConsolidatedBook {
            instrument,
            best_bid,
            best_offer,
            best_bid_size,
            best_offer_size,
            composite_mid,
            bid_depth,
            offer_depth,
            confidence,
            contributions,
            gating_undecidable,
        })
    }
}

/// Which side of the book a computation concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Bid,
    Offer,
}

impl Side {
    /// The quoted price on this side.
    fn price(self, q: &VenueQuote) -> f64 {
        match self {
            Side::Bid => q.bid,
            Side::Offer => q.offer,
        }
    }

    /// The firm size on this side.
    fn size(self, q: &VenueQuote) -> f64 {
        match self {
            Side::Bid => q.bid_size,
            Side::Offer => q.offer_size,
        }
    }
}

/// Whether `a` is the better quote than `b` on `side` under the deterministic
/// total order: better price first (max bid / min offer), then the **earlier**
/// `ts`, then the **lexicographically smaller** venue id. All survivor prices are
/// finite here, so the order is total and the winner is unique. Mirrors the RFQ
/// panel's `(price, epoch, lp_id)` tie-break.
fn better(a: &VenueQuote, b: &VenueQuote, side: Side) -> bool {
    use core::cmp::Ordering;
    let (pa, pb) = (side.price(a), side.price(b));
    let price_cmp = match side {
        // Bid: higher is better ⇒ better means pa > pb.
        Side::Bid => pb.partial_cmp(&pa).unwrap_or(Ordering::Equal),
        // Offer: lower is better ⇒ better means pa < pb.
        Side::Offer => pa.partial_cmp(&pb).unwrap_or(Ordering::Equal),
    };
    let ord = price_cmp
        .then_with(|| a.ts.cmp(&b.ts))
        .then_with(|| a.venue.cmp(&b.venue));
    ord == Ordering::Less
}

/// The best price on `side` across `survivors` (panic-free; `survivors` is
/// non-empty by construction at every call site).
fn best_price(survivors: &[&VenueQuote], side: Side) -> f64 {
    let winner = survivors
        .iter()
        .copied()
        .reduce(|acc, q| if better(q, acc, side) { q } else { acc })
        .expect("best_price is only called with a non-empty survivor set");
    side.price(winner)
}

/// Total firm size at exactly `price` on `side` (price-level stacking, exact
/// bit-equality on the level price).
fn size_at(survivors: &[&VenueQuote], side: Side, price: f64) -> f64 {
    survivors
        .iter()
        .filter(|q| side.price(q).to_bits() == price.to_bits())
        .map(|q| side.size(q))
        .sum()
}

/// The stacked depth ladder on `side`: one [`DepthLevel`] per distinct price,
/// summing sizes and collecting venues, sorted best-price-first (highest bid /
/// lowest offer). Venue lists within a level are sorted for determinism.
fn depth(survivors: &[&VenueQuote], side: Side) -> Vec<DepthLevel> {
    let mut levels: Vec<DepthLevel> = Vec::new();
    for q in survivors {
        let price = side.price(q);
        if let Some(level) = levels
            .iter_mut()
            .find(|l| l.price.to_bits() == price.to_bits())
        {
            level.size += side.size(q);
            level.venues.push(q.venue.clone());
        } else {
            levels.push(DepthLevel {
                price,
                size: side.size(q),
                venues: vec![q.venue.clone()],
            });
        }
    }
    for level in &mut levels {
        level.venues.sort();
    }
    // Best-price-first: descending for bids, ascending for offers.
    levels.sort_by(|a, b| match side {
        Side::Bid => b
            .price
            .partial_cmp(&a.price)
            .unwrap_or(core::cmp::Ordering::Equal),
        Side::Offer => a
            .price
            .partial_cmp(&b.price)
            .unwrap_or(core::cmp::Ordering::Equal),
    });
    levels
}

/// The median of `xs` (average of the two central order statistics for an even
/// count). Returns `0.0` for an empty slice (only reached when there are no fresh
/// venues, in which case the consensus is unused). Sorts a copy by total order so
/// it is `NaN`-safe (all inputs here are finite by construction).
fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instrument::VenueId;
    use celnet_types::{Ccy, CcyPair, Tenor};

    fn instr() -> Instrument {
        Instrument::new(CcyPair::new(Ccy::EUR, Ccy::USD), Tenor::Months(3))
    }

    fn quote(venue: &str, bid: f64, offer: f64, ts: i64) -> VenueQuote {
        VenueQuote {
            venue: VenueId::new(venue),
            instrument: instr(),
            bid,
            offer,
            bid_size: 1_000_000.0,
            offer_size: 1_000_000.0,
            ts,
            quality: 1.0,
        }
    }

    fn cfg() -> ConsolidationConfig {
        ConsolidationConfig {
            staleness_half_life_secs: 30.0,
            staleness_cutoff_secs: 60.0,
            divergence_tolerance: 0.01, // 100 bp on a ~1.10 mid
        }
    }

    // now = 100 s in nanos.
    const NOW: i64 = 100_000_000_000;
    const S: i64 = 1_000_000_000;

    #[test]
    fn composite_mid_within_surviving_source_spread() {
        let quotes = vec![
            quote("a", 1.0998, 1.1002, NOW),
            quote("b", 1.1000, 1.1004, NOW),
            quote("c", 1.0996, 1.1000, NOW),
        ];
        let book = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap();
        let min_bid = quotes.iter().map(|q| q.bid).fold(f64::INFINITY, f64::min);
        let max_offer = quotes
            .iter()
            .map(|q| q.offer)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            book.composite_mid >= min_bid && book.composite_mid <= max_offer,
            "composite {} not within [{min_bid}, {max_offer}]",
            book.composite_mid
        );
    }

    #[test]
    fn stale_venue_is_excluded() {
        // "c" last ticked 120 s before now → beyond the 60 s cutoff.
        let quotes = vec![
            quote("a", 1.0998, 1.1002, NOW),
            quote("b", 1.1000, 1.1004, NOW),
            quote("c", 1.0996, 1.1000, NOW - 120 * S),
        ];
        let book = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap();
        let c = book
            .contributions
            .iter()
            .find(|k| k.venue.as_str() == "c")
            .unwrap();
        assert_eq!(c.excluded, Some(ExclusionReason::Stale));
        assert!(!c.contributed());
        assert_eq!(c.weight, 0.0);
    }

    #[test]
    fn divergent_venue_is_excluded() {
        // "bad" quotes ~500 bp rich vs a tight consensus around 1.10.
        let quotes = vec![
            quote("a", 1.0999, 1.1001, NOW),
            quote("b", 1.1000, 1.1002, NOW),
            quote("c", 1.1001, 1.1003, NOW),
            quote("bad", 1.1548, 1.1552, NOW),
        ];
        let book = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap();
        let bad = book
            .contributions
            .iter()
            .find(|k| k.venue.as_str() == "bad")
            .unwrap();
        assert_eq!(bad.excluded, Some(ExclusionReason::Divergent));
        assert!(!book.gating_undecidable);
        // The divergent venue must not set the BBO: best offer stays tight.
        assert!(book.best_offer <= 1.1003);
    }

    #[test]
    fn gating_suppressed_below_min_sources() {
        // Two venues: median is the midpoint → gating undecidable, both survive.
        let quotes = vec![
            quote("a", 1.0999, 1.1001, NOW),
            quote("b", 1.1400, 1.1402, NOW),
        ];
        let book = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap();
        assert!(book.gating_undecidable);
        assert_eq!(book.contributing(), 2);
    }

    #[test]
    fn bbo_picks_true_best_across_venues() {
        let quotes = vec![
            quote("a", 1.0998, 1.1002, NOW),
            quote("b", 1.1001, 1.1004, NOW), // best bid
            quote("c", 1.0996, 1.1000, NOW), // best offer
        ];
        let book = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap();
        assert_eq!(book.best_bid.to_bits(), 1.1001_f64.to_bits());
        assert_eq!(book.best_offer.to_bits(), 1.1000_f64.to_bits());
    }

    #[test]
    fn bbo_tie_break_earlier_ts_then_smaller_venue() {
        // Equal best bid 1.1001: earlier ts wins.
        let quotes = vec![
            quote("late", 1.1001, 1.2000, NOW),
            quote("early", 1.1001, 1.2000, NOW - 5 * S),
        ];
        let book = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap();
        // Both quote the same best bid; the size stacks across both at that level.
        let top = &book.bid_depth[0];
        assert_eq!(top.price.to_bits(), 1.1001_f64.to_bits());
        assert_eq!(
            top.venues,
            vec![VenueId::new("early"), VenueId::new("late")]
        );

        // Equal bid AND ts: smaller venue id wins the tie-break winner selection.
        let quotes = vec![
            quote("bbb", 1.1001, 1.2000, NOW),
            quote("aaa", 1.1001, 1.2000, NOW),
        ];
        let book = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap();
        assert_eq!(book.best_bid.to_bits(), 1.1001_f64.to_bits());
    }

    #[test]
    fn depth_stacks_by_price_level() {
        let quotes = vec![
            quote("a", 1.1000, 1.1005, NOW),
            quote("b", 1.1000, 1.1006, NOW), // same bid level as a
            quote("c", 1.0998, 1.1005, NOW), // same offer level as a
        ];
        let book = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap();
        // Best bid level 1.1000 aggregates a + b sizes.
        let top_bid = &book.bid_depth[0];
        assert_eq!(top_bid.price.to_bits(), 1.1000_f64.to_bits());
        assert_eq!(top_bid.size, 2_000_000.0);
        // Best offer level 1.1005 aggregates a + c sizes.
        let top_offer = &book.offer_depth[0];
        assert_eq!(top_offer.price.to_bits(), 1.1005_f64.to_bits());
        assert_eq!(top_offer.size, 2_000_000.0);
    }

    #[test]
    fn non_finite_venue_is_excluded_and_never_wins() {
        let quotes = vec![
            quote("nan", f64::NAN, 1.0000, NOW), // malformed: would be best offer
            quote("a", 1.0999, 1.1001, NOW),
            quote("b", 1.1000, 1.1002, NOW),
        ];
        let book = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap();
        let nan = book
            .contributions
            .iter()
            .find(|k| k.venue.as_str() == "nan")
            .unwrap();
        assert_eq!(nan.excluded, Some(ExclusionReason::NonFinite));
        // The malformed 1.0000 offer must not win.
        assert_eq!(book.best_offer.to_bits(), 1.1001_f64.to_bits());
    }

    #[test]
    fn all_excluded_is_an_error() {
        let quotes = vec![
            quote("a", 1.0999, 1.1001, NOW - 200 * S),
            quote("b", 1.1000, 1.1002, NOW - 200 * S),
        ];
        let err = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap_err();
        assert_eq!(err, ConsolidationError::AllExcluded);
    }

    #[test]
    fn empty_and_mixed_instruments_error() {
        assert_eq!(
            ConsolidatedBook::from_quotes(&[], NOW, &cfg()).unwrap_err(),
            ConsolidationError::NoQuotes
        );
        let mut other = quote("x", 1.10, 1.11, NOW);
        other.instrument = Instrument::new(CcyPair::new(Ccy::GBP, Ccy::USD), Tenor::Months(3));
        let quotes = vec![quote("a", 1.10, 1.11, NOW), other];
        assert_eq!(
            ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap_err(),
            ConsolidationError::MixedInstruments
        );
    }

    #[test]
    fn fresher_venue_dominates_composite_mid() {
        // Same tolerance, three venues, one much older but still within cutoff.
        let quotes = vec![
            quote("fresh1", 1.0999, 1.1001, NOW),
            quote("fresh2", 1.1000, 1.1002, NOW),
            quote("old", 1.1040, 1.1044, NOW - 45 * S), // decayed weight
        ];
        let book = ConsolidatedBook::from_quotes(&quotes, NOW, &cfg()).unwrap();
        // The old venue is within cutoff (not excluded) but staleness-decayed, so
        // the composite mid stays much closer to the fresh ~1.1001 than to 1.1042.
        assert!(
            book.composite_mid < 1.102,
            "mid {} pulled by stale venue",
            book.composite_mid
        );
        let old = book
            .contributions
            .iter()
            .find(|k| k.venue.as_str() == "old")
            .unwrap();
        assert!(
            old.staleness_weight < 0.5,
            "45 s > 30 s half-life ⇒ <0.5 weight"
        );
    }

    #[test]
    fn confidence_in_unit_interval_and_higher_when_venues_agree() {
        let tight = vec![
            quote("a", 1.0999, 1.1001, NOW),
            quote("b", 1.1000, 1.1002, NOW),
            quote("c", 1.1001, 1.1003, NOW),
        ];
        let dispersed = vec![
            quote("a", 1.0950, 1.0954, NOW),
            quote("b", 1.1000, 1.1004, NOW),
            quote("c", 1.1050, 1.1054, NOW),
        ];
        let cfg = cfg();
        let bt = ConsolidatedBook::from_quotes(&tight, NOW, &cfg).unwrap();
        let bd = ConsolidatedBook::from_quotes(&dispersed, NOW, &cfg).unwrap();
        for b in [&bt, &bd] {
            assert!((0.0..=1.0).contains(&b.confidence));
        }
        assert!(
            bt.confidence > bd.confidence,
            "tighter agreement ⇒ higher confidence ({} vs {})",
            bt.confidence,
            bd.confidence
        );
    }
}
