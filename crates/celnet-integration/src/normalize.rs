//! Normalize a decoded vendor smile message into the canonical Celnet surface
//! input — [`celnet_surface::MarketQuotes`] + [`celnet_surface::MarketContext`].
//!
//! This is the convention-aware ingest step. It performs three jobs:
//!
//! 1. **Unit normalization** — wire vols are in percentage points and forwards
//!    may be points; the canonical types are absolute vols and outright
//!    forwards. Percentages become absolutes (`÷100`), points become outrights.
//! 2. **Tenor / pair parsing** — the feed's tokens (`"3M"`, `"1Y"`, `"EURUSD"`)
//!    become [`celnet_types::Tenor`] / [`celnet_types::CcyPair`].
//! 3. **Convention resolution & cross-check** — the canonical convention record
//!    for the `(pair, tenor)` is resolved from [`celnet_conventions`], and the
//!    feed's *self-declared* convention is checked against it. A material
//!    mismatch (e.g. the feed declares forward-delta where Celnet resolves
//!    spot-delta) is surfaced as an error rather than silently corrupting every
//!    downstream strike — convention error dwarfs model error
//!    (`docs/ANALYTICS-SPEC.md` §1.1).
//!
//! The two rates carried by [`celnet_surface::MarketContext`] are reconstructed
//! so the slice's outright forward is reproduced **exactly**: given a domestic
//! (numeraire) discount rate from the curve layer, the foreign rate is implied
//! from `F = S·e^{(r_dom − r_for)·t}` ⇒ `r_for = r_dom − ln(F/S)/t`. The feed's
//! forward is therefore honoured to machine precision regardless of the rate the
//! curve layer supplies, which is what the surface construction consumes.

use celnet_conventions::resolve;
use celnet_core::math::ln;
use celnet_surface::{MarketContext, MarketQuotes};
use celnet_types::{CcyPair, DeltaConvention, Tenor};

use crate::vendor::{VendorSmileMessage, WireWing};

/// Percentage-point → absolute scale (`10.5%` ⇒ `0.105`).
const PCT: f64 = 0.01;

/// A normalized, canonical surface input for one `(pair, tenor)` slice, paired
/// with the source/observation metadata the aggregation layer needs.
///
/// This is the output of ingest and the input to multi-source blending: the
/// canonical [`MarketQuotes`] + [`MarketContext`] plus the source id, the
/// observation timestamp, and any retained NDF fixing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedSlice {
    /// The `(pair, tenor)` this slice prices.
    pub pair: CcyPair,
    /// The slice tenor.
    pub tenor: Tenor,
    /// Canonical broker quotes (absolute vols, on the resolved conventions).
    pub quotes: MarketQuotes,
    /// Canonical market context (spot, two rates, vol-time, conventions).
    pub context: MarketContext,
    /// Source feed identifier (for divergence attribution).
    pub source: SourceId,
    /// Observation instant in epoch nanoseconds (for staleness decay).
    pub observed_at_nanos: i64,
    /// Retained NDF cash-settlement fixing, if the message carried one.
    pub ndf_fixing: Option<f64>,
}

/// A compact, `Copy` source identifier derived from a feed's free-form source
/// string, so normalized slices stay POD on the hot path. Stores up to 16 ASCII
/// bytes of the source name (longer names are truncated); this is sufficient to
/// attribute divergence/staleness to a feed without allocating per slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId {
    bytes: [u8; 16],
    len: u8,
}

impl SourceId {
    /// Build a source id from a name, retaining up to 16 bytes.
    #[must_use]
    pub fn new(name: &str) -> Self {
        let src = name.as_bytes();
        let len = src.len().min(16);
        let mut bytes = [0u8; 16];
        bytes[..len].copy_from_slice(&src[..len]);
        Self {
            bytes,
            #[allow(clippy::cast_possible_truncation)]
            len: len as u8,
        }
    }

    /// The retained source name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("?")
    }
}

impl core::fmt::Display for SourceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What can go wrong turning a wire message into a canonical slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizeError {
    /// The `pair` token was not a valid 6-letter currency pair.
    BadPair(String),
    /// The `tenor_label` token was not a recognised tenor.
    BadTenor(String),
    /// A required numeric input was non-finite or non-positive.
    BadValue(&'static str),
    /// The vol-time year fraction was non-positive (a zero/negative tenor).
    BadMaturity,
    /// The feed's declared delta convention disagrees materially with the
    /// canonical convention Celnet resolves for the `(pair, tenor)`. Carries the
    /// declared and the resolved convention for diagnosis.
    DeltaConventionMismatch {
        /// What the feed declared.
        declared: DeltaConvention,
        /// What Celnet resolved for the `(pair, tenor)`.
        resolved: DeltaConvention,
    },
    /// The feed's premium-currency flag disagrees with its declared delta
    /// convention (an internally inconsistent message).
    PremiumFlagInconsistent,
}

impl core::fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            NormalizeError::BadPair(s) => write!(f, "invalid currency pair: {s:?}"),
            NormalizeError::BadTenor(s) => write!(f, "invalid tenor token: {s:?}"),
            NormalizeError::BadValue(w) => write!(f, "invalid numeric input: {w}"),
            NormalizeError::BadMaturity => write!(f, "vol-time year fraction must be positive"),
            NormalizeError::DeltaConventionMismatch { declared, resolved } => write!(
                f,
                "feed delta convention {declared:?} disagrees with resolved {resolved:?}"
            ),
            NormalizeError::PremiumFlagInconsistent => {
                write!(
                    f,
                    "feed premium-currency flag disagrees with its delta convention"
                )
            }
        }
    }
}

impl std::error::Error for NormalizeError {}

/// Parse a feed tenor token (`"ON"`, `"1W"`, `"2W"`, `"1M"`, `"3M"`, `"1Y"`)
/// into a canonical [`Tenor`]. Case-insensitive on the unit letter.
fn parse_tenor(tok: &str) -> Result<Tenor, NormalizeError> {
    let t = tok.trim();
    if t.eq_ignore_ascii_case("ON") {
        return Ok(Tenor::Overnight);
    }
    let (num, unit) = t.split_at(t.len().saturating_sub(1));
    let n: u16 = num
        .parse()
        .map_err(|_| NormalizeError::BadTenor(tok.to_owned()))?;
    match unit {
        "W" | "w" => Ok(Tenor::Weeks(n)),
        "M" | "m" => Ok(Tenor::Months(n)),
        "Y" | "y" => Ok(Tenor::Years(n)),
        _ => Err(NormalizeError::BadTenor(tok.to_owned())),
    }
}

/// Approximate vol-time year fraction for a tenor, on the ACT/365-fixed FX
/// vol-time basis. Calendar-exact dates require a horizon date and the
/// `celnet-calendar` schedule; for a snapshot surface input (no booking date)
/// this nominal mapping is used — `celnet-conventions::vol_year_fraction`
/// supplies the calendar-exact value when a horizon is available downstream.
fn nominal_year_fraction(tenor: Tenor) -> f64 {
    match tenor {
        Tenor::Overnight => 1.0 / 365.0,
        Tenor::Weeks(w) => f64::from(w) * 7.0 / 365.0,
        Tenor::Months(m) => f64::from(m) * 30.0 / 365.0,
        Tenor::Years(y) => f64::from(y) * 365.0 / 365.0,
    }
}

fn require_finite_positive(x: f64, what: &'static str) -> Result<f64, NormalizeError> {
    if x.is_finite() && x > 0.0 {
        Ok(x)
    } else {
        Err(NormalizeError::BadValue(what))
    }
}

fn require_finite(x: f64, what: &'static str) -> Result<f64, NormalizeError> {
    if x.is_finite() {
        Ok(x)
    } else {
        Err(NormalizeError::BadValue(what))
    }
}

/// Convert a wire wing (percent) to canonical absolute risk-reversal/butterfly.
fn wing_absolute(w: &WireWing) -> Result<(f64, f64), NormalizeError> {
    Ok((
        require_finite(w.risk_reversal_pct, "risk_reversal")? * PCT,
        require_finite(w.butterfly_pct, "butterfly")? * PCT,
    ))
}

/// Normalize a decoded vendor smile message into a canonical [`NormalizedSlice`].
///
/// `r_dom` is the domestic (numeraire) continuously-compounded discount rate
/// from the curve layer; the foreign rate is implied from the feed's forward so
/// the outright forward is reproduced exactly (see the module docs). The feed's
/// declared delta convention is cross-checked against the canonical convention
/// resolved for the `(pair, tenor)` and a material mismatch is an error.
///
/// # Errors
///
/// Returns [`NormalizeError`] on an unparseable pair/tenor, a non-finite or
/// non-positive numeric input, a non-positive maturity, or a convention
/// mismatch.
pub fn normalize(msg: &VendorSmileMessage, r_dom: f64) -> Result<NormalizedSlice, NormalizeError> {
    let pair =
        CcyPair::parse(&msg.pair).ok_or_else(|| NormalizeError::BadPair(msg.pair.clone()))?;
    let tenor = parse_tenor(&msg.tenor_label)?;

    let spot = require_finite_positive(msg.spot, "spot")?;
    let forward = require_finite_positive(msg.forward.outright(spot), "forward")?;
    let atm_vol = require_finite_positive(msg.atm_vol_pct, "atm_vol")? * PCT;
    let r_dom = require_finite(r_dom, "r_dom")?;

    let t = nominal_year_fraction(tenor);
    if !(t.is_finite() && t > 0.0) {
        return Err(NormalizeError::BadMaturity);
    }

    // Imply the foreign rate so F = S·e^{(r_dom − r_for)·t} is reproduced exactly.
    // r_for = r_dom − ln(F/S)/t.
    let r_for = r_dom - ln(forward / spot) / t;

    // Resolve the canonical convention and cross-check the feed's declaration.
    let resolved = resolve(pair, tenor).record;
    let declared = msg.conventions.delta.canonical();
    if declared != resolved.delta {
        return Err(NormalizeError::DeltaConventionMismatch {
            declared,
            resolved: resolved.delta,
        });
    }
    // The feed's premium-currency flag must agree with its declared delta.
    if msg.conventions.premium_in_foreign != declared_is_premium_adjusted(declared) {
        return Err(NormalizeError::PremiumFlagInconsistent);
    }

    // Build the canonical quotes (three- or five-point).
    let (rr25, bf25) = wing_absolute(&msg.inner)?;
    let quotes = match &msg.outer {
        Some(outer) => {
            let (rr10, bf10) = wing_absolute(outer)?;
            MarketQuotes::five_point(atm_vol, rr25, bf25, rr10, bf10)
        }
        None => MarketQuotes::three_point(atm_vol, rr25, bf25),
    };

    let context = MarketContext::new(spot, r_dom, r_for, t, resolved);

    Ok(NormalizedSlice {
        pair,
        tenor,
        quotes,
        context,
        source: SourceId::new(&msg.source),
        observed_at_nanos: msg.observed_at_nanos,
        ndf_fixing: msg.ndf_fixing,
    })
}

/// Whether a delta convention is premium-adjusted (foreign-premium).
const fn declared_is_premium_adjusted(d: DeltaConvention) -> bool {
    matches!(
        d,
        DeltaConvention::SpotPremiumAdjusted | DeltaConvention::ForwardPremiumAdjusted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::{WireAtmConvention, WireConventions, WireDeltaConvention, WireForward};
    use celnet_core::{assert_close, is_close};

    /// A representative EURUSD 1Y message in feed units/conventions.
    fn eurusd_1y_msg() -> VendorSmileMessage {
        VendorSmileMessage {
            pair: "EURUSD".into(),
            tenor_label: "1Y".into(),
            spot: 1.10,
            forward: WireForward::Points {
                points: 110.0,
                pip_factor: 10_000.0,
            },
            atm_vol_pct: 10.5,
            inner: WireWing {
                delta_pct: 25.0,
                risk_reversal_pct: 0.45,
                butterfly_pct: 0.20,
            },
            outer: Some(WireWing {
                delta_pct: 10.0,
                risk_reversal_pct: 0.80,
                butterfly_pct: 0.55,
            }),
            ndf_fixing: None,
            conventions: WireConventions {
                // EURUSD resolves to spot premium-adjusted at 1Y (≤1Y rule).
                delta: WireDeltaConvention::SpotPremiumAdjusted,
                atm: WireAtmConvention::DeltaNeutralStraddle,
                premium_in_foreign: true,
            },
            source: "feed-a".into(),
            observed_at_nanos: 1_700_000_000_000_000_000,
        }
    }

    #[test]
    fn parses_tenors() {
        assert_eq!(parse_tenor("ON").unwrap(), Tenor::Overnight);
        assert_eq!(parse_tenor("2W").unwrap(), Tenor::Weeks(2));
        assert_eq!(parse_tenor("3M").unwrap(), Tenor::Months(3));
        assert_eq!(parse_tenor("1Y").unwrap(), Tenor::Years(1));
        assert_eq!(parse_tenor("1y").unwrap(), Tenor::Years(1));
        assert!(parse_tenor("zz").is_err());
    }

    #[test]
    fn normalizes_units_and_reproduces_forward_exactly() {
        let msg = eurusd_1y_msg();
        let slice = normalize(&msg, 0.02).unwrap();
        // Percent → absolute.
        assert_close!(slice.quotes.atm_vol, 0.105);
        assert_close!(slice.quotes.inner.risk_reversal, 0.0045);
        assert_close!(slice.quotes.inner.butterfly, 0.0020);
        let outer = slice.quotes.outer.unwrap();
        assert_close!(outer.risk_reversal, 0.0080);
        assert_close!(outer.butterfly, 0.0055);
        // Forward reproduced exactly through the implied foreign rate.
        let f_expected = 1.10 + 110.0 / 10_000.0;
        assert!(is_close(slice.context.forward(), f_expected, 1e-12, 1e-13));
        // Domestic rate preserved.
        assert_close!(slice.context.r_dom, 0.02);
    }

    #[test]
    fn rejects_declared_convention_mismatch() {
        let mut msg = eurusd_1y_msg();
        // Declare forward-delta where EURUSD 1Y resolves spot-delta.
        msg.conventions.delta = WireDeltaConvention::ForwardPremiumAdjusted;
        let err = normalize(&msg, 0.02).unwrap_err();
        assert!(matches!(
            err,
            NormalizeError::DeltaConventionMismatch { .. }
        ));
    }

    #[test]
    fn rejects_inconsistent_premium_flag() {
        let mut msg = eurusd_1y_msg();
        msg.conventions.premium_in_foreign = false; // contradicts premium-adjusted delta
        assert_eq!(
            normalize(&msg, 0.02).unwrap_err(),
            NormalizeError::PremiumFlagInconsistent
        );
    }

    #[test]
    fn rejects_bad_inputs() {
        let mut msg = eurusd_1y_msg();
        msg.spot = -1.0;
        assert!(matches!(
            normalize(&msg, 0.02).unwrap_err(),
            NormalizeError::BadValue("spot")
        ));
        let mut msg = eurusd_1y_msg();
        msg.pair = "EUR".into();
        assert!(matches!(
            normalize(&msg, 0.02).unwrap_err(),
            NormalizeError::BadPair(_)
        ));
    }

    #[test]
    fn source_id_truncates_and_round_trips() {
        let s = SourceId::new("feed-a");
        assert_eq!(s.as_str(), "feed-a");
        let long = SourceId::new("a-very-long-source-name-beyond-sixteen");
        assert_eq!(long.as_str().len(), 16);
    }
}
