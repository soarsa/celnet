//! Linear fixed-income positions that reprice off a discount curve.
//!
//! Each variant delegates its valuation to the **existing** celnet-rates / celnet-bond pricing — the
//! same math the landed server `RatesOisEngine` / `BondEngine` use — so a scenario reprice is a
//! plain re-evaluation on a shocked [`Curve`], never a re-implementation of the pricing.

use celnet_bond::{Bond, price_from_curve};
use celnet_rates::{Curve, OisSchedule, ois_pv};
use celnet_types::Rate;

use crate::error::RateRiskError;

/// A linear fixed-income position priced off a single discount curve.
///
/// Both variants value **through the discount curve**, which is exactly why one shocked curve
/// reprices a heterogeneous FI book without the scenario engine branching on the instrument.
#[derive(Clone, Debug)]
pub enum FiPosition {
    /// A fixed-vs-OIS swap on a self-discounting curve, valued by [`ois_pv`].
    ///
    /// `ois_pv` returns the **receive-fixed** present value `N·(K·A − (DF(start) − DF(maturity)))`;
    /// `receive_fixed = false` negates it for a pay-fixed position.
    OisSwap {
        /// The fixed-leg schedule (payment times + accruals) in curve year-fraction coordinates.
        schedule: OisSchedule,
        /// The fixed rate `K` paid/received.
        fixed_rate: Rate,
        /// The swap notional `N` (currency units).
        notional: f64,
        /// `true` to receive fixed (the raw [`ois_pv`] sign), `false` to pay fixed (negated).
        receive_fixed: bool,
    },
    /// A cash bond holding, valued by [`price_from_curve`] scaled by the number of units held.
    ///
    /// [`price_from_curve`] returns the dirty price per the bond's own `redemption` face; `holdings`
    /// scales it to the position (`holdings = 1` is one bond of `redemption` face).
    CashBond {
        /// The bond definition (coupon, schedule, redemption, day count).
        bond: Bond,
        /// The number of per-redemption units held (the position value = price · `holdings`).
        holdings: f64,
    },
}

impl FiPosition {
    /// Present value of the position on `curve`, through the wrapped celnet-rates / celnet-bond
    /// pricing.
    ///
    /// # Errors
    ///
    /// [`RateRiskError::Bond`] if a [`FiPosition::CashBond`] cannot be scheduled/priced. The
    /// [`FiPosition::OisSwap`] arm is infallible (a valid schedule always prices on a valid curve).
    pub fn pv_on_curve(&self, curve: &Curve) -> Result<f64, RateRiskError> {
        match self {
            Self::OisSwap {
                schedule,
                fixed_rate,
                notional,
                receive_fixed,
            } => {
                let receive = ois_pv(curve, schedule, *fixed_rate, *notional);
                Ok(if *receive_fixed { receive } else { -receive })
            }
            Self::CashBond { bond, holdings } => {
                let price = price_from_curve(bond, curve)?;
                Ok(price * holdings)
            }
        }
    }
}
