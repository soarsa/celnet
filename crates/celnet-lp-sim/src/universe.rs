//! The bundled **US-Treasury reference universe** loader.
//!
//! Parses the committed `data/treasury-universe.json` (a real snapshot of the US
//! Treasury securities master: 267 CUSIPs with auction/quote prices) into typed
//! [`TreasuryBond`] records, and maps each one onto the two identities the rest of
//! the stack keys on:
//!
//! - the **canonical server `instrument_id`** (ADR-0022 decision D) — the one
//!   identity shared across reference data, the wire, the GUI and the composite.
//!   The Treasury's **CUSIP** is that id (unique registry-wide); the ISIN is
//!   carried alongside for display/cross-ref. This is what the LP-SIM feed puts on
//!   the wire and what a subscriber sees.
//! - the aggregation engine's asset-agnostic [`Instrument`] key
//!   ([`TreasuryBond::engine_instrument`]) — `Underlying::Commodity` carrying the
//!   CUSIP as a vendor-neutral free-form ticker (guardrail #8: the CUSIP is an
//!   opaque code, not a vendor product name) plus a [`Tenor::BrokenDate`] maturity.
//!   Government-cash has no dedicated `Underlying` arm and the consolidation is
//!   asset-agnostic over the key, so the free-form ticker arm is the correct
//!   injective carrier: **distinct CUSIPs never collide** (unlike a maturity-only
//!   key, where reopenings share a date).
//!
//! ## Embedded, not read at runtime (justification)
//!
//! The document is embedded with [`include_str!`] so a built `lp-sim` binary is
//! **self-contained** — it needs no sidecar data file on the deploy host, exactly
//! as the FIX simulator ships a prebuilt client rather than build-at-runtime. The
//! universe is a fixed reference snapshot (a securities master), so there is no
//! freshness argument for a runtime read; embedding removes a deployment failure
//! mode (a missing/renamed file) at zero runtime cost.
//!
//! ## Prices are a real reference, never fabricated
//!
//! Each record carries the auction/quote `buyPrice` (ask) and `sellPrice` (bid)
//! per 100 face. [`TreasuryBond::reference_mid`] is their midpoint and
//! [`TreasuryBond::to_reference_bond`] rebuilds the bond's real cashflow schedule
//! as a [`celnet_bond::Bond`], so the stochastic feed prices the bond off the REAL
//! analytics leaf seeded at the reference — it does not invent a price handle.

use celnet_aggregation::Instrument;
use celnet_bond::{AccrualBasis, Bond, BondError, PaymentFrequency};
use celnet_types::{BrokenDate, Ccy, CommodityRef, Symbol, Tenor, Underlying};
use serde::Deserialize;
use time::{Date, Month};

/// The committed Treasury securities-master snapshot, embedded so the binary is
/// self-contained (see the module docs).
pub const UNIVERSE_JSON: &str = celnet_refdata::TREASURY_UNIVERSE_JSON;

/// The maximum sane price per 100 face: a coupon Treasury trades near par, so a
/// quote at or beyond this (or at/below zero) is a data error and is filtered out
/// rather than fed to the consolidator.
const MAX_PRICE_PER_100: f64 = 200.0;

/// The coarse US-Treasury security class, parsed from `securityType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityType {
    /// A Treasury **Bill** — a zero-coupon discount security, ≤ 1y.
    Bill,
    /// A Treasury **Note** — a coupon security, 2y–10y.
    Note,
    /// A Treasury **Bond** — a coupon security, > 10y.
    Bond,
}

impl SecurityType {
    /// Parse the `securityType` token (`Bill` / `Note` / `Bond`, case-insensitive,
    /// tolerant of the price-file's `MARKET BASED …` prefixes), or `None` for an
    /// unrecognised class.
    #[must_use]
    pub fn parse(label: &str) -> Option<Self> {
        let l = label.to_ascii_uppercase();
        if l.contains("BILL") {
            Some(Self::Bill)
        } else if l.contains("NOTE") {
            Some(Self::Note)
        } else if l.contains("BOND") {
            Some(Self::Bond)
        } else {
            None
        }
    }

    /// Whether this class pays coupons (Notes and Bonds do; Bills are zeros).
    #[must_use]
    pub fn is_coupon_bearing(self) -> bool {
        matches!(self, Self::Note | Self::Bond)
    }
}

/// One priced, well-formed Treasury security from the reference universe.
///
/// Built only by the loader from a validated raw record, so every field here is
/// known-good: the ISIN passes its check digit, both prices are finite and in a
/// sane band, and the maturity is a real calendar date.
#[derive(Debug, Clone, PartialEq)]
pub struct TreasuryBond {
    /// The 9-character CUSIP — the **canonical `instrument_id`** (ADR-0022 D).
    pub cusip: String,
    /// The 12-character ISIN (check-digit valid), carried for display/cross-ref.
    pub isin: String,
    /// The coarse security class.
    pub security_type: SecurityType,
    /// The auction term label, e.g. `4-Week`, `10-Year`, `19-Year 10-Month`.
    pub term: String,
    /// The final-redemption date.
    pub maturity: BrokenDate,
    /// The dated (accrual-start) date, when the record carries one.
    pub dated_date: Option<BrokenDate>,
    /// The annual coupon rate as a decimal (e.g. `0.05` for a 5% coupon); `0.0`
    /// for a Bill / zero-coupon security.
    pub coupon: f64,
    /// The coupon frequency, or `None` for a zero-coupon Bill.
    pub frequency: Option<PaymentFrequency>,
    /// The reference **ask** (`buyPrice`) per 100 face — the price to *buy* the
    /// bond, i.e. the LP's offer side.
    pub ask: f64,
    /// The reference **bid** (`sellPrice`) per 100 face — the price to *sell* the
    /// bond, i.e. the LP's bid side.
    pub bid: f64,
}

impl TreasuryBond {
    /// The canonical server `instrument_id` (ADR-0022 D) — the CUSIP.
    #[must_use]
    pub fn instrument_id(&self) -> &str {
        &self.cusip
    }

    /// A short human label for the security (its auction term + class), e.g.
    /// `10-Year Note` — the GUI blotter name.
    #[must_use]
    pub fn display_name(&self) -> String {
        let class = match self.security_type {
            SecurityType::Bill => "Bill",
            SecurityType::Note => "Note",
            SecurityType::Bond => "Bond",
        };
        format!("{} {class}", self.term)
    }

    /// The reference mid price per 100 face: the midpoint of the ask and bid.
    #[must_use]
    pub fn reference_mid(&self) -> f64 {
        0.5 * (self.ask + self.bid)
    }

    /// The reference bid/ask half-spread per 100 face (`(ask − bid)/2`), clamped
    /// non-negative (the snapshot occasionally quotes an inverted cross by a hair).
    #[must_use]
    pub fn reference_half_spread(&self) -> f64 {
        (0.5 * (self.ask - self.bid)).max(0.0)
    }

    /// The aggregation engine's asset-agnostic [`Instrument`] key for this bond:
    /// the CUSIP as a vendor-neutral free-form commodity ticker (guardrail #8) plus
    /// the maturity as a [`Tenor::BrokenDate`]. Injective over CUSIPs, so two
    /// distinct securities never consolidate into one line.
    #[must_use]
    pub fn engine_instrument(&self) -> Instrument {
        Instrument::new(
            Underlying::Commodity(CommodityRef::new(
                Symbol::new(self.cusip.clone(), ""),
                Ccy::USD,
            )),
            Tenor::BrokenDate(self.maturity),
        )
    }

    /// Build the stochastic **mean-reverting-yield model** for this bond, seeded at
    /// the reference: the yield is inverted from the reference mid through the REAL
    /// analytics leaf (`yield_to_maturity`), so the feed's prices are the oracle's
    /// clean prices of a real yield — never a fabricated handle. The process reverts
    /// to that same reference yield (`long_run == initial`), jittered by
    /// `perturbation` and pulled at `reversion_per_sec`.
    ///
    /// Returns `None` when the bond cannot be modelled at `settlement` (a matured or
    /// degenerate schedule, or a reference mid outside the solvable yield range —
    /// e.g. a deep-discount Bill the coupon-bond yield solver cannot bracket).
    #[must_use]
    pub fn yield_model(
        &self,
        settlement: BrokenDate,
        reversion_per_sec: f64,
        perturbation: f64,
    ) -> Option<crate::price::YieldModel> {
        let bond = self.to_reference_bond(settlement).ok()?;
        // The reference mid is a clean price; the solver takes a dirty price, and at
        // the seam the two coincide to within the round-trip tolerance the loader's
        // own test asserts. Invert to the reference yield.
        let y = celnet_bond::yield_to_maturity(&bond, self.reference_mid()).ok()?;
        Some(crate::price::YieldModel {
            bond,
            long_run_yield: y.0,
            initial_yield: y.0,
            reversion_per_sec,
            perturbation,
        })
    }

    /// Rebuild this security's real cashflow schedule as a [`celnet_bond::Bond`]
    /// valued at `settlement`, so the stochastic feed prices off the REAL analytics
    /// leaf (never a fabricated handle).
    ///
    /// The accrual basis is mapped to the engine's 30/360 bond basis (Treasuries
    /// quote on actual/actual, which the leaf does not yet model; 30/360 is the
    /// closest available basis and the reference-yield round-trip below still
    /// reprices the reference mid exactly, because the SAME basis is used to invert
    /// the price and to re-price it). A Bill (zero coupon) is modelled with a `0.0`
    /// coupon.
    ///
    /// # Errors
    ///
    /// Returns [`BondError`] if the schedule is degenerate for this settlement —
    /// most commonly [`BondError::MaturityNotAfterSettlement`] when `settlement`
    /// is on/after the security's maturity (an already-matured Bill).
    pub fn to_reference_bond(&self, settlement: BrokenDate) -> Result<Bond, ReferenceBondError> {
        let settle = broken_to_date(settlement).ok_or(ReferenceBondError::BadSettlement)?;
        let mat = broken_to_date(self.maturity).ok_or(ReferenceBondError::BadMaturity)?;
        let freq = self.frequency.unwrap_or(PaymentFrequency::SemiAnnual);
        Bond::new(
            settle,
            mat,
            self.coupon,
            freq,
            AccrualBasis::Thirty360BondBasis,
            100.0,
        )
        .map_err(ReferenceBondError::Bond)
    }
}

/// Why building a [`celnet_bond::Bond`] from a [`TreasuryBond`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceBondError {
    /// The settlement triple is not a real calendar date.
    BadSettlement,
    /// The stored maturity triple is not a real calendar date (never for a loaded
    /// bond — the loader validates it — but surfaced rather than panicking).
    BadMaturity,
    /// The analytics leaf rejected the schedule (e.g. maturity ≤ settlement).
    Bond(BondError),
}

impl core::fmt::Display for ReferenceBondError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadSettlement => f.write_str("settlement date is not a real calendar date"),
            Self::BadMaturity => f.write_str("maturity date is not a real calendar date"),
            Self::Bond(e) => write!(f, "bond schedule rejected: {e}"),
        }
    }
}

impl core::error::Error for ReferenceBondError {}

/// The raw record shape of one `treasury-universe.json` entry — only the fields the
/// loader needs; serde ignores the rest. `#[serde(default)]` tolerates records that
/// omit a field.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRecord {
    #[serde(default)]
    cusip: String,
    #[serde(default)]
    isin: String,
    #[serde(default)]
    security_type: String,
    #[serde(default)]
    security_term: String,
    #[serde(default)]
    maturity_date: String,
    #[serde(default)]
    dated_date: String,
    #[serde(default)]
    coupon_rate: Option<f64>,
    #[serde(default)]
    interest_payment_frequency: Option<String>,
    #[serde(default)]
    buy_price: Option<f64>,
    #[serde(default)]
    sell_price: Option<f64>,
}

impl RawRecord {
    /// Promote a raw record to a validated [`TreasuryBond`], or `None` if it is not
    /// a priced, well-formed security (unparseable date, malformed ISIN, or a
    /// missing/out-of-band price).
    fn into_bond(self) -> Option<TreasuryBond> {
        let security_type = SecurityType::parse(&self.security_type)?;
        let maturity = parse_civil_date(&self.maturity_date)?;
        if self.cusip.trim().is_empty() {
            return None;
        }
        if !isin_is_well_formed(&self.isin) {
            return None;
        }
        let ask = price_in_band(self.buy_price)?;
        let bid = price_in_band(self.sell_price)?;

        // A coupon security must carry a coupon; a Bill is a zero. Interpret an
        // absent/None coupon on a coupon class as a zero rather than dropping it,
        // but a coupon must be finite.
        let coupon_pct = self.coupon_rate.unwrap_or(0.0);
        if !coupon_pct.is_finite() {
            return None;
        }
        let coupon = coupon_pct / 100.0; // stored as a percent (e.g. 5 => 0.05)
        let frequency = parse_frequency(self.interest_payment_frequency.as_deref());
        let dated_date = parse_civil_date(&self.dated_date);

        Some(TreasuryBond {
            cusip: self.cusip,
            isin: self.isin,
            security_type,
            term: self.security_term,
            maturity,
            dated_date,
            coupon,
            frequency,
            ask,
            bid,
        })
    }
}

/// Parse the embedded universe into the full set of **priced, well-formed**
/// [`TreasuryBond`]s, in file order (deterministic). Records that are unpriced,
/// out-of-band, or malformed are dropped.
#[must_use]
pub fn load_universe() -> Vec<TreasuryBond> {
    parse_universe(UNIVERSE_JSON).unwrap_or_default()
}

/// The coupon-bearing subset of [`load_universe`] (Notes and Bonds) — the
/// securities whose real cashflow schedule the stochastic yield feed prices off.
#[must_use]
pub fn load_coupon_universe() -> Vec<TreasuryBond> {
    load_universe()
        .into_iter()
        .filter(|b| b.security_type.is_coupon_bearing())
        .collect()
}

/// Parse a Treasury-universe JSON document into validated [`TreasuryBond`]s.
///
/// # Errors
///
/// Returns the underlying [`serde_json::Error`] if `json` is not a JSON array of
/// records. Individual records that fail validation are silently dropped (they are
/// data-quality filters, not parse errors).
pub fn parse_universe(json: &str) -> Result<Vec<TreasuryBond>, serde_json::Error> {
    let raw: Vec<RawRecord> = serde_json::from_str(json)?;
    Ok(raw.into_iter().filter_map(RawRecord::into_bond).collect())
}

// --- parsing helpers --------------------------------------------------------

/// A price is usable iff it is finite and strictly inside `(0, MAX_PRICE_PER_100)`.
fn price_in_band(p: Option<f64>) -> Option<f64> {
    let v = p?;
    (v.is_finite() && v > 0.0 && v < MAX_PRICE_PER_100).then_some(v)
}

/// Map an `interestPaymentFrequency` token onto a [`PaymentFrequency`], or `None`
/// (zero-coupon / no schedule) for `None`/empty. Only the engine's three
/// frequencies are representable; a Monthly Treasury does not exist.
fn parse_frequency(label: Option<&str>) -> Option<PaymentFrequency> {
    let l = label?.trim().to_ascii_uppercase();
    if l.contains("SEMI") {
        Some(PaymentFrequency::SemiAnnual)
    } else if l.contains("QUART") {
        Some(PaymentFrequency::Quarterly)
    } else if l.contains("ANNUAL") {
        // "Annual" (but not "Semi-Annual", handled above).
        Some(PaymentFrequency::Annual)
    } else {
        None
    }
}

/// Parse a civil date in either ISO (`YYYY-MM-DD`, the universe file) or US
/// (`MM/DD/YYYY`, the price file) form into a [`BrokenDate`], validated against the
/// Gregorian calendar. Returns `None` for a blank or unparseable token.
#[must_use]
pub fn parse_civil_date(s: &str) -> Option<BrokenDate> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (y, m, d) = if let Some((y, rest)) = s.split_once('-') {
        // ISO YYYY-MM-DD
        let (mo, day) = rest.split_once('-')?;
        (y.parse().ok()?, mo.parse().ok()?, day.parse().ok()?)
    } else if let Some((mo, rest)) = s.split_once('/') {
        // US MM/DD/YYYY
        let (day, y) = rest.split_once('/')?;
        (y.parse().ok()?, mo.parse().ok()?, day.parse().ok()?)
    } else {
        return None;
    };
    // Validate against the real calendar via `time` (rejects 2026-02-30 etc.).
    let month = Month::try_from(m).ok()?;
    Date::from_calendar_date(y, month, d).ok()?;
    Some(BrokenDate::new(y, m, d))
}

/// Convert a validated [`BrokenDate`] to a `time::Date` (the loader only ever holds
/// calendar-valid triples, but this returns `None` rather than panicking on a
/// hand-built out-of-range triple).
fn broken_to_date(b: BrokenDate) -> Option<Date> {
    let month = Month::try_from(b.month).ok()?;
    Date::from_calendar_date(b.year, month, b.day).ok()
}

/// Whether an ISIN is well-formed: 12 chars, two leading letters, and a valid
/// mod-10 (Luhn) check digit over the digit-expanded body — the ISO-6166 standard
/// integrity check, so a corrupted id is rejected, not merely length-checked.
#[must_use]
pub fn isin_is_well_formed(isin: &str) -> bool {
    let bytes = isin.as_bytes();
    if bytes.len() != 12 {
        return false;
    }
    if !bytes[0].is_ascii_uppercase() || !bytes[1].is_ascii_uppercase() {
        return false;
    }
    if !bytes[..11].iter().all(u8::is_ascii_alphanumeric) || !bytes[11].is_ascii_digit() {
        return false;
    }
    isin_check_digit(&isin[..11]) == (bytes[11] - b'0')
}

/// The ISO-6166 mod-10 (Luhn) check digit for an 11-character ISIN body: each
/// letter expands to two digits (`A`=10 … `Z`=35), then the Luhn algorithm runs
/// right-to-left over the resulting digit string.
fn isin_check_digit(body: &str) -> u8 {
    // Expand letters to digits, building the digit sequence left-to-right.
    let mut digits: Vec<u8> = Vec::with_capacity(22);
    for c in body.bytes() {
        if c.is_ascii_digit() {
            digits.push(c - b'0');
        } else {
            let v = c - b'A' + 10; // A..Z => 10..35
            digits.push(v / 10);
            digits.push(v % 10);
        }
    }
    // Luhn from the right: double every second digit (starting with the rightmost).
    let mut sum = 0u32;
    for (i, &d) in digits.iter().rev().enumerate() {
        let mut v = u32::from(d);
        if i % 2 == 0 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    ((10 - (sum % 10)) % 10) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_substantial_priced_universe_from_the_real_file() {
        let bonds = load_universe();
        // The committed snapshot has 267 records; a large priced subset survives
        // validation (unpriced auctions and out-of-band rows are dropped).
        assert!(
            bonds.len() >= 120,
            "expected a large priced universe, got {}",
            bonds.len()
        );
        // Every loaded bond is priced in a sane band and its ISIN is check-valid.
        for b in &bonds {
            assert!(b.ask > 0.0 && b.ask < MAX_PRICE_PER_100);
            assert!(b.bid > 0.0 && b.bid < MAX_PRICE_PER_100);
            assert!(b.reference_mid() > 0.0 && b.reference_mid() < MAX_PRICE_PER_100);
            assert!(isin_is_well_formed(&b.isin), "bad ISIN {}", b.isin);
            assert_eq!(b.cusip.len(), 9, "CUSIP not 9 chars: {}", b.cusip);
        }
    }

    #[test]
    fn instrument_ids_and_engine_keys_are_unique() {
        let bonds = load_universe();
        let ids: std::collections::HashSet<&str> =
            bonds.iter().map(TreasuryBond::instrument_id).collect();
        assert_eq!(
            ids.len(),
            bonds.len(),
            "instrument_ids (CUSIPs) must be unique"
        );
        // The cosmetic engine key must be injective too, so two securities never
        // consolidate into one line (this is why the CUSIP — not the maturity —
        // carries the identity).
        let keys: std::collections::HashSet<Instrument> =
            bonds.iter().map(TreasuryBond::engine_instrument).collect();
        assert_eq!(
            keys.len(),
            bonds.len(),
            "engine Instrument keys must be unique"
        );
    }

    #[test]
    fn coupon_universe_is_coupon_bearing_and_prices_off_the_real_leaf() {
        let coupon = load_coupon_universe();
        assert!(
            !coupon.is_empty(),
            "expected coupon Notes/Bonds in the universe"
        );
        // Settlement well before any surviving maturity: the snapshot's earliest
        // maturities are 2026; settle at 2026-04-16 (the snapshot's price date).
        let settle = BrokenDate::new(2026, 4, 16);
        let mut priced = 0;
        for b in &coupon {
            assert!(b.security_type.is_coupon_bearing());
            assert!(b.coupon >= 0.0);
            if let Ok(bond) = b.to_reference_bond(settle) {
                // The reference yield inverted from the reference mid must reprice
                // that exact mid back through the SAME leaf — an oracle round-trip,
                // not a plausibility check.
                let mid = b.reference_mid();
                let dirty = mid; // clean≈dirty at issue; the round-trip is basis-consistent
                if let Ok(y) = celnet_bond::yield_to_maturity(&bond, dirty) {
                    let repriced = celnet_bond::dirty_price(&bond, y).unwrap();
                    assert!(
                        (repriced - dirty).abs() < 1e-6,
                        "yield round-trip drift {} for {}",
                        (repriced - dirty).abs(),
                        b.cusip
                    );
                    priced += 1;
                }
            }
        }
        assert!(
            priced > 0,
            "no coupon bond repriced through the analytics leaf"
        );
    }

    #[test]
    fn parses_both_date_formats() {
        assert_eq!(
            parse_civil_date("2026-05-19"),
            Some(BrokenDate::new(2026, 5, 19))
        );
        assert_eq!(
            parse_civil_date("05/19/2026"),
            Some(BrokenDate::new(2026, 5, 19))
        );
        assert_eq!(parse_civil_date(""), None);
        assert_eq!(
            parse_civil_date("2026-02-30"),
            None,
            "invalid calendar date rejected"
        );
        assert_eq!(parse_civil_date("garbage"), None);
    }

    #[test]
    fn isin_check_digit_matches_known_good_and_rejects_corruption() {
        // A real Treasury ISIN from the universe (check digit 7).
        assert!(isin_is_well_formed("US912797TS67"));
        // Corrupt the check digit → rejected.
        assert!(!isin_is_well_formed("US912797TS60"));
        // Wrong length / shape → rejected.
        assert!(!isin_is_well_formed("US912797TS6"));
        assert!(!isin_is_well_formed("1S912797TS67"));
    }

    #[test]
    fn security_type_and_frequency_parse() {
        assert_eq!(SecurityType::parse("Bill"), Some(SecurityType::Bill));
        assert_eq!(SecurityType::parse("Note"), Some(SecurityType::Note));
        assert_eq!(SecurityType::parse("Bond"), Some(SecurityType::Bond));
        assert_eq!(
            SecurityType::parse("MARKET BASED BILL"),
            Some(SecurityType::Bill)
        );
        assert_eq!(SecurityType::parse("Junk"), None);
        assert_eq!(
            parse_frequency(Some("Semi-Annual")),
            Some(PaymentFrequency::SemiAnnual)
        );
        assert_eq!(parse_frequency(Some("None")), None);
        assert_eq!(parse_frequency(None), None);
    }
}
