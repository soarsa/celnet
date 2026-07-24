//! The bundled **US-Treasury** reference universe: parses the embedded securities
//! master into government [`GovBondSpec`] records tagged `region = us`,
//! `sub_asset_type = government`.
//!
//! The document is embedded with [`include_str!`] so both the server registry seed
//! and the LP simulator are self-contained — they need no sidecar file on the deploy
//! host. It is a fixed reference snapshot (a securities master), so there is no
//! freshness argument for a runtime read.
//!
//! The instrument identity is the **CUSIP** (what the LP-SIM feed streams), with the
//! check-valid ISIN carried as a cross-reference — so a registry entry seeded from a
//! record here resolves the exact `instrument_id` the feed puts on the wire.

use crate::isin;
use crate::model::{CivilYmd, GovBondSpec};

/// The committed Treasury securities-master snapshot, embedded so downstream crates
/// are self-contained (see the module docs). This is the single canonical copy in
/// the workspace; the LP simulator re-exports it rather than bundling its own.
pub const TREASURY_UNIVERSE_JSON: &str = include_str!("../data/treasury-universe.json");

/// The raw record shape of one securities-master entry — only the fields the
/// reference generator needs; serde ignores the rest.
#[derive(Debug, serde::Deserialize)]
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
}

/// Parse the embedded universe into US-Treasury [`GovBondSpec`]s, in file order
/// (deterministic). Records with a malformed ISIN, an unparseable maturity, or an
/// empty CUSIP are dropped (data-quality filters).
#[must_use]
pub fn treasury_universe() -> Vec<GovBondSpec> {
    parse(TREASURY_UNIVERSE_JSON).unwrap_or_default()
}

/// Parse a securities-master JSON document into US-Treasury [`GovBondSpec`]s.
///
/// # Errors
/// Returns the [`serde_json::Error`] if `json` is not a JSON array of records.
/// Individual records that fail validation are dropped, not errored.
pub fn parse(json: &str) -> Result<Vec<GovBondSpec>, serde_json::Error> {
    let raw: Vec<RawRecord> = serde_json::from_str(json)?;
    Ok(raw.into_iter().filter_map(into_spec).collect())
}

/// Promote a raw record to a validated US [`GovBondSpec`], or `None` if it is not a
/// well-formed security.
fn into_spec(r: RawRecord) -> Option<GovBondSpec> {
    if r.cusip.trim().is_empty() {
        return None;
    }
    if !isin::is_well_formed(&r.isin) {
        return None;
    }
    let maturity_date = parse_civil_date(&r.maturity_date)?;
    let dated_date = parse_civil_date(&r.dated_date);

    // A coupon is stored as a percent (e.g. 4.125 => 0.04125); an absent coupon or a
    // Bill is a zero-coupon security.
    let coupon_pct = r.coupon_rate.unwrap_or(0.0);
    if !coupon_pct.is_finite() {
        return None;
    }
    let coupon_rate = coupon_pct / 100.0;
    let is_bill = r.security_type.to_ascii_uppercase().contains("BILL");
    let (coupon_type, coupon_frequency) = if is_bill || coupon_rate <= 0.0 {
        ("zero", "")
    } else {
        ("fixed", frequency_label(r.interest_payment_frequency.as_deref()))
    };

    let name = us_name(&r.security_term, coupon_rate, coupon_type, maturity_date);

    Some(GovBondSpec {
        instrument_id: r.cusip.clone(),
        name,
        issuer: "US Treasury".to_string(),
        currency: "USD",
        region: "us",
        sub_asset_type: "government",
        isin: r.isin,
        cusip: Some(r.cusip),
        coupon_rate,
        coupon_type,
        coupon_frequency,
        day_count: "act_act",
        dated_date,
        maturity_date,
        redemption: 100.0,
        calendars: vec!["united_states"],
    })
}

/// Map a securities-master `interestPaymentFrequency` token onto a payment-frequency
/// label, defaulting a coupon-bearing security with no recognised token to the
/// Treasury standard of semi-annual.
fn frequency_label(token: Option<&str>) -> &'static str {
    match token.map(|s| s.to_ascii_uppercase()) {
        Some(s) if s.contains("SEMI") => "semi_annual",
        Some(s) if s.contains("QUART") => "quarterly",
        Some(s) if s.contains("ANNUAL") => "annual",
        _ => "semi_annual",
    }
}

/// Build a compact, readable US name, e.g. `US 10Y GOV 4.125% May-36` (or
/// `US 4W GOV 0% May-26` for a Bill).
fn us_name(term: &str, coupon_rate: f64, coupon_type: &str, maturity: CivilYmd) -> String {
    let tenor = compact_term(term);
    let cpn = if coupon_type == "zero" {
        "0%".to_string()
    } else {
        format!("{}%", trim_pct(coupon_rate * 100.0))
    };
    format!("US {tenor} GOV {cpn} {}", maturity.month_year_label())
}

/// Compact an auction term (`10-Year`, `4-Week`, `19-Year 10-Month`) to `10Y`,
/// `4W`, `19Y10M`.
fn compact_term(term: &str) -> String {
    let mut out = term.replace("-Year", "Y");
    out = out.replace("-Week", "W");
    out = out.replace("-Month", "M");
    out.retain(|c| !c.is_whitespace());
    if out.is_empty() {
        "GOV".to_string()
    } else {
        out
    }
}

/// Format a percentage with up to three decimals, trimming trailing zeros
/// (`4.125` -> `4.125`, `4.000` -> `4`, `4.500` -> `4.5`).
fn trim_pct(pct: f64) -> String {
    let s = format!("{pct:.3}");
    let s = s.trim_end_matches('0');
    s.trim_end_matches('.').to_string()
}

/// Parse a civil date in ISO (`YYYY-MM-DD`) or US (`MM/DD/YYYY`) form, validated
/// against the real Gregorian calendar. Returns `None` for a blank or unparseable
/// token.
fn parse_civil_date(s: &str) -> Option<CivilYmd> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (y, m, d): (i32, u32, u32) = if let Some((y, rest)) = s.split_once('-') {
        let (mo, day) = rest.split_once('-')?;
        (y.parse().ok()?, mo.parse().ok()?, day.parse().ok()?)
    } else if let Some((mo, rest)) = s.split_once('/') {
        let (day, y) = rest.split_once('/')?;
        (y.parse().ok()?, mo.parse().ok()?, day.parse().ok()?)
    } else {
        return None;
    };
    let civil = CivilYmd::new(y, m, d);
    civil.is_valid().then_some(civil)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_substantial_us_universe() {
        let specs = treasury_universe();
        assert!(
            specs.len() >= 200,
            "expected the full securities master, got {}",
            specs.len()
        );
        for s in &specs {
            assert_eq!(s.region, "us");
            assert_eq!(s.sub_asset_type, "government");
            assert_eq!(s.currency, "USD");
            assert!(isin::is_well_formed(&s.isin), "bad ISIN {}", s.isin);
            assert_eq!(s.cusip.as_deref(), Some(s.instrument_id.as_str()));
            assert!(s.maturity_date.is_valid());
            if s.coupon_type == "zero" {
                assert_eq!(s.coupon_frequency, "");
                assert_eq!(s.coupon_rate, 0.0);
            } else {
                assert!(!s.coupon_frequency.is_empty());
                assert!(s.coupon_rate > 0.0);
            }
        }
    }

    #[test]
    fn instrument_ids_are_unique() {
        let specs = treasury_universe();
        let ids: std::collections::HashSet<&str> =
            specs.iter().map(|s| s.instrument_id.as_str()).collect();
        assert_eq!(ids.len(), specs.len(), "CUSIP instrument_ids must be unique");
    }

    #[test]
    fn compacts_terms_and_trims_coupons() {
        assert_eq!(compact_term("10-Year"), "10Y");
        assert_eq!(compact_term("4-Week"), "4W");
        assert_eq!(compact_term("19-Year 10-Month"), "19Y10M");
        assert_eq!(trim_pct(4.000), "4");
        assert_eq!(trim_pct(4.125), "4.125");
        assert_eq!(trim_pct(4.500), "4.5");
    }
}
