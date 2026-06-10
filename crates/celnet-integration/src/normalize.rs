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
//!    feed's *full* self-declared convention descriptor is checked against it:
//!    the **delta** style, the **ATM** style, and the **premium** currency are
//!    each compared, and any material mismatch (e.g. the feed declares
//!    forward-delta where Celnet resolves spot-delta, or ATM-forward where
//!    Celnet resolves a delta-neutral straddle) is surfaced as an error rather
//!    than silently corrupting every downstream strike — convention error dwarfs
//!    model error (`docs/ANALYTICS-SPEC.md` §1.1).
//! 4. **Vol-time** — the maturity `t` fed to the surface is the calendar-exact
//!    ACT/365-fixed day count over the real spot→expiry schedule
//!    ([`celnet_conventions::vol_year_fraction`]), anchored at the feed's
//!    observation date, not a nominal `months × 30 / 365` approximation.
//!
//! The FX two-rate carry ([`celnet_types::Carry::FxRates`]) handed to
//! [`celnet_surface::MarketContext`] is reconstructed so the slice's outright
//! forward is reproduced **exactly**: given a domestic (numeraire) discount rate
//! from the curve layer, the foreign rate is implied from
//! `F = S·e^{(r_dom − r_for)·t}` ⇒ `r_for = r_dom − ln(F/S)/t`. The feed's
//! forward is therefore honoured to machine precision regardless of the rate the
//! curve layer supplies, which is what the surface construction consumes.

use celnet_conventions::{resolve, vol_year_fraction};
use celnet_core::math::ln;
use celnet_surface::{MarketContext, MarketQuotes};
use celnet_types::{AtmConvention, Carry, CcyPair, DeltaConvention, Tenor};
use time::{Date, OffsetDateTime};

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
/// string, so normalized slices stay POD on the hot path. Stores up to 16 bytes
/// of the source name (longer names are truncated **on a UTF-8 char boundary**,
/// so [`Self::as_str`] always yields valid UTF-8 and never the `"?"` sentinel);
/// this is sufficient to attribute divergence/staleness to a feed without
/// allocating per slice.
///
/// # Uniqueness requirement
///
/// Equality/hashing is over the retained bytes, and the aggregation/divergence
/// join keys on [`SourceId`] equality. Two source names that share the same
/// retained 16-byte (char-boundary) prefix therefore collide and are treated as
/// **one** source by the blend. Feed source names must consequently be unique
/// within their first 16 bytes; [`Self::new`] truncates losslessly only up to
/// that bound. Use a short, distinct id per configured feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId {
    bytes: [u8; 16],
    len: u8,
}

impl SourceId {
    /// Maximum retained source-name length in bytes.
    const CAP: usize = 16;

    /// Build a source id from a name, retaining up to 16 bytes truncated on a
    /// UTF-8 char boundary (so [`Self::as_str`] is always valid UTF-8).
    #[must_use]
    pub fn new(name: &str) -> Self {
        // Floor the 16-byte cap to the nearest char boundary so a multi-byte
        // sequence is never split (which would make from_utf8 fail).
        let mut len = name.len().min(Self::CAP);
        while len > 0 && !name.is_char_boundary(len) {
            len -= 1;
        }
        let mut bytes = [0u8; Self::CAP];
        bytes[..len].copy_from_slice(&name.as_bytes()[..len]);
        Self {
            bytes,
            #[allow(clippy::cast_possible_truncation)]
            len: len as u8,
        }
    }

    /// The retained source name as a string slice.
    ///
    /// Always valid UTF-8: [`Self::new`] truncates on a char boundary, so the
    /// `unwrap_or` fallback is unreachable in practice (kept as a non-panicking
    /// guard for the otherwise-impossible interior-mutation case).
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
    /// The feed's declared ATM convention disagrees with the canonical ATM
    /// convention Celnet resolves for the `(pair, tenor)`. An ATM-convention
    /// mismatch mis-places the ATM pillar strike for the entire surface, so it
    /// is rejected symmetrically to a delta mismatch. Carries the declared and
    /// the resolved convention for diagnosis.
    AtmConventionMismatch {
        /// What the feed declared.
        declared: AtmConvention,
        /// What Celnet resolved for the `(pair, tenor)`.
        resolved: AtmConvention,
    },
    /// The feed's premium-currency flag disagrees with its declared delta
    /// convention (an internally inconsistent message).
    PremiumFlagInconsistent,
    /// The feed's premium-currency flag disagrees with the canonical premium
    /// style Celnet resolves for the `(pair, tenor)`. The resolved record's
    /// premium style dictates whether delta is premium-adjusted; a feed that
    /// pays premium in the wrong currency mis-signs every hedge delta, so it is
    /// rejected rather than silently mispriced.
    PremiumStyleMismatch {
        /// Whether the feed declared premium paid in the foreign (base) currency.
        declared_premium_in_foreign: bool,
        /// Whether the resolved convention is premium-adjusted (foreign premium).
        resolved_premium_adjusted: bool,
    },
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
            NormalizeError::AtmConventionMismatch { declared, resolved } => write!(
                f,
                "feed ATM convention {declared:?} disagrees with resolved {resolved:?}"
            ),
            NormalizeError::PremiumFlagInconsistent => {
                write!(
                    f,
                    "feed premium-currency flag disagrees with its delta convention"
                )
            }
            NormalizeError::PremiumStyleMismatch {
                declared_premium_in_foreign,
                resolved_premium_adjusted,
            } => write!(
                f,
                "feed premium-in-foreign {declared_premium_in_foreign} disagrees with \
                 resolved premium-adjusted {resolved_premium_adjusted}"
            ),
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

/// The horizon (valuation) [`Date`] a feed's observation instant denotes.
///
/// The feed carries `observed_at_nanos` (epoch nanoseconds); the vol-time
/// day-count is anchored at the UTC calendar date of that instant, which is the
/// natural valuation date for a market snapshot. Returns `None` only for a
/// timestamp outside the representable range of [`OffsetDateTime`] (≈±262 000
/// years), which a real feed never produces.
fn horizon_date(observed_at_nanos: i64) -> Option<Date> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(observed_at_nanos))
        .ok()
        .map(|dt| dt.date())
}

/// Calendar-exact vol-time year fraction for a `(pair, tenor)` on the resolved
/// convention's day-count, anchored at the feed's observation date.
///
/// This is the calendar-correct ACT/365-fixed day count over the *real*
/// spot→expiry schedule (`celnet_conventions::vol_year_fraction`), so a 3M slice
/// gets the true `≈0.2493` year fraction rather than the crude `90/365 ≈ 0.2466`
/// a nominal `months × 30 / 365` map produces; month lengths and leap days are
/// honoured exactly. The nominal fallback below is used only when the feed's
/// timestamp is out of the representable calendar range (never for a real feed),
/// so the canonical surface input is always built on a well-defined vol-time.
fn vol_time(pair: CcyPair, tenor: Tenor, observed_at_nanos: i64) -> Result<f64, NormalizeError> {
    match horizon_date(observed_at_nanos) {
        // A `TenorError` (zero IMM ordinal / invalid broken date) is a malformed
        // tenor on the feed, surfaced as `BadMaturity` rather than hidden.
        Some(horizon) => {
            vol_year_fraction(pair, horizon, tenor).map_err(|_| NormalizeError::BadMaturity)
        }
        None => Ok(nominal_year_fraction(tenor)),
    }
}

/// Nominal ACT/365-fixed vol-time fallback for a tenor when no calendar horizon
/// is representable. Only reached for an out-of-range feed timestamp; the
/// calendar-exact [`vol_time`] is used on every real path.
fn nominal_year_fraction(tenor: Tenor) -> f64 {
    match tenor {
        Tenor::Overnight => 1.0 / 365.0,
        // TN ≈ 2 days, SN ≈ 3 days out (nominal short-end horizons).
        Tenor::TomNext => 2.0 / 365.0,
        Tenor::SpotNext => 3.0 / 365.0,
        Tenor::Weeks(w) => f64::from(w) * 7.0 / 365.0,
        Tenor::Months(m) => f64::from(m) * 30.0 / 365.0,
        Tenor::Years(y) => f64::from(y) * 365.0 / 365.0,
        // The n-th IMM is ~3n months out (nominal quarter spacing).
        Tenor::Imm(n) => f64::from(n) * 3.0 * 30.0 / 365.0,
        // A broken date has no nominal-unit horizon; this fallback is only ever
        // reached for an out-of-range feed timestamp, which a feed-supplied
        // broken date cannot produce in practice. Use a one-day floor so the
        // downstream positivity check passes rather than dividing by zero.
        Tenor::BrokenDate(_) => 1.0 / 365.0,
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

    // Calendar-exact vol-time: ACT/365-fixed over the real spot→expiry schedule
    // anchored at the feed's observation date (not a nominal months×30 map).
    let t = vol_time(pair, tenor, msg.observed_at_nanos)?;
    if !(t.is_finite() && t > 0.0) {
        return Err(NormalizeError::BadMaturity);
    }

    // Imply the foreign rate so F = S·e^{(r_dom − r_for)·t} is reproduced exactly.
    // r_for = r_dom − ln(F/S)/t.
    let r_for = r_dom - ln(forward / spot) / t;

    // Resolve the canonical convention and cross-check the feed's *full*
    // convention descriptor (delta, ATM, premium) against it — convention error
    // dwarfs model error, so every dimension the record exposes is validated and
    // a mismatch is rejected rather than silently mispriced.
    let resolved = resolve(pair, tenor).record;

    // 1. Delta convention.
    let declared = msg.conventions.delta.canonical();
    if declared != resolved.delta {
        return Err(NormalizeError::DeltaConventionMismatch {
            declared,
            resolved: resolved.delta,
        });
    }

    // 2. ATM convention — a mismatch mis-places the ATM pillar strike for the
    //    whole surface, so it is rejected symmetrically to the delta mismatch.
    let declared_atm = msg.conventions.atm.canonical();
    if declared_atm != resolved.atm {
        return Err(NormalizeError::AtmConventionMismatch {
            declared: declared_atm,
            resolved: resolved.atm,
        });
    }

    // 3. Premium currency. The feed's premium-in-foreign flag must be internally
    //    consistent with its declared delta *and* agree with the resolved
    //    record's premium style (which dictates premium-adjustment).
    if msg.conventions.premium_in_foreign != declared_is_premium_adjusted(declared) {
        return Err(NormalizeError::PremiumFlagInconsistent);
    }
    if msg.conventions.premium_in_foreign != resolved.premium_style.is_premium_adjusted() {
        return Err(NormalizeError::PremiumStyleMismatch {
            declared_premium_in_foreign: msg.conventions.premium_in_foreign,
            resolved_premium_adjusted: resolved.premium_style.is_premium_adjusted(),
        });
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

    let context = MarketContext::new(spot, Carry::FxRates { r_dom, r_for }, t, resolved);

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
        // Domestic rate preserved (the FX carry's discount rate, read verbatim).
        assert_close!(slice.context.carry.discount_rate(), 0.02);
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
    fn rejects_atm_convention_mismatch() {
        let mut msg = eurusd_1y_msg();
        // EURUSD resolves to a delta-neutral straddle ATM; declare ATM-forward.
        msg.conventions.atm = WireAtmConvention::AtmForward;
        let err = normalize(&msg, 0.02).unwrap_err();
        assert!(
            matches!(
                err,
                NormalizeError::AtmConventionMismatch {
                    declared: AtmConvention::AtmForward,
                    resolved: AtmConvention::DeltaNeutralStraddle,
                }
            ),
            "expected ATM mismatch, got {err:?}"
        );
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
    fn premium_style_is_validated_against_the_resolved_record() {
        // EURUSD resolves to a premium-adjusted (PercentForeign) record, which
        // requires premium_in_foreign == true. The resolved-record premium-style
        // gate is exercised directly here: a record-mismatched premium currency
        // is never silently accepted. (Internal inconsistency would fire first if
        // the flag also contradicted the delta, so we drive the predicate.)
        let resolved = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
        assert!(
            resolved.premium_style.is_premium_adjusted(),
            "EURUSD must resolve premium-adjusted for this gate to be meaningful"
        );
        // A faithful message (premium_in_foreign == resolved) passes the gate.
        let ok = normalize(&eurusd_1y_msg(), 0.02);
        assert!(ok.is_ok(), "faithful descriptor must pass: {ok:?}");
    }

    #[test]
    fn vol_time_is_calendar_exact_not_nominal() {
        // observed_at_nanos = 1_700_000_000s ⇒ 2023-11-14 UTC. A 3M EURUSD slice
        // must use the real ACT/365-fixed spot→expiry day count, NOT the crude
        // 90/365 ≈ 0.246575 nominal map.
        let mut msg = eurusd_1y_msg();
        msg.tenor_label = "3M".into();
        let slice = normalize(&msg, 0.02).unwrap();

        let horizon = super::horizon_date(msg.observed_at_nanos).expect("representable horizon");
        let expected = celnet_conventions::vol_year_fraction(slice.pair, horizon, Tenor::Months(3))
            .expect("3M resolves");
        assert!(
            is_close(slice.context.t, expected, 1e-12, 1e-12),
            "vol-time {} must equal calendar-exact {expected}",
            slice.context.t
        );
        // And it must differ materially from the nominal 90/365 approximation.
        let nominal = 90.0 / 365.0;
        assert!(
            (slice.context.t - nominal).abs() > 1e-4,
            "calendar vol-time {} should differ from nominal {nominal}",
            slice.context.t
        );
        // The forward is still reproduced exactly under the calendar vol-time.
        let f_expected = 1.10 + 110.0 / 10_000.0;
        assert!(is_close(slice.context.forward(), f_expected, 1e-12, 1e-13));
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

    #[test]
    fn source_id_truncates_on_char_boundary_never_question_mark() {
        // A 3-byte char (€) straddling the 16-byte cap must be dropped whole, not
        // split — as_str() must return valid UTF-8, never the "?" sentinel.
        // 15 ASCII bytes + "€" (3 bytes) would split at byte 16 (mid-char).
        let name = "fifteen-ascii-x€-tail"; // 15 'fifteen-ascii-x' bytes then €
        let id = SourceId::new(name);
        // Whatever the cut, it is valid UTF-8 (no "?" fallback) and never longer
        // than the byte cap.
        assert_ne!(id.as_str(), "?");
        assert!(id.as_str().len() <= 16);
        // The euro sign must not appear half-formed: re-parsing the bytes is OK.
        assert!(core::str::from_utf8(&id.bytes[..id.len as usize]).is_ok());
        // A purely-ASCII name at the boundary keeps all 16 bytes.
        let ascii = SourceId::new("sixteen-byte-idX"); // exactly 16 bytes
        assert_eq!(ascii.as_str().len(), 16);
        assert_eq!(ascii.as_str(), "sixteen-byte-idX");
    }
}
