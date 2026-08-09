//! The versioned marked market-data registry: a book of calibrated
//! [smiles](MarkedSlice) AND bootstrapped discount [curves](MarkedCurve) the edge
//! serves *pinned* requests against, so a quote / stream / scenario / curve read
//! tied to a version reproduces exactly the market-data a `MarkSurface` /
//! `MarkCurve` published — independent of any subsequent live re-mark.
//!
//! # One store, two asset-class-agnostic market-data families (ADR-0021)
//!
//! The FX vol surface and the fixed-income discount curve are the two marked
//! market-data families a desk pins against, and they share the SAME versioning
//! seam: one monotonic [version authority](SurfaceBook::next_version) stamps both,
//! and each family has its own version→mark map ([`versions`](SurfaceBook) for
//! surfaces, [`curves`](SurfaceBook) for curves). `MarkSurface` deposits a
//! calibrated smile under a fresh version; `MarkCurve` deposits a bootstrapped curve
//! under a fresh version through the *same* authority — the store is generalized to
//! hold curves alongside surfaces rather than forked into a bespoke curve book, so
//! fixed income rides the market-data query seam the FX surface already has.
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
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use celnet_core::Smile;
use celnet_rates::Curve;
use celnet_surface::CalibratedSmile;
use celnet_types::Time;

use crate::config::curve_definitions::{
    CurveDefinitionDef, CurveDefinitionStore, CurveRemoveError,
};

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

/// One marked (versioned, pinned) discount curve — the fixed-income analogue of a
/// [`MarkedVersion`]'s per-pair smiles. Holds the bootstrapped discount [`Curve`]
/// (behind an `Arc` so a pinned read is a cheap handle clone, never a curve copy)
/// plus the metadata a `GetCurve` echoes: the currency, the reference (spot-anchor)
/// date, and the resolved calibrating par pillars.
///
/// A pinned `GetCurve` reads its zero rates / discount factors straight off this
/// stored curve, so it is bit-for-bit identical to the curve the `MarkCurve`
/// bootstrapped (the FI counterpart of the pinned-surface reproducibility).
#[derive(Debug, Clone)]
pub struct MarkedCurve {
    /// The bootstrapped self-discounting discount curve.
    curve: Arc<Curve>,
    /// ISO 4217 currency of the curve (echoed on a `GetCurve`).
    currency: String,
    /// The curve reference (spot-anchor) civil date `(year, month, day)`, echoed as
    /// the `GetCurveResponse.reference_date`.
    reference_date: (i32, u32, u32),
    /// The resolved calibrating par pillars `(tenor_years, par_rate)`, echoed as the
    /// `GetCurveResponse.par_pillars`.
    par_pillars: Vec<(f64, f64)>,
}

impl MarkedCurve {
    /// Assemble a marked curve from a bootstrapped [`Curve`], its currency, the
    /// reference civil date `(year, month, day)`, and the resolved par pillars.
    #[must_use]
    pub fn new(
        curve: Curve,
        currency: String,
        reference_date: (i32, u32, u32),
        par_pillars: Vec<(f64, f64)>,
    ) -> Self {
        Self {
            curve: Arc::new(curve),
            currency,
            reference_date,
            par_pillars,
        }
    }

    /// The continuously-compounded zero rate z(t) at `tenor_years` off the marked
    /// curve.
    #[must_use]
    pub fn zero_rate(&self, tenor_years: f64) -> f64 {
        self.curve.zero_rate(Time(tenor_years)).0
    }

    /// The discount factor DF(t) at `tenor_years` off the marked curve.
    #[must_use]
    pub fn discount_factor(&self, tenor_years: f64) -> f64 {
        self.curve.discount_factor(Time(tenor_years)).0
    }

    /// The curve currency.
    #[must_use]
    pub fn currency(&self) -> &str {
        &self.currency
    }

    /// The curve reference (spot-anchor) civil date `(year, month, day)`.
    #[must_use]
    pub fn reference_date(&self) -> (i32, u32, u32) {
        self.reference_date
    }

    /// The resolved calibrating par pillars `(tenor_years, par_rate)`.
    #[must_use]
    pub fn par_pillars(&self) -> &[(f64, f64)] {
        &self.par_pillars
    }
}

/// The versioned marked market-data registry shared across the edge services.
///
/// Construct one with [`SurfaceBook::new`], share it behind an `Arc`, and hand it
/// to every edge. `MarkSurface` deposits via [`SurfaceBook::deposit`] and pinned
/// price paths resolve via [`SurfaceBook::pinned_vol`]; `MarkCurve` deposits via
/// [`SurfaceBook::deposit_curve`] and a pinned `GetCurve` resolves via
/// [`SurfaceBook::pinned_curve`] — both stamped by the same
/// [version authority](SurfaceBook::next_version).
#[derive(Debug)]
pub struct SurfaceBook {
    /// The monotonic version stamped on the next [`SurfaceBook::deposit`] /
    /// [`SurfaceBook::deposit_curve`]. Starts at 1 so version `0` is never a valid
    /// mark (a sentinel a client can treat as "unmarked"). One authority stamps
    /// both the surface and the curve families.
    next_version: AtomicU64,
    /// The deposited surface versions, keyed by their stamped version id.
    versions: RwLock<HashMap<u64, MarkedVersion>>,
    /// The deposited curve versions, keyed by their stamped version id — the
    /// fixed-income family alongside the surfaces (never a forked store).
    curves: RwLock<HashMap<u64, MarkedCurve>>,
    /// The **latest** marked wire [`CurveSet`](celnet_proto::CurveSet) per currency (the
    /// currency upper-cased), overwritten on every `MarkCurve` deposit. Unlike
    /// [`curves`](Self::curves) — which is a version-pinned, append-only history — this
    /// is a single "current curve" slot so a live consumer (the FIX rates/bond
    /// auto-quote path) can price off the operator's most recent mark without a version
    /// id, falling back to its own static default when the slot is empty. Stores the
    /// exact wire pillars the operator marked (lossless — no re-derivation from the
    /// bootstrapped curve).
    live_curves: RwLock<HashMap<String, celnet_proto::CurveSet>>,
    /// The **multi-curve registry**: the named, definable curve definitions the
    /// `SurfaceService` curve-CRUD verbs list/edit, keyed within the store by
    /// `curve_id`. Seeded on construction with the primary USD-SOFR definition
    /// ([`crate::rates_pricing::default_curve_definition`]) so the registry is never
    /// empty and a bare currency resolution always finds a primary. Distinct from
    /// [`live_curves`](Self::live_curves): that is the operator's latest `MarkCurve`
    /// slot; this is the definitional registry (reference data + interpolation +
    /// primary designation).
    curve_registry: RwLock<CurveDefinitionStore>,
    /// The on-disk path the registry persists to on every mutation (atomic
    /// write-then-rename). `None` ⇒ in-memory only (unit tests / an edge with no
    /// data dir wired); production installs it via
    /// [`set_curve_persistence`](Self::set_curve_persistence) at startup.
    curve_config_path: RwLock<Option<PathBuf>>,
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
        // Seed the registry with the primary USD-SOFR definition so a fresh install
        // resolves a curve identically to the shipping default (its pillars ARE the
        // P0 ladder). A later disk load / CRUD mutation overrides it.
        let mut registry = CurveDefinitionStore::default();
        registry.upsert(crate::rates_pricing::default_curve_definition());
        Self {
            next_version: AtomicU64::new(1),
            versions: RwLock::new(HashMap::new()),
            curves: RwLock::new(HashMap::new()),
            live_curves: RwLock::new(HashMap::new()),
            curve_registry: RwLock::new(registry),
            curve_config_path: RwLock::new(None),
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

    /// Deposit a bootstrapped discount [`MarkedCurve`] under `version` (the
    /// fixed-income analogue of [`SurfaceBook::deposit`]). The `version` must come
    /// from [`SurfaceBook::next_version`], so surfaces and curves share one
    /// monotonic version space.
    pub fn deposit_curve(&self, version: u64, curve: MarkedCurve) {
        self.curves
            .write()
            .expect("surface book not poisoned")
            .insert(version, curve);
    }

    /// Record `curve_set` as the **latest live** wire curve for its currency (upper-cased
    /// key), overwriting any prior mark. Called from `MarkCurve` alongside
    /// [`deposit_curve`](Self::deposit_curve) so a live consumer can price off the
    /// operator's most recent mark without a version id (see [`live_curve`](Self::live_curve)).
    pub fn set_live_curve(&self, curve_set: celnet_proto::CurveSet) {
        let key = curve_set.currency.to_ascii_uppercase();
        self.live_curves
            .write()
            .expect("surface book not poisoned")
            .insert(key, curve_set);
    }

    /// The latest live wire curve marked for `currency` (case-insensitive), or `None` when
    /// no curve has been marked for it. A cheap clone taken under a short read lock — a
    /// **snapshot**, so the caller prices off a stable copy off the hot path. The FIX
    /// rates/bond auto-quote path reads this and falls back to its static default when
    /// `None`.
    #[must_use]
    pub fn live_curve(&self, currency: &str) -> Option<celnet_proto::CurveSet> {
        let key = currency.to_ascii_uppercase();
        self.live_curves
            .read()
            .expect("surface book not poisoned")
            .get(&key)
            .cloned()
    }

    /// Resolve the marked curve deposited under `version` (a cheap handle clone; the
    /// curve itself is `Arc`-backed). The fixed-income analogue of
    /// [`SurfaceBook::pinned_vol`].
    ///
    /// # Errors
    ///
    /// [`PinError::UnknownVersion`] if no curve was deposited under `version` (the
    /// pin cannot be honoured — the caller surfaces `failed_precondition`).
    pub fn pinned_curve(&self, version: u64) -> Result<MarkedCurve, PinError> {
        self.curves
            .read()
            .expect("surface book not poisoned")
            .get(&version)
            .cloned()
            .ok_or(PinError::UnknownVersion(version))
    }

    /// `true` if a curve has been deposited under `version`.
    #[must_use]
    pub fn has_curve_version(&self, version: u64) -> bool {
        self.curves
            .read()
            .expect("surface book not poisoned")
            .contains_key(&version)
    }

    // --- multi-curve registry (named curve definitions) ----------------------

    /// Wire the registry's on-disk persistence path (production startup). After this,
    /// every registry mutation atomically writes the whole document to `path`.
    pub fn set_curve_persistence(&self, path: PathBuf) {
        *self
            .curve_config_path
            .write()
            .expect("surface book not poisoned") = Some(path);
    }

    /// Replace the whole registry with `store` (production startup, loading the
    /// persisted document from disk). Overrides the constructor's seeded default.
    pub fn install_curve_registry(&self, store: CurveDefinitionStore) {
        *self
            .curve_registry
            .write()
            .expect("surface book not poisoned") = store;
    }

    /// A snapshot of every registered curve definition (registry order).
    #[must_use]
    pub fn list_curve_defs(&self) -> Vec<CurveDefinitionDef> {
        self.curve_registry
            .read()
            .expect("surface book not poisoned")
            .curves
            .clone()
    }

    /// The registered definition with `curve_id`, if any (a cheap clone).
    #[must_use]
    pub fn get_curve_def(&self, curve_id: &str) -> Option<CurveDefinitionDef> {
        self.curve_registry
            .read()
            .expect("surface book not poisoned")
            .get(curve_id)
            .cloned()
    }

    /// The primary (default) curve definition for `currency` (case-insensitive).
    #[must_use]
    pub fn primary_curve_def(&self, currency: &str) -> Option<CurveDefinitionDef> {
        self.curve_registry
            .read()
            .expect("surface book not poisoned")
            .primary_for(currency)
            .cloned()
    }

    /// The wire [`CurveSet`](celnet_proto::CurveSet) a currency's PRIMARY curve
    /// calibrates against — the registry-resolved market a bare currency prices off.
    /// For USD this is byte-identical to
    /// [`default_usd_sofr_curve_set`](crate::rates_pricing::default_usd_sofr_curve_set)
    /// on a fresh registry (the seed IS the P0 ladder), so routing the FIX rates edge
    /// through the registry preserves today's pricing exactly.
    #[must_use]
    pub fn primary_curve_set(&self, currency: &str) -> Option<celnet_proto::CurveSet> {
        self.primary_curve_def(currency)
            .map(|def| crate::rates_pricing::curve_def_to_curve_set(&def))
    }

    /// Insert or replace a curve definition, maintaining the single-primary-per-
    /// currency invariant and (when persistence is wired) atomically writing the
    /// registry to disk BEFORE committing the change in memory — so disk and memory
    /// never diverge on a write failure.
    ///
    /// Returns the stored (normalised) definition.
    ///
    /// # Errors
    /// An IO/serialization failure persisting the registry (the in-memory registry is
    /// left unchanged).
    pub fn upsert_curve_def(&self, def: CurveDefinitionDef) -> std::io::Result<CurveDefinitionDef> {
        let mut guard = self
            .curve_registry
            .write()
            .expect("surface book not poisoned");
        let mut next = guard.clone();
        let stored = next.upsert(def);
        self.persist_registry(&next)?;
        *guard = next;
        Ok(stored)
    }

    /// Remove a curve definition by id, enforcing the delete rules (never orphan a
    /// currency's siblings by deleting its primary; never delete the last curve of the
    /// engine-priced currency), then atomically persist before committing in memory.
    ///
    /// The change is applied to a CLONE that is persisted first; the in-memory
    /// registry is committed only on a successful write, so disk and memory never
    /// diverge.
    ///
    /// # Errors
    /// [`CurveDefRemoveError::Rule`] when the id is unknown or a delete rule forbids
    /// removal; [`CurveDefRemoveError::Persist`] on an IO failure writing the registry
    /// (the in-memory registry is left unchanged).
    pub fn remove_curve_def(&self, curve_id: &str) -> Result<(), CurveDefRemoveError> {
        let mut guard = self
            .curve_registry
            .write()
            .expect("surface book not poisoned");
        let mut next = guard.clone();
        next.remove(curve_id, Some(crate::rates_pricing::SUPPORTED_CURRENCY))
            .map_err(CurveDefRemoveError::Rule)?;
        self.persist_registry(&next)
            .map_err(CurveDefRemoveError::Persist)?;
        *guard = next;
        Ok(())
    }

    /// Persist the CURRENT registry to the wired path (atomic write-then-rename).
    /// Used at startup to materialise the seeded default on a first run; a no-op when
    /// no path is wired.
    ///
    /// # Errors
    /// An IO/serialization failure writing the registry.
    pub fn persist_curves(&self) -> std::io::Result<()> {
        let store = self
            .curve_registry
            .read()
            .expect("surface book not poisoned")
            .clone();
        self.persist_registry(&store)
    }

    /// Persist `store` to the wired path (atomic write-then-rename). A no-op when no
    /// path is wired (in-memory registry).
    fn persist_registry(&self, store: &CurveDefinitionStore) -> std::io::Result<()> {
        let path = self
            .curve_config_path
            .read()
            .expect("surface book not poisoned")
            .clone();
        match path {
            Some(p) => store.save(&p),
            None => Ok(()),
        }
    }
}

/// A [`SurfaceBook::remove_curve_def`] failure: either a registry delete-rule
/// violation or an IO error persisting the change.
#[derive(Debug)]
pub enum CurveDefRemoveError {
    /// A delete-rule violation (unknown id / primary-with-siblings / last engine curve).
    Rule(CurveRemoveError),
    /// An IO/serialization failure persisting the registry (registry unchanged).
    Persist(std::io::Error),
}

impl core::fmt::Display for CurveDefRemoveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Rule(e) => write!(f, "{e}"),
            Self::Persist(e) => write!(f, "failed to persist curve registry: {e}"),
        }
    }
}

impl core::error::Error for CurveDefRemoveError {}

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
        let ctx = MarketContext::new(
            1.10,
            celnet_types::Carry::FxRates {
                r_dom: 0.02,
                r_for: 0.01,
            },
            tenor,
            record(),
        );
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

    // --- multi-curve registry (named curve definitions) ----------------------

    use crate::config::curve_definitions::{CurvePillarDef, PillarTenorDef};
    use crate::rates_pricing::{
        DEFAULT_CURVE_ID, default_curve_definition, default_usd_sofr_curve_set,
    };

    /// A distinct, non-primary named USD curve (a slightly shifted ladder) — proves a
    /// second curve of the priced currency coexists without disturbing the primary.
    fn alt_usd_curve(id: &str) -> CurveDefinitionDef {
        let mut def = default_curve_definition();
        def.curve_id = id.to_string();
        def.display_name = "USD SOFR (alt)".to_string();
        def.primary = false;
        def.pillars = vec![
            CurvePillarDef {
                tenor: PillarTenorDef::Years(1),
                par_rate: 0.0450,
            },
            CurvePillarDef {
                tenor: PillarTenorDef::Years(5),
                par_rate: 0.0430,
            },
        ];
        def
    }

    #[test]
    fn new_seeds_the_usd_sofr_primary() {
        let book = SurfaceBook::new();
        let defs = book.list_curve_defs();
        assert_eq!(defs.len(), 1, "a fresh registry holds exactly the seed");
        let seed = book
            .get_curve_def(DEFAULT_CURVE_ID)
            .expect("seeded default present at startup");
        assert!(seed.primary, "the seed is USD's primary");
        assert_eq!(seed.currency, "USD");
    }

    #[test]
    fn primary_curve_set_usd_is_byte_identical_to_shipping_default() {
        // The regression guard for `FixSession::live_rates_curve`: with no operator
        // `MarkCurve` slot, its resolution is `live_curve(USD).or_else(primary_curve_set
        // (USD)).unwrap_or_else(default)`. On a fresh book `live_curve` is absent and
        // `primary_curve_set` MUST equal the shipping default — so FIX pricing is
        // byte-for-byte unchanged by routing through the registry.
        let book = SurfaceBook::new();
        assert!(
            book.live_curve("USD").is_none(),
            "no MarkCurve slot on a fresh book"
        );
        assert_eq!(
            book.primary_curve_set("USD"),
            Some(default_usd_sofr_curve_set())
        );
        // Replicate the exact `live_rates_curve` fallback chain end-to-end.
        let resolved = book
            .live_curve("USD")
            .or_else(|| book.primary_curve_set("USD"))
            .unwrap_or_else(default_usd_sofr_curve_set);
        assert_eq!(
            resolved,
            default_usd_sofr_curve_set(),
            "live_rates_curve resolves USD identically after seeding"
        );
    }

    #[test]
    fn second_named_usd_curve_coexists_without_clobbering_primary() {
        let book = SurfaceBook::new();
        book.upsert_curve_def(alt_usd_curve("usd-sofr-alt"))
            .expect("in-memory upsert (no persistence wired) always succeeds");
        assert_eq!(book.list_curve_defs().len(), 2, "seed + alt coexist");
        assert_eq!(
            book.primary_curve_def("USD").unwrap().curve_id,
            DEFAULT_CURVE_ID,
            "the primary is untouched by a non-primary sibling"
        );
        assert_eq!(
            book.primary_curve_set("USD"),
            Some(default_usd_sofr_curve_set()),
            "USD pricing still resolves to the shipping default"
        );
    }

    #[test]
    fn remove_refuses_the_last_engine_curve() {
        let book = SurfaceBook::new();
        // Deleting the sole USD curve would leave pricing without a curve — refused.
        match book.remove_curve_def(DEFAULT_CURVE_ID) {
            Err(CurveDefRemoveError::Rule(CurveRemoveError::LastEngineCurve)) => {}
            other => panic!("expected LastEngineCurve, got {other:?}"),
        }
        assert!(
            book.get_curve_def(DEFAULT_CURVE_ID).is_some(),
            "seed intact"
        );
    }

    #[test]
    fn registry_survives_restart_and_still_resolves_usd_identically() {
        let dir = std::env::temp_dir().join("celnet-surfacebook-restart");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("curves-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        // First "process": seed + a defined alt curve, persisted to disk on each write.
        let book = SurfaceBook::new();
        book.set_curve_persistence(path.clone());
        book.persist_curves().expect("materialise the seed");
        book.upsert_curve_def(alt_usd_curve("usd-sofr-alt"))
            .expect("persisted upsert");

        // Second "process": load the document from disk into a fresh book (as Edge boot
        // does) and confirm both curves survived and USD still resolves to the default.
        let reloaded = CurveDefinitionStore::load(&path).expect("reload persisted registry");
        let fresh = SurfaceBook::new();
        fresh.install_curve_registry(reloaded);
        assert_eq!(
            fresh.list_curve_defs().len(),
            2,
            "seed + alt survived restart"
        );
        assert!(fresh.get_curve_def("usd-sofr-alt").is_some());
        assert_eq!(
            fresh.primary_curve_set("USD"),
            Some(default_usd_sofr_curve_set()),
            "the primary (and thus FIX pricing) is unchanged across restart"
        );
        let _ = std::fs::remove_file(&path);
    }
}
