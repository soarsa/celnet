//! Persisted named interest-rate curve definitions — the multi-curve registry.
//!
//! The edge generalises the single-curve-per-currency rates model into a registry
//! of **named, definable** curves. Each definition carries reference-data metadata
//! (index label, day-count, calendar), the interpolation the engine bootstraps/reads
//! it under, its calibrating par-OIS pillars, and whether it is its currency's
//! **primary** (default) curve — the one the FIX rates edge and Excel resolve
//! against. The document is stored as a small JSON file so definitions survive
//! restarts and auto-load on boot, mirroring [`super::fix_connections`].
//!
//! This module owns only the *data + persistence + registry invariants*: the
//! wire↔domain conversion (to/from [`celnet_proto::CurveDefinition`]) and the seed
//! of the default USD-SOFR primary live in [`crate::services::surface`], which knows
//! the proto contract and the engine default, keeping this module proto-free (the
//! same separation `fix_connections` keeps from `fix_admin`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Env var naming the JSON config file. Absent ⇒ [`DEFAULT_CONFIG_PATH`].
pub const CONFIG_ENV: &str = "CELNET_CURVE_CONFIG";
/// Default config path (repo-/cwd-local, human-editable) when the env is unset.
pub const DEFAULT_CONFIG_PATH: &str = "curve-definitions.json";

/// The term-structure interpolation a curve is bootstrapped and read under. The
/// domain mirror of `celnet_proto::CurveInterpolation` and `celnet_rates::Interpolation`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveInterpolationKind {
    /// Linear in `ln DF` against time — piecewise-constant instantaneous forwards
    /// (the shipping default; proto3/serde zero value).
    #[default]
    LogLinearDf,
    /// Piecewise-quadratic, continuous, monotonicity-preserving forwards (smooth view).
    MonotoneConvexForward,
}

impl CurveInterpolationKind {
    /// A stable lowercase wire/display token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CurveInterpolationKind::LogLinearDf => "log_linear_df",
            CurveInterpolationKind::MonotoneConvexForward => "monotone_convex_forward",
        }
    }
}

/// Where a calibrating pillar matures — the domain mirror of the wire
/// `PillarTenor` oneof (whole-year tenor, month tenor, or explicit broken date).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum PillarTenorDef {
    /// A regular whole-year tenor from spot (e.g. 1, 2, 5, 10); must be `>= 1`.
    Years(u32),
    /// A month tenor from spot (e.g. 3, 18, 30) for sub-/broken-year pillars; `>= 1`.
    Months(u32),
    /// An explicit odd-dated ("broken date") maturity `(year, month, day)`.
    MaturityDate(i32, u32, u32),
}

/// One calibrating par-OIS pillar of a curve definition.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePillarDef {
    /// Where this pillar matures.
    pub tenor: PillarTenorDef,
    /// The observed par (fair fixed) rate as a decimal (0.0405 = 4.05%).
    pub par_rate: f64,
}

/// One persisted named curve definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurveDefinitionDef {
    /// Stable registry key / slug (the API key). Never reused.
    pub curve_id: String,
    /// Human-friendly display name shown in the curve picker.
    pub display_name: String,
    /// The floating-index / discount label (reference-data metadata; descriptive).
    #[serde(default)]
    pub index_label: String,
    /// The accrual day-count convention label (reference-data metadata; descriptive).
    #[serde(default)]
    pub day_count: String,
    /// The calendar / business-day convention label (reference-data metadata).
    #[serde(default)]
    pub calendar: String,
    /// The interpolation the engine bootstraps/reads this curve under (functional).
    #[serde(default)]
    pub interpolation: CurveInterpolationKind,
    /// ISO 4217 currency of the curve (upper-cased on ingest).
    pub currency: String,
    /// The curve reference (spot-anchor) civil date `(year, month, day)` the pillar
    /// schedules roll from.
    pub reference_date: (i32, u32, u32),
    /// The calibrating par-OIS pillars, in strictly increasing tenor order.
    pub pillars: Vec<CurvePillarDef>,
    /// Whether this is its currency's PRIMARY (default) curve.
    #[serde(default)]
    pub primary: bool,
}

impl CurveDefinitionDef {
    /// Structural validation independent of the rest of the registry. Cross-cutting
    /// checks (unique id, single primary) live in [`CurveDefinitionStore`]; numeric
    /// bootstrap validation lives in the handler (it knows the engine currency).
    ///
    /// # Errors
    /// Returns a human-readable message naming the first failing field.
    pub fn validate(&self) -> Result<(), String> {
        if self.curve_id.trim().is_empty() {
            return Err("curve_id must not be empty".to_string());
        }
        if self.currency.trim().is_empty() {
            return Err("currency must not be empty".to_string());
        }
        if self.pillars.is_empty() {
            return Err("a curve needs at least one pillar".to_string());
        }
        for p in &self.pillars {
            match p.tenor {
                PillarTenorDef::Years(y) | PillarTenorDef::Months(y) if y == 0 => {
                    return Err("pillar tenor must be >= 1".to_string());
                }
                PillarTenorDef::MaturityDate(y, m, d) => {
                    if time::Date::from_calendar_date(
                        y,
                        time::Month::try_from(u8::try_from(m).unwrap_or(0))
                            .map_err(|_| format!("pillar maturity month {m} is not 1..=12"))?,
                        u8::try_from(d).unwrap_or(0),
                    )
                    .is_err()
                    {
                        return Err(format!("pillar maturity {y}-{m}-{d} is not a real date"));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Why a [`CurveDefinitionStore::remove`] was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurveRemoveError {
    /// No curve with the given id.
    NotFound,
    /// The curve is its currency's primary while other curves of that currency still
    /// exist — reassign the primary before deleting it.
    PrimaryHasSiblings,
    /// The curve is the last one of the engine-priced currency — deleting it would
    /// leave the pricing engine without a curve.
    LastEngineCurve,
}

impl core::fmt::Display for CurveRemoveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            Self::NotFound => "no curve with that id",
            Self::PrimaryHasSiblings => {
                "cannot delete the primary curve while other curves of its currency exist \
                 — reassign the primary first"
            }
            Self::LastEngineCurve => {
                "cannot delete the last curve of the priced currency — pricing needs a curve"
            }
        };
        f.write_str(msg)
    }
}

/// The persisted document: an ordered list of curve definitions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CurveDefinitionStore {
    /// The ordered set of named curve definitions.
    #[serde(default)]
    pub curves: Vec<CurveDefinitionDef>,
}

impl CurveDefinitionStore {
    /// Resolve the config path from [`CONFIG_ENV`], falling back to
    /// [`DEFAULT_CONFIG_PATH`].
    #[must_use]
    pub fn config_path() -> PathBuf {
        std::env::var_os(CONFIG_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH))
    }

    /// Load from `path`. A **missing** file is a first run ⇒ empty store (not an
    /// error); a present-but-corrupt file is an `InvalidData` error so a broken
    /// config fails loudly rather than silently dropping saved curves.
    ///
    /// # Errors
    /// Propagates IO errors other than not-found, and JSON parse failures.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Persist **atomically**: pretty-print to a sibling `*.tmp` file then rename over
    /// `path`, so a crash mid-write can never leave a half-written config (the same
    /// durability discipline as [`super::fix_connections`]).
    ///
    /// # Errors
    /// Propagates IO/serialization failures.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = tmp_sibling(path);
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Borrow a curve by id.
    #[must_use]
    pub fn get(&self, curve_id: &str) -> Option<&CurveDefinitionDef> {
        self.curves.iter().find(|c| c.curve_id == curve_id)
    }

    /// The primary (default) curve for `currency` (case-insensitive), if any.
    #[must_use]
    pub fn primary_for(&self, currency: &str) -> Option<&CurveDefinitionDef> {
        self.curves
            .iter()
            .find(|c| c.primary && c.currency.eq_ignore_ascii_case(currency))
    }

    /// Count the curves of `currency` (case-insensitive).
    #[must_use]
    pub fn count_for(&self, currency: &str) -> usize {
        self.curves
            .iter()
            .filter(|c| c.currency.eq_ignore_ascii_case(currency))
            .count()
    }

    /// Insert a new curve or replace the existing one with the same id (preserving
    /// position on replace), maintaining the **single-primary-per-currency**
    /// invariant:
    ///
    /// * the currency is upper-cased on ingest;
    /// * if `def.primary`, every other curve of the same currency is demoted;
    /// * if the currency ends up with **no** primary (e.g. the first curve of a
    ///   currency, or the sole remaining one was demoted), the just-upserted curve
    ///   is promoted so a currency with curves always has exactly one primary.
    ///
    /// Returns the stored definition (post-normalisation).
    pub fn upsert(&mut self, mut def: CurveDefinitionDef) -> CurveDefinitionDef {
        def.currency = def.currency.trim().to_ascii_uppercase();
        let currency = def.currency.clone();

        if def.primary {
            for other in self.curves.iter_mut() {
                if other.curve_id != def.curve_id && other.currency.eq_ignore_ascii_case(&currency)
                {
                    other.primary = false;
                }
            }
        }

        if let Some(slot) = self.curves.iter_mut().find(|c| c.curve_id == def.curve_id) {
            *slot = def;
        } else {
            self.curves.push(def);
        }

        // Guarantee a primary exists for the currency if it has any curves.
        if self.primary_for(&currency).is_none()
            && let Some(first) = self
                .curves
                .iter_mut()
                .find(|c| c.currency.eq_ignore_ascii_case(&currency))
        {
            first.primary = true;
        }

        // Return the normalised stored copy.
        self.get(
            &self
                .curves
                .iter()
                .rev()
                .find(|c| c.currency.eq_ignore_ascii_case(&currency))
                .map(|c| c.curve_id.clone())
                .unwrap_or_default(),
        )
        .cloned()
        .unwrap_or_else(|| self.curves.last().cloned().expect("just upserted"))
    }

    /// Remove the curve `curve_id`, enforcing the delete rules. `engine_currency` is
    /// the currency the pricing engine resolves against (the last curve of which may
    /// never be deleted); pass `None` to skip that guard.
    ///
    /// After a successful removal, if the removed curve was its currency's primary and
    /// siblings remain, this would orphan them — so that case is **refused**
    /// ([`CurveRemoveError::PrimaryHasSiblings`]) rather than silently re-promoting,
    /// forcing an explicit primary reassignment.
    ///
    /// # Errors
    /// [`CurveRemoveError`] when the id is unknown or a delete rule forbids removal.
    pub fn remove(
        &mut self,
        curve_id: &str,
        engine_currency: Option<&str>,
    ) -> Result<(), CurveRemoveError> {
        let target = self
            .get(curve_id)
            .ok_or(CurveRemoveError::NotFound)?
            .clone();
        let siblings = self.count_for(&target.currency);

        if let Some(engine) = engine_currency
            && target.currency.eq_ignore_ascii_case(engine)
            && siblings == 1
        {
            return Err(CurveRemoveError::LastEngineCurve);
        }
        if target.primary && siblings > 1 {
            return Err(CurveRemoveError::PrimaryHasSiblings);
        }

        self.curves.retain(|c| c.curve_id != curve_id);
        Ok(())
    }
}

/// `path` with `.tmp` appended to its file name (a sibling temp for atomic save).
fn tmp_sibling(path: &Path) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pillar_y(years: u32, rate: f64) -> CurvePillarDef {
        CurvePillarDef {
            tenor: PillarTenorDef::Years(years),
            par_rate: rate,
        }
    }

    fn sample(id: &str, currency: &str, primary: bool) -> CurveDefinitionDef {
        CurveDefinitionDef {
            curve_id: id.to_string(),
            display_name: format!("{currency} curve"),
            index_label: format!("{currency}-SOFR"),
            day_count: "ACT/360".to_string(),
            calendar: "US / Modified Following".to_string(),
            interpolation: CurveInterpolationKind::LogLinearDf,
            currency: currency.to_string(),
            reference_date: (2026, 6, 25),
            pillars: vec![pillar_y(1, 0.043), pillar_y(2, 0.041), pillar_y(5, 0.0405)],
            primary,
        }
    }

    #[test]
    fn json_round_trips_through_store() {
        let mut store = CurveDefinitionStore::default();
        store.upsert(sample("usd-sofr", "USD", true));
        let bytes = serde_json::to_vec(&store).unwrap();
        let back: CurveDefinitionStore = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(store, back);
        let d = back.get("usd-sofr").unwrap();
        assert_eq!(d.interpolation, CurveInterpolationKind::LogLinearDf);
        assert!(d.primary);
        assert_eq!(d.pillars.len(), 3);
    }

    #[test]
    fn monotone_convex_pillar_and_broken_date_round_trip() {
        let mut def = sample("mc", "USD", false);
        def.interpolation = CurveInterpolationKind::MonotoneConvexForward;
        def.pillars.push(CurvePillarDef {
            tenor: PillarTenorDef::MaturityDate(2031, 12, 19),
            par_rate: 0.0402,
        });
        def.pillars.push(CurvePillarDef {
            tenor: PillarTenorDef::Months(18),
            par_rate: 0.0421,
        });
        let json = serde_json::to_string(&def).unwrap();
        let back: CurveDefinitionDef = serde_json::from_str(&json).unwrap();
        assert_eq!(def, back);
        assert_eq!(
            back.interpolation,
            CurveInterpolationKind::MonotoneConvexForward
        );
    }

    #[test]
    fn missing_file_is_an_empty_store() {
        let dir = std::env::temp_dir().join("celnet-curvecfg-missing");
        let path = dir.join("does-not-exist.json");
        let _ = std::fs::remove_file(&path);
        let store = CurveDefinitionStore::load(&path).unwrap();
        assert!(store.curves.is_empty());
    }

    #[test]
    fn save_is_atomic_and_reloads_equal() {
        let dir = std::env::temp_dir().join("celnet-curvecfg-save");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("curves-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut store = CurveDefinitionStore::default();
        store.upsert(sample("usd-sofr", "USD", true));
        store.save(&path).unwrap();
        assert!(!tmp_sibling(&path).exists(), "atomic temp left behind");

        let reloaded = CurveDefinitionStore::load(&path).unwrap();
        assert_eq!(store, reloaded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upsert_normalises_currency_and_promotes_first_primary() {
        let mut store = CurveDefinitionStore::default();
        // A first curve of a currency that did NOT ask to be primary is still
        // promoted (a currency with curves always has exactly one primary).
        let stored = store.upsert(sample("usd-sofr", "usd", false));
        assert_eq!(stored.currency, "USD", "currency upper-cased on ingest");
        assert!(
            stored.primary,
            "first curve of a currency is promoted to primary"
        );
        assert!(store.primary_for("USD").is_some());
    }

    #[test]
    fn upsert_second_primary_demotes_the_first() {
        let mut store = CurveDefinitionStore::default();
        store.upsert(sample("usd-sofr", "USD", true));
        store.upsert(sample("usd-sofr-2", "USD", true));
        // Exactly one USD primary, and it is the newly-designated one.
        let primaries: Vec<&str> = store
            .curves
            .iter()
            .filter(|c| c.primary && c.currency == "USD")
            .map(|c| c.curve_id.as_str())
            .collect();
        assert_eq!(primaries, vec!["usd-sofr-2"]);
        assert!(!store.get("usd-sofr").unwrap().primary);
    }

    #[test]
    fn second_non_primary_curve_coexists_without_touching_primary() {
        let mut store = CurveDefinitionStore::default();
        store.upsert(sample("usd-sofr", "USD", true));
        store.upsert(sample("usd-sofr-alt", "USD", false));
        assert_eq!(store.count_for("USD"), 2);
        assert_eq!(store.primary_for("USD").unwrap().curve_id, "usd-sofr");
    }

    #[test]
    fn remove_refuses_last_engine_curve() {
        let mut store = CurveDefinitionStore::default();
        store.upsert(sample("usd-sofr", "USD", true));
        assert_eq!(
            store.remove("usd-sofr", Some("USD")),
            Err(CurveRemoveError::LastEngineCurve)
        );
        // But a non-engine currency's last curve is removable.
        store.upsert(sample("eur-estr", "EUR", true));
        assert!(store.remove("eur-estr", Some("USD")).is_ok());
    }

    #[test]
    fn remove_refuses_primary_with_siblings() {
        let mut store = CurveDefinitionStore::default();
        store.upsert(sample("usd-sofr", "USD", true));
        store.upsert(sample("usd-sofr-alt", "USD", false));
        assert_eq!(
            store.remove("usd-sofr", Some("USD")),
            Err(CurveRemoveError::PrimaryHasSiblings)
        );
        // The non-primary sibling is removable, leaving the primary intact.
        assert!(store.remove("usd-sofr-alt", Some("USD")).is_ok());
        assert_eq!(store.count_for("USD"), 1);
        assert!(store.get("usd-sofr").unwrap().primary);
    }

    #[test]
    fn remove_unknown_is_not_found() {
        let mut store = CurveDefinitionStore::default();
        assert_eq!(
            store.remove("nope", Some("USD")),
            Err(CurveRemoveError::NotFound)
        );
    }

    #[test]
    fn validate_rejects_empty_and_bad_pillars() {
        assert!(sample("usd-sofr", "USD", true).validate().is_ok());

        let mut no_ccy = sample("x", "  ", true);
        no_ccy.currency = "  ".to_string();
        assert!(no_ccy.validate().is_err());

        let mut no_pillars = sample("x", "USD", true);
        no_pillars.pillars.clear();
        assert!(no_pillars.validate().is_err());

        let mut zero_tenor = sample("x", "USD", true);
        zero_tenor.pillars = vec![pillar_y(0, 0.04)];
        assert!(zero_tenor.validate().is_err());

        let mut bad_date = sample("x", "USD", true);
        bad_date.pillars = vec![CurvePillarDef {
            tenor: PillarTenorDef::MaturityDate(2031, 13, 40),
            par_rate: 0.04,
        }];
        assert!(bad_date.validate().is_err());
    }
}
