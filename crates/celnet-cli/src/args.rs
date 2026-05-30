//! Shared clap value-enum vocabulary mapped onto the domain types.
//!
//! These are the user-facing spellings for option type, delta convention, ATM
//! convention, digital settlement, and barrier topology; each maps to a single
//! domain enum so the subcommand modules stay free of CLI-parsing concerns.

use celnet_exotics::{BarrierKind, BarrierStyle, DigitalKind, DigitalStyle};
use celnet_types::{DeltaConvention, OptionType, VanillaInputs};
use clap::ValueEnum;

/// The shared Garman-Kohlhagen market state for a single quote: spot, flat vol,
/// time, and the two continuously-compounded rates. Carrying these together keeps
/// the subcommand cores to a small, cohesive argument list (and is the natural
/// source for a [`VanillaInputs`] at any strike).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Market {
    /// Spot FX rate (quote per 1 unit of base).
    pub(crate) spot: f64,
    /// Flat annualized volatility (absolute, e.g. 0.10 = 10 vol).
    pub(crate) vol: f64,
    /// Time to expiry in years.
    pub(crate) t: f64,
    /// Continuously-compounded domestic (quote) rate.
    pub(crate) r_dom: f64,
    /// Continuously-compounded foreign (base) rate.
    pub(crate) r_for: f64,
}

impl Market {
    /// The [`VanillaInputs`] for this market state at the given `strike`.
    #[must_use]
    pub(crate) fn inputs(self, strike: f64) -> VanillaInputs {
        VanillaInputs::new(self.spot, strike, self.vol, self.t, self.r_dom, self.r_for)
    }
}

/// Call or put on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum CliOptionType {
    /// Right to buy the base currency.
    Call,
    /// Right to sell the base currency.
    Put,
}

impl From<CliOptionType> for OptionType {
    fn from(v: CliOptionType) -> Self {
        match v {
            CliOptionType::Call => OptionType::Call,
            CliOptionType::Put => OptionType::Put,
        }
    }
}

/// Delta convention on the command line (the four FX deltas).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum CliDeltaConvention {
    /// Spot delta, premium-unadjusted.
    SpotUnadj,
    /// Forward delta, premium-unadjusted.
    ForwardUnadj,
    /// Spot delta, premium-adjusted.
    SpotPa,
    /// Forward delta, premium-adjusted.
    ForwardPa,
}

impl From<CliDeltaConvention> for DeltaConvention {
    fn from(v: CliDeltaConvention) -> Self {
        match v {
            CliDeltaConvention::SpotUnadj => DeltaConvention::SpotUnadjusted,
            CliDeltaConvention::ForwardUnadj => DeltaConvention::ForwardUnadjusted,
            CliDeltaConvention::SpotPa => DeltaConvention::SpotPremiumAdjusted,
            CliDeltaConvention::ForwardPa => DeltaConvention::ForwardPremiumAdjusted,
        }
    }
}

/// Digital cash settlement direction on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum CliDigital {
    /// Cash-or-nothing digital call (pays 1 domestic if `S_T > K`).
    DigitalCall,
    /// Cash-or-nothing digital put (pays 1 domestic if `S_T < K`).
    DigitalPut,
}

impl From<CliDigital> for DigitalKind {
    fn from(v: CliDigital) -> Self {
        match v {
            CliDigital::DigitalCall => DigitalKind {
                style: DigitalStyle::CashOrNothing,
                option: OptionType::Call,
            },
            CliDigital::DigitalPut => DigitalKind {
                style: DigitalStyle::CashOrNothing,
                option: OptionType::Put,
            },
        }
    }
}

/// Single-barrier topology on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum CliBarrier {
    /// Knock-out, barrier below spot.
    DownAndOut,
    /// Knock-out, barrier above spot.
    UpAndOut,
    /// Knock-in, barrier below spot.
    DownAndIn,
    /// Knock-in, barrier above spot.
    UpAndIn,
}

impl CliBarrier {
    /// `true` if the barrier sits above spot.
    const fn up(self) -> bool {
        matches!(self, CliBarrier::UpAndOut | CliBarrier::UpAndIn)
    }

    /// The knock style (in/out).
    const fn style(self) -> BarrierStyle {
        match self {
            CliBarrier::DownAndOut | CliBarrier::UpAndOut => BarrierStyle::KnockOut,
            CliBarrier::DownAndIn | CliBarrier::UpAndIn => BarrierStyle::KnockIn,
        }
    }

    /// The [`BarrierKind`] for the given underlying option type.
    #[must_use]
    pub(crate) fn kind(self, option: OptionType) -> BarrierKind {
        BarrierKind {
            up: self.up(),
            style: self.style(),
            option,
        }
    }
}
