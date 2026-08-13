//! Persisted **instrument reference data**: the admin-managed registry of
//! instrument *definitions* (static / reference data) that curve-building and
//! pricing will later resolve against
//! (`docs/CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md` §B/§C).
//!
//! This module owns only the *data + validation + seed* of the registry; it is
//! persisted inside the same JSON document as the identity / Entity / Book store
//! ([`super::identity::IdentityStore`]) so it auto-loads on boot and survives
//! restarts on the existing durability chassis (atomic temp-then-rename save).
//! The management RPCs live on `AuthService`; the WS mirror lives in
//! [`crate::ws::codec`].
//!
//! # Keying & resolution
//!
//! Every definition is keyed by a stable internal [`InstrumentDef::instrument_id`]
//! slug and carries a list of [`ExternalId`] cross-refs (ISIN/CUSIP/SEDOL/FIGI/
//! ticker/internal). The resolver seam the next phase consumes is
//! [`instrument_by_id`] / [`instrument_by_external_id`] / [`list`](IdentityStore::instruments)
//! — pricing and curve-building resolve a *ref → definition* instead of carrying
//! inline conventions.
//!
//! # Convention labels reuse the engine vocabularies
//!
//! Convention values are stored as stable snake_case **labels** (the same
//! human-editable, exactly-round-tripping discipline as the capability overlay),
//! and every label is validated against the *existing* engine enums so the
//! registry can never name a convention the engine cannot resolve:
//!
//! * `day_count` ⇒ [`celnet_rates::AccrualBasis`] (`act_360` / `act_365_fixed` /
//!   `thirty_360_bond_basis`), plus `act_act` accepted for the bond family (the
//!   one convention not yet in an engine enum — resolved by the future bond
//!   pricer; CLAUDE.md §10 "extend only if a needed convention is missing").
//! * `business_day_convention` ⇒ [`celnet_calendar::RollRule`].
//! * each `calendars` entry ⇒ [`celnet_calendar::CentreId`].
//! * a leg `*_frequency` ⇒ [`celnet_rates::PaymentFrequency`].
//!
//! Period-generation `roll_convention` (`eom` / `imm` / `none`) has no engine
//! enum yet, so its accepted token set is defined here and resolved later.

use celnet_calendar::{CentreId, RollRule};
use celnet_rates::{AccrualBasis, vanilla_swap::PaymentFrequency};
use serde::{Deserialize, Serialize};

// --- external identifiers ---------------------------------------------------

/// The recognised external-identifier schemes (ISO 6166 ISIN, CUSIP, SEDOL,
/// FIGI, a vendor/display ticker, or a free internal handle). Stored as a stable
/// snake_case label on [`ExternalId::scheme`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalScheme {
    /// ISO 6166 ISIN.
    Isin,
    /// CUSIP.
    Cusip,
    /// SEDOL.
    Sedol,
    /// FIGI.
    Figi,
    /// A vendor/display ticker (RIC-style).
    Ticker,
    /// A free internal handle.
    Internal,
}

impl ExternalScheme {
    /// The stable snake_case label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Isin => "isin",
            Self::Cusip => "cusip",
            Self::Sedol => "sedol",
            Self::Figi => "figi",
            Self::Ticker => "ticker",
            Self::Internal => "internal",
        }
    }

    /// Parse the snake_case label (case-insensitive).
    #[must_use]
    pub fn from_label(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "isin" => Some(Self::Isin),
            "cusip" => Some(Self::Cusip),
            "sedol" => Some(Self::Sedol),
            "figi" => Some(Self::Figi),
            "ticker" => Some(Self::Ticker),
            "internal" => Some(Self::Internal),
            _ => None,
        }
    }
}

/// One external-identifier cross-reference: a `(scheme, value)` pair, stored as
/// labels so the JSON stays human-editable and round-trips exactly. The pair is
/// unique across the whole registry (validated at load and at every admin write).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalId {
    /// The scheme label (`isin` / `cusip` / `sedol` / `figi` / `ticker` / `internal`).
    pub scheme: String,
    /// The identifier value under that scheme (verbatim).
    pub value: String,
}

// --- family convention blocks ----------------------------------------------

/// Rates — a cash money-market deposit definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositDef {
    /// The reference overnight/term index token (e.g. `sofr`).
    pub index: String,
    /// The deposit tenor token (e.g. `ON`, `1M`, `3M`) or a broken-date token.
    pub tenor: String,
    /// Accrual day-count label.
    pub day_count: String,
    /// Business-day convention label.
    pub business_day_convention: String,
    /// Settlement calendar centre labels (at least one).
    #[serde(default)]
    pub calendars: Vec<String>,
    /// Spot/settlement lag in business days.
    pub spot_lag_days: u32,
}

/// Rates — a forward rate agreement (FRA) definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FraDef {
    /// The projected float index token.
    pub float_index: String,
    /// The window start / fixing tenor or broken-date token.
    pub start_tenor: String,
    /// The window end / maturity tenor or broken-date token.
    pub end_tenor: String,
    /// Accrual day-count label for the `tau` basis.
    pub accrual_day_count: String,
    /// Business-day convention label.
    pub business_day_convention: String,
    /// Settlement calendar centre labels (at least one).
    #[serde(default)]
    pub calendars: Vec<String>,
    /// Spot/settlement lag in business days.
    pub spot_lag_days: u32,
}

/// Rates — a short-term-interest-rate (STIR) futures contract definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StirFutureDef {
    /// The listed contract code (e.g. `SR3H6`).
    pub contract_code: String,
    /// The fixing-window start tenor/date token.
    pub reference_start: String,
    /// The fixing-window end tenor/date token.
    pub reference_end: String,
    /// Accrual day-count label.
    pub day_count: String,
    /// Settlement calendar centre labels (at least one).
    #[serde(default)]
    pub calendars: Vec<String>,
    /// Convexity-adjustment volatility (σ); 0 ⇒ no adjustment.
    #[serde(default)]
    pub convexity_vol: f64,
    /// Contract notional/size in the currency; 0 ⇒ unspecified.
    #[serde(default)]
    pub contract_size: f64,
}

/// Rates — a listed **bond (Treasury) futures** contract definition: the benchmark
/// hedge vehicle an interest-rate exposure is transferred into.
///
/// Every field is a published contract term (the CBOT rulebook chapters cited on
/// [`celnet_refdata::TreasuryFutureSpec`]) except
/// [`dv01_per_contract_at_notional_yield`](Self::dv01_per_contract_at_notional_yield),
/// which is derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BondFutureDef {
    /// The listed contract code (e.g. `ZNZ26`) — also the registry `instrument_id`.
    pub contract_code: String,
    /// The product symbol without the delivery month (`ZT`/`ZF`/`ZN`/`TN`/`ZB`/`UB`).
    pub contract_symbol: String,
    /// The issuer of the deliverable securities, e.g. `US Treasury`.
    pub underlying_issuer: String,
    /// Face value at maturity of the deliverable, per contract.
    pub contract_face_value: f64,
    /// The minimum price increment for an OUTRIGHT trade, in points of 100 face.
    pub tick_size_points: f64,
    /// The cash value of one minimum price increment, per contract.
    pub tick_value: f64,
    /// The notional coupon the conversion factor is defined against (0.06 for every
    /// CBOT Treasury futures contract).
    pub notional_coupon_rate: f64,
    /// Shortest deliverable remaining term to maturity, in months.
    pub deliverable_min_months: u32,
    /// Longest deliverable remaining term to maturity, in months.
    pub deliverable_max_months: u32,
    /// The first calendar day of the delivery month.
    pub delivery_month_start: CivilDate,
    /// The first delivery day (first business day of the delivery month).
    pub first_delivery_date: CivilDate,
    /// The last day the contract trades.
    pub last_trading_date: CivilDate,
    /// The last delivery day.
    pub last_delivery_date: CivilDate,
    /// The contract's **standardized** DV01 per contract, in the contract currency per
    /// basis point, at the contract's own 6% notional yield.
    ///
    /// Derived, not published: a futures contract has no constant basis-point value —
    /// the live figure is the cheapest-to-deliver security's DV01 divided by its
    /// conversion factor and moves daily with the yield level. This is the stable,
    /// displayable figure. **A hedge must be sized off the live curve yield**
    /// (`celnet_refdata::TreasuryFutureSpec::dv01_per_contract`), because this one
    /// understates the live basis-point value whenever yields are below 6%.
    pub dv01_per_contract_at_notional_yield: f64,
    /// Settlement calendar centre labels (at least one).
    #[serde(default)]
    pub calendars: Vec<String>,
}

/// Rates — a vanilla fixed-vs-float interest-rate swap (IRS) definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VanillaIrsDef {
    /// The swap tenor or broken-date maturity token.
    pub tenor: String,
    /// Fixed-leg payment frequency label.
    pub fixed_frequency: String,
    /// Fixed-leg accrual day-count label.
    pub fixed_day_count: String,
    /// The float-leg projected index token.
    pub float_index: String,
    /// Float-leg payment frequency label.
    pub float_frequency: String,
    /// Float-leg accrual day-count label.
    pub float_day_count: String,
    /// Business-day convention label.
    pub business_day_convention: String,
    /// Settlement calendar centre labels (at least one).
    #[serde(default)]
    pub calendars: Vec<String>,
    /// Period-generation roll convention token (`eom` / `imm` / `none`); blank ⇒ `none`.
    #[serde(default)]
    pub roll_convention: String,
    /// Spot/settlement lag in business days.
    pub spot_lag_days: u32,
}

/// Rates — an overnight-indexed swap (OIS) definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OisDef {
    /// The swap tenor or broken-date maturity token.
    pub tenor: String,
    /// The overnight index token (e.g. `sofr`).
    pub index: String,
    /// Fixed-leg payment frequency label.
    pub fixed_frequency: String,
    /// Fixed-leg accrual day-count label.
    pub fixed_day_count: String,
    /// Float (compounded-ON) leg accrual day-count label.
    pub float_day_count: String,
    /// Business-day convention label.
    pub business_day_convention: String,
    /// Settlement calendar centre labels (at least one).
    #[serde(default)]
    pub calendars: Vec<String>,
    /// Spot/settlement lag in business days.
    pub spot_lag_days: u32,
}

/// A civil (broken) date — mirrors the wire `BrokenDate`. Stored structurally so
/// the JSON round-trips and the next phase maps it to a calendar `Date`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CivilDate {
    /// Gregorian year (e.g. 2026).
    pub year: i32,
    /// Month of year, 1..=12.
    pub month: u32,
    /// Day of month, 1..=31 (validated against the month).
    pub day: u32,
}

impl CivilDate {
    /// Whether this is a valid civil date (the month/day combine to a real date).
    #[must_use]
    pub fn is_valid(self) -> bool {
        let Ok(month) = u8::try_from(self.month) else {
            return false;
        };
        let Ok(month) = time::Month::try_from(month) else {
            return false;
        };
        let Ok(day) = u8::try_from(self.day) else {
            return false;
        };
        time::Date::from_calendar_date(self.year, month, day).is_ok()
    }
}

/// A cash-bond definition (no bond pricer yet — feeds the future bond pricing
/// path and the FI deal blotter). ISIN/CUSIP live in the instrument's
/// `external_ids`; redemption/face and the dates live here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BondDef {
    /// The issuer name (or LEI).
    pub issuer: String,
    /// The annual coupon rate as a decimal; 0 for zeros/FRNs.
    #[serde(default)]
    pub coupon_rate: f64,
    /// The coupon type token (`fixed` / `frn` / `zero`).
    pub coupon_type: String,
    /// Coupon payment frequency label; blank for `zero`.
    #[serde(default)]
    pub coupon_frequency: String,
    /// Accrual day-count label.
    pub day_count: String,
    /// The issue date; optional.
    #[serde(default)]
    pub issue_date: Option<CivilDate>,
    /// The dated (accrual-start) date; optional.
    #[serde(default)]
    pub dated_date: Option<CivilDate>,
    /// The first-coupon date (drives a stub); optional.
    #[serde(default)]
    pub first_coupon_date: Option<CivilDate>,
    /// The maturity (final-redemption) date; required.
    pub maturity_date: CivilDate,
    /// The par redemption / face value (strictly positive).
    pub redemption: f64,
    /// Settlement calendar centre labels (at least one).
    #[serde(default)]
    pub calendars: Vec<String>,
}

/// The family-specific convention block of an instrument definition — exactly one
/// variant, mirroring the wire `InstrumentDefDesc.definition` oneof. Serialised
/// externally-tagged so the JSON carries a self-describing `{ "deposit": { … } }`
/// shape that round-trips exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentFamily {
    /// A cash money-market deposit.
    Deposit(DepositDef),
    /// A forward rate agreement.
    Fra(FraDef),
    /// A STIR futures contract.
    StirFuture(StirFutureDef),
    /// A vanilla fixed-vs-float IRS.
    VanillaIrs(VanillaIrsDef),
    /// An overnight-indexed swap.
    Ois(OisDef),
    /// A cash bond.
    Bond(BondDef),
    /// A listed bond (Treasury) futures contract.
    BondFuture(BondFutureDef),
}

impl InstrumentFamily {
    /// A stable token naming the family (for diagnostics/GUI).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Deposit(_) => "deposit",
            Self::Fra(_) => "fra",
            Self::StirFuture(_) => "stir_future",
            Self::VanillaIrs(_) => "vanilla_irs",
            Self::Ois(_) => "ois",
            Self::Bond(_) => "bond",
            Self::BondFuture(_) => "bond_future",
        }
    }
}

/// One persisted instrument definition: the common header plus its family block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentDef {
    /// The stable internal id (a slug); the registry key. Unique across the registry.
    pub instrument_id: String,
    /// Human-friendly label for blotters/GUI.
    pub name: String,
    /// Optional longer human description.
    #[serde(default)]
    pub description: String,
    /// ISO 4217 pricing/settlement currency.
    pub currency: String,
    /// External-identifier cross-refs; each `(scheme, value)` is unique registry-wide.
    #[serde(default)]
    pub external_ids: Vec<ExternalId>,
    /// The sub-asset-type taxonomy label — what KIND of risk this is within its
    /// family (`government` / `corporate` / `government_future` / `rate_future`).
    ///
    /// Stored rather than derived from the issuer name: a client that has to guess
    /// "is `Alpine Financial SA` a sovereign?" will eventually guess wrong, and the
    /// failure mode is advertising credit risk as government risk. Blank when the
    /// definition carries no classification.
    #[serde(default)]
    pub sub_asset_type: String,
    /// The region taxonomy label (`us` / `uk` / `de` / `fr` / `it`, EUR issuers
    /// rolling up to `eu`). Blank when unclassified.
    #[serde(default)]
    pub region: String,
    /// The family-specific convention block.
    pub definition: InstrumentFamily,
}

// --- label parsing (reuse of engine vocabularies) ---------------------------

/// Whether a day-count label is recognised. The three engine bases come from
/// [`AccrualBasis`]; `act_act` is additionally accepted for the bond family (the
/// one missing convention, resolved by the future bond pricer).
#[must_use]
pub fn day_count_is_known(label: &str) -> bool {
    accrual_basis_from_label(label).is_some() || label.eq_ignore_ascii_case("act_act")
}

/// Map a day-count label onto the engine [`AccrualBasis`], where one exists
/// (`act_act` returns `None` — it has no engine basis yet). The resolver seam the
/// next phase uses to turn a stored rates definition into an engine schedule.
#[must_use]
pub fn accrual_basis_from_label(label: &str) -> Option<AccrualBasis> {
    match label.trim().to_ascii_lowercase().as_str() {
        "act_360" => Some(AccrualBasis::Act360),
        "act_365_fixed" => Some(AccrualBasis::Act365Fixed),
        "thirty_360_bond_basis" => Some(AccrualBasis::Thirty360BondBasis),
        _ => None,
    }
}

/// Map a business-day-convention label onto the engine [`RollRule`].
#[must_use]
pub fn roll_rule_from_label(label: &str) -> Option<RollRule> {
    match label.trim().to_ascii_lowercase().as_str() {
        "unadjusted" => Some(RollRule::Unadjusted),
        "following" => Some(RollRule::Following),
        "preceding" => Some(RollRule::Preceding),
        "modified_following" => Some(RollRule::ModifiedFollowing),
        _ => None,
    }
}

/// Map a calendar-centre label onto the engine [`CentreId`].
#[must_use]
pub fn centre_from_label(label: &str) -> Option<CentreId> {
    match label.trim().to_ascii_lowercase().as_str() {
        "united_states" => Some(CentreId::UnitedStates),
        "target2" => Some(CentreId::Target2),
        "united_kingdom" => Some(CentreId::UnitedKingdom),
        "japan" => Some(CentreId::Japan),
        "switzerland" => Some(CentreId::Switzerland),
        "australia" => Some(CentreId::Australia),
        "canada" => Some(CentreId::Canada),
        "new_zealand" => Some(CentreId::NewZealand),
        "mexico" => Some(CentreId::Mexico),
        "south_africa" => Some(CentreId::SouthAfrica),
        "norway" => Some(CentreId::Norway),
        "sweden" => Some(CentreId::Sweden),
        _ => None,
    }
}

/// Map a payment-frequency label onto the engine [`PaymentFrequency`].
#[must_use]
pub fn payment_frequency_from_label(label: &str) -> Option<PaymentFrequency> {
    match label.trim().to_ascii_lowercase().as_str() {
        "annual" => Some(PaymentFrequency::Annual),
        "semi_annual" => Some(PaymentFrequency::SemiAnnual),
        "quarterly" => Some(PaymentFrequency::Quarterly),
        _ => None,
    }
}

/// Whether a period-generation roll-convention token is recognised (`eom` /
/// `imm` / `none`; blank is treated as `none`).
#[must_use]
pub fn roll_convention_is_known(label: &str) -> bool {
    matches!(
        label.trim().to_ascii_lowercase().as_str(),
        "" | "eom" | "imm" | "none"
    )
}

/// Whether a bond coupon-type token is recognised (`fixed` / `frn` / `zero`).
#[must_use]
pub fn coupon_type_is_known(label: &str) -> bool {
    matches!(
        label.trim().to_ascii_lowercase().as_str(),
        "fixed" | "frn" | "zero"
    )
}

// --- validation -------------------------------------------------------------

/// Validate the full instrument registry: unique `instrument_id`s, globally
/// unique `(scheme, value)` external-id pairs, and per-family required-field
/// presence + valid convention labels. Called at load (fail-fast) and before
/// every admin write so a corrupt registry never silently mis-resolves.
///
/// # Errors
/// The first duplicate id, duplicate external id, unknown enum label, or missing
/// required field.
pub fn validate_instruments(instruments: &[InstrumentDef]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    let mut ext = std::collections::HashSet::new();
    for inst in instruments {
        if inst.instrument_id.trim().is_empty() {
            return Err("an instrument has an empty instrument_id".to_string());
        }
        if inst.name.trim().is_empty() {
            return Err(format!(
                "instrument {:?} has an empty name",
                inst.instrument_id
            ));
        }
        if inst.currency.trim().is_empty() {
            return Err(format!(
                "instrument {:?} has an empty currency",
                inst.instrument_id
            ));
        }
        if !ids.insert(inst.instrument_id.to_ascii_lowercase()) {
            return Err(format!("duplicate instrument_id {:?}", inst.instrument_id));
        }
        for id in &inst.external_ids {
            if ExternalScheme::from_label(&id.scheme).is_none() {
                return Err(format!(
                    "instrument {:?}: unknown external-id scheme {:?}",
                    inst.instrument_id, id.scheme
                ));
            }
            if id.value.trim().is_empty() {
                return Err(format!(
                    "instrument {:?}: external id with scheme {:?} has an empty value",
                    inst.instrument_id, id.scheme
                ));
            }
            let key = (
                id.scheme.to_ascii_lowercase(),
                id.value.to_ascii_lowercase(),
            );
            if !ext.insert(key) {
                return Err(format!(
                    "duplicate external id ({}, {}) on instrument {:?}",
                    id.scheme, id.value, inst.instrument_id
                ));
            }
        }
        validate_family(&inst.instrument_id, &inst.definition)?;
    }
    Ok(())
}

/// Validate a single family's required fields and convention labels.
fn validate_family(id: &str, fam: &InstrumentFamily) -> Result<(), String> {
    let ctx = |field: &str| format!("instrument {id:?} ({}): {field}", fam.kind());
    match fam {
        InstrumentFamily::Deposit(d) => {
            require(&d.index, || ctx("index is required"))?;
            require(&d.tenor, || ctx("tenor is required"))?;
            check_day_count(&d.day_count, &ctx)?;
            check_bdc(&d.business_day_convention, &ctx)?;
            check_calendars(&d.calendars, &ctx)?;
        }
        InstrumentFamily::Fra(f) => {
            require(&f.float_index, || ctx("float_index is required"))?;
            require(&f.start_tenor, || ctx("start_tenor is required"))?;
            require(&f.end_tenor, || ctx("end_tenor is required"))?;
            check_day_count(&f.accrual_day_count, &ctx)?;
            check_bdc(&f.business_day_convention, &ctx)?;
            check_calendars(&f.calendars, &ctx)?;
        }
        InstrumentFamily::StirFuture(s) => {
            require(&s.contract_code, || ctx("contract_code is required"))?;
            require(&s.reference_start, || ctx("reference_start is required"))?;
            require(&s.reference_end, || ctx("reference_end is required"))?;
            check_day_count(&s.day_count, &ctx)?;
            check_calendars(&s.calendars, &ctx)?;
            if !(s.convexity_vol.is_finite() && s.convexity_vol >= 0.0) {
                return Err(ctx("convexity_vol must be a finite, non-negative number"));
            }
            if !(s.contract_size.is_finite() && s.contract_size >= 0.0) {
                return Err(ctx("contract_size must be a finite, non-negative number"));
            }
        }
        InstrumentFamily::VanillaIrs(v) => {
            require(&v.tenor, || ctx("tenor is required"))?;
            check_frequency(&v.fixed_frequency, "fixed_frequency", &ctx)?;
            check_day_count(&v.fixed_day_count, &ctx)?;
            require(&v.float_index, || ctx("float_index is required"))?;
            check_frequency(&v.float_frequency, "float_frequency", &ctx)?;
            check_day_count(&v.float_day_count, &ctx)?;
            check_bdc(&v.business_day_convention, &ctx)?;
            check_calendars(&v.calendars, &ctx)?;
            if !roll_convention_is_known(&v.roll_convention) {
                return Err(ctx(&format!(
                    "unknown roll_convention {:?}",
                    v.roll_convention
                )));
            }
        }
        InstrumentFamily::Ois(o) => {
            require(&o.tenor, || ctx("tenor is required"))?;
            require(&o.index, || ctx("index is required"))?;
            check_frequency(&o.fixed_frequency, "fixed_frequency", &ctx)?;
            check_day_count(&o.fixed_day_count, &ctx)?;
            check_day_count(&o.float_day_count, &ctx)?;
            check_bdc(&o.business_day_convention, &ctx)?;
            check_calendars(&o.calendars, &ctx)?;
        }
        InstrumentFamily::Bond(b) => {
            require(&b.issuer, || ctx("issuer is required"))?;
            if !coupon_type_is_known(&b.coupon_type) {
                return Err(ctx(&format!("unknown coupon_type {:?}", b.coupon_type)));
            }
            // A non-zero coupon needs a frequency; a zero-coupon bond must not.
            let is_zero = b.coupon_type.eq_ignore_ascii_case("zero");
            if is_zero {
                if !b.coupon_frequency.trim().is_empty() {
                    return Err(ctx("a zero-coupon bond must not carry a coupon_frequency"));
                }
            } else {
                check_frequency(&b.coupon_frequency, "coupon_frequency", &ctx)?;
            }
            check_day_count(&b.day_count, &ctx)?;
            check_calendars(&b.calendars, &ctx)?;
            if !b.maturity_date.is_valid() {
                return Err(ctx("maturity_date is not a valid civil date"));
            }
            for (field, opt) in [
                ("issue_date", b.issue_date),
                ("dated_date", b.dated_date),
                ("first_coupon_date", b.first_coupon_date),
            ] {
                if let Some(d) = opt
                    && !d.is_valid()
                {
                    return Err(ctx(&format!("{field} is not a valid civil date")));
                }
            }
            if !(b.redemption.is_finite() && b.redemption > 0.0) {
                return Err(ctx("redemption must be a finite, strictly-positive number"));
            }
            if !(b.coupon_rate.is_finite() && b.coupon_rate >= 0.0) {
                return Err(ctx("coupon_rate must be a finite, non-negative number"));
            }
        }
        InstrumentFamily::BondFuture(f) => {
            require(&f.contract_code, || ctx("contract_code is required"))?;
            require(&f.contract_symbol, || ctx("contract_symbol is required"))?;
            require(&f.underlying_issuer, || {
                ctx("underlying_issuer is required")
            })?;
            check_calendars(&f.calendars, &ctx)?;
            for (field, value) in [
                ("contract_face_value", f.contract_face_value),
                ("tick_size_points", f.tick_size_points),
                ("tick_value", f.tick_value),
            ] {
                if !(value.is_finite() && value > 0.0) {
                    return Err(ctx(&format!(
                        "{field} must be a finite, strictly-positive number"
                    )));
                }
            }
            if !(f.notional_coupon_rate.is_finite() && f.notional_coupon_rate > 0.0) {
                return Err(ctx(
                    "notional_coupon_rate must be a finite, strictly-positive number",
                ));
            }
            // The DV01 sizes every hedge routed at this contract; a non-positive or
            // non-finite value would silently produce a nonsense contract count.
            if !(f.dv01_per_contract_at_notional_yield.is_finite()
                && f.dv01_per_contract_at_notional_yield > 0.0)
            {
                return Err(ctx(
                    "dv01_per_contract_at_notional_yield must be a finite, strictly-positive number",
                ));
            }
            if f.deliverable_min_months == 0 || f.deliverable_max_months < f.deliverable_min_months
            {
                return Err(ctx(
                    "the deliverable maturity window must be non-empty and correctly ordered",
                ));
            }
            for (field, date) in [
                ("delivery_month_start", f.delivery_month_start),
                ("first_delivery_date", f.first_delivery_date),
                ("last_trading_date", f.last_trading_date),
                ("last_delivery_date", f.last_delivery_date),
            ] {
                if !date.is_valid() {
                    return Err(ctx(&format!("{field} is not a valid civil date")));
                }
            }
        }
    }
    Ok(())
}

fn require(value: &str, msg: impl Fn() -> String) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(msg())
    } else {
        Ok(())
    }
}

fn check_day_count(label: &str, ctx: &impl Fn(&str) -> String) -> Result<(), String> {
    if day_count_is_known(label) {
        Ok(())
    } else {
        Err(ctx(&format!("unknown day_count {label:?}")))
    }
}

fn check_bdc(label: &str, ctx: &impl Fn(&str) -> String) -> Result<(), String> {
    if roll_rule_from_label(label).is_some() {
        Ok(())
    } else {
        Err(ctx(&format!("unknown business_day_convention {label:?}")))
    }
}

fn check_calendars(calendars: &[String], ctx: &impl Fn(&str) -> String) -> Result<(), String> {
    if calendars.is_empty() {
        return Err(ctx("at least one calendar is required"));
    }
    for c in calendars {
        if centre_from_label(c).is_none() {
            return Err(ctx(&format!("unknown calendar {c:?}")));
        }
    }
    Ok(())
}

fn check_frequency(label: &str, field: &str, ctx: &impl Fn(&str) -> String) -> Result<(), String> {
    if payment_frequency_from_label(label).is_some() {
        Ok(())
    } else {
        Err(ctx(&format!("unknown {field} {label:?}")))
    }
}

// --- id minting & resolution ------------------------------------------------

/// Mint a stable, unique, URL-safe id for a new instrument from its name,
/// disambiguating against the existing set with a numeric suffix.
#[must_use]
pub fn mint_instrument_id(name: &str, existing: &[InstrumentDef]) -> String {
    let base = slugify(name);
    let base = if base.is_empty() {
        "instrument".to_string()
    } else {
        base
    };
    unique_id(&base, |cand| {
        existing.iter().any(|i| i.instrument_id == cand)
    })
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = false;
    for ch in s.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn unique_id(base: &str, mut taken: impl FnMut(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|cand| !taken(cand))
        .unwrap_or_else(|| base.to_string())
}

// --- seed -------------------------------------------------------------------

/// Map the curated [`celnet_refdata`] government universe (US Treasuries + UK gilts +
/// EUR govvies) onto instrument reference-data definitions ready to seed the registry:
/// the friendly name becomes the composite / blotter display name and the ISIN (+ CUSIP
/// for US) become external cross-refs. Each `instrument_id` matches what the LP-SIM feed
/// streams, so a seeded entry resolves the exact wire id and the FI Aggregated Book tiles
/// show a real bond name instead of a bare code.
///
/// The universe is NOT government-only despite the name: [`celnet_refdata::government_universe`]
/// also carries the curated EUR credit issuers, and the two are told apart by the
/// [`InstrumentDef::sub_asset_type`] label carried through from
/// [`celnet_refdata::GovBondSpec`] — never by pattern-matching the issuer name. The
/// FIX / RFS security list is still answered verbatim from the advertised universe (no
/// download-time region / sub-asset filter is wired).
#[must_use]
pub fn government_bond_defs() -> Vec<InstrumentDef> {
    celnet_refdata::government_universe()
        .into_iter()
        .map(gov_bond_to_instrument_def)
        .collect()
}

/// Map the committed [`celnet_refdata::treasury_futures_universe`] onto instrument
/// reference-data definitions ready to seed the registry, so the server KNOWS the
/// listed Treasury futures complex exists and an aggregated book carrying those
/// contracts resolves a real contract name instead of a bare code.
///
/// Each `instrument_id` is the public contract code the LP-SIM feed streams, so a
/// seeded entry resolves the exact wire id — the same tradeable/quotable symmetry the
/// government-bond seed relies on. The DV01 carried on the definition is the
/// **standardized** figure at the contract's 6% notional yield; see
/// [`BondFutureDef::dv01_per_contract_at_notional_yield`] for why a hedge must not be
/// sized off it.
///
/// A contract whose DV01 will not derive is **skipped**, not seeded with a placeholder:
/// an instrument advertised as tradeable with a fabricated hedge ratio is worse than
/// one that is absent.
#[must_use]
pub fn treasury_future_defs() -> Vec<InstrumentDef> {
    celnet_refdata::treasury_futures_universe()
        .into_iter()
        .filter_map(treasury_future_to_instrument_def)
        .collect()
}

/// Convert one [`celnet_refdata::TreasuryFutureSpec`] into an [`InstrumentDef`] with a
/// [`InstrumentFamily::BondFuture`] family block, or `None` if its DV01 does not derive.
fn treasury_future_to_instrument_def(
    s: celnet_refdata::TreasuryFutureSpec,
) -> Option<InstrumentDef> {
    let civil = |d: celnet_refdata::CivilYmd| CivilDate {
        year: d.year,
        month: d.month,
        day: d.day,
    };
    let dv01 = s.dv01_at_notional_yield().ok()?;
    if !(dv01.is_finite() && dv01 > 0.0) {
        return None;
    }
    Some(InstrumentDef {
        instrument_id: s.instrument_id.clone(),
        name: s.name.clone(),
        description: String::new(),
        currency: s.currency.to_string(),
        // The contract code is itself the market-standard external cross-ref, so a
        // caller resolving by ticker finds the contract.
        external_ids: vec![ExternalId {
            scheme: "ticker".to_string(),
            value: s.instrument_id.clone(),
        }],
        sub_asset_type: s.sub_asset_type.to_string(),
        region: s.region.to_string(),
        definition: InstrumentFamily::BondFuture(BondFutureDef {
            contract_code: s.instrument_id.clone(),
            contract_symbol: s.terms.symbol.to_string(),
            underlying_issuer: "US Treasury".to_string(),
            contract_face_value: s.terms.face_value,
            tick_size_points: s.terms.tick_size_points,
            tick_value: s.tick_value(),
            notional_coupon_rate: celnet_refdata::NOTIONAL_YIELD,
            deliverable_min_months: s.terms.deliverable_min_months,
            deliverable_max_months: s.terms.deliverable_max_months,
            delivery_month_start: civil(s.delivery_month_start),
            first_delivery_date: civil(s.first_delivery_date),
            last_trading_date: civil(s.last_trading_date),
            last_delivery_date: civil(s.last_delivery_date),
            dv01_per_contract_at_notional_yield: dv01,
            calendars: s.calendars.iter().map(|c| (*c).to_string()).collect(),
        }),
    })
}

/// Convert one curated [`celnet_refdata::GovBondSpec`] into an [`InstrumentDef`] with a
/// [`InstrumentFamily::Bond`] family block, mirroring the crate's civil date onto the
/// server's [`CivilDate`].
fn gov_bond_to_instrument_def(s: celnet_refdata::GovBondSpec) -> InstrumentDef {
    let civil = |d: celnet_refdata::CivilYmd| CivilDate {
        year: d.year,
        month: d.month,
        day: d.day,
    };
    let mut external_ids = vec![ExternalId {
        scheme: "isin".to_string(),
        value: s.isin.clone(),
    }];
    if let Some(cusip) = &s.cusip {
        external_ids.push(ExternalId {
            scheme: "cusip".to_string(),
            value: cusip.clone(),
        });
    }
    InstrumentDef {
        instrument_id: s.instrument_id.clone(),
        name: s.name.clone(),
        description: String::new(),
        currency: s.currency.to_string(),
        external_ids,
        sub_asset_type: s.sub_asset_type.to_string(),
        region: s.region.to_string(),
        definition: InstrumentFamily::Bond(BondDef {
            issuer: s.issuer.clone(),
            coupon_rate: s.coupon_rate,
            coupon_type: s.coupon_type.to_string(),
            coupon_frequency: s.coupon_frequency.to_string(),
            day_count: s.day_count.to_string(),
            issue_date: None,
            dated_date: s.dated_date.map(civil),
            first_coupon_date: None,
            maturity_date: civil(s.maturity_date),
            redemption: s.redemption,
            calendars: s.calendars.iter().map(|c| (*c).to_string()).collect(),
        }),
    }
}

/// Seed a small, realistic default registry on an **empty** instrument list, so a
/// fresh edge can resolve curve/booking instruments immediately; report `true`
/// (the caller should persist). A non-empty list is left untouched and reports
/// `false` (idempotent, mirroring `ensure_seed_registry`). The seeded strings are
/// sample DATA, not product identifiers.
#[must_use]
pub fn ensure_seed_instruments(instruments: &mut Vec<InstrumentDef>) -> bool {
    if !instruments.is_empty() {
        return false;
    }
    *instruments = seed_instruments();
    true
}

/// The seeded set: a USD-SOFR rates strip (deposit, FRA, STIR future, OIS, IRS) — the
/// curve pillars a fresh edge needs to build a discount curve and book a rates trade.
///
/// # Why there are no bonds here
///
/// This seed used to carry two sample USD bonds (`ust-2y-note` and `acme-5y-corp`) with
/// invented ISINs. They were **retired**: a seeded bond is advertised as tradeable, but no
/// liquidity provider quotes an invented security, so the traded and quoted universes
/// disagreed. A hedge on such a bond resolves no LP panel line, finds no named venue, and
/// silently backstops to the synthetic composite — which is precisely how a bond hedge
/// came to record a fill against `COMPOSITE` that no LP ever made.
///
/// The real cash-bond universe is seeded separately and additively by
/// [`government_bond_defs`] (US Treasuries + UK gilts + EUR govvies) and
/// [`treasury_future_defs`], on EVERY boot. Every one of those `instrument_id`s is exactly
/// what the LP feed streams, so the tradeable and quotable universes coincide by
/// construction. Do not reintroduce a sample bond here: add it to the curated universe in
/// `celnet-refdata` where the liquidity providers price it too, or it is a defect the
/// [`audit_unquotable`] guard will report.
///
/// The rates pillars below are curve inputs, not risk shed into an LP panel, so their
/// absence from a quoted book is correct rather than an asymmetry.
///
/// [`audit_unquotable`]: crate::services::aggregation::AggregationHub::audit_unquotable
fn seed_instruments() -> Vec<InstrumentDef> {
    let us = || vec!["united_states".to_string()];
    vec![
        InstrumentDef {
            instrument_id: "usd-sofr-depo-on".to_string(),
            name: "USD SOFR Deposit O/N".to_string(),
            description: "Overnight USD SOFR cash deposit (curve front pillar).".to_string(),
            currency: "USD".to_string(),
            external_ids: vec![ExternalId {
                scheme: "ticker".to_string(),
                value: "USDSOFR-ON".to_string(),
            }],
            sub_asset_type: "money_market".to_string(),
            region: "us".to_string(),
            definition: InstrumentFamily::Deposit(DepositDef {
                index: "sofr".to_string(),
                tenor: "ON".to_string(),
                day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: us(),
                spot_lag_days: 0,
            }),
        },
        InstrumentDef {
            instrument_id: "usd-sofr-depo-3m".to_string(),
            name: "USD SOFR Deposit 3M".to_string(),
            description: "3-month USD SOFR cash deposit.".to_string(),
            currency: "USD".to_string(),
            external_ids: vec![ExternalId {
                scheme: "ticker".to_string(),
                value: "USDSOFR-3M".to_string(),
            }],
            sub_asset_type: "money_market".to_string(),
            region: "us".to_string(),
            definition: InstrumentFamily::Deposit(DepositDef {
                index: "sofr".to_string(),
                tenor: "3M".to_string(),
                day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: us(),
                spot_lag_days: 2,
            }),
        },
        InstrumentDef {
            instrument_id: "usd-sofr-fra-3x6".to_string(),
            name: "USD SOFR FRA 3x6".to_string(),
            description: "3x6 forward rate agreement on USD SOFR.".to_string(),
            currency: "USD".to_string(),
            external_ids: vec![ExternalId {
                scheme: "ticker".to_string(),
                value: "USDSOFR-FRA-3X6".to_string(),
            }],
            sub_asset_type: "money_market".to_string(),
            region: "us".to_string(),
            definition: InstrumentFamily::Fra(FraDef {
                float_index: "sofr".to_string(),
                start_tenor: "3M".to_string(),
                end_tenor: "6M".to_string(),
                accrual_day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: us(),
                spot_lag_days: 2,
            }),
        },
        InstrumentDef {
            instrument_id: "usd-sofr-future-sr3h6".to_string(),
            name: "USD 3M SOFR Future SR3H6".to_string(),
            description: "Listed 3-month SOFR futures contract (Mar-2026).".to_string(),
            currency: "USD".to_string(),
            external_ids: vec![ExternalId {
                scheme: "ticker".to_string(),
                value: "SR3H6".to_string(),
            }],
            sub_asset_type: "rate_future".to_string(),
            region: "us".to_string(),
            definition: InstrumentFamily::StirFuture(StirFutureDef {
                contract_code: "SR3H6".to_string(),
                reference_start: "2026-03-18".to_string(),
                reference_end: "2026-06-17".to_string(),
                day_count: "act_360".to_string(),
                calendars: us(),
                convexity_vol: 0.0,
                contract_size: 2_500_000.0,
            }),
        },
        InstrumentDef {
            instrument_id: "usd-sofr-ois-2y".to_string(),
            name: "USD SOFR OIS 2Y".to_string(),
            description: "2-year USD-SOFR overnight-indexed swap.".to_string(),
            currency: "USD".to_string(),
            external_ids: vec![ExternalId {
                scheme: "ticker".to_string(),
                value: "USDSOFR-OIS-2Y".to_string(),
            }],
            sub_asset_type: "swap".to_string(),
            region: "us".to_string(),
            definition: InstrumentFamily::Ois(OisDef {
                tenor: "2Y".to_string(),
                index: "sofr".to_string(),
                fixed_frequency: "annual".to_string(),
                fixed_day_count: "act_360".to_string(),
                float_day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: us(),
                spot_lag_days: 2,
            }),
        },
        InstrumentDef {
            instrument_id: "usd-sofr-irs-5y".to_string(),
            name: "USD SOFR IRS 5Y".to_string(),
            description: "5-year USD fixed-vs-SOFR vanilla swap.".to_string(),
            currency: "USD".to_string(),
            external_ids: vec![ExternalId {
                scheme: "ticker".to_string(),
                value: "USDSOFR-IRS-5Y".to_string(),
            }],
            sub_asset_type: "swap".to_string(),
            region: "us".to_string(),
            definition: InstrumentFamily::VanillaIrs(VanillaIrsDef {
                tenor: "5Y".to_string(),
                fixed_frequency: "annual".to_string(),
                fixed_day_count: "thirty_360_bond_basis".to_string(),
                float_index: "sofr".to_string(),
                float_frequency: "quarterly".to_string(),
                float_day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: us(),
                roll_convention: "none".to_string(),
                spot_lag_days: 2,
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ois() -> InstrumentDef {
        InstrumentDef {
            instrument_id: "x-ois".to_string(),
            name: "X OIS".to_string(),
            description: String::new(),
            currency: "USD".to_string(),
            external_ids: vec![ExternalId {
                scheme: "ticker".to_string(),
                value: "X-OIS".to_string(),
            }],
            sub_asset_type: "swap".to_string(),
            region: "us".to_string(),
            definition: InstrumentFamily::Ois(OisDef {
                tenor: "2Y".to_string(),
                index: "sofr".to_string(),
                fixed_frequency: "annual".to_string(),
                fixed_day_count: "act_360".to_string(),
                float_day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: vec!["united_states".to_string()],
                spot_lag_days: 2,
            }),
        }
    }

    #[test]
    fn labels_map_to_engine_enums() {
        assert_eq!(
            accrual_basis_from_label("act_360"),
            Some(AccrualBasis::Act360)
        );
        assert_eq!(
            accrual_basis_from_label("thirty_360_bond_basis"),
            Some(AccrualBasis::Thirty360BondBasis)
        );
        assert!(accrual_basis_from_label("act_act").is_none());
        assert!(
            day_count_is_known("act_act"),
            "act_act is a valid bond basis"
        );
        assert_eq!(
            roll_rule_from_label("modified_following"),
            Some(RollRule::ModifiedFollowing)
        );
        assert_eq!(
            centre_from_label("united_states"),
            Some(CentreId::UnitedStates)
        );
        assert_eq!(
            payment_frequency_from_label("semi_annual"),
            Some(PaymentFrequency::SemiAnnual)
        );
        assert!(roll_rule_from_label("teleport").is_none());
    }

    #[test]
    fn seed_is_valid_and_idempotent() {
        let mut v = Vec::new();
        assert!(ensure_seed_instruments(&mut v), "first call seeds");
        // The seed is rates curve pillars ONLY. The sample bonds it used to carry were
        // retired because no LP quotes an invented security — see `seed_instruments`.
        assert_eq!(v.len(), 6);
        assert!(
            !v.iter()
                .any(|i| matches!(i.definition, InstrumentFamily::Bond(_))),
            "the seed must advertise no bond the liquidity panel cannot quote"
        );
        validate_instruments(&v).expect("seeded registry is valid");
        assert!(
            !ensure_seed_instruments(&mut v),
            "second call is idempotent"
        );
    }

    /// A real cash bond to exercise bond validation against.
    ///
    /// Sourced from [`government_bond_defs`] — the curated universe the liquidity
    /// providers actually quote — rather than from [`seed_instruments`], which
    /// deliberately carries no bond at all (a seeded sample bond would be advertised as
    /// tradeable while no LP could price it; see `seed_instruments`).
    /// Coupon-bearing specifically: the curated universe also carries zero-coupon bills,
    /// and a bill already has no payment frequency, so it cannot exercise the
    /// zero-coupon-must-not-carry-frequency rule.
    fn sample_bond() -> InstrumentDef {
        government_bond_defs()
            .into_iter()
            .find(|i| {
                matches!(&i.definition, InstrumentFamily::Bond(b)
                    if b.coupon_type != "zero" && !b.coupon_frequency.is_empty())
            })
            .expect("the curated government universe ships coupon-bearing cash bonds")
    }

    #[test]
    fn each_seeded_family_round_trips_through_json() {
        let v = seed_instruments();
        let bytes = serde_json::to_vec(&v).unwrap();
        let back: Vec<InstrumentDef> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v, back);
        // Every RATES family is represented in the seed — the curve pillars a fresh edge
        // needs. The seed carries no bond by design.
        let kinds: std::collections::HashSet<_> = v.iter().map(|i| i.definition.kind()).collect();
        for k in ["deposit", "fra", "stir_future", "vanilla_irs", "ois"] {
            assert!(kinds.contains(k), "seed missing family {k}");
        }
        assert!(
            !kinds.contains("bond"),
            "the seed must advertise no bond the liquidity panel cannot quote"
        );
        // The bond family is supplied additively by the curated universe instead, and it
        // round-trips through the same JSON contract.
        let bond = sample_bond();
        let back: InstrumentDef = serde_json::from_slice(&serde_json::to_vec(&bond).unwrap())
            .expect("a curated bond round-trips");
        assert_eq!(bond, back);
    }

    #[test]
    fn rejects_duplicate_instrument_id() {
        let a = sample_ois();
        let mut b = sample_ois();
        b.external_ids[0].value = "X-OIS-2".to_string();
        let err = validate_instruments(&[a, b]).expect_err("dup id rejected");
        assert!(err.contains("duplicate instrument_id"), "{err}");
    }

    #[test]
    fn rejects_duplicate_external_id() {
        let a = sample_ois();
        let mut b = sample_ois();
        b.instrument_id = "y-ois".to_string();
        // same (ticker, X-OIS) pair as `a`
        let err = validate_instruments(&[a, b]).expect_err("dup external id rejected");
        assert!(err.contains("duplicate external id"), "{err}");
    }

    #[test]
    fn rejects_unknown_enum_label() {
        let mut a = sample_ois();
        if let InstrumentFamily::Ois(o) = &mut a.definition {
            o.business_day_convention = "wibble".to_string();
        }
        let err = validate_instruments(&[a]).expect_err("unknown label rejected");
        assert!(err.contains("business_day_convention"), "{err}");
    }

    #[test]
    fn government_bond_defs_are_registry_valid() {
        let defs = government_bond_defs();
        // The mapped curated universe passes the SAME validation the registry enforces
        // at load / admin-write, so seeding it can never corrupt the store.
        validate_instruments(&defs).expect("curated government defs are registry-valid");
        assert!(
            defs.len() >= 230,
            "expected the full universe, got {}",
            defs.len()
        );
        // Every mapped def is a bond.
        for d in &defs {
            assert!(
                matches!(d.definition, InstrumentFamily::Bond(_)),
                "{} is not a bond",
                d.instrument_id
            );
        }
    }

    #[test]
    fn treasury_future_defs_are_registry_valid_and_disjoint_from_the_bonds() {
        let futures = treasury_future_defs();
        // Every listed contract in reference data is seeded — nothing is silently
        // dropped, because a contract the server does not know is a hedge vehicle the
        // desk cannot reach.
        assert_eq!(
            futures.len(),
            celnet_refdata::treasury_futures_universe().len(),
            "a listed contract was dropped from the registry seed"
        );
        // They pass the SAME validation the registry enforces at load / admin-write.
        validate_instruments(&futures).expect("futures defs are registry-valid");

        for d in &futures {
            let InstrumentFamily::BondFuture(f) = &d.definition else {
                panic!("{} is not a bond future", d.instrument_id);
            };
            assert_eq!(d.currency, "USD");
            assert_eq!(f.contract_code, d.instrument_id);
            assert!((f.notional_coupon_rate - 0.06).abs() < 1e-12);
            // The DV01 that sizes every hedge is present and positive.
            assert!(
                f.dv01_per_contract_at_notional_yield > 0.0,
                "{}: no DV01",
                d.instrument_id
            );
            // ...and it is exactly what reference data derives — the registry never
            // carries a second, drifting copy of the hedge ratio.
            let spec = celnet_refdata::future_by_id(&d.instrument_id).expect("resolvable");
            assert!(
                (f.dv01_per_contract_at_notional_yield - spec.dv01_at_notional_yield().unwrap())
                    .abs()
                    < 1e-12
            );
            assert!((f.tick_value - spec.tick_value()).abs() < 1e-12);
        }

        // Seeding bonds AND futures together stays valid: no id or external-id
        // collision between a contract code and a CUSIP/ISIN/slug.
        let mut all = government_bond_defs();
        all.extend(futures);
        validate_instruments(&all).expect("the combined seed is registry-valid");
    }

    #[test]
    fn refdata_universe_covers_every_shipped_region_with_a_known_sub_asset_type() {
        // The download taxonomy lives in celnet_refdata (keyed by instrument_id), not on
        // the registry entry. Guarantee its coverage at that source of truth: every shipped
        // bond carries a RECOGNISED sub-asset type and a region, and every region we
        // advertise is present.
        //
        // The universe is no longer sovereign-only — it ships the EUR corporate complex
        // alongside the govvies — so this asserts a CLOSED SET rather than a single value.
        // The closed set is the point: a new taxonomy label added upstream without a
        // decision about how it is priced, bucketed and hedged must fail here rather than
        // flow through as an unclassified bond.
        let mut regions = std::collections::HashSet::new();
        let mut sub_types = std::collections::HashSet::new();
        for s in celnet_refdata::government_universe() {
            assert!(
                s.sub_asset_type == "government" || s.sub_asset_type == "corporate",
                "{} carries an unrecognised sub-asset type {:?}",
                s.instrument_id,
                s.sub_asset_type
            );
            assert!(!s.region.is_empty(), "{} has no region", s.instrument_id);
            regions.insert(s.region.to_string());
            sub_types.insert(s.sub_asset_type);
        }
        for r in ["us", "uk", "de", "fr", "it"] {
            assert!(regions.contains(r), "no {r} government bonds mapped");
        }
        assert!(
            regions.contains("eu"),
            "no EUR corporate bonds mapped — the credit complex is missing"
        );
        // Both halves are actually present: a universe that silently lost either one would
        // otherwise still satisfy every assertion above.
        assert!(sub_types.contains("government"));
        assert!(sub_types.contains("corporate"));
    }

    #[test]
    fn rejects_missing_required_field() {
        let mut a = sample_ois();
        if let InstrumentFamily::Ois(o) = &mut a.definition {
            o.index.clear();
        }
        let err = validate_instruments(&[a]).expect_err("missing index rejected");
        assert!(err.contains("index is required"), "{err}");
    }

    #[test]
    fn rejects_unknown_external_scheme() {
        let mut a = sample_ois();
        a.external_ids[0].scheme = "qqq".to_string();
        let err = validate_instruments(&[a]).expect_err("unknown scheme rejected");
        assert!(err.contains("unknown external-id scheme"), "{err}");
    }

    #[test]
    fn zero_coupon_bond_must_not_carry_frequency() {
        let mut bond = sample_bond();
        if let InstrumentFamily::Bond(b) = &mut bond.definition {
            b.coupon_type = "zero".to_string();
            b.coupon_rate = 0.0;
            // leave the inherited semi_annual frequency → must be rejected
        }
        let err = validate_instruments(&[bond]).expect_err("zero w/ freq rejected");
        assert!(err.contains("zero-coupon"), "{err}");
    }

    #[test]
    fn mint_id_disambiguates() {
        let existing = vec![sample_ois()];
        assert_eq!(mint_instrument_id("Brand New", &existing), "brand-new");
        // colliding name → suffixed
        assert_eq!(mint_instrument_id("X OIS!!", &existing), "x-ois-2");
    }

    #[test]
    fn invalid_civil_date_is_rejected() {
        let mut bond = sample_bond();
        if let InstrumentFamily::Bond(b) = &mut bond.definition {
            b.maturity_date = CivilDate {
                year: 2030,
                month: 2,
                day: 31,
            };
        }
        let err = validate_instruments(&[bond]).expect_err("invalid date rejected");
        assert!(err.contains("maturity_date"), "{err}");
    }
}
