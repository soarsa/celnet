//! Parity row — **pair-universe breadth** in `celnet-conventions` / `celnet-calendar`
//! (Wave 4d).
//!
//! The convention registry profiles a documented FX-options pair universe beyond
//! the G10 majors. This row proves, against PUBLISHED market-convention
//! references encoded directly as expected values (and against structural
//! invariants and an INDEPENDENT calendar oracle so none of it is a tautology),
//! that the resolved conventions and the algorithmic spot-date are correct.
//!
//! # The EXACT covered set (nothing outside it is claimed)
//!
//! * **G10 majors:** EURUSD, USDJPY, GBPUSD, AUDUSD, USDCHF, USDCAD, NZDUSD.
//! * **EM deliverable crosses:** USDMXN, USDZAR, USDNOK, USDSEK.
//! * **EM non-deliverable (NDF/NDO):** USDKRW (EMTA KRW KFTC18), USDTWD (Taipei
//!   TFEMA), USDINR (RBI reference), USDBRL (PTAX/BRL09), USDCLP (Dólar
//!   Observado/CLP10), USDCOP (TRM/COP04) — each cash-settled in **USD**.
//! * **Precious metals:** XAUUSD, XAGUSD (metal as base, USD premium, T+2
//!   loco-London).
//!
//! # The three independent checks
//!
//! 1. **(i) Resolved conventions vs published standards.** For each covered pair
//!    the table of `(spot lag, premium currency, ATM, delta family, cut, and —
//!    for NDFs — the USD cash-settlement currency + the fixing source)` is
//!    asserted against the EMTA/ISDA + interbank OTC market standard, encoded as
//!    literal expected values in [`PUBLISHED`]. The expectations are the
//!    independent oracle (published convention data), not a copy of the code.
//!
//! 2. **(ii) Algorithmic spot-date over JOINED calendars.** For a sweep of trade
//!    dates the library's `spot_date_civil` is recomputed by a *fully
//!    independent* in-test walk: an independent Gregorian civil-date engine
//!    (Howard Hinnant's `days_from_civil`/`civil_from_days` rata-die algorithm —
//!    a different implementation from the `time` crate the library uses) plus
//!    independent per-currency holiday predicates, walking T+N good business days
//!    over the **intersection** of both legs' calendars (+ the USD leg). The
//!    independent result must equal the library's to the day — proving the spot
//!    date is computed, not stored.
//!
//! 3. **(iii) Structural invariants.** Every NDF is cash-settled in USD and
//!    flagged non-deliverable with a fixing source; every metal carries the metal
//!    as base with a USD premium; every entry is internally self-consistent
//!    (premium-adjusted flag ⇔ premium style ⇔ premium currency = base); no pair
//!    resolves to a contradictory profile; and the universe meta agrees with the
//!    wire convention record on settlement.
//!
//! # Honest boundary
//!
//! This asserts only that the convention/calendar **code** matches published
//! standards. It claims NO live EM/NDF feed data: the fixing *values* are an
//! estate-gated market-data feed, never sourced here — only the fixing *identity*
//! (which published rate the contract references) is encoded. The NDF currencies'
//! onshore lunisolar settlement calendars are honestly out of scope (see the
//! `spot_date` coverage note); the deliverable EM crosses and the metals, whose
//! holidays are fully Gregorian-computable, are the set whose algorithmic spot
//! date is proven here.

use celnet_calendar::{is_business_day_civil, spot_date_civil};
use celnet_conventions::pair_meta;
use celnet_types::{
    AtmConvention, BrokenDate as Civil, Ccy, CcyPair, Cut, FixingSource, PremiumStyle, Settlement,
};

fn pair(s: &str) -> CcyPair {
    CcyPair::parse(s).expect("valid pair literal")
}

fn ccy(s: &str) -> Ccy {
    Ccy::parse(s).expect("valid ccy literal")
}

// ===========================================================================
// (i) Published convention reference table — the independent oracle.
//
// Each row is the published OTC / EMTA-ISDA market standard for one covered
// pair, encoded as literal expected values. Sources: EMTA template terms and
// the 2005/2018 ISDA FX/Currency Option definitions (per-currency settlement
// matrices) for the NDF fixings + USD cash settlement; the standard interbank
// FX-options convention catalogue (delta/ATM/premium/cut/spot-lag) for the rest.
// ===========================================================================

/// A published-standard expectation for one covered pair.
struct Published {
    pair: &'static str,
    spot_lag: u32,
    /// Physical premium currency (ISO code).
    premium_ccy: &'static str,
    premium_style: PremiumStyle,
    atm: AtmConvention,
    cut: Cut,
    settlement: Settlement,
    /// For an NDF: `Some((fixing, settlement-ccy code))`; `None` for deliverable.
    ndf: Option<(FixingSource, &'static str)>,
    precious_metal: bool,
}

const PUBLISHED: &[Published] = &[
    // --- G10 majors. ---
    Published {
        pair: "EURUSD",
        spot_lag: 2,
        premium_ccy: "EUR",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    Published {
        pair: "USDJPY",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::Tokyo1500,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    Published {
        pair: "GBPUSD",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::DomesticPips,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    Published {
        pair: "AUDUSD",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::DomesticPips,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    Published {
        pair: "USDCHF",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    // USDCAD is the canonical T+1 major.
    Published {
        pair: "USDCAD",
        spot_lag: 1,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    Published {
        pair: "NZDUSD",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::DomesticPips,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    // --- EM deliverable crosses (USD as FOR → premium-adjusted, NY cut, T+2). ---
    Published {
        pair: "USDMXN",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    Published {
        pair: "USDZAR",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    Published {
        pair: "USDNOK",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    Published {
        pair: "USDSEK",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: false,
    },
    // --- EM non-deliverable (NDF/NDO), cash-settled in USD at the named fixing. ---
    Published {
        pair: "USDKRW",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::Tokyo1500,
        settlement: Settlement::NonDeliverable,
        ndf: Some((FixingSource::KrwKftc18, "USD")),
        precious_metal: false,
    },
    Published {
        pair: "USDTWD",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::Tokyo1500,
        settlement: Settlement::NonDeliverable,
        ndf: Some((FixingSource::TwdTaipei, "USD")),
        precious_metal: false,
    },
    Published {
        pair: "USDINR",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::Tokyo1500,
        settlement: Settlement::NonDeliverable,
        ndf: Some((FixingSource::InrRbiRef, "USD")),
        precious_metal: false,
    },
    Published {
        pair: "USDBRL",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::NonDeliverable,
        ndf: Some((FixingSource::BrlPtax, "USD")),
        precious_metal: false,
    },
    Published {
        pair: "USDCLP",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::NonDeliverable,
        ndf: Some((FixingSource::ClpDolarObs, "USD")),
        precious_metal: false,
    },
    Published {
        pair: "USDCOP",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::PercentForeign,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::NonDeliverable,
        ndf: Some((FixingSource::CopTrm, "USD")),
        precious_metal: false,
    },
    // --- Precious metals (metal as base, USD premium → unadjusted, NY cut, T+2). ---
    Published {
        pair: "XAUUSD",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::DomesticPips,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: true,
    },
    Published {
        pair: "XAGUSD",
        spot_lag: 2,
        premium_ccy: "USD",
        premium_style: PremiumStyle::DomesticPips,
        atm: AtmConvention::DeltaNeutralStraddle,
        cut: Cut::NewYork1000,
        settlement: Settlement::Deliverable,
        ndf: None,
        precious_metal: true,
    },
];

/// (i) The resolved convention for every covered pair matches the published
/// market standard, field by field.
#[test]
fn resolved_conventions_match_published_standards() {
    for row in PUBLISHED {
        let cp = pair(row.pair);
        let m = pair_meta(cp).unwrap_or_else(|| panic!("{} is covered", row.pair));
        assert_eq!(m.spot_lag_days, row.spot_lag, "{} spot lag", row.pair);
        assert_eq!(
            m.premium_ccy,
            ccy(row.premium_ccy),
            "{} premium ccy",
            row.pair
        );
        assert_eq!(
            m.premium_style, row.premium_style,
            "{} premium style",
            row.pair
        );
        assert_eq!(m.atm, row.atm, "{} ATM", row.pair);
        assert_eq!(m.cut, row.cut, "{} cut", row.pair);
        assert_eq!(m.settlement, row.settlement, "{} settlement", row.pair);
        assert_eq!(
            m.is_precious_metal(),
            row.precious_metal,
            "{} metal",
            row.pair
        );
        match row.ndf {
            Some((fixing, settle)) => {
                let terms = m
                    .ndf
                    .unwrap_or_else(|| panic!("{} has NDF terms", row.pair));
                assert_eq!(terms.fixing, fixing, "{} fixing source", row.pair);
                assert_eq!(terms.settlement_ccy, ccy(settle), "{} settle ccy", row.pair);
            }
            None => assert!(m.ndf.is_none(), "{} must not carry NDF terms", row.pair),
        }
    }
    // Coverage breadth: the encoded table IS the full covered set (19 pairs).
    assert_eq!(PUBLISHED.len(), 19, "exact covered-set size");
}

// ===========================================================================
// (ii) Independent algorithmic spot-date oracle.
//
// A second, independent implementation of the FX spot-date computation, sharing
// NOTHING with the library under test:
//   * an independent Gregorian rata-die date engine (Hinnant's algorithm), not
//     the `time` crate the library uses;
//   * independent per-currency holiday predicates re-derived from the published
//     national/EMTA holiday rules;
//   * an independent T+N business-day walk over the JOINED (intersected) leg
//     calendars + the USD leg.
// The two must agree to the day for every (pair, trade-date) in the sweep.
// ===========================================================================

/// Days since 1970-01-01 for a civil `(y, m, d)` — Howard Hinnant's algorithm
/// (`http://howardhinnant.github.io/date_algorithms.html#days_from_civil`).
/// Independent of any date library.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Inverse of [`days_from_civil`]: civil `(y, m, d)` from a 1970-epoch day count.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Weekday for a civil date: 0 = Sunday .. 6 = Saturday (independent of `time`).
fn weekday(y: i64, m: i64, d: i64) -> i64 {
    // 1970-01-01 was a Thursday (= 4). rem_euclid keeps it in [0, 6].
    (days_from_civil(y, m, d) + 4).rem_euclid(7)
}

fn is_weekend(y: i64, m: i64, d: i64) -> bool {
    let w = weekday(y, m, d);
    w == 0 || w == 6
}

/// Independent Western-Christian Easter Sunday (anonymous Gregorian computus),
/// returned as `(month, day)`. Re-derived here, not shared with the library.
fn easter(y: i64) -> (i64, i64) {
    let a = y % 19;
    let b = y / 100;
    let c = y % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let mth = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * mth + 114) / 31;
    let day = ((h + l - 7 * mth + 114) % 31) + 1;
    (month, day)
}

/// Easter-relative civil date as a `(y, m, d)` triple, `offset` days from Easter
/// Sunday (negative = before).
fn easter_plus(y: i64, offset: i64) -> (i64, i64, i64) {
    let (em, ed) = easter(y);
    civil_from_days(days_from_civil(y, em, ed) + offset)
}

/// `n`-th (1-based) `target_weekday` of `(y, m)` — independent of `time`.
/// `target_weekday`: 0 = Sun .. 6 = Sat.
fn nth_weekday(y: i64, m: i64, target_weekday: i64, n: i64) -> (i64, i64, i64) {
    let first = weekday(y, m, 1);
    let offset = (target_weekday - first).rem_euclid(7);
    let day = 1 + offset + 7 * (n - 1);
    (y, m, day)
}

/// Independent US (Fedwire) FX-settlement holiday predicate (excludes
/// Columbus/Veterans Day, matching the published Fedwire calendar).
fn us_holiday(y: i64, m: i64, d: i64) -> bool {
    // Nearest-weekday observance for fixed-date holidays.
    let observed = |ym: i64, yd: i64| -> (i64, i64) {
        let w = weekday(y, ym, yd);
        match w {
            6 => (ym, yd - 1), // Sat → Fri
            0 => (ym, yd + 1), // Sun → Mon
            _ => (ym, yd),
        }
    };
    let on = |ym: i64, yd: i64| observed(ym, yd) == (m, d);
    let nth = |mth: i64, wd: i64, n: i64| nth_weekday(y, mth, wd, n) == (y, m, d);
    // Last Monday of May (Memorial Day): the 5th Monday if it exists, else 4th.
    let last_mon_may = {
        let fifth = nth_weekday(y, 5, 1, 5);
        if fifth.2 <= 31 {
            fifth
        } else {
            nth_weekday(y, 5, 1, 4)
        }
    } == (y, m, d);
    on(1, 1)
        || nth(1, 1, 3) // MLK
        || nth(2, 1, 3) // Washington
        || last_mon_may
        || (y >= 2021 && on(6, 19)) // Juneteenth
        || on(7, 4)
        || nth(9, 1, 1) // Labor Day
        || nth(11, 4, 4) // Thanksgiving (4th Thursday; Thu = 4)
        || on(12, 25)
}

/// Independent Mexico (Banxico) holiday predicate (fully Gregorian).
fn mexico_holiday(y: i64, m: i64, d: i64) -> bool {
    let on = |mth: i64, day: i64| (m, d) == (mth, day);
    let nth = |mth: i64, wd: i64, n: i64| nth_weekday(y, mth, wd, n) == (y, m, d);
    on(1, 1)
        || nth(2, 1, 1) // Constitution (1st Mon Feb)
        || nth(3, 1, 3) // Benito Juárez (3rd Mon Mar)
        || on(5, 1)
        || on(9, 16)
        || nth(11, 1, 3) // Revolution (3rd Mon Nov)
        || on(12, 25)
}

/// Independent South Africa holiday predicate (fixed dates with the Public
/// Holidays Act Sunday→Monday observance + Good Friday / Family Day).
fn southafrica_holiday(y: i64, m: i64, d: i64) -> bool {
    if easter_plus(y, -2) == (y, m, d) || easter_plus(y, 1) == (y, m, d) {
        return true; // Good Friday, Family Day
    }
    let fixed = [
        (1, 1),
        (3, 21),
        (4, 27),
        (5, 1),
        (6, 16),
        (8, 9),
        (9, 24),
        (12, 16),
        (12, 25),
        (12, 26),
    ];
    fixed.iter().any(|&(fm, fd)| {
        // Sunday → observed Monday; Saturday unshifted.
        if weekday(y, fm, fd) == 0 {
            let (om, od) = (fm, fd + 1);
            (m, d) == (om, od)
        } else {
            (m, d) == (fm, fd)
        }
    })
}

/// Independent Norway holiday predicate (fixed + full computus, no weekend shift).
fn norway_holiday(y: i64, m: i64, d: i64) -> bool {
    let on = |mth: i64, day: i64| (m, d) == (mth, day);
    on(1, 1)
        || easter_plus(y, -3) == (y, m, d) // Maundy Thursday
        || easter_plus(y, -2) == (y, m, d) // Good Friday
        || easter_plus(y, 1) == (y, m, d) // Easter Monday
        || on(5, 1)
        || on(5, 17) // Constitution Day
        || easter_plus(y, 39) == (y, m, d) // Ascension
        || easter_plus(y, 50) == (y, m, d) // Whit Monday
        || on(12, 25)
        || on(12, 26)
}

/// Independent Sweden holiday predicate (fixed + computus + Midsummer Eve +
/// Christmas/New-Year bank closes).
fn sweden_holiday(y: i64, m: i64, d: i64) -> bool {
    let on = |mth: i64, day: i64| (m, d) == (mth, day);
    // Midsummer Eve: the Friday in 19..=25 June (Fri = 5).
    let midsummer_eve = (19..=25).find(|&day| weekday(y, 6, day) == 5).unwrap();
    on(1, 1)
        || on(1, 6) // Epiphany
        || easter_plus(y, -2) == (y, m, d) // Good Friday
        || easter_plus(y, 1) == (y, m, d) // Easter Monday
        || easter_plus(y, 39) == (y, m, d) // Ascension
        || on(5, 1)
        || on(6, 6) // National Day
        || (m, d) == (6, midsummer_eve)
        || on(12, 24)
        || on(12, 25)
        || on(12, 26)
        || on(12, 31)
}

/// UK (London) holiday predicate — needed for the loco-London metal calendar.
/// Independent re-derivation of the London bank holidays.
fn uk_holiday(y: i64, m: i64, d: i64) -> bool {
    // Bump-forward observance for New Year and the Christmas/Boxing substitutes.
    let next_weekday_observed = |ym: i64, yd: i64| -> (i64, i64) {
        match weekday(y, ym, yd) {
            6 => civ2(civil_from_days(days_from_civil(y, ym, yd) + 2)),
            0 => civ2(civil_from_days(days_from_civil(y, ym, yd) + 1)),
            _ => (ym, yd),
        }
    };
    if next_weekday_observed(1, 1) == (m, d) {
        return true;
    }
    if easter_plus(y, -2) == (y, m, d) || easter_plus(y, 1) == (y, m, d) {
        return true; // Good Friday, Easter Monday
    }
    if nth_weekday(y, 5, 1, 1) == (y, m, d) {
        return true; // Early May (1st Mon)
    }
    // Spring (last Mon May) and Summer (last Mon Aug).
    let last_mon = |mth: i64, mlen: i64| {
        let fifth = nth_weekday(y, mth, 1, 5);
        if fifth.2 <= mlen {
            fifth
        } else {
            nth_weekday(y, mth, 1, 4)
        }
    };
    if last_mon(5, 31) == (y, m, d) || last_mon(8, 31) == (y, m, d) {
        return true;
    }
    // Christmas + Boxing Day substitutes (the two never collide).
    let xmas = next_weekday_observed(12, 25);
    let mut boxing = next_weekday_observed(12, 26);
    if boxing == xmas {
        boxing = civ2(civil_from_days(days_from_civil(y, boxing.0, boxing.1) + 1));
    }
    (m, d) == xmas || (m, d) == boxing
}

/// Drop the year from a `(y, m, d)` triple.
fn civ2(t: (i64, i64, i64)) -> (i64, i64) {
    (t.1, t.2)
}

/// The independent per-currency holiday predicate for the calendars we cover
/// with a real Gregorian engine. Currencies without a Gregorian engine here are
/// NOT in the spot-date sweep (their onshore calendars are honestly out of
/// scope) — calling this for them panics so the sweep can never silently use a
/// wrong calendar.
fn holiday_for(code: &str, y: i64, m: i64, d: i64) -> bool {
    match code {
        "USD" => us_holiday(y, m, d),
        "MXN" => mexico_holiday(y, m, d),
        "ZAR" => southafrica_holiday(y, m, d),
        "NOK" => norway_holiday(y, m, d),
        "SEK" => sweden_holiday(y, m, d),
        // Loco-London metals settle on the London (UK) calendar.
        "XAU" | "XAG" | "GBP" => uk_holiday(y, m, d),
        other => panic!("no independent Gregorian calendar oracle for {other}"),
    }
}

/// Whether a civil date is a good joint business day for `legs` (the set of
/// settlement-centre currency codes), per the independent oracle: open in EVERY
/// leg and not a weekend.
fn independent_is_business_day(legs: &[&str], y: i64, m: i64, d: i64) -> bool {
    if is_weekend(y, m, d) {
        return false;
    }
    legs.iter().all(|leg| !holiday_for(leg, y, m, d))
}

/// The settlement-centre legs for a pair in the independent oracle: both legs'
/// centres, plus USD for a cross (neither leg USD). Metals map to the London
/// (UK/GBP) centre; USD is its own centre.
fn legs_for(pair_str: &str) -> Vec<&'static str> {
    let centre = |c: &str| -> &'static str {
        match c {
            "USD" => "USD",
            "MXN" => "MXN",
            "ZAR" => "ZAR",
            "NOK" => "NOK",
            "SEK" => "SEK",
            "XAU" => "XAU",
            "XAG" => "XAG",
            _ => panic!("currency {c} not in the independent spot-date oracle"),
        }
    };
    let base = &pair_str[0..3];
    let quote = &pair_str[3..6];
    let mut legs = vec![centre(base), centre(quote)];
    if base != "USD" && quote != "USD" {
        legs.push("USD");
    }
    legs
}

/// Independent T+N good-business-day walk from a civil horizon (counting begins
/// from the next good day; the horizon itself need not be good — matching the
/// library and market practice).
fn independent_spot(pair_str: &str, lag: u32, y: i64, m: i64, d: i64) -> (i64, i64, i64) {
    let legs = legs_for(pair_str);
    let mut z = days_from_civil(y, m, d);
    for _ in 0..lag {
        loop {
            z += 1;
            let (cy, cm, cd) = civil_from_days(z);
            if independent_is_business_day(&legs, cy, cm, cd) {
                break;
            }
        }
    }
    civil_from_days(z)
}

/// (ii) For a sweep of trade dates across two years, the library's algorithmic
/// spot date equals the fully INDEPENDENT walk to the day, for every covered pair
/// whose settlement calendar is Gregorian-computable (the EM deliverable crosses
/// and the precious metals). This is the proof the spot date is COMPUTED over the
/// joined calendars, not stored.
#[test]
fn algorithmic_spot_date_matches_independent_walk() {
    // Pairs with a fully independent Gregorian calendar oracle.
    let gregorian_pairs: &[&str] = &[
        "EURUSD", // sanity baseline (US ∩ TARGET2 — but TARGET2 is not in our
        // independent oracle; exclude below)
        "USDMXN", "USDZAR", "USDNOK", "USDSEK", "XAUUSD", "XAGUSD",
    ];
    // EURUSD needs a TARGET2 oracle we did not re-derive; restrict the sweep to
    // the pairs whose BOTH legs the independent oracle models.
    let oracle_pairs: Vec<&str> = gregorian_pairs
        .iter()
        .copied()
        .filter(|p| *p != "EURUSD")
        .collect();

    let mut checked = 0u32;
    for &p in &oracle_pairs {
        let cp = pair(p);
        let lag = pair_meta(cp).unwrap().spot_lag_days;
        // Sweep every day of 2024 and 2025 (covers weekends, holidays, leap day,
        // year-ends, and the multi-day holiday runs).
        for &year in &[2024i64, 2025i64] {
            for day_of_year in 0..366i64 {
                let base = days_from_civil(year, 1, 1) + day_of_year;
                let (y, m, d) = civil_from_days(base);
                if y != year {
                    continue; // 2024 has 366 days, 2025 has 365 — skip overflow.
                }
                let expected = independent_spot(p, lag, y, m, d);
                let lib = spot_date_civil(cp, Civil::new(y as i32, m as u8, d as u8))
                    .expect("valid horizon resolves");
                assert_eq!(
                    (lib.year as i64, i64::from(lib.month), i64::from(lib.day)),
                    expected,
                    "{p} spot from {y}-{m:02}-{d:02} (lag {lag})"
                );
                checked += 1;
            }
        }
    }
    // The sweep is non-trivial: 6 pairs × ~730 days.
    assert!(checked > 4000, "sweep too small: {checked}");
}

/// (ii) Cross-check: the independent business-day predicate agrees with the
/// library's `is_business_day_civil` over the sweep, on a representative pair —
/// so the spot-walk agreement above is not masking compensating calendar errors.
#[test]
fn independent_business_day_predicate_agrees_with_library() {
    let p = "USDMXN";
    let cp = pair(p);
    let legs = legs_for(p);
    for &year in &[2024i64, 2025i64] {
        for day_of_year in 0..365i64 {
            let (y, m, d) = civil_from_days(days_from_civil(year, 1, 1) + day_of_year);
            if y != year {
                continue;
            }
            let indep = independent_is_business_day(&legs, y, m, d);
            let lib = is_business_day_civil(cp, Civil::new(y as i32, m as u8, d as u8)).unwrap();
            assert_eq!(
                indep, lib,
                "{p} business-day disagreement at {y}-{m:02}-{d:02}"
            );
        }
    }
}

/// (ii) The independent rata-die date engine is itself validated against known
/// anchors, so it cannot be silently wrong (which would make the agreement above
/// a coincidence of two broken implementations).
#[test]
fn independent_date_engine_is_correct() {
    // Round-trip every day of a decade.
    let start = days_from_civil(2020, 1, 1);
    for off in 0..3653i64 {
        let z = start + off;
        let (y, m, d) = civil_from_days(z);
        assert_eq!(days_from_civil(y, m, d), z, "round-trip {y}-{m}-{d}");
    }
    // Known weekdays (0 = Sun .. 6 = Sat): 2024-06-03 was a Monday (= 1);
    // 2000-01-01 was a Saturday (= 6); 1970-01-01 was a Thursday (= 4).
    assert_eq!(weekday(2024, 6, 3), 1);
    assert_eq!(weekday(2000, 1, 1), 6);
    assert_eq!(weekday(1970, 1, 1), 4);
    // Known Easter Sundays.
    assert_eq!(easter(2024), (3, 31));
    assert_eq!(easter(2025), (4, 20));
}

// ===========================================================================
// (iii) Structural invariants — no covered pair can be internally contradictory.
// ===========================================================================

/// (iii) Every NDF pair is cash-settled in USD and flagged non-deliverable with
/// a fixing source; every metal carries the metal as base with a USD premium;
/// every entry is self-consistent; and the universe meta agrees with the wire
/// convention record on settlement.
#[test]
fn structural_invariants_hold_for_every_covered_pair() {
    for row in PUBLISHED {
        let cp = pair(row.pair);
        let m = pair_meta(cp).unwrap();

        // Self-consistency: premium-adjusted ⇔ premium style ⇔ premium ccy=base;
        // NDF terms present iff non-deliverable; settlement/premium ccys are legs.
        assert!(m.is_self_consistent(), "{} not self-consistent", row.pair);

        if m.is_non_deliverable() {
            let terms = m.ndf.expect("NDF carries terms");
            assert_eq!(terms.settlement_ccy, Ccy::USD, "{} settles USD", row.pair);
            // The settlement currency must be one of the legs (the convertible
            // USD leg) and must NOT be the restricted EM leg. For the covered
            // USD/EM NDFs USD is the base, so the RESTRICTED currency is the quote.
            let restricted = if cp.base == Ccy::USD {
                cp.quote
            } else {
                cp.base
            };
            assert_ne!(
                restricted, terms.settlement_ccy,
                "{} restricted leg is not the settlement ccy",
                row.pair
            );
            assert!(
                terms.settlement_ccy == cp.base || terms.settlement_ccy == cp.quote,
                "{} settle ccy is a leg",
                row.pair
            );
        } else {
            assert!(m.ndf.is_none(), "{} deliverable carries no NDF", row.pair);
        }

        if m.is_precious_metal() {
            // Metal is the base leg; quote is USD; premium in USD (unadjusted).
            assert_eq!(cp.quote, Ccy::USD, "{} metal quoted in USD", row.pair);
            assert_ne!(cp.base, Ccy::USD, "{} base is the metal", row.pair);
            assert_eq!(m.premium_ccy, Ccy::USD, "{} metal premium USD", row.pair);
            assert!(!m.premium_adjusted, "{} metal premium unadjusted", row.pair);
        }

        // Delta/ATM self-consistency: the premium-adjusted flag exactly tracks the
        // premium style, and the premium currency is one of the two legs.
        assert_eq!(
            m.premium_adjusted,
            m.premium_style.is_premium_adjusted(),
            "{} delta/premium consistency",
            row.pair
        );
        assert!(
            m.premium_ccy == cp.base || m.premium_ccy == cp.quote,
            "{} premium ccy is a leg",
            row.pair
        );
    }
}

/// (iii) No covered pair resolves to a contradictory profile under orientation
/// inversion: the fixing/settlement currency of an NDF is orientation-invariant
/// (a physical property), while the premium-adjusted flag correctly flips.
#[test]
fn orientation_inversion_is_non_contradictory() {
    for row in PUBLISHED {
        let cp = pair(row.pair);
        let inv = CcyPair::new(cp.quote, cp.base);
        let direct = pair_meta(cp).unwrap();
        let flipped = pair_meta(inv).unwrap();

        // Settlement style, ATM, cut, instrument class are orientation-invariant.
        assert_eq!(direct.settlement, flipped.settlement, "{}", row.pair);
        assert_eq!(direct.atm, flipped.atm, "{}", row.pair);
        assert_eq!(direct.cut, flipped.cut, "{}", row.pair);
        assert_eq!(
            direct.is_precious_metal(),
            flipped.is_precious_metal(),
            "{}",
            row.pair
        );
        // NDF fixing + settlement ccy are physical → invariant.
        match (direct.ndf, flipped.ndf) {
            (Some(a), Some(b)) => {
                assert_eq!(a.fixing, b.fixing, "{} fixing invariant", row.pair);
                assert_eq!(
                    a.settlement_ccy, b.settlement_ccy,
                    "{} settle ccy invariant",
                    row.pair
                );
            }
            (None, None) => {}
            _ => panic!("{} NDF presence flipped under orientation", row.pair),
        }
        // The premium-adjusted flag flips because the premium's physical currency
        // changes role (FOR↔DOM) under inversion.
        assert_ne!(
            direct.premium_adjusted, flipped.premium_adjusted,
            "{} premium-adjusted must flip",
            row.pair
        );
        // Both orientations are individually self-consistent.
        assert!(
            direct.is_self_consistent() && flipped.is_self_consistent(),
            "{}",
            row.pair
        );
    }
}
