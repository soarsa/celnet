//! Typed views over the frozen golden tables.
//!
//! Each loader reads its CSV with the dependency-free [`crate::csv`] reader and
//! materializes strongly-typed records (`Copy`, finite) so the test suite can
//! iterate the grid without re-parsing field names. The option-type and
//! barrier-type strings are mapped onto the canonical [`celnet_types`] vocabulary
//! so there is exactly one source of truth for the enums.

use celnet_types::OptionType;

use crate::csv::{CsvError, CsvTable};

/// One frozen vanilla reference: the Garman-Kohlhagen inputs plus the QuantLib
/// price and the Greeks that the analytic European engine exposes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VanillaRecord {
    /// Call or put.
    pub option_type: OptionType,
    /// Spot FX rate (quote per base).
    pub spot: f64,
    /// Strike (quote per base).
    pub strike: f64,
    /// Annualized Black volatility (absolute).
    pub vol: f64,
    /// Time to expiry in years (Act/365F vol-time).
    pub t: f64,
    /// Continuously-compounded domestic (quote) rate.
    pub r_dom: f64,
    /// Continuously-compounded foreign (base) rate.
    pub r_for: f64,
    /// Reference present value (domestic premium per base notional).
    pub price: f64,
    /// Reference spot delta `e^{-r_f T} N(d1)` (= QuantLib `delta`).
    pub delta_spot: f64,
    /// Reference gamma `∂²V/∂S²`.
    pub gamma: f64,
    /// Reference vega `∂V/∂σ` (per `1.0` absolute vol).
    pub vega: f64,
    /// Reference theta `∂V/∂t` per year (= QuantLib `theta`).
    pub theta: f64,
    /// Reference domestic rho `∂V/∂r_dom` (= QuantLib `rho`).
    pub rho_dom: f64,
    /// Reference foreign rho `∂V/∂r_for` (= QuantLib `dividendRho`).
    pub rho_for: f64,
}

/// Single-barrier analytic reference (continuous monitoring).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarrierRecord {
    /// Knock kind, e.g. `"DOWN_OUT"` — kept as the frozen oracle string until
    /// the exotics lane defines its canonical barrier enum.
    pub barrier_type: BarrierType,
    /// Call or put on the underlying payoff.
    pub option_type: OptionType,
    /// Spot FX rate.
    pub spot: f64,
    /// Strike of the underlying vanilla payoff.
    pub strike: f64,
    /// Barrier level.
    pub barrier: f64,
    /// Rebate paid on knock-out (zero in the current grid).
    pub rebate: f64,
    /// Annualized volatility.
    pub vol: f64,
    /// Time to expiry in years.
    pub t: f64,
    /// Domestic rate.
    pub r_dom: f64,
    /// Foreign rate.
    pub r_for: f64,
    /// Reference present value.
    pub price: f64,
}

/// European digital (binary) reference for one settlement style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DigitalRecord {
    /// Cash- or asset-settled.
    pub style: DigitalSettlement,
    /// Call or put digital.
    pub option_type: OptionType,
    /// Spot FX rate.
    pub spot: f64,
    /// Strike.
    pub strike: f64,
    /// Cash payout on in-the-money expiry.
    pub payout: f64,
    /// Annualized volatility.
    pub vol: f64,
    /// Time to expiry in years.
    pub t: f64,
    /// Domestic rate.
    pub r_dom: f64,
    /// Foreign rate.
    pub r_for: f64,
    /// Reference present value.
    pub price: f64,
}

/// One frozen touch-family reference (one-touch / no-touch / double-no-touch /
/// double-touch), all priced by QuantLib's *independent* double-barrier-binary
/// engine (single rows via the wide-corridor limit). The rebate is one unit of
/// domestic cash, paid at expiry (`AT_EXPIRY` timing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchRecord {
    /// Which touch product this row encodes.
    pub kind: TouchKind,
    /// Spot FX rate.
    pub spot: f64,
    /// Single barrier `H` (`Some` for `OneTouch`/`NoTouch`, `None` for the
    /// double-corridor kinds).
    pub barrier: Option<f64>,
    /// Lower corridor wall `L` (`Some` for `Dnt`/`DoubleTouch`).
    pub lower: Option<f64>,
    /// Upper corridor wall `U` (`Some` for `Dnt`/`DoubleTouch`).
    pub upper: Option<f64>,
    /// Rebate (one unit of domestic cash in this grid).
    pub rebate: f64,
    /// Annualized volatility.
    pub vol: f64,
    /// Time to expiry in years.
    pub t: f64,
    /// Domestic rate.
    pub r_dom: f64,
    /// Foreign rate.
    pub r_for: f64,
    /// Reference present value.
    pub price: f64,
}

/// Touch-family product kind in the frozen touch table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchKind {
    /// Single one-touch (deferred / at-expiry rebate): pays if `barrier` is hit.
    OneTouch,
    /// Single no-touch: pays if `barrier` is never hit.
    NoTouch,
    /// Double-no-touch: pays if spot stays inside `(lower, upper)`.
    Dnt,
    /// Double-touch: pays if *either* corridor wall is hit.
    DoubleTouch,
}

/// One frozen double-barrier reference (corridor knock-out / knock-in of a
/// vanilla payoff), with the knock-out priced by QuantLib's independent
/// `AnalyticDoubleBarrierEngine` and the knock-in derived as
/// `vanilla_quantlib − ko_quantlib` (both QuantLib-sourced).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoubleBarrierRecord {
    /// Knock-out or knock-in corridor.
    pub kind: DoubleBarrierKind,
    /// Call or put underlying payoff.
    pub option_type: OptionType,
    /// Spot FX rate.
    pub spot: f64,
    /// Strike of the underlying vanilla.
    pub strike: f64,
    /// Lower corridor wall `L`.
    pub lower: f64,
    /// Upper corridor wall `U`.
    pub upper: f64,
    /// Annualized volatility.
    pub vol: f64,
    /// Time to expiry in years.
    pub t: f64,
    /// Domestic rate.
    pub r_dom: f64,
    /// Foreign rate.
    pub r_for: f64,
    /// Reference present value.
    pub price: f64,
}

/// Corridor knock kind in the frozen double-barrier table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubleBarrierKind {
    /// Knocks out if *either* wall is touched.
    KnockOut,
    /// Knocks in if *either* wall is touched.
    KnockIn,
}

/// Settlement style of a digital in the frozen digital table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigitalSettlement {
    /// Cash-or-nothing: pays the cash `payout` if in the money.
    CashOrNothing,
    /// Asset-or-nothing: pays one unit of the foreign asset (worth `S_T`).
    AssetOrNothing,
}

/// Single-barrier knock kind in the frozen barrier table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierType {
    /// Knocks out if spot trades at or below the barrier.
    DownOut,
    /// Knocks in if spot trades at or below the barrier.
    DownIn,
    /// Knocks out if spot trades at or above the barrier.
    UpOut,
    /// Knocks in if spot trades at or above the barrier.
    UpIn,
}

fn parse_option_type(s: &str) -> Result<OptionType, CsvError> {
    match s {
        "CALL" => Ok(OptionType::Call),
        "PUT" => Ok(OptionType::Put),
        other => Err(CsvError::Parse {
            row: 0,
            column: "option_type".to_owned(),
            value: other.to_owned(),
        }),
    }
}

fn parse_digital_style(s: &str) -> Result<DigitalSettlement, CsvError> {
    match s {
        "CASH" => Ok(DigitalSettlement::CashOrNothing),
        "ASSET" => Ok(DigitalSettlement::AssetOrNothing),
        other => Err(CsvError::Parse {
            row: 0,
            column: "style".to_owned(),
            value: other.to_owned(),
        }),
    }
}

fn parse_barrier_type(s: &str) -> Result<BarrierType, CsvError> {
    match s {
        "DOWN_OUT" => Ok(BarrierType::DownOut),
        "DOWN_IN" => Ok(BarrierType::DownIn),
        "UP_OUT" => Ok(BarrierType::UpOut),
        "UP_IN" => Ok(BarrierType::UpIn),
        other => Err(CsvError::Parse {
            row: 0,
            column: "barrier_type".to_owned(),
            value: other.to_owned(),
        }),
    }
}

fn parse_touch_kind(s: &str) -> Result<TouchKind, CsvError> {
    match s {
        "ONE_TOUCH" => Ok(TouchKind::OneTouch),
        "NO_TOUCH" => Ok(TouchKind::NoTouch),
        "DNT" => Ok(TouchKind::Dnt),
        "DOUBLE_TOUCH" => Ok(TouchKind::DoubleTouch),
        other => Err(CsvError::Parse {
            row: 0,
            column: "kind".to_owned(),
            value: other.to_owned(),
        }),
    }
}

fn parse_double_barrier_kind(s: &str) -> Result<DoubleBarrierKind, CsvError> {
    match s {
        "KO" => Ok(DoubleBarrierKind::KnockOut),
        "KI" => Ok(DoubleBarrierKind::KnockIn),
        other => Err(CsvError::Parse {
            row: 0,
            column: "kind".to_owned(),
            value: other.to_owned(),
        }),
    }
}

/// Path to a named data file under this crate's `data/` directory.
fn data_path(file: &str) -> std::path::PathBuf {
    std::path::Path::new(crate::DATA_DIR).join(file)
}

/// Load the frozen vanilla price+Greek grid (`data/vanilla_gk.csv`).
///
/// # Errors
/// Propagates any [`CsvError`] from reading or parsing the table.
pub fn load_vanilla() -> Result<Vec<VanillaRecord>, CsvError> {
    let t = CsvTable::load(data_path("vanilla_gk.csv"))?;
    let mut out = Vec::with_capacity(t.len());
    for row in 0..t.len() {
        out.push(VanillaRecord {
            option_type: parse_option_type(t.get(row, "option_type")?)?,
            spot: t.get_f64(row, "spot")?,
            strike: t.get_f64(row, "strike")?,
            vol: t.get_f64(row, "vol")?,
            t: t.get_f64(row, "t")?,
            r_dom: t.get_f64(row, "r_dom")?,
            r_for: t.get_f64(row, "r_for")?,
            price: t.get_f64(row, "price")?,
            delta_spot: t.get_f64(row, "delta_spot")?,
            gamma: t.get_f64(row, "gamma")?,
            vega: t.get_f64(row, "vega")?,
            theta: t.get_f64(row, "theta")?,
            rho_dom: t.get_f64(row, "rho_dom")?,
            rho_for: t.get_f64(row, "rho_for")?,
        });
    }
    Ok(out)
}

/// Load the frozen single-barrier table (`data/barrier_gk.csv`).
///
/// # Errors
/// Propagates any [`CsvError`] from reading or parsing the table.
pub fn load_barrier() -> Result<Vec<BarrierRecord>, CsvError> {
    let t = CsvTable::load(data_path("barrier_gk.csv"))?;
    let mut out = Vec::with_capacity(t.len());
    for row in 0..t.len() {
        out.push(BarrierRecord {
            barrier_type: parse_barrier_type(t.get(row, "barrier_type")?)?,
            option_type: parse_option_type(t.get(row, "option_type")?)?,
            spot: t.get_f64(row, "spot")?,
            strike: t.get_f64(row, "strike")?,
            barrier: t.get_f64(row, "barrier")?,
            rebate: t.get_f64(row, "rebate")?,
            vol: t.get_f64(row, "vol")?,
            t: t.get_f64(row, "t")?,
            r_dom: t.get_f64(row, "r_dom")?,
            r_for: t.get_f64(row, "r_for")?,
            price: t.get_f64(row, "price")?,
        });
    }
    Ok(out)
}

/// Load the frozen cash-or-nothing digital table (`data/digital_gk.csv`).
///
/// # Errors
/// Propagates any [`CsvError`] from reading or parsing the table.
pub fn load_digital() -> Result<Vec<DigitalRecord>, CsvError> {
    let t = CsvTable::load(data_path("digital_gk.csv"))?;
    let mut out = Vec::with_capacity(t.len());
    for row in 0..t.len() {
        out.push(DigitalRecord {
            style: parse_digital_style(t.get(row, "style")?)?,
            option_type: parse_option_type(t.get(row, "option_type")?)?,
            spot: t.get_f64(row, "spot")?,
            strike: t.get_f64(row, "strike")?,
            payout: t.get_f64(row, "payout")?,
            vol: t.get_f64(row, "vol")?,
            t: t.get_f64(row, "t")?,
            r_dom: t.get_f64(row, "r_dom")?,
            r_for: t.get_f64(row, "r_for")?,
            price: t.get_f64(row, "price")?,
        });
    }
    Ok(out)
}

/// Load the frozen touch-family table (`data/touch_gk.csv`).
///
/// # Errors
/// Propagates any [`CsvError`] from reading or parsing the table.
pub fn load_touch() -> Result<Vec<TouchRecord>, CsvError> {
    let t = CsvTable::load(data_path("touch_gk.csv"))?;
    let mut out = Vec::with_capacity(t.len());
    for row in 0..t.len() {
        out.push(TouchRecord {
            kind: parse_touch_kind(t.get(row, "kind")?)?,
            spot: t.get_f64(row, "spot")?,
            barrier: t.get_opt_f64(row, "barrier")?,
            lower: t.get_opt_f64(row, "lower")?,
            upper: t.get_opt_f64(row, "upper")?,
            rebate: t.get_f64(row, "rebate")?,
            vol: t.get_f64(row, "vol")?,
            t: t.get_f64(row, "t")?,
            r_dom: t.get_f64(row, "r_dom")?,
            r_for: t.get_f64(row, "r_for")?,
            price: t.get_f64(row, "price")?,
        });
    }
    Ok(out)
}

/// Load the frozen double-barrier table (`data/double_barrier_gk.csv`).
///
/// # Errors
/// Propagates any [`CsvError`] from reading or parsing the table.
pub fn load_double_barrier() -> Result<Vec<DoubleBarrierRecord>, CsvError> {
    let t = CsvTable::load(data_path("double_barrier_gk.csv"))?;
    let mut out = Vec::with_capacity(t.len());
    for row in 0..t.len() {
        out.push(DoubleBarrierRecord {
            kind: parse_double_barrier_kind(t.get(row, "kind")?)?,
            option_type: parse_option_type(t.get(row, "option_type")?)?,
            spot: t.get_f64(row, "spot")?,
            strike: t.get_f64(row, "strike")?,
            lower: t.get_f64(row, "lower")?,
            upper: t.get_f64(row, "upper")?,
            vol: t.get_f64(row, "vol")?,
            t: t.get_f64(row, "t")?,
            r_dom: t.get_f64(row, "r_dom")?,
            r_for: t.get_f64(row, "r_for")?,
            price: t.get_f64(row, "price")?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_table_loads_and_is_well_formed() {
        let recs = load_touch().expect("touch table loads");
        assert!(!recs.is_empty(), "touch grid must be non-empty");
        for r in &recs {
            for v in [r.spot, r.rebate, r.vol, r.t, r.r_dom, r.r_for, r.price] {
                assert!(v.is_finite(), "non-finite touch field: {v}");
            }
            // A touch payout is non-negative and bounded by its *discounted*
            // rebate (which exceeds the face under negative domestic rates).
            let df = (-r.r_dom * r.t).exp();
            assert!(r.price >= -1e-9 && r.price <= r.rebate * df.max(1.0) + 1e-9);
            // Single kinds carry a barrier and no corridor; double kinds the inverse.
            match r.kind {
                TouchKind::OneTouch | TouchKind::NoTouch => {
                    assert!(r.barrier.is_some() && r.lower.is_none() && r.upper.is_none());
                }
                TouchKind::Dnt | TouchKind::DoubleTouch => {
                    assert!(r.barrier.is_none());
                    let (l, u) = (r.lower.expect("lower"), r.upper.expect("upper"));
                    assert!(0.0 < l && l < u, "corridor must satisfy 0<L<U: {l},{u}");
                }
            }
        }
        // Every kind must be exercised by the grid.
        for k in [
            TouchKind::OneTouch,
            TouchKind::NoTouch,
            TouchKind::Dnt,
            TouchKind::DoubleTouch,
        ] {
            assert!(
                recs.iter().any(|r| r.kind == k),
                "touch kind {k:?} missing from grid"
            );
        }
    }

    #[test]
    fn double_barrier_table_loads_and_is_well_formed() {
        let recs = load_double_barrier().expect("double-barrier table loads");
        assert!(!recs.is_empty());
        for r in &recs {
            for v in [r.spot, r.strike, r.lower, r.upper, r.vol, r.t, r.price] {
                assert!(v.is_finite(), "non-finite double-barrier field: {v}");
            }
            assert!(0.0 < r.lower && r.lower < r.upper);
            assert!(
                r.price >= -1e-9,
                "double-barrier price must be non-negative"
            );
        }
        for k in [DoubleBarrierKind::KnockOut, DoubleBarrierKind::KnockIn] {
            assert!(recs.iter().any(|r| r.kind == k), "kind {k:?} missing");
        }
    }

    #[test]
    fn vanilla_table_loads_and_is_finite() {
        let recs = load_vanilla().expect("vanilla table loads");
        assert!(!recs.is_empty(), "vanilla grid must be non-empty");
        for r in &recs {
            for v in [
                r.spot,
                r.strike,
                r.vol,
                r.t,
                r.r_dom,
                r.r_for,
                r.price,
                r.delta_spot,
                r.gamma,
                r.vega,
                r.theta,
                r.rho_dom,
                r.rho_for,
            ] {
                assert!(v.is_finite(), "non-finite reference field: {v}");
            }
            assert!(r.spot > 0.0 && r.strike > 0.0 && r.vol > 0.0 && r.t > 0.0);
        }
    }

    #[test]
    fn barrier_and_digital_tables_load_and_are_finite() {
        let bars = load_barrier().expect("barrier table loads");
        assert!(!bars.is_empty());
        for b in &bars {
            for v in [b.spot, b.strike, b.barrier, b.vol, b.t, b.price] {
                assert!(v.is_finite());
            }
            assert!(b.price >= -1e-9, "barrier price must be non-negative");
        }
        let digs = load_digital().expect("digital table loads");
        assert!(!digs.is_empty());
        for d in &digs {
            for v in [d.spot, d.strike, d.payout, d.vol, d.t, d.price] {
                assert!(v.is_finite());
            }
            // A cash-or-nothing digital is bounded by its discounted cash payout;
            // an asset-or-nothing digital is bounded by the discounted asset, i.e.
            // at most spot (the undiscounted asset value) under non-negative rates,
            // and unconditionally by spot regardless of rate sign within this grid.
            let upper = match d.style {
                DigitalSettlement::CashOrNothing => d.payout,
                DigitalSettlement::AssetOrNothing => d.spot,
            };
            assert!(d.price >= -1e-9 && d.price <= upper + 1e-9);
        }
    }
}
