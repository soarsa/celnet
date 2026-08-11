//! The hedge **vehicle** — *with what* a size-bearing exit action hedges — plus the
//! firm's **vehicle registry** (which vehicle hedges which risk, by instrument and
//! maturity bucket) and the DV01-ratio sizing that turns a risk to shed into a
//! tradeable quantity of that vehicle.
//!
//! # Why this exists
//!
//! The exit-policy graph ([`crate::HedgeGraph`]) says *what kind* of exit to take
//! (`WAREHOUSE`, `SKEW`, `SUBMIT_MARKET_ORDER`, …). It historically said nothing about
//! *which instrument* the exit trades, because the only supported answer was "the same
//! security, sold back" — a self-hedge whose DV01 ratio is identically `1`.
//!
//! That answer is wrong for the instrument a credit desk actually warehouses. A
//! **corporate bond is not hedged with itself**: there is no two-way street liquidity in
//! a single corp line. It is hedged with a **benchmark at matching maturity** — in
//! practice a government-bond future (a 9-year corp against the 10Y contract). The
//! vehicle model below makes that a first-class, trader-configurable choice **in the
//! rules**, so a desk can express "breach on BOOK-CREDIT ⇒ submit a market order, hedged
//! into the 10Y future" as data.
//!
//! # The three things a vehicle hedge needs
//!
//! 1. **A choice, on the rule leaf** — [`HedgeVehicle`], carried on
//!    [`HedgeNode::Action`](crate::HedgeNode::Action). Defaults to
//!    [`HedgeVehicle::SelfInstrument`], so a policy authored before vehicles existed
//!    behaves EXACTLY as it did (guardrail: no silent behaviour change).
//! 2. **A registry** — [`HedgeVehicleRegistry`], resolving *(instrument, product, ccy,
//!    maturity)* to the [`HedgeVehicleRule`] that names the hedge instrument and,
//!    critically, its **DV01 per unit**. A DV01 is never guessed: an unregistered
//!    vehicle cannot be sized, and the caller falls back to the self-hedge rather than
//!    trading a fabricated quantity (guardrail 2).
//! 3. **A ratio** — [`plan_hedge_ratio`], `units = position_DV01 / vehicle_DV01_per_unit`,
//!    rounded to **whole contracts** for a future, with the rounding residual reported
//!    rather than hidden. You cannot trade 318.47 contracts; the desk trades 318 and
//!    carries the remainder.
//!
//! # The DV01 the ratio must be computed on
//!
//! The ratio is only as good as its numerator. The server's coarse pre-trade exposure
//! proxy measures a cash bond as `redemption × 1bp` — i.e. as though every bond had a
//! duration of `1`. A DV01-ratio hedge sized off THAT number is wrong by the bond's
//! actual duration (a 10-year is roughly 8×). This module therefore never accepts an
//! unlabelled number: [`HedgeRatioPlan::basis`] records which [`Dv01Basis`] the target
//! DV01 came from, and the caller is expected to supply an analytic
//! ([`Dv01Basis::Analytic`]) DV01 for a cash bond. The label is carried all the way to
//! the trader's screen so a proxy-based ratio is never mistaken for an exact one.

use serde::{Deserialize, Serialize};
use std::fmt;

/// **With what** a size-bearing exit action hedges — the vehicle choice a trader makes
/// on a rule leaf.
///
/// [`SelfInstrument`](Self::SelfInstrument) is the default and reproduces the historical
/// behaviour byte-for-byte: the same security sold back, DV01 ratio identically `1`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgeVehicle {
    /// Sell the **same security** back. The DV01 ratio is identically `1` (the leg and
    /// the position are the identical instrument), so the shed is exact under any
    /// duration measure and needs no registry entry. **The default.**
    #[default]
    SelfInstrument,
    /// Hedge into whatever the firm's [`HedgeVehicleRegistry`] maps this risk to, by
    /// instrument and **maturity bucket** — the "hedge a corp with the benchmark at
    /// matching maturity" rule expressed once, centrally, rather than per policy leaf.
    Benchmark,
    /// Hedge into an explicitly named instrument (a specific benchmark bond, an
    /// on-the-run govvie). The id must be registered so its DV01 per unit is known.
    Instrument {
        /// The hedge instrument's canonical id.
        instrument_id: String,
    },
    /// Hedge into an explicitly named **futures contract**. Sizing rounds to whole
    /// contracts and reports the residual. The id must be registered so its DV01 per
    /// contract is known.
    Future {
        /// The futures contract's id (e.g. a delivery-month code).
        contract_id: String,
    },
}

impl HedgeVehicle {
    /// A short, stable kind label for provenance / UI / logging.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            HedgeVehicle::SelfInstrument => "SELF",
            HedgeVehicle::Benchmark => "BENCHMARK",
            HedgeVehicle::Instrument { .. } => "INSTRUMENT",
            HedgeVehicle::Future { .. } => "FUTURE",
        }
    }

    /// Whether this is the default self-hedge (the same security sold back). The caller
    /// short-circuits the whole vehicle path on `true`, preserving legacy behaviour.
    #[must_use]
    pub fn is_self(&self) -> bool {
        matches!(self, HedgeVehicle::SelfInstrument)
    }

    /// The explicitly named hedge instrument, when the trader named one on the leaf.
    /// `None` for [`SelfInstrument`](Self::SelfInstrument) (no other instrument) and for
    /// [`Benchmark`](Self::Benchmark) (the registry names it).
    #[must_use]
    pub fn named_instrument(&self) -> Option<&str> {
        match self {
            HedgeVehicle::Instrument { instrument_id } => Some(instrument_id.as_str()),
            HedgeVehicle::Future { contract_id } => Some(contract_id.as_str()),
            HedgeVehicle::SelfInstrument | HedgeVehicle::Benchmark => None,
        }
    }

    /// Whether the leaf itself declares the vehicle to be a futures contract (so sizing
    /// rounds to whole lots even if the registry entry says otherwise).
    #[must_use]
    pub fn is_future(&self) -> bool {
        matches!(self, HedgeVehicle::Future { .. })
    }
}

impl fmt::Display for HedgeVehicle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HedgeVehicle::SelfInstrument => f.write_str("self"),
            HedgeVehicle::Benchmark => f.write_str("benchmark (registry)"),
            HedgeVehicle::Instrument { instrument_id } => write!(f, "instrument {instrument_id}"),
            HedgeVehicle::Future { contract_id } => write!(f, "future {contract_id}"),
        }
    }
}

/// Which measure a hedge ratio's **numerator** was computed on — carried through to the
/// trader so a duration-blind proxy ratio can never be mistaken for an exact one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dv01Basis {
    /// A genuine analytic DV01 — the closed-form yield derivative of the instrument's own
    /// cashflow schedule. Correct to the instrument's real duration.
    Analytic,
    /// The **undiscounted annuity PV01** (`notional × years × 1bp`). Exact in shape for a
    /// linear swap leg and a conservative upper bound on the discounted PV01; adequate
    /// for a swap-vs-swap ratio.
    AnnuityPv01,
    /// The firm's coarse pre-trade **exposure proxy**, which measures a cash bond as
    /// `redemption × 1bp` — i.e. as though its duration were `1`. **A ratio on this basis
    /// is wrong by the instrument's actual duration** (roughly 8× for a 10-year bond) and
    /// must be surfaced as such, never quietly traded.
    #[default]
    ExposureProxy,
}

impl Dv01Basis {
    /// A short, stable label for provenance / UI.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Dv01Basis::Analytic => "analytic",
            Dv01Basis::AnnuityPv01 => "annuity-pv01",
            Dv01Basis::ExposureProxy => "exposure-proxy",
        }
    }

    /// Whether a ratio computed on this basis is **duration-correct**. `false` for
    /// [`ExposureProxy`](Self::ExposureProxy), whose bond arm is duration-blind — the
    /// caller must warn rather than present the size as exact.
    #[must_use]
    pub fn is_duration_correct(self) -> bool {
        matches!(self, Dv01Basis::Analytic | Dv01Basis::AnnuityPv01)
    }
}

/// One registry row: *"risk matching this shape hedges into THAT instrument, whose DV01
/// per unit is THIS"*.
///
/// The match axes are deliberately the ones a desk thinks in — a specific security, a
/// product family, a currency, and a **maturity bucket** — and the resolution is
/// most-specific-wins, mirroring how warehouse thresholds resolve.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HedgeVehicleRule {
    /// A stable id for the row (the CRUD key and the tie-break for equally specific rules).
    pub id: String,
    /// Match on the risk cell's **executable security id**, exactly. Empty ⇒ matches any.
    #[serde(default)]
    pub instrument_id: String,
    /// Match on the product family (`"BOND"`, `"OIS"`, …), case-insensitively. Empty ⇒
    /// matches any.
    #[serde(default)]
    pub product: String,
    /// Match on the currency, case-insensitively. Empty ⇒ matches any.
    #[serde(default)]
    pub ccy: String,
    /// The maturity bucket's inclusive lower bound, in years.
    #[serde(default)]
    pub min_maturity_years: f64,
    /// The maturity bucket's exclusive upper bound, in years. A rule with
    /// `min = 7, max = 12` is the "10Y bucket" that catches a 9-year corp.
    #[serde(default)]
    pub max_maturity_years: f64,
    /// The instrument this risk hedges into — the id the LP panel is asked for.
    #[serde(default)]
    pub hedge_instrument_id: String,
    /// Whether that instrument is a **futures contract**, and therefore trades in whole
    /// lots (sizing rounds and reports the residual).
    #[serde(default)]
    pub is_future: bool,
    /// The DV01 of **one unit** of the hedge instrument (one futures contract, or one
    /// quoted unit of a benchmark bond), in the same currency the position DV01 is
    /// measured in. Strictly positive — a non-positive value makes the row unusable and
    /// the ratio is refused rather than fabricated.
    #[serde(default)]
    pub dv01_per_unit: f64,
    /// The unit's human label (`"contract"`, `"1mm face"`) for the trader-facing size.
    #[serde(default)]
    pub unit_label: String,
}

/// A defect in a [`HedgeVehicleRule`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HedgeVehicleError {
    /// The row carries no id.
    MissingId,
    /// The row names no hedge instrument, so it could never route.
    MissingHedgeInstrument {
        /// The offending row id.
        id: String,
    },
    /// `dv01_per_unit` is not strictly positive and finite — the ratio's denominator.
    NonPositiveDv01 {
        /// The offending row id.
        id: String,
    },
    /// The maturity bucket is empty or inverted (`max <= min`), or carries a non-finite
    /// bound.
    InvalidMaturityBucket {
        /// The offending row id.
        id: String,
    },
    /// Two rows share an id.
    DuplicateId {
        /// The duplicated id.
        id: String,
    },
}

impl fmt::Display for HedgeVehicleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HedgeVehicleError::MissingId => f.write_str("a hedge-vehicle rule carries no id"),
            HedgeVehicleError::MissingHedgeInstrument { id } => {
                write!(f, "hedge-vehicle rule {id:?} names no hedge instrument")
            }
            HedgeVehicleError::NonPositiveDv01 { id } => write!(
                f,
                "hedge-vehicle rule {id:?} has a non-positive DV01 per unit — a hedge ratio \
                 cannot be sized against it"
            ),
            HedgeVehicleError::InvalidMaturityBucket { id } => write!(
                f,
                "hedge-vehicle rule {id:?} has an empty or inverted maturity bucket"
            ),
            HedgeVehicleError::DuplicateId { id } => {
                write!(f, "two hedge-vehicle rules share the id {id:?}")
            }
        }
    }
}

impl std::error::Error for HedgeVehicleError {}

impl HedgeVehicleRule {
    /// Whether this row matches a risk cell.
    ///
    /// Every non-empty axis must match (string axes case-insensitively); the maturity
    /// must lie in `[min, max)`. A row whose bucket is `[0, 0)` is treated as
    /// maturity-agnostic **only** when the caller has no maturity to offer
    /// (`maturity_years = None`), which is what lets a swap cell — which carries a tenor,
    /// not a maturity date — still resolve a vehicle.
    #[must_use]
    pub fn matches(
        &self,
        instrument_id: &str,
        product: &str,
        ccy: &str,
        maturity_years: Option<f64>,
    ) -> bool {
        if !self.instrument_id.is_empty() && !self.instrument_id.eq_ignore_ascii_case(instrument_id)
        {
            return false;
        }
        if !self.product.is_empty() && !self.product.eq_ignore_ascii_case(product) {
            return false;
        }
        if !self.ccy.is_empty() && !self.ccy.eq_ignore_ascii_case(ccy) {
            return false;
        }
        match maturity_years {
            Some(m) => {
                if !self.has_maturity_bucket() {
                    // A bucket-less row is maturity-agnostic: it matches any maturity.
                    return true;
                }
                m >= self.min_maturity_years && m < self.max_maturity_years
            }
            // No maturity known: only a bucket-less row can honestly claim to match.
            None => !self.has_maturity_bucket(),
        }
    }

    /// Whether this row declares a real maturity bucket (a positive-width `[min, max)`).
    #[must_use]
    pub fn has_maturity_bucket(&self) -> bool {
        self.min_maturity_years.is_finite()
            && self.max_maturity_years.is_finite()
            && self.max_maturity_years > self.min_maturity_years
    }

    /// The bucket's width in years (`f64::INFINITY` for a bucket-less row, so a narrower
    /// bucket always wins the tie-break).
    #[must_use]
    pub fn bucket_width(&self) -> f64 {
        if self.has_maturity_bucket() {
            self.max_maturity_years - self.min_maturity_years
        } else {
            f64::INFINITY
        }
    }

    /// How specific this row is: instrument `4` > product `2` > ccy `1`, summed. A row
    /// naming a security beats one naming only a family, which beats a currency-wide
    /// default.
    #[must_use]
    pub fn specificity(&self) -> u8 {
        u8::from(!self.instrument_id.is_empty()) * 4
            + u8::from(!self.product.is_empty()) * 2
            + u8::from(!self.ccy.is_empty())
    }

    /// Validate this row, collecting **every** defect at once (the same discipline as
    /// [`HedgeGraph::validate`](crate::HedgeGraph::validate)).
    ///
    /// # Errors
    /// Every defect found: a missing id / hedge instrument, a non-positive DV01, or an
    /// empty-but-declared maturity bucket.
    pub fn validate(&self) -> Result<(), Vec<HedgeVehicleError>> {
        let mut errs = Vec::new();
        if self.id.trim().is_empty() {
            errs.push(HedgeVehicleError::MissingId);
        }
        if self.hedge_instrument_id.trim().is_empty() {
            errs.push(HedgeVehicleError::MissingHedgeInstrument {
                id: self.id.clone(),
            });
        }
        if !(self.dv01_per_unit.is_finite() && self.dv01_per_unit > 0.0) {
            errs.push(HedgeVehicleError::NonPositiveDv01 {
                id: self.id.clone(),
            });
        }
        // A row that declares ANY maturity bound must declare a well-formed bucket.
        let declares_bucket = self.min_maturity_years != 0.0 || self.max_maturity_years != 0.0;
        if declares_bucket && !self.has_maturity_bucket() {
            errs.push(HedgeVehicleError::InvalidMaturityBucket {
                id: self.id.clone(),
            });
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }
}

/// The firm's hedge-vehicle registry: which vehicle hedges which risk.
///
/// Resolution is **most-specific-wins**, then **narrowest maturity bucket**, then id
/// order — a total, deterministic function of the rows, so two servers with the same
/// registry always pick the same vehicle.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HedgeVehicleRegistry {
    /// The rows, in configuration order (resolution does not depend on it).
    #[serde(default)]
    pub rules: Vec<HedgeVehicleRule>,
}

impl HedgeVehicleRegistry {
    /// A registry over `rules`.
    #[must_use]
    pub fn new(rules: Vec<HedgeVehicleRule>) -> Self {
        Self { rules }
    }

    /// Whether the registry has no rows (so `Benchmark` can never resolve).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Validate every row plus the cross-row uniqueness of ids, collecting **all**
    /// defects at once.
    ///
    /// # Errors
    /// The union of every row's [`HedgeVehicleRule::validate`] defects plus one
    /// [`HedgeVehicleError::DuplicateId`] per repeated id.
    pub fn validate(&self) -> Result<(), Vec<HedgeVehicleError>> {
        let mut errs = Vec::new();
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for rule in &self.rules {
            if let Err(mut e) = rule.validate() {
                errs.append(&mut e);
            }
            if !rule.id.is_empty() && !seen.insert(rule.id.as_str()) {
                errs.push(HedgeVehicleError::DuplicateId {
                    id: rule.id.clone(),
                });
            }
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    /// Resolve the vehicle for a risk cell: the most specific matching row, tie-broken by
    /// the narrowest maturity bucket and then by id. `None` when nothing matches — the
    /// caller must then NOT fabricate a vehicle (it falls back to the self-hedge).
    #[must_use]
    pub fn resolve(
        &self,
        instrument_id: &str,
        product: &str,
        ccy: &str,
        maturity_years: Option<f64>,
    ) -> Option<&HedgeVehicleRule> {
        self.rules
            .iter()
            .filter(|r| r.matches(instrument_id, product, ccy, maturity_years))
            .filter(|r| r.validate().is_ok())
            .min_by(|a, b| {
                b.specificity()
                    .cmp(&a.specificity())
                    .then_with(|| {
                        a.bucket_width()
                            .partial_cmp(&b.bucket_width())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| a.id.cmp(&b.id))
            })
    }

    /// Look a row up by the **hedge instrument** it names — how an explicitly named
    /// [`HedgeVehicle::Instrument`] / [`HedgeVehicle::Future`] recovers its DV01 per unit.
    /// A vehicle named on a rule leaf but absent from the registry has no known DV01 and
    /// therefore cannot be sized (guardrail 2: never guess a hedge quantity).
    #[must_use]
    pub fn by_hedge_instrument(&self, hedge_instrument_id: &str) -> Option<&HedgeVehicleRule> {
        self.rules
            .iter()
            .filter(|r| {
                r.hedge_instrument_id
                    .eq_ignore_ascii_case(hedge_instrument_id)
            })
            .find(|r| r.validate().is_ok())
    }
}

/// The sized plan for one vehicle hedge: how many units of what, what that actually
/// removes, and what is honestly left over.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HedgeRatioPlan {
    /// The instrument actually traded.
    pub hedge_instrument_id: String,
    /// The unit's human label (`"contract"`, `"1mm face"`).
    pub unit_label: String,
    /// Whether the vehicle trades in whole lots (a future).
    pub whole_units: bool,
    /// Which measure [`Self::target_dv01`] was computed on. **Read this before trusting
    /// the size** — an [`Dv01Basis::ExposureProxy`] target is duration-blind.
    pub basis: Dv01Basis,
    /// The DV01 the desk wants to remove (always non-negative; the direction is the
    /// position's, carried by the caller).
    pub target_dv01: f64,
    /// The DV01 of one unit of the vehicle.
    pub dv01_per_unit: f64,
    /// The unrounded ratio `target_dv01 / dv01_per_unit`.
    pub exact_units: f64,
    /// The tradeable quantity — [`Self::exact_units`] rounded to the nearest whole unit
    /// when [`Self::whole_units`], else unrounded.
    pub units: f64,
    /// The DV01 actually removed: `units × dv01_per_unit`.
    pub hedged_dv01: f64,
    /// `target_dv01 − hedged_dv01`. **Positive ⇒ under-hedged** (rounded down, risk
    /// remains); **negative ⇒ over-hedged** (rounded up, the desk is now short the
    /// difference). Never suppressed.
    pub residual_dv01: f64,
}

impl HedgeRatioPlan {
    /// The fraction of the target DV01 this plan actually removes — the scalar the caller
    /// applies to the position's face to build a risk-equivalent offset.
    /// `0.0` when the target is zero (nothing to hedge).
    #[must_use]
    pub fn fraction_of_target(&self) -> f64 {
        if self.target_dv01 > 0.0 {
            self.hedged_dv01 / self.target_dv01
        } else {
            0.0
        }
    }

    /// Whether rounding to whole lots changed the size (so the residual is a rounding
    /// residual, not a sizing one).
    #[must_use]
    pub fn was_rounded(&self) -> bool {
        self.whole_units && (self.units - self.exact_units).abs() > f64::EPSILON
    }

    /// Whether this plan trades anything at all. `false` when the target rounds to zero
    /// whole contracts — an honest "too small to hedge with this vehicle", not a silent
    /// no-op.
    #[must_use]
    pub fn is_tradeable(&self) -> bool {
        self.units > 0.0 && self.hedged_dv01 > 0.0
    }

    /// A one-line trader-facing summary: *"sell 318 contract of TY-DEC26 (318.47 exact;
    /// residual 470 DV01)"* — the text the suggestion surface shows.
    #[must_use]
    pub fn summary(&self) -> String {
        let unit = if self.unit_label.is_empty() {
            "unit"
        } else {
            self.unit_label.as_str()
        };
        let plural = if (self.units - 1.0).abs() < f64::EPSILON {
            ""
        } else {
            "s"
        };
        let precision = usize::from(!self.whole_units) * 2;
        format!(
            "{:.*} {unit}{plural} of {} ({:.2} exact, {} basis; residual {:.1} DV01)",
            precision,
            self.units,
            self.hedge_instrument_id,
            self.exact_units,
            self.basis.label(),
            self.residual_dv01,
        )
    }
}

/// Size a vehicle hedge by the DV01 ratio.
///
/// ```text
/// exact_units = target_dv01 / dv01_per_unit
/// units       = whole_units ? round(exact_units) : exact_units
/// hedged_dv01 = units × dv01_per_unit
/// residual    = target_dv01 − hedged_dv01
/// ```
///
/// Rounding is **to nearest** (half away from zero), which minimises `|residual|`; the
/// residual's sign records whether the desk ended up under- or over-hedged, and is never
/// suppressed. A target that rounds to zero whole contracts yields a plan with
/// `units = 0` and the whole target as residual — an honest "below one contract", not a
/// fabricated fractional trade.
///
/// Returns `None` for a non-finite / non-positive `dv01_per_unit` or a non-finite
/// `target_dv01` — a ratio that cannot be computed is refused, never guessed.
#[must_use]
pub fn plan_hedge_ratio(
    target_dv01: f64,
    basis: Dv01Basis,
    rule: &HedgeVehicleRule,
    whole_units: bool,
) -> Option<HedgeRatioPlan> {
    if !target_dv01.is_finite() || target_dv01 < 0.0 {
        return None;
    }
    if !(rule.dv01_per_unit.is_finite() && rule.dv01_per_unit > 0.0) {
        return None;
    }
    let exact_units = target_dv01 / rule.dv01_per_unit;
    let units = if whole_units {
        exact_units.round()
    } else {
        exact_units
    };
    let hedged_dv01 = units * rule.dv01_per_unit;
    Some(HedgeRatioPlan {
        hedge_instrument_id: rule.hedge_instrument_id.clone(),
        unit_label: rule.unit_label.clone(),
        whole_units,
        basis,
        target_dv01,
        dv01_per_unit: rule.dv01_per_unit,
        exact_units,
        units,
        hedged_dv01,
        residual_dv01: target_dv01 - hedged_dv01,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 10Y Treasury-future bucket a 7–12 year corp hedges into.
    fn ty_bucket() -> HedgeVehicleRule {
        HedgeVehicleRule {
            id: "US-BOND-10Y".into(),
            instrument_id: String::new(),
            product: "BOND".into(),
            ccy: "USD".into(),
            min_maturity_years: 7.0,
            max_maturity_years: 12.0,
            hedge_instrument_id: "TY-DEC26".into(),
            is_future: true,
            dv01_per_unit: 78.0,
            unit_label: "contract".into(),
        }
    }

    fn fv_bucket() -> HedgeVehicleRule {
        HedgeVehicleRule {
            id: "US-BOND-5Y".into(),
            product: "BOND".into(),
            ccy: "USD".into(),
            min_maturity_years: 3.0,
            max_maturity_years: 7.0,
            hedge_instrument_id: "FV-DEC26".into(),
            is_future: true,
            dv01_per_unit: 42.0,
            unit_label: "contract".into(),
            ..HedgeVehicleRule::default()
        }
    }

    // --- the vehicle choice --------------------------------------------------

    #[test]
    fn default_vehicle_is_the_self_hedge() {
        assert_eq!(HedgeVehicle::default(), HedgeVehicle::SelfInstrument);
        assert!(HedgeVehicle::default().is_self());
        assert_eq!(HedgeVehicle::default().kind(), "SELF");
        assert_eq!(HedgeVehicle::default().named_instrument(), None);
    }

    /// An action leaf persisted BEFORE vehicles existed carries no `vehicle` key; it must
    /// reload as the self-hedge so behaviour is byte-identical (the back-compat contract).
    #[test]
    fn legacy_json_without_a_vehicle_reloads_as_self() {
        #[derive(Deserialize)]
        struct Leaf {
            #[serde(default)]
            vehicle: HedgeVehicle,
        }
        let leaf: Leaf = serde_json::from_str("{}").expect("a leaf with no vehicle must load");
        assert_eq!(leaf.vehicle, HedgeVehicle::SelfInstrument);
    }

    #[test]
    fn vehicle_round_trips_every_variant_through_json() {
        for v in [
            HedgeVehicle::SelfInstrument,
            HedgeVehicle::Benchmark,
            HedgeVehicle::Instrument {
                instrument_id: "US912810TM0".into(),
            },
            HedgeVehicle::Future {
                contract_id: "TY-DEC26".into(),
            },
        ] {
            let json = serde_json::to_string(&v).expect("serialize");
            let back: HedgeVehicle = serde_json::from_str(&json).expect("reload");
            assert_eq!(back, v, "{json}");
        }
    }

    #[test]
    fn named_instrument_reads_both_named_variants() {
        assert_eq!(
            HedgeVehicle::Instrument {
                instrument_id: "A".into()
            }
            .named_instrument(),
            Some("A")
        );
        assert_eq!(
            HedgeVehicle::Future {
                contract_id: "TY".into()
            }
            .named_instrument(),
            Some("TY")
        );
        assert!(
            HedgeVehicle::Future {
                contract_id: "TY".into()
            }
            .is_future()
        );
        assert!(!HedgeVehicle::Benchmark.is_future());
    }

    // --- the registry: resolution by instrument + maturity -------------------

    /// The headline case: a **9-year corp bond maps to the 10Y future**, not to the 5Y.
    #[test]
    fn nine_year_corp_resolves_to_the_ten_year_future_bucket() {
        let reg = HedgeVehicleRegistry::new(vec![fv_bucket(), ty_bucket()]);
        let hit = reg
            .resolve("XS-CORP-9Y", "BOND", "USD", Some(9.0))
            .expect("a 9y bond must resolve a vehicle");
        assert_eq!(hit.hedge_instrument_id, "TY-DEC26");
        assert!(hit.is_future);
    }

    #[test]
    fn a_five_year_corp_resolves_to_the_five_year_bucket() {
        let reg = HedgeVehicleRegistry::new(vec![fv_bucket(), ty_bucket()]);
        let hit = reg.resolve("XS-CORP-5Y", "BOND", "USD", Some(5.0)).unwrap();
        assert_eq!(hit.hedge_instrument_id, "FV-DEC26");
    }

    #[test]
    fn maturity_outside_every_bucket_resolves_nothing() {
        let reg = HedgeVehicleRegistry::new(vec![fv_bucket(), ty_bucket()]);
        assert!(
            reg.resolve("XS-CORP-30Y", "BOND", "USD", Some(30.0))
                .is_none(),
            "a 30y bond with no 30y bucket must NOT silently fall into the 10Y"
        );
    }

    #[test]
    fn bucket_bounds_are_inclusive_low_exclusive_high() {
        let reg = HedgeVehicleRegistry::new(vec![ty_bucket()]);
        assert!(
            reg.resolve("X", "BOND", "USD", Some(7.0)).is_some(),
            "min is inclusive"
        );
        assert!(
            reg.resolve("X", "BOND", "USD", Some(12.0)).is_none(),
            "max is exclusive"
        );
    }

    #[test]
    fn an_instrument_specific_row_beats_a_family_row() {
        let mut specific = ty_bucket();
        specific.id = "PIN".into();
        specific.instrument_id = "XS-CORP-9Y".into();
        specific.hedge_instrument_id = "UXY-DEC26".into();
        let reg = HedgeVehicleRegistry::new(vec![ty_bucket(), specific]);
        let hit = reg.resolve("XS-CORP-9Y", "BOND", "USD", Some(9.0)).unwrap();
        assert_eq!(
            hit.hedge_instrument_id, "UXY-DEC26",
            "the security-specific row wins over the family bucket"
        );
    }

    #[test]
    fn equally_specific_rows_tie_break_on_the_narrower_bucket() {
        let wide = HedgeVehicleRule {
            id: "WIDE".into(),
            product: "BOND".into(),
            ccy: "USD".into(),
            min_maturity_years: 1.0,
            max_maturity_years: 30.0,
            hedge_instrument_id: "US-DEC26".into(),
            is_future: true,
            dv01_per_unit: 190.0,
            unit_label: "contract".into(),
            ..HedgeVehicleRule::default()
        };
        let reg = HedgeVehicleRegistry::new(vec![wide, ty_bucket()]);
        let hit = reg.resolve("X", "BOND", "USD", Some(9.0)).unwrap();
        assert_eq!(hit.hedge_instrument_id, "TY-DEC26", "narrower bucket wins");
    }

    #[test]
    fn a_bucketless_row_matches_a_cell_with_no_maturity() {
        let anyswap = HedgeVehicleRule {
            id: "OIS-ANY".into(),
            product: "OIS".into(),
            hedge_instrument_id: "SOFR-SWAP-10Y".into(),
            dv01_per_unit: 900.0,
            unit_label: "1mm face".into(),
            ..HedgeVehicleRule::default()
        };
        let reg = HedgeVehicleRegistry::new(vec![anyswap, ty_bucket()]);
        assert_eq!(
            reg.resolve("USD-OIS-10Y", "OIS", "USD", None)
                .unwrap()
                .hedge_instrument_id,
            "SOFR-SWAP-10Y"
        );
        // A bucketed row can NOT match a cell whose maturity is unknown.
        assert!(reg.resolve("X", "BOND", "USD", None).is_none());
    }

    #[test]
    fn matching_is_case_insensitive_on_the_string_axes() {
        let reg = HedgeVehicleRegistry::new(vec![ty_bucket()]);
        assert!(reg.resolve("x", "bond", "usd", Some(9.0)).is_some());
    }

    #[test]
    fn an_invalid_row_never_resolves() {
        let mut broken = ty_bucket();
        broken.dv01_per_unit = 0.0;
        let reg = HedgeVehicleRegistry::new(vec![broken]);
        assert!(
            reg.resolve("X", "BOND", "USD", Some(9.0)).is_none(),
            "a row with no usable DV01 must never be picked — the ratio would be fabricated"
        );
    }

    #[test]
    fn by_hedge_instrument_recovers_a_named_vehicles_dv01() {
        let reg = HedgeVehicleRegistry::new(vec![fv_bucket(), ty_bucket()]);
        assert_eq!(
            reg.by_hedge_instrument("TY-DEC26").unwrap().dv01_per_unit,
            78.0
        );
        assert!(
            reg.by_hedge_instrument("UNKNOWN-CONTRACT").is_none(),
            "an unregistered vehicle has no known DV01"
        );
    }

    // --- validation ----------------------------------------------------------

    #[test]
    fn validation_collects_every_defect_at_once() {
        let bad = HedgeVehicleRule {
            id: String::new(),
            hedge_instrument_id: String::new(),
            dv01_per_unit: -1.0,
            min_maturity_years: 12.0,
            max_maturity_years: 7.0,
            ..HedgeVehicleRule::default()
        };
        let errs = bad.validate().unwrap_err();
        assert!(errs.contains(&HedgeVehicleError::MissingId));
        assert!(
            errs.iter()
                .any(|e| matches!(e, HedgeVehicleError::MissingHedgeInstrument { .. }))
        );
        assert!(
            errs.iter()
                .any(|e| matches!(e, HedgeVehicleError::NonPositiveDv01 { .. }))
        );
        assert!(
            errs.iter()
                .any(|e| matches!(e, HedgeVehicleError::InvalidMaturityBucket { .. }))
        );
    }

    #[test]
    fn duplicate_ids_are_rejected_by_the_registry() {
        let reg = HedgeVehicleRegistry::new(vec![ty_bucket(), ty_bucket()]);
        let errs = reg.validate().unwrap_err();
        assert!(errs.contains(&HedgeVehicleError::DuplicateId {
            id: "US-BOND-10Y".into()
        }));
    }

    #[test]
    fn a_well_formed_registry_validates() {
        assert!(
            HedgeVehicleRegistry::new(vec![fv_bucket(), ty_bucket()])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn error_display_is_nonempty() {
        assert!(!HedgeVehicleError::MissingId.to_string().is_empty());
        assert!(
            !HedgeVehicleError::NonPositiveDv01 { id: "X".into() }
                .to_string()
                .is_empty()
        );
    }

    // --- the ratio maths (the part most likely to be quietly wrong) ----------

    /// The worked example from the brief: hedging 24,840 DV01 with a 78-DV01 contract is
    /// 318.46… contracts, which trades as **318** — you cannot trade a fraction of a lot —
    /// leaving a real, reported residual.
    #[test]
    fn futures_ratio_rounds_to_whole_contracts_and_reports_the_residual() {
        let rule = ty_bucket();
        let plan = plan_hedge_ratio(24_840.0, Dv01Basis::Analytic, &rule, true)
            .expect("a well-formed rule sizes");
        assert!(
            (plan.exact_units - 318.461_538_461_538_5).abs() < 1e-9,
            "{plan:?}"
        );
        assert_eq!(plan.units, 318.0, "whole contracts only");
        assert!((plan.hedged_dv01 - 24_804.0).abs() < 1e-9);
        assert!(
            (plan.residual_dv01 - 36.0).abs() < 1e-9,
            "the rounding residual is reported, not hidden: {}",
            plan.residual_dv01
        );
        assert!(plan.was_rounded());
        assert!(plan.is_tradeable());
        assert!(
            plan.residual_dv01 > 0.0,
            "rounding DOWN leaves risk on the book"
        );
    }

    /// Rounding UP over-hedges — the residual goes negative and must say so, because the
    /// desk is now short the difference rather than long it.
    #[test]
    fn rounding_up_reports_a_negative_over_hedged_residual() {
        let rule = ty_bucket();
        // 78 × 318.6 = 24,850.8 → rounds to 319 contracts = 24,882 DV01.
        let plan = plan_hedge_ratio(24_850.8, Dv01Basis::Analytic, &rule, true).unwrap();
        assert_eq!(plan.units, 319.0);
        assert!(
            plan.residual_dv01 < 0.0,
            "an over-hedge reports a NEGATIVE residual: {}",
            plan.residual_dv01
        );
        assert!((plan.residual_dv01 + 31.2).abs() < 1e-9);
    }

    /// A target below half a contract cannot be hedged with that vehicle at all. The plan
    /// says so (0 units, the whole target as residual) rather than inventing a part lot.
    #[test]
    fn a_target_below_half_a_contract_trades_nothing_and_says_so() {
        let rule = ty_bucket();
        let plan = plan_hedge_ratio(30.0, Dv01Basis::Analytic, &rule, true).unwrap();
        assert_eq!(plan.units, 0.0);
        assert_eq!(plan.hedged_dv01, 0.0);
        assert!((plan.residual_dv01 - 30.0).abs() < 1e-12);
        assert!(!plan.is_tradeable(), "0 contracts is not a tradeable hedge");
        assert_eq!(plan.fraction_of_target(), 0.0);
    }

    /// A non-futures vehicle (a cash benchmark) trades in fractional units — no rounding.
    #[test]
    fn a_non_futures_vehicle_keeps_the_exact_ratio() {
        let rule = HedgeVehicleRule {
            id: "BENCH".into(),
            product: "BOND".into(),
            min_maturity_years: 7.0,
            max_maturity_years: 12.0,
            hedge_instrument_id: "US912810TM0".into(),
            is_future: false,
            dv01_per_unit: 780.0,
            unit_label: "1mm face".into(),
            ..HedgeVehicleRule::default()
        };
        let plan = plan_hedge_ratio(24_840.0, Dv01Basis::Analytic, &rule, false).unwrap();
        assert!((plan.units - 31.846_153_846_153_85).abs() < 1e-9);
        assert!(!plan.was_rounded());
        assert!(
            plan.residual_dv01.abs() < 1e-9,
            "an unrounded ratio has no residual"
        );
        assert!((plan.fraction_of_target() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_non_positive_unit_dv01_refuses_to_size() {
        let mut rule = ty_bucket();
        rule.dv01_per_unit = 0.0;
        assert!(plan_hedge_ratio(10_000.0, Dv01Basis::Analytic, &rule, true).is_none());
        rule.dv01_per_unit = f64::NAN;
        assert!(plan_hedge_ratio(10_000.0, Dv01Basis::Analytic, &rule, true).is_none());
    }

    #[test]
    fn a_non_finite_or_negative_target_refuses_to_size() {
        let rule = ty_bucket();
        assert!(plan_hedge_ratio(f64::NAN, Dv01Basis::Analytic, &rule, true).is_none());
        assert!(plan_hedge_ratio(-1.0, Dv01Basis::Analytic, &rule, true).is_none());
    }

    /// The basis label rides all the way through — a proxy-based ratio is flagged as not
    /// duration-correct so no surface can present it as exact.
    #[test]
    fn the_dv01_basis_is_carried_and_flags_the_duration_blind_proxy() {
        let rule = ty_bucket();
        let proxy = plan_hedge_ratio(24_840.0, Dv01Basis::ExposureProxy, &rule, true).unwrap();
        assert_eq!(proxy.basis, Dv01Basis::ExposureProxy);
        assert!(!proxy.basis.is_duration_correct());
        assert!(proxy.summary().contains("exposure-proxy"));

        let exact = plan_hedge_ratio(24_840.0, Dv01Basis::Analytic, &rule, true).unwrap();
        assert!(exact.basis.is_duration_correct());
        assert!(Dv01Basis::AnnuityPv01.is_duration_correct());
    }

    #[test]
    fn summary_reads_as_a_trader_instruction() {
        let plan = plan_hedge_ratio(24_840.0, Dv01Basis::Analytic, &ty_bucket(), true).unwrap();
        let s = plan.summary();
        assert!(s.contains("318 contracts of TY-DEC26"), "{s}");
        assert!(s.contains("318.46 exact"), "{s}");
        assert!(s.contains("residual 36.0 DV01"), "{s}");
    }

    /// The fraction a plan removes is the scalar a caller applies to the position's face
    /// to build the risk-equivalent offset — it must equal `hedged/target` exactly.
    #[test]
    fn fraction_of_target_is_the_realised_shed_fraction() {
        let plan = plan_hedge_ratio(24_840.0, Dv01Basis::Analytic, &ty_bucket(), true).unwrap();
        assert!((plan.fraction_of_target() - 24_804.0 / 24_840.0).abs() < 1e-15);
        assert!(
            plan.fraction_of_target() < 1.0,
            "rounding down sheds less than asked"
        );
    }
}
