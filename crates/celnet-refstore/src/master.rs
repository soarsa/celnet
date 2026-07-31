//! The mastered instrument reference record and its per-field provenance.
//!
//! `InstrumentMaster` is the golden record per security (§6.1): the vendor-neutral static terms,
//! the OSS-clean identifier triple (FIGI + LEI + natively-present ISIN/CUSIP — §4.2), the current
//! (possibly post-corporate-action) cashflow schedule, and the mastering metadata that makes the
//! store effective-dated and auditable. It is serde-clean so every version round-trips through the
//! journal and the JSON admin snapshot.

use celnet_corpactions::{BondSchedule, CivilDate};
use serde::{Deserialize, Serialize};

/// The OSS-clean identifier set for a security (§4.2). FIGI + LEI are genuinely open; ISIN is
/// carried where the security natively has one (its ISO-6166 check digit is validated on ingest);
/// CUSIP is carried opportunistically only where the security streams one — never bulk-licensed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalIds {
    /// OpenFIGI FIGI — the vendor-neutral identifier of choice (MIT / free).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub figi: Option<String>,
    /// ISO 6166 ISIN, where the security carries one (check-digit validated on ingest).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isin: Option<String>,
    /// GLEIF LEI (CC0) of the issuer entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lei_issuer: Option<String>,
    /// CUSIP, only where natively present (e.g. a US Treasury) — reference-only, never licensed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cusip: Option<String>,
}

/// The coupon type of a mastered instrument (extends `GovBondSpec`'s `fixed|zero` with `frn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CouponType {
    /// A fixed coupon.
    Fixed,
    /// A zero-coupon / discount security (no coupon flows).
    Zero,
    /// A floating-rate note (index + margin + reset — carried in `terms` labels).
    Frn,
}

/// The static contractual terms of a mastered instrument, carried as stable snake_case labels (the
/// same discipline `GovBondSpec` + the server registry use, so the store never depends on server
/// enums and stays human-editable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentTerms {
    /// A short, human-friendly blotter/GUI name.
    pub name: String,
    /// The issuer name.
    pub issuer: String,
    /// ISO 4217 currency label (`USD` / `GBP` / `EUR`).
    pub currency: String,
    /// The coupon type.
    pub coupon_type: CouponType,
    /// The annual coupon rate as a decimal (`0.0` for a zero).
    pub coupon_rate: f64,
    /// Coupons per year (`2` semi-annual, `1` annual, `4` quarterly; `0` for a zero).
    pub coupons_per_year: u32,
    /// The accrual day-count label (`act_act` / `thirty_360` / …).
    pub day_count: String,
    /// The dated (accrual-start) date, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dated_date: Option<CivilDate>,
    /// The final-redemption date.
    pub maturity_date: CivilDate,
    /// The par redemption / face value (100.0 per 100 face).
    pub redemption: f64,
    /// The settlement calendar-centre labels.
    pub calendars: Vec<String>,
}

/// Per-field mastering metadata — the survivorship + bitemporal + lineage triple (§4.3).
///
/// Attached to each stored version so a value's state "as known on `recorded_at`, effective on
/// `valid_from`" is reconstructable, and a cross-source conflict resolves by `(source priority,
/// quality)`. Recorded explicitly by the caller — the store carries **no clock** (determinism).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The source label that produced this version (e.g. `treasury_fiscaldata`, `dmo`, `derived`,
    /// or a vendor-adapter id). Drives survivorship priority.
    #[serde(default)]
    pub source_priority: u8,
    /// The originating source id / message reference for audit lineage.
    // (a String; kept separate from the u8 priority so both survive the JSON snapshot)
    #[serde(default)]
    pub source: SourceRef,
    /// The valuation date from which this version is effective (bitemporal *valid* axis).
    pub valid_from: CivilDate,
    /// The date this version became known to the store (bitemporal *recorded* axis).
    pub recorded_at: CivilDate,
    /// A per-field quality score (0..=100) used with `source_priority` in survivorship.
    #[serde(default)]
    pub quality: u8,
}

/// A source reference label (its own newtype so a default is available for serde and the priority
/// stays a separate field).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef(pub String);

impl Default for SourceRef {
    fn default() -> Self {
        Self("derived".to_string())
    }
}

impl From<&str> for SourceRef {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// One effective-dated version of the golden record for a security.
///
/// The store holds an append-only history of these; a read resolves the version effective for a
/// `(valuation, known-as-of)` pair (§7.3). The `schedule` is the current — possibly post-CA —
/// cashflow schedule, so the pricing/hedging seams (§8) re-derive off exactly this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentMaster {
    /// The canonical internal registry key.
    pub instrument_id: String,
    /// The OSS-clean external identifier set.
    pub external_ids: ExternalIds,
    /// The static contractual terms.
    pub terms: InstrumentTerms,
    /// The current cashflow schedule (post-any-applied-CA).
    pub schedule: BondSchedule,
    /// The mastering metadata for this version.
    pub provenance: Provenance,
}

impl InstrumentMaster {
    /// The ISIN if one is carried (the CA join key).
    #[must_use]
    pub fn isin(&self) -> Option<&str> {
        self.external_ids.isin.as_deref()
    }
}
