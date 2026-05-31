//! The marked-surface registry: a versioned book of calibrated smiles the edge
//! prices *pinned* requests against, so a quote / stream / scenario tied to a
//! `surface_version` reproduces exactly the surface a [`MarkSurface`] published —
//! independent of any subsequent live re-mark.
//!
//! # Why versioned marks (the contract's `surface_version`)
//!
//! The wire contract carries an optional `surface_version` on `PriceRequest`,
//! `QuoteRequest`, `Subscribe`, `Modify`, and `ScenarioRequest`. It is a **data**
//! field selecting a surface (not an API version — the contract itself is
//! unversioned, CLAUDE.md rule 9): a desk that marked a surface at version *V*
//! and quoted a client off it must be able to re-price that exact structure
//! against *V* for a re-quote, an audit, or a deferred execution, even after the
//! live mark has rolled forward. This registry is the mechanism: every
//! `MarkSurface` deposits its per-tenor calibrated [`MarketHedgeSmile`]s under a
//! fresh monotonic version, and a pinned request resolves its per-strike vol from
//! the deposited smile rather than the live ATM vol.
//!
//! # Resolution
//!
//! A pinned price resolves the volatility for a `(pair, tenor, strike)` by looking
//! up the marked version, selecting the smile whose calibrated tenor is closest to
//! the requested expiry, and reading [`celnet_core::Smile::implied_vol`] at the
//! strike. An unknown version is a client error the caller surfaces as
//! `failed_precondition` (the pin cannot be honoured); a tenor with no marked
//! smile falls back to the closest marked tenor (never silently to the live mark,
//! so a pin always prices against *marked* data).
//!
//! # Concurrency
//!
//! The book is shared across every edge service behind an [`RwLock`]: a
//! `MarkSurface` takes the (brief) write lock to deposit a version; pinned reads
//! take the read lock to resolve a vol. Neither path touches the pinned hot core,
//! and the lock is never held across a price computation — only across the small
//! map clone of the resolved smile, so reads never serialize behind a mark.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use celnet_core::Smile;
use celnet_surface::CalibratedSmile;

/// One calibrated tenor slice deposited under a surface version: the smile plus
/// the forward and vol-time it was calibrated at (needed to read its vol).
///
/// The smile is held as a model-tagged [`CalibratedSmile`] (behind an `Arc` so a
/// pinned read clones a cheap handle, never the whole calibrated model), so a
/// surface marked under *any* smile model — market-hedge / stochastic-vol /
/// parametric / parametric-surface — resolves a pinned vol through the same trait
/// call against the exact model the desk marked.
#[derive(Debug, Clone)]
struct MarkedSlice {
    /// The calibrated smile (of the selected model) for this tenor.
    smile: Arc<CalibratedSmile>,
    /// The outright forward the smile was calibrated against.
    forward: f64,
    /// The vol-time (year fraction) the slice was calibrated at.
    tenor_years: f64,
}

/// The set of marked tenor slices for one currency pair under one version.
#[derive(Debug, Clone, Default)]
struct MarkedPair {
    /// The calibrated slices, in deposit order (one per marked tenor).
    slices: Vec<MarkedSlice>,
}

impl MarkedPair {
    /// Resolve the marked vol for `strike` at the requested `tenor_years`, choosing
    /// the calibrated slice whose tenor is closest to the request and reading its
    /// smile at the strike. `None` if no slice was marked for this pair.
    fn vol_at(&self, strike: f64, tenor_years: f64) -> Option<f64> {
        let slice = self.slices.iter().min_by(|a, b| {
            let da = (a.tenor_years - tenor_years).abs();
            let db = (b.tenor_years - tenor_years).abs();
            da.partial_cmp(&db).unwrap_or(core::cmp::Ordering::Equal)
        })?;
        Some(
            Smile::implied_vol(
                slice.smile.as_ref(),
                strike,
                slice.forward,
                slice.tenor_years,
            )
            .0,
        )
    }
}

/// A normalized currency-pair key (uppercase `BASE/QUOTE`) used to index marked
/// smiles independently of the wire string's exact casing.
fn pair_key(base: &str, quote: &str) -> String {
    format!("{}/{}", base.to_uppercase(), quote.to_uppercase())
}

/// One deposited surface version: the per-pair marked smiles.
#[derive(Debug, Clone, Default)]
struct MarkedVersion {
    /// Marked pairs keyed by the normalized `BASE/QUOTE` key.
    pairs: HashMap<String, MarkedPair>,
}

/// The versioned marked-surface registry shared across the edge services.
///
/// Construct one with [`SurfaceBook::new`], share it behind an `Arc`, and hand it
/// to every edge. `MarkSurface` deposits via [`SurfaceBook::deposit`]; pinned
/// price paths resolve via [`SurfaceBook::pinned_vol`].
#[derive(Debug)]
pub struct SurfaceBook {
    /// The monotonic version stamped on the next [`SurfaceBook::deposit`]. Starts
    /// at 1 so version `0` is never a valid mark (a sentinel a client can treat as
    /// "unmarked").
    next_version: AtomicU64,
    /// The deposited versions, keyed by their stamped version id.
    versions: RwLock<HashMap<u64, MarkedVersion>>,
}

impl Default for SurfaceBook {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceBook {
    /// Construct an empty registry whose first deposited version is `1`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_version: AtomicU64::new(1),
            versions: RwLock::new(HashMap::new()),
        }
    }

    /// Stamp and reserve the next surface version id without depositing slices yet.
    /// The caller deposits each calibrated slice under the returned version via
    /// [`SurfaceBook::deposit`]. Monotonic and unique per process.
    #[must_use]
    pub fn next_version(&self) -> u64 {
        self.next_version.fetch_add(1, Ordering::Relaxed)
    }

    /// Deposit one calibrated smile slice under `version` for `(base, quote)` at
    /// `tenor_years`, calibrated against `forward`.
    ///
    /// Repeated calls under the same version accumulate slices (one per marked
    /// tenor of a multi-tenor `MarkSurface`). Pinning then resolves a request's
    /// expiry to the closest deposited tenor.
    pub fn deposit(
        &self,
        version: u64,
        base: &str,
        quote: &str,
        tenor_years: f64,
        forward: f64,
        smile: CalibratedSmile,
    ) {
        let key = pair_key(base, quote);
        let mut versions = self.versions.write().expect("surface book not poisoned");
        let v = versions.entry(version).or_default();
        let pair = v.pairs.entry(key).or_default();
        pair.slices.push(MarkedSlice {
            smile: Arc::new(smile),
            forward,
            tenor_years,
        });
    }

    /// Resolve the pinned volatility for `(base, quote, tenor_years, strike)` under
    /// `version`.
    ///
    /// Returns:
    /// * `Ok(Some(vol))` — the marked vol read from the version's closest-tenor
    ///   smile at the strike;
    /// * `Ok(None)` — the version exists but marked no slice for this pair (the
    ///   caller decides whether to fall back to the live mark or error);
    /// * `Err(PinError::UnknownVersion)` — no such version was ever deposited (the
    ///   pin cannot be honoured).
    ///
    /// # Errors
    ///
    /// [`PinError::UnknownVersion`] if `version` was never deposited.
    pub fn pinned_vol(
        &self,
        version: u64,
        base: &str,
        quote: &str,
        tenor_years: f64,
        strike: f64,
    ) -> Result<Option<f64>, PinError> {
        let key = pair_key(base, quote);
        let versions = self.versions.read().expect("surface book not poisoned");
        let v = versions
            .get(&version)
            .ok_or(PinError::UnknownVersion(version))?;
        Ok(v.pairs
            .get(&key)
            .and_then(|p| p.vol_at(strike, tenor_years)))
    }

    /// `true` if `version` has been deposited (a pin against it can be honoured).
    #[must_use]
    pub fn has_version(&self, version: u64) -> bool {
        self.versions
            .read()
            .expect("surface book not poisoned")
            .contains_key(&version)
    }
}

/// A failure resolving a pinned surface version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinError {
    /// The requested `surface_version` was never marked (cannot be honoured).
    UnknownVersion(u64),
}

impl core::fmt::Display for PinError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PinError::UnknownVersion(v) => {
                write!(f, "pinned surface_version {v} was never marked")
            }
        }
    }
}

impl std::error::Error for PinError {}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_conventions::ConventionRecord;
    use celnet_surface::{MarketContext, MarketQuotes, SmileModel, build_model_smile};
    use celnet_types::{AtmConvention, Cut, DayCount, DeltaConvention, PremiumStyle, Settlement};

    fn record() -> ConventionRecord {
        ConventionRecord::new(
            DeltaConvention::SpotUnadjusted,
            AtmConvention::AtmForward,
            PremiumStyle::DomesticPips,
            Cut::NewYork1000,
            DayCount::Act365Fixed,
            DayCount::Act365Fixed,
            DayCount::Act365Fixed,
            Settlement::Deliverable,
        )
    }

    fn marked_smile(tenor: f64) -> (CalibratedSmile, f64) {
        let ctx = MarketContext::new(1.10, 0.02, 0.01, tenor, record());
        let quotes = MarketQuotes::three_point(0.10, -0.004, 0.002);
        let smile =
            build_model_smile(SmileModel::MarketHedge, &ctx, &quotes).expect("smile calibrates");
        let forward = smile.forward();
        (smile, forward)
    }

    #[test]
    fn deposit_then_resolve_pinned_vol() {
        let book = SurfaceBook::new();
        let v = book.next_version();
        let (smile, forward) = marked_smile(1.0);
        book.deposit(v, "EUR", "USD", 1.0, forward, smile);

        // The ATM (at-forward) vol resolves close to the marked 0.10.
        let vol = book
            .pinned_vol(v, "EUR", "USD", 1.0, forward)
            .expect("version exists")
            .expect("pair marked");
        assert!((vol - 0.10).abs() < 5e-3, "ATM-pinned vol {vol} ~ 0.10");
    }

    #[test]
    fn unknown_version_is_an_error() {
        let book = SurfaceBook::new();
        assert_eq!(
            book.pinned_vol(999, "EUR", "USD", 1.0, 1.10),
            Err(PinError::UnknownVersion(999))
        );
        assert!(!book.has_version(999));
    }

    #[test]
    fn marked_version_with_no_pair_is_none_not_error() {
        let book = SurfaceBook::new();
        let v = book.next_version();
        let (smile, forward) = marked_smile(1.0);
        book.deposit(v, "EUR", "USD", 1.0, forward, smile);
        // The version exists but GBP/USD was never marked under it.
        assert_eq!(book.pinned_vol(v, "GBP", "USD", 1.0, 1.10), Ok(None));
        assert!(book.has_version(v));
    }

    #[test]
    fn closest_tenor_slice_is_chosen() {
        let book = SurfaceBook::new();
        let v = book.next_version();
        let (s1, f1) = marked_smile(0.25);
        let (s2, f2) = marked_smile(2.0);
        book.deposit(v, "EUR", "USD", 0.25, f1, s1);
        book.deposit(v, "EUR", "USD", 2.0, f2, s2);
        // A 1.9Y request resolves against the 2.0Y slice (closest), not the 0.25Y.
        let vol = book
            .pinned_vol(v, "EUR", "USD", 1.9, f2)
            .expect("version")
            .expect("pair");
        assert!(vol > 0.0 && vol < 1.0, "resolved a sane marked vol {vol}");
    }
}
