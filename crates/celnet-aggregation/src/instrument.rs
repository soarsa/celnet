//! The venue-quote vocabulary: a venue identity, an asset-agnostic instrument
//! key, and one venue's two-way top-of-book observation.

use core::fmt;

use celnet_types::{Tenor, Underlying};

/// A stable, human-readable venue identifier (e.g. `"venue-a"`, a sim venue, or a
/// real ECN's mnemonic in a later lane).
///
/// Ordered lexicographically so it can serve as the final, deterministic
/// tie-break key when two venues quote the identical best price at the identical
/// timestamp (mirroring the RFQ panel's stable `lp_id` tie-break). Cloneable and
/// hashable so it keys the per-venue contribution report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VenueId(pub String);

impl VenueId {
    /// Construct a venue id from any string-like value.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VenueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An asset-agnostic instrument key — *what* is being consolidated, independent
/// of asset class.
///
/// Built entirely from the frozen [`celnet_types`] vocabulary: an [`Underlying`]
/// (FX pair, precious metal, equity, commodity, or digital asset) plus a
/// [`Tenor`] (the point on the curve / expiry the two-way is good for — spot,
/// a forward date, or a bond maturity). One key type serves every asset class, so
/// the consolidation and risk-pricing machinery never forks per asset.
///
/// `Eq`/`Hash` so it keys per-instrument state; `Clone` (not `Copy`) because
/// [`Underlying`] carries owned tickers for its equity/commodity/crypto arms.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Instrument {
    /// The underlying asset (asset-class discriminator).
    pub underlying: Underlying,
    /// The tenor / curve point the two-way references.
    pub tenor: Tenor,
}

impl Instrument {
    /// Construct an instrument key from its underlying and tenor.
    #[must_use]
    pub fn new(underlying: impl Into<Underlying>, tenor: Tenor) -> Self {
        Self {
            underlying: underlying.into(),
            tenor,
        }
    }
}

impl fmt::Display for Instrument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{:?}", self.underlying, self.tenor)
    }
}

/// One venue's two-way top-of-book observation for a single [`Instrument`].
///
/// Prices are in the instrument's natural cash units (quote-currency price for a
/// cash pair, or a yield for a bond — the consolidation math is unit-agnostic).
/// Sizes are the firm quantity available at the quoted price on each side. `ts`
/// is the venue's observation instant in epoch **nanoseconds** (the consolidator
/// decays a quote by its age relative to the valuation instant, so a laggy feed's
/// stale tick contributes geometrically less). `quality` is the venue's
/// self-reported confidence in `[0, 1]`, carried through to the report.
#[derive(Debug, Clone, PartialEq)]
pub struct VenueQuote {
    /// The venue that produced this quote.
    pub venue: VenueId,
    /// The instrument this quote is for.
    pub instrument: Instrument,
    /// Bid price (the venue buys the base/asset at this level).
    pub bid: f64,
    /// Offer price (the venue sells the base/asset at this level).
    pub offer: f64,
    /// Firm size available at the bid.
    pub bid_size: f64,
    /// Firm size available at the offer.
    pub offer_size: f64,
    /// Observation instant, epoch nanoseconds.
    pub ts: i64,
    /// Venue-reported quality in `[0, 1]`.
    pub quality: f64,
}

impl VenueQuote {
    /// The venue's mid price, `(bid + offer) / 2`.
    #[must_use]
    pub fn mid(&self) -> f64 {
        0.5 * (self.bid + self.offer)
    }

    /// Whether both sides of the two-way are finite (a malformed non-finite quote
    /// is folded to "worst" and never contributes — mirrors the RFQ panel's
    /// non-finite handling).
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.bid.is_finite()
            && self.offer.is_finite()
            && self.bid_size.is_finite()
            && self.offer_size.is_finite()
    }
}
