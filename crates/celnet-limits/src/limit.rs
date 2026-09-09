//! Limit taxonomy, thresholds, utilization and RAG status
//! (`docs/RISK-HIERARCHY.md` §5.1/§5.2).
//!
//! A **limit** is a single risk constraint: *which* exposure it caps
//! ([`LimitMetric`]) and *at what level* it warns vs blocks ([`LimitSpec`]).
//! Limits are dimensioned in the **same units** the cube reports its exposures in
//! (a delta limit is a base-currency delta amount; a vega-bucket limit is in the
//! cube's per-`1.0`-vol vega units; a VaR limit is a loss magnitude), so a
//! utilization is a pure ratio with no convention work — that lives one layer down
//! in `celnet-risk-normalize`, by construction of the canonical leaf.
//!
//! # The taxonomy (RH §5.1, an industry-standard set)
//!
//! - **Greek limits** — delta / gamma / vega / vanna / volga (the FX-specific
//!   additions), per-pair and aggregate, against the additive [`NetGreeks`].
//! - **Bucketed-vega limits** — per `(tenor × delta)` pillar, against the additive
//!   `VegaLadder`.
//! - **Tenor-bucket limits** — the aggregate vega in one tenor across all delta
//!   pillars (the gap/pin-risk concentration cut, RH §5.1).
//! - **Concentration limits** — the gross (un-netted) exposure in a single
//!   dimension slice, capping a single-pair / single-tenor concentration.
//! - **VaR / ES limits** — a non-additive loss magnitude, re-derived per node.
//! - **Stop-loss limits** — a realized/scenario loss magnitude.
//!
//! # Soft vs hard (RH §5.2)
//!
//! Every limit is **soft** (early-warning: utilization crosses an amber/red
//! threshold but the trade is allowed) or **hard** (blocking: a hard breach
//! rejects a pre-trade and triggers escalation post-trade). Utilization =
//! exposure / limit is a **first-class node measure** (RH §5.2) — it is computed
//! here, not derived ad-hoc at the GUI.

use celnet_risk_cube::VegaPillar;

/// Which exposure a limit constrains (`docs/RISK-HIERARCHY.md` §5.1). Each variant
/// names the scalar the [`crate::check`] layer extracts from a cube node aggregate
/// to compare against the limit's threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitMetric {
    /// Net base-currency **delta** amount (additive `NetGreeks::delta_base`).
    Delta,
    /// Net **gamma** (additive `NetGreeks::gamma`).
    Gamma,
    /// Net **vega** per `1.0` absolute vol (additive `NetGreeks::vega`).
    Vega,
    /// Net **vanna** — the FX-specific spot×vol cross-Greek (additive).
    Vanna,
    /// Net **volga** — the FX-specific vol-convexity Greek (additive).
    Volga,
    /// **Vega in one `(tenor × delta)` pillar** (additive `VegaLadder` bucket).
    VegaBucket(VegaPillar),
    /// **Aggregate vega in one tenor** across all delta pillars (the gap/pin-risk
    /// tenor cut, RH §5.1).
    TenorVega {
        /// The tenor pillar (calendar days) whose vega is summed across deltas.
        tenor_days: u32,
    },
    /// **Concentration**: the gross (sum-of-absolute) magnitude of a metric across
    /// the node's constituents — caps a single-pair / single-tenor concentration
    /// even when long/short legs net to near zero (RH §5.1).
    Concentration(ConcentrationMetric),
    /// **Value-at-Risk** — a non-additive loss magnitude (re-derived per node).
    Var,
    /// **Expected Shortfall** — a non-additive tail-loss magnitude.
    ExpectedShortfall,
    /// **Stop-loss** — a realized/scenario loss magnitude cap (RH §5.1).
    StopLoss,
    /// **Net parallel DV01** — the linear fixed-income counterpart to
    /// [`LimitMetric::Delta`]: the book's signed present-value change for a +1bp upward
    /// parallel bump of every discount-curve zero rate
    /// (`celnet_risk_fleet::RatesNodeAggregate::net_dv01`), in the platform's one signed
    /// P&L convention (a rate rise is a loss ⇒ negative for a long bond / receive-fixed
    /// swap; `celnet-rates-risk` ladder). Enforced against the rates risk **aggregate**,
    /// not a Greeks node.
    Dv01,
    /// **Net analytic PV01** — the annuity price-value-of-a-basis-point
    /// (`celnet_risk_fleet::RatesNodeAggregate::net_pv01`), the analytic sibling of
    /// [`LimitMetric::Dv01`] in the same signed convention.
    Pvbp,
    /// **Signed DV01 in one key-rate tenor bucket** — the linear fixed-income
    /// counterpart to [`LimitMetric::TenorVega`]: the net DV01 attributable to one
    /// calibrating-tenor pillar of the key-rate ladder
    /// (`celnet_risk_fleet::KeyRateBucket`), capping a single-tenor curve concentration
    /// even when the parallel DV01 nets small.
    RateTenorBucket {
        /// The calibrating-instrument tenor, in whole years — the ladder bucket key,
        /// matching `celnet_risk_fleet::KeyRateBucket::tenor_years`.
        tenor_years: u32,
    },
}

impl LimitMetric {
    /// Whether this metric constrains **linear fixed-income** rate risk
    /// ([`LimitMetric::Dv01`] / [`LimitMetric::Pvbp`] / [`LimitMetric::RateTenorBucket`])
    /// — the family read off a `celnet_risk_fleet::RatesNodeAggregate` by
    /// [`crate::check::exposure_of_rates`], as opposed to the FX-Greeks family read off a
    /// `celnet_risk_cube::NodeAggregate` by [`crate::check::exposure_of`].
    ///
    /// A single [`crate::tree::LimitTree`] may carry limits from **both** families (a
    /// linear-rates book gates a coarse delta proxy at booking *and* precise DV01/tenor
    /// caps against the aggregate). Each family reads a true `0` (is inert) on the
    /// other's node, and the rates aggregate gate evaluates only this family, so the two
    /// never double-charge.
    #[must_use]
    pub fn is_fixed_income(self) -> bool {
        matches!(
            self,
            LimitMetric::Dv01 | LimitMetric::Pvbp | LimitMetric::RateTenorBucket { .. }
        )
    }
}

/// The underlying additive metric whose **gross** (un-netted) magnitude a
/// concentration limit caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConcentrationMetric {
    /// Gross base-currency delta concentration.
    Delta,
    /// Gross vega concentration.
    Vega,
}

/// Whether a limit warns (early-warning) or blocks (`docs/RISK-HIERARCHY.md` §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Enforcement {
    /// Soft: a breach **warns** but never blocks a trade.
    Soft,
    /// Hard: a breach **blocks** a pre-trade and escalates post-trade.
    Hard,
}

/// A single limit: the metric it caps, the (non-negative) cap amount, the
/// soft early-warning thresholds, and whether it is blocking
/// (`docs/RISK-HIERARCHY.md` §5.1/§5.2).
///
/// `cap` is expressed as a **magnitude** (the limit is two-sided: a delta limit of
/// `10e6` caps `|delta|` at 10mm in either direction; a VaR limit of `5e6` caps a
/// 5mm loss). `amber`/`red` are utilization fractions in `[0, 1]` (RH §5.2's
/// illustrative 80 %/90 % warnings are the default, but they are configurable
/// data). A limit at or below the red threshold is amber; at/above red is red;
/// above the cap is a breach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimitSpec {
    /// What this limit constrains.
    pub metric: LimitMetric,
    /// The cap, as a non-negative magnitude in the metric's native units.
    pub cap: f64,
    /// The amber (early-warning) utilization fraction, in `[0, red]`.
    pub amber: f64,
    /// The red (pre-breach) utilization fraction, in `[amber, 1]`.
    pub red: f64,
    /// Soft (warn-only) or hard (blocking).
    pub enforcement: Enforcement,
}

impl LimitSpec {
    /// A hard limit with the conventional 80 %/90 % amber/red warning bands
    /// (RH §5.2's illustrative defaults).
    #[must_use]
    pub fn hard(metric: LimitMetric, cap: f64) -> Self {
        Self {
            metric,
            cap,
            amber: 0.80,
            red: 0.90,
            enforcement: Enforcement::Hard,
        }
    }

    /// A soft (warn-only) limit with the conventional 80 %/90 % warning bands.
    #[must_use]
    pub fn soft(metric: LimitMetric, cap: f64) -> Self {
        Self {
            metric,
            cap,
            amber: 0.80,
            red: 0.90,
            enforcement: Enforcement::Soft,
        }
    }

    /// Override the amber/red warning bands (utilization fractions). Values are
    /// clamped to `0 ≤ amber ≤ red ≤ 1` so a malformed band can never invert the
    /// RAG ordering.
    #[must_use]
    pub fn with_bands(mut self, amber: f64, red: f64) -> Self {
        let amber = amber.clamp(0.0, 1.0);
        let red = red.clamp(amber, 1.0);
        self.amber = amber;
        self.red = red;
        self
    }

    /// The utilization of a (signed) `exposure` against this limit: the ratio of
    /// the exposure **magnitude** to the cap, in `[0, ∞)`. A zero or negative cap
    /// is treated as "no headroom": any non-zero exposure is fully utilized
    /// (`f64::INFINITY` for a positive exposure against a zero cap), which the RAG
    /// classifier reads as a breach — a limit can never be silently unbounded.
    #[must_use]
    pub fn utilization(&self, exposure: f64) -> f64 {
        let mag = exposure.abs();
        if self.cap > 0.0 {
            mag / self.cap
        } else if mag > 0.0 {
            f64::INFINITY
        } else {
            0.0
        }
    }

    /// Classify a (signed) exposure against this limit into a RAG status.
    #[must_use]
    pub fn classify(&self, exposure: f64) -> Utilization {
        let ratio = self.utilization(exposure);
        let status = if ratio > 1.0 {
            RagStatus::Breach
        } else if ratio >= self.red {
            RagStatus::Red
        } else if ratio >= self.amber {
            RagStatus::Amber
        } else {
            RagStatus::Green
        };
        Utilization {
            metric: self.metric,
            enforcement: self.enforcement,
            exposure,
            cap: self.cap,
            ratio,
            status,
        }
    }
}

/// Traffic-light status of a limit's utilization (`docs/RISK-HIERARCHY.md` §5.2;
/// the RAG overlay of `docs/clients/EXPERIENCE-ARCHITECTURE.md` §"Limits overlay").
///
/// Ordered by severity so the worst status across a set of limits is `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RagStatus {
    /// Below the amber band — comfortable headroom.
    Green,
    /// At/above the amber band, below red — early warning.
    Amber,
    /// At/above the red band, at/below the cap — pre-breach.
    Red,
    /// Over the cap — a breach.
    Breach,
}

impl RagStatus {
    /// Whether this status is a breach (over the cap).
    #[must_use]
    pub fn is_breach(self) -> bool {
        matches!(self, RagStatus::Breach)
    }
}

/// The computed utilization of one limit against a node exposure — a first-class
/// node measure (`docs/RISK-HIERARCHY.md` §5.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Utilization {
    /// Which metric this utilization is for.
    pub metric: LimitMetric,
    /// Whether the limit is soft or hard.
    pub enforcement: Enforcement,
    /// The signed exposure that was measured.
    pub exposure: f64,
    /// The limit cap (magnitude) it was measured against.
    pub cap: f64,
    /// `|exposure| / cap`, in `[0, ∞)`.
    pub ratio: f64,
    /// The RAG classification of `ratio` against the limit's bands.
    pub status: RagStatus,
}

impl Utilization {
    /// The remaining headroom (signed: positive = room left, negative = over the
    /// cap by this magnitude), in the metric's native units. Saturates to
    /// `f64::NEG_INFINITY` headroom when the cap is non-positive and exposure is
    /// non-zero.
    #[must_use]
    pub fn headroom(&self) -> f64 {
        if self.cap > 0.0 {
            self.cap - self.exposure.abs()
        } else if self.exposure.abs() > 0.0 {
            f64::NEG_INFINITY
        } else {
            0.0
        }
    }
}
