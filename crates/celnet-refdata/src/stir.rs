//! The **listed short-term interest-rate (STIR) futures** reference universe — the
//! SOFR strip (`SR3`, `SR1`).
//!
//! # Why these are a separate spec from the deliverable contracts
//!
//! A Treasury future ([`crate::futures`]) settles by **delivering a bond**: its price
//! is a bond price, its terms describe a deliverable basket, and its expiry rules
//! govern a delivery window. A STIR future settles in **cash against a realised
//! interest rate**: there is no deliverable, no conversion factor and no delivery
//! window, and its price is an *index*, not a price per 100 face. Forcing both onto
//! one `ContractTerms` would mean a deliverable-basket description with every field
//! nulled for half the universe — a shape that lies about what the contract is. They
//! share the streaming seam (one `instrument_id`, one `LpQuote`, one aggregated book)
//! and diverge exactly where the economics diverge.
//!
//! # Quotation
//!
//! Both contracts are quoted as `100 − R`, where `R` is the realised reference rate
//! over the contract's accrual period expressed in percent. A rate of 4.05% is an
//! index of `95.95`; the index falls as rates rise. Because the index is linear in
//! the rate, one basis point of rate is a fixed cash amount per contract:
//!
//! | Contract | Notional | Accrual | 1bp |
//! |---|---|---|---|
//! | `SR3` | $1,000,000 | 3 months | `1_000_000 × 0.0001 × 0.25` = **$25.00** |
//! | `SR1` | $5,000,000 | 1 month | `5_000_000 × 0.0001 × (1/12)` = **$41.67** |
//!
//! Those figures are **derived from the contract unit and accrual fraction above**,
//! not asserted from memory — the arithmetic is shown so it can be checked, and a
//! unit test re-derives it rather than tabulating it.
//!
//! # Reference period and cycle
//!
//! `SR3` references compounded SOFR over an **IMM quarter**: from the third Wednesday
//! of the delivery month to the third Wednesday of the following quarter's month, on
//! the March/June/September/December cycle. `SR1` references the average of daily
//! SOFR over a **calendar month**, and lists monthly.
//!
//! # What is not modelled
//!
//! The finer minimum tick that the nearest-expiring contract trades at on some venues
//! is **not** modelled: one outright grid of half a basis point applies to every
//! contract month here. Nor is the convexity between a compounded average and a
//! single forward rate, nor the daily-fixing path once a contract has entered its
//! reference period. These are deliberate boundaries of a **liquidity** simulator,
//! stated rather than papered over — the same discipline the deliverable contracts
//! take with the delivery option.

use crate::model::CivilYmd;

/// The tick grid every listed contract month trades on here: half a basis point of
/// rate, i.e. `0.005` index points. See the module note on the finer nearest-month
/// tick, which is deliberately not modelled.
pub const STIR_TICK_POINTS: f64 = 0.005;

/// How many contract months of each STIR product the committed universe carries.
pub const LISTED_STIR_MONTHS: usize = 4;

/// The first delivery month the committed STIR cycle lists, aligned to the quarterly
/// cycle the deliverable complex starts from.
pub const STIR_CYCLE_START: CivilYmd = CivilYmd::new(2026, 9, 1);

/// Which listing cycle a STIR product follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StirCycle {
    /// March / June / September / December, referencing an **IMM quarter** that runs
    /// third-Wednesday to third-Wednesday (`SR3`).
    ImmQuarterly,
    /// Every calendar month, referencing that **calendar month's** daily average
    /// (`SR1`).
    CalendarMonthly,
}

impl StirCycle {
    /// The next listed month after `from` under this cycle.
    #[must_use]
    pub fn next_month(self, from: CivilYmd) -> CivilYmd {
        let step = match self {
            Self::ImmQuarterly => 3,
            Self::CalendarMonthly => 1,
        };
        add_months(from, step)
    }
}

/// The invariant terms of one STIR product.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StirContractTerms {
    /// The product code (`SR3` / `SR1`) — an opaque public contract code carried as
    /// **data**, never as a product identifier.
    pub symbol: &'static str,
    /// The display label stem, e.g. `3M SOFR FUT`.
    pub label: &'static str,
    /// The contract's notional principal.
    pub notional: f64,
    /// The accrual fraction of a year the reference rate is applied over (`0.25` for
    /// the three-month contract, `1/12` for the one-month).
    pub accrual_fraction: f64,
    /// The minimum outright price increment, in index points.
    pub tick_size_points: f64,
    /// Which listing cycle the product follows.
    pub cycle: StirCycle,
}

impl StirContractTerms {
    /// The cash value of one basis point of rate for this contract:
    /// `notional × 0.0001 × accrual_fraction`. Derived, never tabulated — the table
    /// in the module doc is the worked check of this expression.
    #[must_use]
    pub fn basis_point_value(&self) -> f64 {
        self.notional * 0.000_1 * self.accrual_fraction
    }

    /// The cash value of one minimum tick: [`Self::basis_point_value`] scaled by the
    /// tick expressed in basis points (`0.005` index points = half a basis point).
    #[must_use]
    pub fn tick_value(&self) -> f64 {
        self.basis_point_value() * (self.tick_size_points / 0.01)
    }
}

/// The committed SOFR STIR products.
pub const STIR_FUTURES_TERMS: [StirContractTerms; 2] = [
    StirContractTerms {
        // Three-Month SOFR. Unit $1,000,000 over a quarter ⇒ 1bp = $25.00.
        symbol: "SR3",
        label: "3M SOFR FUT",
        notional: 1_000_000.0,
        accrual_fraction: 0.25,
        tick_size_points: STIR_TICK_POINTS,
        cycle: StirCycle::ImmQuarterly,
    },
    StirContractTerms {
        // One-Month SOFR. Unit $5,000,000 over a month ⇒ 1bp = $41.666…
        symbol: "SR1",
        label: "1M SOFR FUT",
        notional: 5_000_000.0,
        accrual_fraction: 1.0 / 12.0,
        tick_size_points: STIR_TICK_POINTS,
        cycle: StirCycle::CalendarMonthly,
    },
];

/// One listed STIR contract month.
#[derive(Debug, Clone, PartialEq)]
pub struct StirFutureSpec {
    /// The canonical server `instrument_id`, e.g. `SR3U26` — the identity an
    /// aggregated book scopes on and an LP streams under. Same shape as the
    /// deliverable complex: product code, delivery-month code, two-digit year.
    pub instrument_id: String,
    /// Display name, e.g. `3M SOFR FUT Sep 2026`.
    pub name: String,
    /// The product's invariant terms.
    pub terms: StirContractTerms,
    /// Settlement currency.
    pub currency: &'static str,
    /// Region tag, matching the deliverable complex's vocabulary.
    pub region: &'static str,
    /// Reference-data sub-asset type. Distinct from the deliverable complex's
    /// `government_future`: these settle to a rate, not a bond.
    pub sub_asset_type: &'static str,
    /// First day of the delivery/reference month.
    pub delivery_month_start: CivilYmd,
    /// Start of the accrual period the settlement rate is observed over.
    pub reference_period_start: CivilYmd,
    /// End of that accrual period.
    pub reference_period_end: CivilYmd,
    /// Last day the contract trades.
    pub last_trading_date: CivilYmd,
    /// Holiday calendars the contract observes.
    pub calendars: Vec<&'static str>,
}

impl StirFutureSpec {
    /// Whether the contract is still trading on `as_of` (inclusive of the last
    /// trading day). A venue must not quote, and a registry must not advertise, a
    /// contract past this date — quoting one fabricates a market.
    #[must_use]
    pub fn is_listed_on(&self, as_of: CivilYmd) -> bool {
        ymd_key(as_of) <= ymd_key(self.last_trading_date)
    }

    /// The index price for a reference rate quoted as a **decimal** (`0.0405` ⇒
    /// `95.95`). The whole quotation convention in one place.
    #[must_use]
    pub fn index_for_rate(rate_decimal: f64) -> f64 {
        100.0 - rate_decimal * 100.0
    }

    /// The reference rate, as a decimal, implied by an index price — the inverse of
    /// [`Self::index_for_rate`].
    #[must_use]
    pub fn rate_for_index(index: f64) -> f64 {
        (100.0 - index) / 100.0
    }

    /// Snap `index` to the contract's outright tick grid. `round_up` takes the offer
    /// side (away from the mid), matching how a listed market maker quotes.
    #[must_use]
    pub fn snap_to_tick(&self, index: f64, round_up: bool) -> f64 {
        let ticks = index / self.terms.tick_size_points;
        let snapped = if round_up {
            ticks.ceil()
        } else {
            ticks.floor()
        };
        snapped * self.terms.tick_size_points
    }
}

/// A sortable key for a civil date (no calendar arithmetic, just ordering).
fn ymd_key(d: CivilYmd) -> (i32, u32, u32) {
    (d.year, d.month, d.day)
}

/// Advance a civil date by whole months, holding the day-of-month.
fn add_months(from: CivilYmd, months: u32) -> CivilYmd {
    let zero_based = from.month - 1 + months;
    CivilYmd::new(
        from.year + i32::try_from(zero_based / 12).unwrap_or(0),
        zero_based % 12 + 1,
        from.day,
    )
}

/// The exchange delivery-month code (`H`/`M`/`U`/`Z` on the quarterly cycle, and the
/// full monthly set for the serial contract).
fn delivery_month_code(month: u32) -> Option<char> {
    Some(match month {
        1 => 'F',
        2 => 'G',
        3 => 'H',
        4 => 'J',
        5 => 'K',
        6 => 'M',
        7 => 'N',
        8 => 'Q',
        9 => 'U',
        10 => 'V',
        11 => 'X',
        12 => 'Z',
        _ => return None,
    })
}

/// Day-of-week for a civil date via Zeller's congruence: `0` = Sunday.
///
/// Self-contained rather than pulled from a date library: this module needs exactly
/// one calendar fact (which day the third Wednesday falls on), and the reference
/// universe must stay deterministic and dependency-light.
fn weekday(d: CivilYmd) -> u32 {
    let (mut m, mut y) = (d.month, d.year);
    if m < 3 {
        m += 12;
        y -= 1;
    }
    let k = y.rem_euclid(100);
    let j = y.div_euclid(100);
    let t = i32::try_from(d.day).unwrap_or(1)
        + (13 * (i32::try_from(m).unwrap_or(1) + 1)) / 5
        + k
        + k / 4
        + j / 4
        + 5 * j;
    // Zeller yields 0 = Saturday; rotate so 0 = Sunday.
    u32::try_from((t.rem_euclid(7) + 6).rem_euclid(7)).unwrap_or(0)
}

/// The third Wednesday of `year`-`month` — the IMM date the quarterly contract's
/// reference period runs between.
#[must_use]
pub fn third_wednesday(year: i32, month: u32) -> CivilYmd {
    let first = CivilYmd::new(year, month, 1);
    // Wednesday is 3 with Sunday = 0.
    let shift = (3 + 7 - weekday(first)) % 7;
    CivilYmd::new(year, month, 1 + shift + 14)
}

/// The last day of `year`-`month`.
fn last_day_of_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            if leap { 29 } else { 28 }
        }
    }
}

/// The listed contract of one STIR product whose delivery month is `year`-`month`,
/// or `None` when the product does not list that month.
#[must_use]
pub fn contract_for(terms: StirContractTerms, year: i32, month: u32) -> Option<StirFutureSpec> {
    if terms.cycle == StirCycle::ImmQuarterly && !matches!(month, 3 | 6 | 9 | 12) {
        return None;
    }
    let code = delivery_month_code(month)?;
    let start = CivilYmd::new(year, month, 1);
    let (period_start, period_end, last_trading) = match terms.cycle {
        StirCycle::ImmQuarterly => {
            // Third Wednesday to third Wednesday of the next quarter; the contract
            // stops trading when its reference period ends.
            let s = third_wednesday(year, month);
            let next = add_months(start, 3);
            let e = third_wednesday(next.year, next.month);
            (s, e, e)
        }
        StirCycle::CalendarMonthly => {
            // The whole calendar month; the contract stops trading on its last day.
            let e = CivilYmd::new(year, month, last_day_of_month(year, month));
            (start, e, e)
        }
    };
    Some(StirFutureSpec {
        instrument_id: format!("{}{code}{:02}", terms.symbol, year.rem_euclid(100)),
        name: format!("{} {}", terms.label, start.month_year_label()),
        terms,
        currency: "USD",
        region: "us",
        sub_asset_type: "rate_future",
        delivery_month_start: start,
        reference_period_start: period_start,
        reference_period_end: period_end,
        last_trading_date: last_trading,
        calendars: vec!["united_states"],
    })
}

/// The committed STIR universe: [`LISTED_STIR_MONTHS`] contract months of each
/// product from [`STIR_CYCLE_START`], in a stable order (product, then month).
#[must_use]
pub fn stir_futures_universe() -> Vec<StirFutureSpec> {
    let mut out = Vec::with_capacity(STIR_FUTURES_TERMS.len() * LISTED_STIR_MONTHS);
    for terms in STIR_FUTURES_TERMS {
        let mut month = STIR_CYCLE_START;
        let mut listed = 0;
        // Walk forward until the product has its full complement — the quarterly
        // product skips the months it does not list rather than short-counting.
        while listed < LISTED_STIR_MONTHS {
            if let Some(spec) = contract_for(terms, month.year, month.month) {
                out.push(spec);
                listed += 1;
                month = terms.cycle.next_month(month);
            } else {
                month = add_months(month, 1);
            }
        }
    }
    out
}

/// The committed universe restricted to contracts still trading on `as_of`, in the
/// same stable order — the set a venue may quote and a registry may advertise.
#[must_use]
pub fn listed_stir_on(as_of: CivilYmd) -> Vec<StirFutureSpec> {
    stir_futures_universe()
        .into_iter()
        .filter(|s| s.is_listed_on(as_of))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basis_point_values_follow_from_unit_and_accrual() {
        // The published $25.00 / $41.67 figures must FALL OUT of the terms rather
        // than be tabulated, or the table and the terms can silently disagree.
        let sr3 = STIR_FUTURES_TERMS[0];
        let sr1 = STIR_FUTURES_TERMS[1];
        assert_eq!(sr3.symbol, "SR3");
        assert!((sr3.basis_point_value() - 25.0).abs() < 1e-9);
        assert_eq!(sr1.symbol, "SR1");
        assert!((sr1.basis_point_value() - 41.666_666_666).abs() < 1e-6);
        // Half a basis point of tick ⇒ half the bpv.
        assert!((sr3.tick_value() - 12.5).abs() < 1e-9);
    }

    #[test]
    fn the_index_is_one_hundred_minus_the_rate_and_inverts() {
        assert!((StirFutureSpec::index_for_rate(0.0405) - 95.95).abs() < 1e-9);
        assert!((StirFutureSpec::rate_for_index(95.95) - 0.0405).abs() < 1e-12);
        // Rising rates LOWER the index — the sign that makes a STIR hedge work.
        assert!(StirFutureSpec::index_for_rate(0.05) < StirFutureSpec::index_for_rate(0.04));
    }

    #[test]
    fn third_wednesday_is_a_wednesday_and_lands_in_the_third_week() {
        for (y, m) in [(2026, 3), (2026, 9), (2026, 12), (2027, 6), (2028, 2)] {
            let d = third_wednesday(y, m);
            assert_eq!(weekday(d), 3, "{y}-{m} third Wednesday must be a Wednesday");
            assert!((15..=21).contains(&d.day), "{y}-{m} got day {}", d.day);
        }
    }

    #[test]
    fn the_quarterly_product_lists_only_imm_months() {
        let sr3 = STIR_FUTURES_TERMS[0];
        assert!(contract_for(sr3, 2026, 9).is_some(), "September is IMM");
        assert!(contract_for(sr3, 2026, 10).is_none(), "October is not");
        let sr1 = STIR_FUTURES_TERMS[1];
        assert!(contract_for(sr1, 2026, 10).is_some(), "the serial lists it");
    }

    #[test]
    fn the_universe_carries_a_full_strip_of_each_product_with_unique_ids() {
        let u = stir_futures_universe();
        assert_eq!(u.len(), STIR_FUTURES_TERMS.len() * LISTED_STIR_MONTHS);
        let ids: std::collections::BTreeSet<&str> =
            u.iter().map(|s| s.instrument_id.as_str()).collect();
        assert_eq!(ids.len(), u.len(), "instrument ids must be unique");
        // The quarterly strip advances a quarter at a time, the serial a month.
        let sr3: Vec<_> = u.iter().filter(|s| s.terms.symbol == "SR3").collect();
        assert_eq!(sr3[0].delivery_month_start.month, 9);
        assert_eq!(sr3[1].delivery_month_start.month, 12);
        let sr1: Vec<_> = u.iter().filter(|s| s.terms.symbol == "SR1").collect();
        assert_eq!(sr1[0].delivery_month_start.month, 9);
        assert_eq!(sr1[1].delivery_month_start.month, 10);
    }

    #[test]
    fn a_contract_stops_being_listed_after_its_last_trading_day() {
        let spec = contract_for(STIR_FUTURES_TERMS[0], 2026, 9).expect("Sep-26 SR3");
        assert!(spec.is_listed_on(spec.last_trading_date), "inclusive");
        let after = CivilYmd::new(
            spec.last_trading_date.year,
            spec.last_trading_date.month,
            spec.last_trading_date.day + 1,
        );
        assert!(!spec.is_listed_on(after));
    }

    #[test]
    fn ticks_snap_outward_so_a_quote_never_tightens_through_the_grid() {
        let spec = contract_for(STIR_FUTURES_TERMS[0], 2026, 9).expect("Sep-26 SR3");
        // 95.9531 sits between grid points 95.950 and 95.955.
        let bid = spec.snap_to_tick(95.9531, false);
        let offer = spec.snap_to_tick(95.9531, true);
        assert!((bid - 95.950).abs() < 1e-9, "bid rounds down, got {bid}");
        assert!(
            (offer - 95.955).abs() < 1e-9,
            "offer rounds up, got {offer}"
        );
        assert!(bid <= offer);
    }
}
