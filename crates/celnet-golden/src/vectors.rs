//! The frozen **golden-vector corpus** — the executable cross-client oracle.
//!
//! Each vector is one independent-oracle reference for one product of the single
//! `celnet.proto` `Instrument` `product` oneof. The corpus is the shared contract
//! every Celnet client (server, Rust SDK, CLI, Excel, GUI) is gated against: a
//! conformance harness builds the instrument from `{family, underlying, terms}`,
//! prices it under the vector's own `market`, and asserts the result equals
//! [`Expected::price`] (and any quoted Greeks) within [`Tolerance`] — or, for the
//! Monte-Carlo families, within `k · price_std_error`.
//!
//! ## The anti-circular-oracle rule (non-negotiable)
//!
//! [`Expected::price`] **must never** be the production pricer's own output. Every
//! value is produced by an oracle independent of the wire/server path:
//!
//! * `vanilla`, `single_barrier`, `double_barrier`, `digital`, `touch` — the
//!   frozen QuantLib reference CSVs under `data/` (an independent third-party
//!   library used purely as an offline oracle).
//! * `strategy` — the sum of independent vanilla-leg Garman-Kohlhagen oracle
//!   values.
//! * `variance_swap` / `volatility_swap` — the model-free flat-smile closed forms
//!   `K_var = σ²` and `K_vol = σ` (a flat σ replicates exactly; the convexity gap
//!   is zero).
//! * `forward_start` / `cliquet` (plain ratchet) / `quanto` / `lookback`
//!   (continuous) / `american` — the published closed-form / reference value,
//!   **re-derived from the market parameters in the generator** (or hand-pinned to
//!   a published constant), not read back from `celnet-exotics`.
//! * `asian`, `tarf`, `accumulator`, `lookback` (discrete), `cliquet` (clamped),
//!   `basket`, `window_barrier` — Monte-Carlo families: a **code-disjoint**
//!   Monte-Carlo reimplementation (a `splitmix64` RNG + Box–Muller + the payoff,
//!   independent of the production counter-RNG path) with a reported
//!   `price_std_error`; conformance asserts `|client − expected| ≤ k · stderr`.
//!
//! The corpus is **frozen**: it is committed to disk and regenerated only
//! deliberately via `cargo run -p celnet-golden --bin gen_vectors`. The
//! `tests/vectors_selfcheck.rs` gate re-derives every committed vector from its
//! independent oracle so the frozen artifact cannot silently drift.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The exact 18 product-oneof family names (mirroring the `celnet.proto`
/// `Instrument.product` oneof arm names). A vector's [`GoldenVector::family`] must
/// be one of these; the corpus is required to cover all 18.
pub const FAMILIES: [&str; 18] = [
    "vanilla",
    "strategy",
    "single_barrier",
    "double_barrier",
    "digital",
    "touch",
    "variance_swap",
    "volatility_swap",
    "asian_option",
    "forward_start",
    "cliquet",
    "quanto",
    "tarf",
    "accumulator",
    "lookback",
    "window_barrier",
    "american",
    "basket",
];

/// The families priced by Monte-Carlo, whose [`Expected::price_std_error`] is a
/// positive number and whose conformance tolerance is `k · stderr`.
pub const MC_FAMILIES: [&str; 7] = [
    "asian_option",
    "tarf",
    "accumulator",
    "lookback",
    "cliquet",
    "basket",
    "window_barrier",
];

/// The market context a vector is priced against — the four Garman-Kohlhagen
/// inputs the wire `MarketContext` carries (the server prices every closed-form /
/// flat-smile family against this single Black vol).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Market {
    /// Spot FX rate (quote per base).
    pub spot: f64,
    /// Annualised Black volatility (absolute, e.g. `0.105` = 10.5 vol).
    pub vol: f64,
    /// Continuously-compounded domestic (quote) rate.
    pub r_dom: f64,
    /// Continuously-compounded foreign (base) rate.
    pub r_for: f64,
}

/// The independent-oracle expectation for a vector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expected {
    /// The independent-oracle present value (premium per 1 unit of base notional).
    /// **Never** the production pricer's output (see the module docs).
    pub price: f64,
    /// The oracle Greeks where the oracle provides them (only `vanilla` carries the
    /// full QuantLib Greek strip today; omitted otherwise).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub greeks: BTreeMap<String, f64>,
    /// `null` for closed-form / deterministic families; a positive standard error
    /// for Monte-Carlo families (conformance tolerance becomes `k · stderr`).
    #[serde(default)]
    pub price_std_error: Option<f64>,
    /// Human-readable provenance of the oracle value.
    pub oracle: String,
}

/// The conformance tolerance pair `(relative, absolute)`. For Monte-Carlo
/// families these are set loose and the `k · price_std_error` band governs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tolerance {
    /// Relative tolerance.
    pub rel: f64,
    /// Absolute tolerance.
    pub abs: f64,
}

/// One frozen golden vector — a single independent-oracle reference for one
/// product family. The on-disk JSON shape is the cross-client contract; it must
/// not change shape without coordinating every client lane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldenVector {
    /// Stable, unique identifier (e.g. `"vanilla-eurusd-1y-call-k1.12"`).
    pub id: String,
    /// Exactly one of the 18 product-oneof family names ([`FAMILIES`]).
    pub family: String,
    /// The underlying currency-pair token (e.g. `"EURUSD"`). For `basket` this is
    /// the settlement / numeraire pair; the underlyings live in `terms.legs`.
    pub underlying: String,
    /// The tenor token (e.g. `"1Y"`), informational; the priced maturity is
    /// `terms.expiry_years`.
    pub tenor: String,
    /// The market context the vector is priced against.
    pub market: Market,
    /// Family-specific terms, mirroring the proto product-message field names. A
    /// free-form JSON object so a new family adds no struct churn; the generator
    /// and the conformance harness agree on each family's key set.
    pub terms: serde_json::Value,
    /// The independent-oracle expectation.
    pub expected: Expected,
    /// The conformance tolerance.
    pub tolerance: Tolerance,
}

impl GoldenVector {
    /// `true` iff this vector's family is priced by Monte-Carlo (its `expected`
    /// carries a positive `price_std_error` and conformance uses the `k · stderr`
    /// band).
    #[must_use]
    pub fn is_monte_carlo(&self) -> bool {
        MC_FAMILIES.contains(&self.family.as_str())
    }

    /// Read a `terms` field as `f64`, panicking with a clear message if absent /
    /// not a number. Used by the generator and conformance harness, which both
    /// know each family's schema.
    ///
    /// # Panics
    /// If `key` is missing or is not a JSON number.
    #[must_use]
    pub fn term_f64(&self, key: &str) -> f64 {
        self.terms
            .get(key)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_else(|| panic!("vector {} missing numeric term `{key}`", self.id))
    }

    /// Read a `terms` field as a string.
    ///
    /// # Panics
    /// If `key` is missing or is not a JSON string.
    #[must_use]
    pub fn term_str(&self, key: &str) -> &str {
        self.terms
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("vector {} missing string term `{key}`", self.id))
    }

    /// Read an optional `terms` field as `f64` (absent / JSON `null` ⇒ `None`).
    #[must_use]
    pub fn term_opt_f64(&self, key: &str) -> Option<f64> {
        match self.terms.get(key) {
            None | Some(serde_json::Value::Null) => None,
            Some(v) => Some(
                v.as_f64()
                    .unwrap_or_else(|| panic!("vector {} term `{key}` is not a number", self.id)),
            ),
        }
    }

    /// Read a `terms` field as `u64`.
    ///
    /// # Panics
    /// If `key` is missing or is not a JSON unsigned integer.
    #[must_use]
    pub fn term_u64(&self, key: &str) -> u64 {
        self.terms
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| panic!("vector {} missing u64 term `{key}`", self.id))
    }
}

/// The directory holding the frozen vector corpus, relative to the crate root.
pub const VECTORS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vectors");

/// The path to one family's vector file under [`VECTORS_DIR`].
#[must_use]
pub fn vectors_file(family: &str) -> PathBuf {
    Path::new(VECTORS_DIR).join(format!("{family}.json"))
}

/// An error loading the vector corpus.
#[derive(Debug)]
pub enum VectorError {
    /// An I/O error reading a vector file.
    Io {
        /// The file that failed.
        path: PathBuf,
        /// The underlying error text.
        source: String,
    },
    /// A JSON parse / shape error in a vector file.
    Parse {
        /// The file that failed.
        path: PathBuf,
        /// The underlying error text.
        source: String,
    },
}

impl std::fmt::Display for VectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VectorError::Io { path, source } => {
                write!(f, "reading {}: {source}", path.display())
            }
            VectorError::Parse { path, source } => {
                write!(f, "parsing {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for VectorError {}

/// Load **all** committed golden vectors from `vectors/*.json` (one file per
/// family, each a JSON array), sorted by `id` for a deterministic order.
///
/// # Errors
/// [`VectorError`] on any I/O or JSON parse failure.
pub fn load_vectors() -> Result<Vec<GoldenVector>, VectorError> {
    let mut all = Vec::new();
    for family in FAMILIES {
        // `window_barrier` shares the `single_barrier`/exotics vocabulary but is
        // its own oneof arm and its own file; every family has exactly one file.
        let path = vectors_file(family);
        if !path.exists() {
            // A missing file is a real gap the selfcheck catches (it asserts all
            // 18 families present); skip here so a partial regenerate still loads.
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| VectorError::Io {
            path: path.clone(),
            source: e.to_string(),
        })?;
        let mut vecs: Vec<GoldenVector> =
            serde_json::from_str(&text).map_err(|e| VectorError::Parse {
                path: path.clone(),
                source: e.to_string(),
            })?;
        all.append(&mut vecs);
    }
    all.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(all)
}
