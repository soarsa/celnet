//! The vendor-neutral government-bond specification record and its civil-date type.
//!
//! A [`GovBondSpec`] is a fully-specified, static government-bond reference record:
//! everything the server needs to mint a reference-data `InstrumentDef` and the LP
//! simulator needs to rebuild a real `celnet_bond::Bond` cashflow schedule. Every
//! convention is carried as a stable snake_case **label** (the same discipline the
//! server registry uses), so this crate never depends on the server's enums and the
//! JSON stays human-editable.

/// A civil (broken) date: a plain `(year, month, day)` triple validated against the
/// real Gregorian calendar via [`CivilYmd::is_valid`]. Mirrors the server's
/// `CivilDate` structurally so the server maps one onto the other without this crate
/// depending on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivilYmd {
    /// Gregorian year (e.g. 2036).
    pub year: i32,
    /// Month of year, 1..=12.
    pub month: u32,
    /// Day of month, 1..=31 (validated against the month).
    pub day: u32,
}

impl CivilYmd {
    /// Construct a civil date from its components (unchecked; validate with
    /// [`is_valid`](Self::is_valid)).
    #[must_use]
    pub const fn new(year: i32, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    /// Whether this triple is a real calendar date (rejects e.g. 2036-02-30).
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

    /// A compact `Mon-YY` label (e.g. `May-36`) for building a friendly bond name.
    #[must_use]
    pub fn month_year_label(self) -> String {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let idx = self.month.clamp(1, 12) as usize - 1;
        format!("{}-{:02}", MONTHS[idx], self.year.rem_euclid(100))
    }
}

/// One fully-specified government-bond reference record.
///
/// Built only by this crate's generators from validated inputs, so every field is
/// known-good: the ISIN passes its ISO-6166 check digit, the maturity is a real
/// calendar date, and the coupon/frequency pairing is internally consistent (a zero
/// carries no frequency; a fixed coupon carries one).
#[derive(Debug, Clone, PartialEq)]
pub struct GovBondSpec {
    /// The canonical internal `instrument_id` — the identity shared across the
    /// reference registry, the wire, and the LP feed. For US Treasuries this is the
    /// 9-character CUSIP (what the LP-SIM feed already streams); for curated non-US
    /// govvies it is a stable, readable slug (e.g. `uk-gilt-10y-2036`).
    pub instrument_id: String,
    /// A short, human-friendly blotter/GUI name (e.g. `UK 10Y GILT 4.000% Jan-36`).
    pub name: String,
    /// The issuer name (e.g. `US Treasury`, `UK DMO`, `Bundesrepublik Deutschland`).
    pub issuer: String,
    /// ISO 4217 pricing/settlement currency label (`USD` / `GBP` / `EUR`).
    pub currency: &'static str,
    /// The region label (`us` / `uk` / `de` / `fr` / `it`); the EUR issuers roll up
    /// to `eu` via the server's region helper.
    pub region: &'static str,
    /// The sub-asset-type label — always `government` for this Phase-1 universe.
    pub sub_asset_type: &'static str,
    /// The ISO 6166 ISIN (12 chars, check-digit valid).
    pub isin: String,
    /// The CUSIP, when the security carries one (US Treasuries); `None` otherwise.
    pub cusip: Option<String>,
    /// The annual coupon rate as a decimal (e.g. `0.04125` for a 4.125% coupon);
    /// `0.0` for a Bill / zero-coupon security.
    pub coupon_rate: f64,
    /// The coupon-type label (`fixed` / `zero`).
    pub coupon_type: &'static str,
    /// The coupon payment-frequency label (`semi_annual` / `annual`); empty for a
    /// `zero`.
    pub coupon_frequency: &'static str,
    /// The accrual day-count label — `act_act` for every government curve here.
    pub day_count: &'static str,
    /// The dated (accrual-start) date, when known.
    pub dated_date: Option<CivilYmd>,
    /// The final-redemption date (required).
    pub maturity_date: CivilYmd,
    /// The par redemption / face value (100.0 per 100 face).
    pub redemption: f64,
    /// The settlement calendar-centre labels (at least one).
    pub calendars: Vec<&'static str>,
}
