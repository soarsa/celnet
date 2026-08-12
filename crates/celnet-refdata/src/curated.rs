//! The curated **non-US** cash-bond curves: the sovereigns (UK gilts, German Bunds,
//! French OATs, Italian BTPs) and the EUR **corporate** complex (industrial, utility,
//! telecom, financial).
//!
//! Each is a fully-specified [`GovBondSpec`] with real market conventions — UK gilts
//! semi-annual act/act (GBP), EUR govvies and EUR corporates annual act/act (EUR),
//! TARGET2 settlement — and an ISO-6166
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
    /// The sub-asset-type label — `government` for a sovereign curve, `corporate` for a
    /// credit issuer. Carried per-curve rather than hard-coded so a corporate is never
    /// silently advertised as government risk: this label is what the GUI filters on and
    /// what a risk book buckets by.
    sub_asset_type: &'static str,
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
    sub_asset_type: "government",
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
    sub_asset_type: "government",
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
    sub_asset_type: "government",
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
    sub_asset_type: "government",
    points: &[
        (2, 3.100),
        (5, 3.500),
        (10, 3.900),
        (15, 4.200),
        (20, 4.400),
        (30, 4.550),
    ],
};

// ---------------------------------------------------------------------------------
// EUR CORPORATE curves.
//
// Four European credit issuers spanning the sectors a EUR corporate desk actually runs
// (industrial, utility, telecom, financial), each an annual act/act EUR Eurobond
// settling TARGET2 — the same conventions as the EUR govvies above, which is what makes
// a corporate hedgeable against a Bund/OAT/BTP line without a convention adjustment.
//
// The coupons are the sovereign curve PLUS a plausible, sector-differentiated credit
// spread that widens with tenor (a utility inside a telecom inside the financial, and
// every one wide of Germany), so a book holding these carries genuine spread risk rather
// than a parallel copy of the govvie curve.
//
// The ISINs carry the `XS` prefix — the international (Euroclear/Clearstream) code EUR
// Eurobonds are issued under — and the check digit is computed, so they are well-formed.
// The ISSUER NAMES ARE DELIBERATELY SYNTHETIC. A real company name attached to invented
// terms would be a fabricated security record: it would look like a real bond, quote like
// one, and book like one, while none of its terms came from that issuer. Swapping in a
// vendor reference feed replaces these rows wholesale; nothing downstream is name-bound.
// ---------------------------------------------------------------------------------

/// EUR industrial credit — the tight end of the corporate complex.
const EU_CORP_INDUSTRIAL: IssuerCurve = IssuerCurve {
    id_prefix: "eu-corp-industrial",
    name_region: "EU",
    product: "CORP",
    nsin_mnemonic: "RHIN",
    country: "XS",
    issuer: "Rhine Industrials NV",
    currency: "EUR",
    region: "eu",
    frequency: "annual",
    calendar: "target2",
    anchor: (4, 12),
    sub_asset_type: "corporate",
    points: &[(2, 2.900), (3, 3.050), (5, 3.200), (7, 3.400), (10, 3.600)],
};

/// EUR utility credit — regulated cash flows, so inside the industrial at every tenor.
const EU_CORP_UTILITY: IssuerCurve = IssuerCurve {
    id_prefix: "eu-corp-utility",
    name_region: "EU",
    product: "CORP",
    nsin_mnemonic: "IBUT",
    country: "XS",
    issuer: "Iberia Utilities SA",
    currency: "EUR",
    region: "eu",
    frequency: "annual",
    calendar: "target2",
    anchor: (9, 30),
    sub_asset_type: "corporate",
    points: &[(2, 2.800), (3, 2.950), (5, 3.100), (7, 3.300), (10, 3.500)],
};

/// EUR telecom credit — the widest of the four: capex-heavy, BBB-band.
const EU_CORP_TELECOM: IssuerCurve = IssuerCurve {
    id_prefix: "eu-corp-telecom",
    name_region: "EU",
    product: "CORP",
    nsin_mnemonic: "NDTL",
    country: "XS",
    issuer: "Nordic Telecom AB",
    currency: "EUR",
    region: "eu",
    frequency: "annual",
    calendar: "target2",
    anchor: (11, 18),
    sub_asset_type: "corporate",
    points: &[(2, 3.250), (3, 3.450), (5, 3.700), (7, 3.950), (10, 4.200)],
};

/// EUR financial (senior preferred) — bank credit, wide of the industrial.
const EU_CORP_FINANCIAL: IssuerCurve = IssuerCurve {
    id_prefix: "eu-corp-financial",
    name_region: "EU",
    product: "CORP",
    nsin_mnemonic: "ALPF",
    country: "XS",
    issuer: "Alpine Financial SA",
    currency: "EUR",
    region: "eu",
    frequency: "annual",
    calendar: "target2",
    anchor: (1, 22),
    sub_asset_type: "corporate",
    points: &[(2, 3.050), (3, 3.200), (5, 3.400), (7, 3.650), (10, 3.900)],
};

/// The curated non-US universe, in a stable order: the government curves (UK + DE + FR +
/// IT) first, then the EUR corporate curves (industrial, utility, telecom, financial).
///
/// Order is part of the contract — the server seed and the LP-SIM plan both walk this
/// sequence, so they must agree row-for-row.
#[must_use]
pub fn curated_universe() -> Vec<GovBondSpec> {
    [
        &UK_GILTS,
        &DE_BUNDS,
        &FR_OATS,
        &IT_BTPS,
        &EU_CORP_INDUSTRIAL,
        &EU_CORP_UTILITY,
        &EU_CORP_TELECOM,
        &EU_CORP_FINANCIAL,
    ]
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
        sub_asset_type: c.sub_asset_type,
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
        assert_eq!(
            u.len(),
            10 + 8 + 6 + 6 + (5 * 4),
            "UK+DE+FR+IT govvies plus the four 5-point EUR corporate curves"
        );
        let count = |r: &str| u.iter().filter(|s| s.region == r).count();
        assert_eq!(count("uk"), 10);
        assert_eq!(count("de"), 8);
        assert_eq!(count("fr"), 6);
        assert_eq!(count("it"), 6);
        assert_eq!(count("eu"), 20, "four EUR corporate issuers × 5 tenors");
    }

    /// The corporates are labelled as CREDIT, not sovereign. A corporate mislabelled
    /// `government` would be filtered into the govvie screens, bucketed as sovereign risk,
    /// and hedged as if it carried no spread.
    #[test]
    fn corporates_are_labelled_corporate_and_sovereigns_government() {
        let u = curated_universe();
        let corps: Vec<_> = u
            .iter()
            .filter(|s| s.sub_asset_type == "corporate")
            .collect();
        assert_eq!(corps.len(), 20);
        for s in &corps {
            assert_eq!(s.currency, "EUR", "{} is a EUR corporate", s.instrument_id);
            assert_eq!(s.coupon_frequency, "annual");
            assert_eq!(s.calendars, vec!["target2"]);
            assert!(
                s.isin.starts_with("XS"),
                "{} should carry the international Eurobond prefix, got {}",
                s.instrument_id,
                s.isin
            );
        }
        // Nothing was silently retyped: every remaining row is still a sovereign.
        assert!(
            u.iter()
                .all(|s| s.sub_asset_type == "corporate" || s.sub_asset_type == "government")
        );
    }

    /// Every EUR corporate prices WIDE of the German sovereign at the same tenor — i.e. it
    /// carries real credit spread. If a corporate curve were ever copied from the govvie
    /// curve (or a spread sign inverted) this fails: a corporate inside the risk-free curve
    /// is not a credit instrument, and a book holding it would show no spread risk at all.
    #[test]
    fn every_eur_corporate_trades_wide_of_the_bund_at_the_same_tenor() {
        let u = curated_universe();
        let bund_at = |tenor_year: i32| {
            u.iter()
                .find(|s| {
                    s.instrument_id == format!("de-bund-{}y-{}", tenor_year, 2026 + tenor_year)
                })
                .map(|s| s.coupon_rate)
        };
        let mut checked = 0;
        for s in u.iter().filter(|s| s.sub_asset_type == "corporate") {
            let tenor: i32 = s.maturity_date.year - REFERENCE_YEAR;
            let Some(govvie) = bund_at(tenor) else {
                continue;
            };
            assert!(
                s.coupon_rate > govvie,
                "{} ({:.4}) must be wide of the {}y Bund ({:.4})",
                s.instrument_id,
                s.coupon_rate,
                tenor,
                govvie
            );
            checked += 1;
        }
        assert_eq!(
            checked, 20,
            "every corporate point must have been compared against a Bund"
        );
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
