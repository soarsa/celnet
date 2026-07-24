//! The curated **non-US** on-the-run government curves: UK gilts and the EUR govvies
//! (German Bunds, French OATs, Italian BTPs).
//!
//! Each is a fully-specified [`GovBondSpec`] with real market conventions — UK gilts
//! semi-annual act/act (GBP), EUR govvies annual act/act (EUR) — and an ISO-6166
//! check-valid ISIN minted by [`crate::isin::build`]. The coupons and maturities are
//! a representative, deterministic on-the-run curve anchored to a fixed reference
//! year (no `Date::now`, no runtime randomness), so the generated universe is
//! byte-identical on every build. These are curated sample identifiers, not vendor
//! product codes (guardrail: vendor-neutral, purpose-named).

use crate::isin;
use crate::model::{CivilYmd, GovBondSpec};

/// The fixed reference year the curve is anchored to: an `NY`-tenor bond matures in
/// `REFERENCE_YEAR + N`. Deterministic — the universe never depends on the wall clock.
const REFERENCE_YEAR: i32 = 2026;

/// The static shape of one issuer's curve: the identity/convention metadata plus the
/// `(tenor_years, coupon_percent)` on-the-run points.
struct IssuerCurve {
    /// The `instrument_id` slug prefix (e.g. `uk-gilt`).
    id_prefix: &'static str,
    /// The name prefix / region tag shown in the blotter name (e.g. `UK`).
    name_region: &'static str,
    /// The product word in the name (e.g. `GILT`, `BUND`, `OAT`, `BTP`).
    product: &'static str,
    /// The 4-character NSIN mnemonic used to mint the ISIN body (e.g. `GILT`).
    nsin_mnemonic: &'static str,
    /// The ISO 3166 country prefix for the ISIN (e.g. `GB`, `DE`, `FR`, `IT`).
    country: &'static str,
    /// The issuer name.
    issuer: &'static str,
    /// ISO 4217 currency label.
    currency: &'static str,
    /// The region label (`uk` / `de` / `fr` / `it`).
    region: &'static str,
    /// The coupon frequency label (`semi_annual` for gilts, `annual` for EUR govvies).
    frequency: &'static str,
    /// The settlement calendar-centre label.
    calendar: &'static str,
    /// The coupon anchor `(month, day)` — the maturity/coupon date within the year.
    anchor: (u32, u32),
    /// The on-the-run points: `(tenor_years, coupon_percent)`.
    points: &'static [(i32, f64)],
}

/// UK gilts — GBP, semi-annual, act/act; issued by the UK Debt Management Office.
const UK_GILTS: IssuerCurve = IssuerCurve {
    id_prefix: "uk-gilt",
    name_region: "UK",
    product: "GILT",
    nsin_mnemonic: "GILT",
    country: "GB",
    issuer: "UK DMO",
    currency: "GBP",
    region: "uk",
    frequency: "semi_annual",
    calendar: "united_kingdom",
    anchor: (6, 7),
    points: &[
        (2, 4.250),
        (3, 4.200),
        (5, 4.150),
        (7, 4.200),
        (10, 4.250),
        (15, 4.500),
        (20, 4.750),
        (30, 4.750),
        (40, 4.625),
        (50, 4.500),
    ],
};

/// German Bunds (incl. the Schatz/Bobl short end) — EUR, annual, act/act.
const DE_BUNDS: IssuerCurve = IssuerCurve {
    id_prefix: "de-bund",
    name_region: "DE",
    product: "BUND",
    nsin_mnemonic: "BUND",
    country: "DE",
    issuer: "Bundesrepublik Deutschland",
    currency: "EUR",
    region: "de",
    frequency: "annual",
    calendar: "target2",
    anchor: (2, 15),
    points: &[
        (2, 2.100),
        (3, 2.200),
        (5, 2.300),
        (7, 2.450),
        (10, 2.550),
        (15, 2.750),
        (20, 2.850),
        (30, 2.900),
    ],
};

/// French OATs — EUR, annual, act/act.
const FR_OATS: IssuerCurve = IssuerCurve {
    id_prefix: "fr-oat",
    name_region: "FR",
    product: "OAT",
    nsin_mnemonic: "OATT",
    country: "FR",
    issuer: "Republique Francaise",
    currency: "EUR",
    region: "fr",
    frequency: "annual",
    calendar: "target2",
    anchor: (5, 25),
    points: &[
        (2, 2.600),
        (5, 2.850),
        (10, 3.150),
        (15, 3.400),
        (20, 3.550),
        (30, 3.650),
    ],
};

/// Italian BTPs — EUR, annual, act/act.
const IT_BTPS: IssuerCurve = IssuerCurve {
    id_prefix: "it-btp",
    name_region: "IT",
    product: "BTP",
    nsin_mnemonic: "BTPP",
    country: "IT",
    issuer: "Repubblica Italiana",
    currency: "EUR",
    region: "it",
    frequency: "annual",
    calendar: "target2",
    anchor: (3, 1),
    points: &[
        (2, 3.100),
        (5, 3.500),
        (10, 3.900),
        (15, 4.200),
        (20, 4.400),
        (30, 4.550),
    ],
};

/// The curated non-US government universe (UK + DE + FR + IT), in a stable order.
#[must_use]
pub fn curated_universe() -> Vec<GovBondSpec> {
    [&UK_GILTS, &DE_BUNDS, &FR_OATS, &IT_BTPS]
        .into_iter()
        .flat_map(build_curve)
        .collect()
}

/// Materialise one issuer's curve into specs.
fn build_curve(c: &IssuerCurve) -> Vec<GovBondSpec> {
    c.points
        .iter()
        .map(|&(tenor, coupon_pct)| build_point(c, tenor, coupon_pct))
        .collect()
}

/// Build one on-the-run point into a fully-specified [`GovBondSpec`].
fn build_point(c: &IssuerCurve, tenor: i32, coupon_pct: f64) -> GovBondSpec {
    let maturity_year = REFERENCE_YEAR + tenor;
    let (month, day) = c.anchor;
    let maturity_date = CivilYmd::new(maturity_year, month, day);
    let dated_date = CivilYmd::new(REFERENCE_YEAR, month, day);

    // The 9-character NSIN: a 4-char mnemonic + zero-padded tenor + `Y` + the
    // 2-digit maturity year (e.g. `GILT10Y36`), unique per issuer point. The ISIN is
    // the country prefix + NSIN + the computed ISO-6166 check digit.
    let nsin = format!(
        "{}{:02}Y{:02}",
        c.nsin_mnemonic,
        tenor,
        maturity_year.rem_euclid(100)
    );
    let body = format!("{}{nsin}", c.country);
    let isin = isin::build(&body).expect("curated ISIN body is well-formed by construction");

    let coupon_rate = coupon_pct / 100.0;
    let name = format!(
        "{} {tenor}Y {} {}% {}",
        c.name_region,
        c.product,
        trim_pct(coupon_pct),
        maturity_date.month_year_label()
    );

    GovBondSpec {
        instrument_id: format!("{}-{tenor}y-{maturity_year}", c.id_prefix),
        name,
        issuer: c.issuer.to_string(),
        currency: c.currency,
        region: c.region,
        sub_asset_type: "government",
        isin,
        cusip: None,
        coupon_rate,
        coupon_type: "fixed",
        coupon_frequency: c.frequency,
        day_count: "act_act",
        dated_date: Some(dated_date),
        maturity_date,
        redemption: 100.0,
        calendars: vec![c.calendar],
    }
}

/// Format a coupon percent with up to three decimals, trimming trailing zeros.
fn trim_pct(pct: f64) -> String {
    let s = format!("{pct:.3}");
    let s = s.trim_end_matches('0');
    s.trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curated_isin_is_check_valid() {
        for s in curated_universe() {
            assert!(
                isin::is_well_formed(&s.isin),
                "curated ISIN {} for {} failed the ISO-6166 check",
                s.isin,
                s.instrument_id
            );
            assert_eq!(s.isin.len(), 12);
            assert!(s.cusip.is_none(), "non-US govvies carry no CUSIP");
            assert_eq!(s.coupon_type, "fixed");
            assert!(!s.coupon_frequency.is_empty());
            assert!(s.maturity_date.is_valid());
        }
    }

    #[test]
    fn counts_and_regions_match_the_curated_curves() {
        let u = curated_universe();
        assert_eq!(u.len(), 10 + 8 + 6 + 6, "UK+DE+FR+IT point count");
        let count = |r: &str| u.iter().filter(|s| s.region == r).count();
        assert_eq!(count("uk"), 10);
        assert_eq!(count("de"), 8);
        assert_eq!(count("fr"), 6);
        assert_eq!(count("it"), 6);
    }

    #[test]
    fn instrument_ids_and_isins_are_unique() {
        let u = curated_universe();
        let ids: std::collections::HashSet<&str> =
            u.iter().map(|s| s.instrument_id.as_str()).collect();
        assert_eq!(ids.len(), u.len(), "slugs unique");
        let isins: std::collections::HashSet<&str> = u.iter().map(|s| s.isin.as_str()).collect();
        assert_eq!(isins.len(), u.len(), "ISINs unique");
    }
}
