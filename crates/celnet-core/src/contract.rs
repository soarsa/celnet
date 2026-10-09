//! The central pricing + risk **contract** — the asset-class-agnostic seam every
//! product family (FX/metals options today; equity/commodity/crypto and linear FI
//! as they migrate) prices and risks against.
//!
//! This is ADR-0017 **Phase A1**: *define the one contract and prove it on the FX
//! options path with ZERO numeric drift.* It formalises what is already latent in
//! the platform (`docs/plan/CENTRAL-CORE-UNIFICATION.md`): the shared
//! [`DiscountCurve`] discounting seam, the asset-class-agnostic carry kernel
//! ([`crate::carry`]), and the additive-then-nonadditive risk split.
//!
//! # Why the contract lives here (celnet-core) and stays acyclic
//!
//! `celnet-core` sits just above `celnet-types` and depends on **nothing else**
//! (see the crate docs) — it is the natural, cycle-free home for a cross-crate
//! seam, exactly like the existing [`crate::CarryPricer`] / [`crate::ExoticLegPricer`]
//! inversions. But the concrete request-tier types the contract binds —
//! `Priced`, the per-request engine context, the server `ConventionSet`, the
//! `celnet-surface` vol surface, and the `celnet-rates` bootstrapped `Curve` — all
//! live *above* core (in `celnet-server` / `celnet-surface` / `celnet-rates`).
//! Naming them here would invert the dependency graph into a cycle.
//!
//! The contract therefore **abstracts** those types behind generics + associated
//! (GAT) types: core owns the *shape* of the one contract; each leaf `impl`
//! (in `celnet-server`, a one-way `→ celnet-core` edge) binds the *concrete* types
//! once. There is exactly one binding in practice — this is not "many contracts",
//! it is one contract whose irreducibly-upstream types are supplied by the leaf.
//!
//! # Hot-core embargo (ADR-0016)
//!
//! [`ResolvedMarket`] is a **borrowed, request/batch-tier** handle (critique F4):
//! it models per-request market resolution and NEVER enters the pinned streaming
//! `celnet-engine::MarketState` (flat `f64`, published lock-free). Its borrowed
//! `&dyn DiscountCurve` fields carry a non-`'static` lifetime, so the type system
//! itself forbids it as a field of the `'static`, `ArcSwap`-published hot state —
//! asserted by the embargo test delivered alongside this phase
//! (`celnet-engine::rt::tests::hot_core_embargoes_request_tier_curve_handles`,
//! critique F5).

use celnet_types::DiscountCurve;

use crate::carry::CarryGreeks;
use crate::math::exp;

/// The **degenerate flat (one-pillar) term structure** — a single continuously-
/// compounded rate `r` as a [`DiscountCurve`] (`DF(t) = e^{−r·t}`, `ADR-0010`).
///
/// This is the general-curve seam's trivial case: a bootstrapped
/// `celnet_rates::curve::Curve` is the full term structure; this is the one-rate
/// limit. A request-tier [`ResolvedMarket`] whose asset class is a flat two-rate
/// carry (FX / metals) resolves its domestic and foreign legs to a pair of these.
///
/// Fully implemented (not a placeholder): the flat curve every discounting seam
/// can consume through the shared [`DiscountCurve`] trait.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlatDiscountCurve {
    /// The continuously-compounded flat rate `r`.
    pub rate: f64,
}

impl FlatDiscountCurve {
    /// Construct a flat one-pillar discount curve at continuously-compounded `rate`.
    #[must_use]
    pub const fn new(rate: f64) -> Self {
        Self { rate }
    }
}

impl DiscountCurve for FlatDiscountCurve {
    #[inline]
    fn discount_factor(&self, t: f64) -> f64 {
        exp(-self.rate * t)
    }
}

/// A purely-**additive** rate-risk ladder — the linear FI risk measure.
///
/// This is the unified vocabulary for the additive rate sensitivities (present
/// value, parallel PV01/DV01) that linear FI contributes to the risk cube *without*
/// the nonlinear (VaR/ES/FRTB) machinery an option needs. It is deliberately
/// allocation-free (celnet-core cannot allocate): the per-tenor key-rate bucket
/// ladder — which needs an owned collection — is populated by the FI leaf at the
/// request tier in Phase B, which maps `celnet_rates`' rate risk into this measure.
///
/// The measure is additive at the linear layer (a portfolio's ladder is the sum of
/// its legs'), pinned by [`RateLadder::combine`] / [`core::ops::Add`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLadder {
    /// Present value in the numeraire currency.
    pub pv: f64,
    /// Sensitivity to a 1bp parallel shift of the *fixed* rate (PV01).
    pub pv01: f64,
    /// Sensitivity to a 1bp parallel shift of the *discount* curve (DV01).
    pub dv01: f64,
}

impl RateLadder {
    /// The additive identity (a zero position): all sensitivities zero.
    pub const ZERO: Self = Self {
        pv: 0.0,
        pv01: 0.0,
        dv01: 0.0,
    };

    /// Construct a rate ladder from its present value and parallel sensitivities.
    #[must_use]
    pub const fn new(pv: f64, pv01: f64, dv01: f64) -> Self {
        Self { pv, pv01, dv01 }
    }

    /// Field-wise additive combination — the linear roll-up of two positions'
    /// ladders. Additive by construction (no optionality ⇒ no curvature term).
    #[must_use]
    pub fn combine(self, other: Self) -> Self {
        Self {
            pv: self.pv + other.pv,
            pv01: self.pv01 + other.pv01,
            dv01: self.dv01 + other.dv01,
        }
    }
}

impl core::ops::Add for RateLadder {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        self.combine(rhs)
    }
}

/// The unified, **tagged risk measure** every asset class reports (design §3, (4)).
///
/// One measure type flows into one risk cube regardless of asset class. The tags
/// are *additive* — a portfolio carries option Greeks *and* rate ladders
/// side-by-side, each summed within its tag; the non-additive VaR/ES/FRTB layer
/// (Phase C) re-derives over the union. Adding an asset class is an additive tag
/// (e.g. a future credit-spread arm), never a contract break (GUIDE.md rule 9).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RiskMeasure {
    /// Options / cross-asset: the generalized 13-member Greek strip (nonlinear;
    /// the VaR/FRTB layer re-derives it under spot/vol scenarios). The FX vanilla
    /// leaf reports this arm via [`crate::fx_carry_greeks`].
    OptionGreeks(CarryGreeks),
    /// Linear FI / rates: the purely-additive PV + parallel ladder. Produced by the
    /// FI leaf in Phase B; sums bit-exactly at the linear layer.
    RateLadder(RateLadder),
}

/// The resolved market a [`Priceable`] prices against — the **request / batch /
/// surface-tier** market handle (critique F4).
///
/// It is the one handle every pricer consumes: discounting is the already-shared
/// [`DiscountCurve`] seam (a flat [`FlatDiscountCurve`]/[`celnet_types::Carry`] for
/// a two-rate carry, a bootstrapped `celnet_rates` curve for FI — interchangeable
/// behind the trait). It carries the resolved spot and volatility, and — per F4 —
/// the trade **conventions**, so a leaf can resolve a delta-keyed strike through
/// the convention set's delta solver (the FX path threads exactly this today as
/// `conv: &ConventionSet`).
///
/// Generic over the convention carrier `C` (the server `ConventionSet`) so the
/// contract stays at the bottom of the dependency graph; the leaf binds `C` once.
///
/// # Borrowed, never owned (the embargo)
///
/// Every field is a borrow or a `Copy` scalar — `ResolvedMarket` holds no owned
/// heap handle, so celnet-core stays allocation-free, and its non-`'static`
/// lifetime `'a` structurally bars it (and its `&dyn DiscountCurve` legs) from the
/// `'static` hot-core `MarketState` (ADR-0016; asserted by the F5 embargo test).
///
/// # Volatility at the request tier
///
/// `vol` is the **resolved scalar** Black volatility the request-tier price uses
/// (for a pinned FX price the marked-surface pin — `resolve_pinned_vol` — already
/// stamped it upstream). A full smile/surface *handle* (for a smile-sampling
/// exotic leg) is the Phase-A2 extension of this contract, added additively.
pub struct ResolvedMarket<'a, C: ?Sized> {
    /// The numeraire / discount curve — supplies `DF_dom(0,t)` and the discounting.
    pub discount: &'a dyn DiscountCurve,
    /// The foreign / asset-leg curve — supplies `DF_for(0,t)` for the outright
    /// forward (the second leg of an FX two-curve carry). `None` for a
    /// single-curve (pure-rates) market.
    pub foreign: Option<&'a dyn DiscountCurve>,
    /// The resolved scalar Black volatility (`None` for a linear/pure-rates market).
    pub vol: Option<f64>,
    /// The resolved spot (`None` for a pure-rates market).
    pub spot: Option<f64>,
    /// The resolved trade conventions — carries the delta-key solver seam (F4).
    pub conventions: &'a C,
}

impl<'a, C: ?Sized> ResolvedMarket<'a, C> {
    /// Assemble a resolved market from its discounting legs, resolved spot/vol, and
    /// conventions.
    #[must_use]
    pub fn new(
        discount: &'a dyn DiscountCurve,
        foreign: Option<&'a dyn DiscountCurve>,
        vol: Option<f64>,
        spot: Option<f64>,
        conventions: &'a C,
    ) -> Self {
        Self {
            discount,
            foreign,
            vol,
            spot,
            conventions,
        }
    }
}

/// The universal **market-resolution** seam (design §3, (2)): "get me the market
/// this needs." One resolver per market kind — the FX path snapshots the marked
/// [`SurfaceBook`](https://docs) pin; FI bootstraps a curve — both yield a
/// [`ResolvedMarket`].
///
/// Request-tier only (F4): this models per-request/batch resolution; the FX
/// *streaming* hot path keeps its own flat `celnet-engine::MarketState`.
///
/// The associated `Market<'a>` is a GAT so the produced [`ResolvedMarket`] can
/// borrow from the resolver (which owns the resolved legs) — the standard
/// lending pattern.
pub trait MarketResolver {
    /// Per-resolution input (e.g. the pinned instrument + surface version).
    type Request;
    /// The borrowed resolved-market handle produced (a [`ResolvedMarket`]).
    type Market<'a>
    where
        Self: 'a;
    /// Why resolution failed (unknown surface pin, uncalibrated tenor, …).
    type Error;

    /// Resolve the market for `request` into a borrowed [`ResolvedMarket`].
    ///
    /// # Errors
    /// Returns [`Self::Error`] when the market cannot be resolved (e.g. a pinned
    /// surface version that was never marked).
    fn resolve(&self, request: &Self::Request) -> Result<Self::Market<'_>, Self::Error>;
}

/// The universal **pricing + risk** seam (design §3, (1)): every product family
/// implements it. Generalises the existing per-family `ProductEngine` — a leaf's
/// [`price`](Priceable::price) is byte-identical to its established pricing body;
/// [`risk`](Priceable::risk) reports the unified tagged [`RiskMeasure`].
///
/// The associated types bind the irreducibly-upstream request-tier types once at
/// the leaf: `Market<'a>` = a [`ResolvedMarket`] over the concrete conventions,
/// `Ctx<'a>` = the per-request engine context, `Priced` = the priced result,
/// `Error` = the pricing error. celnet-core names none of them, staying acyclic.
pub trait Priceable {
    /// The resolved-market handle this leaf reads (a [`ResolvedMarket`]).
    type Market<'a>;
    /// The per-request engine context (borrowed instrument + market + conventions).
    type Ctx<'a>;
    /// The priced result of a successful [`price`](Priceable::price).
    type Priced;
    /// Why pricing failed.
    type Error;

    /// Present value + the priced result for this product against `market`/`ctx`.
    ///
    /// # Errors
    /// Returns [`Self::Error`] for a malformed product or an out-of-domain input.
    fn price(
        &self,
        market: &Self::Market<'_>,
        ctx: &Self::Ctx<'_>,
    ) -> Result<Self::Priced, Self::Error>;

    /// The product's risk in the unified tagged [`RiskMeasure`] form.
    ///
    /// # Errors
    /// Returns [`Self::Error`] for a malformed product or an out-of-domain input.
    fn risk(
        &self,
        market: &Self::Market<'_>,
        ctx: &Self::Ctx<'_>,
    ) -> Result<RiskMeasure, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::is_close;
    use celnet_types::{Carry, RateSensitivities};

    /// The flat one-pillar curve reproduces `e^{−r·t}` and matches the flat
    /// [`Carry`] domestic discount to within a rounding ULP (both are a single
    /// `exp`) — the ADR-0010 degenerate-curve bridge.
    #[test]
    fn flat_curve_is_the_degenerate_discount() {
        for &r in &[0.0, 0.02, -0.01, 0.075] {
            let c = FlatDiscountCurve::new(r);
            for &t in &[0.25_f64, 1.0, 5.0] {
                assert!(is_close(c.discount_factor(t), exp(-r * t), 1e-15, 1e-15));
                // Matches the domestic leg of a flat FX carry (both `e^{−r_dom·t}`).
                let fx = Carry::FxRates {
                    r_dom: r,
                    r_for: 0.037,
                };
                assert!(is_close(
                    c.discount_factor(t),
                    fx.discount_factor(t),
                    1e-15,
                    1e-15
                ));
            }
        }
    }

    /// The rate ladder is additive: combining two positions sums each sensitivity,
    /// `ZERO` is the identity, and `+` agrees with `combine`.
    #[test]
    fn rate_ladder_is_additive() {
        let a = RateLadder::new(1000.0, 12.5, -12.4);
        let b = RateLadder::new(-250.0, 3.0, -3.1);
        let s = a.combine(b);
        assert_eq!(s.pv, 750.0);
        assert_eq!(s.pv01, 15.5);
        assert!(is_close(s.dv01, -15.5, 1e-12, 1e-12));
        assert_eq!(a.combine(RateLadder::ZERO), a);
        assert_eq!(a + b, s);
    }

    /// The risk measure carries either tag; the option arm wraps the generalized
    /// Greek strip, the rate arm the additive ladder. (A `ResolvedMarket` binding
    /// its conventions to a `Copy` unit stands in for the acyclicity check.)
    #[test]
    fn risk_measure_tags_both_asset_classes() {
        let greeks = CarryGreeks {
            price: 1.0,
            delta_spot: 0.5,
            delta_forward: 0.55,
            gamma: 0.01,
            vega: 0.2,
            theta: -0.03,
            rates: RateSensitivities::Fx {
                rho_dom: 0.1,
                rho_for: -0.05,
            },
            vanna: 0.0,
            volga: 0.0,
            charm: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
        };
        let option = RiskMeasure::OptionGreeks(greeks);
        let rates = RiskMeasure::RateLadder(RateLadder::new(500.0, 5.0, -5.0));
        assert_ne!(option, rates);
        match option {
            RiskMeasure::OptionGreeks(g) => assert_eq!(g.price, 1.0),
            RiskMeasure::RateLadder(_) => panic!("expected the option arm"),
        }
    }

    /// A `ResolvedMarket` is a genuinely-populated borrowed handle: its flat FX
    /// legs discount correctly and its resolved spot/vol/conventions read back.
    #[test]
    fn resolved_market_is_a_real_borrowed_handle() {
        let dom = FlatDiscountCurve::new(0.05);
        let for_ = FlatDiscountCurve::new(0.02);
        let conv = (); // a Copy stand-in convention carrier for the core-level test
        let mkt = ResolvedMarket::new(&dom, Some(&for_), Some(0.1), Some(1.10), &conv);
        assert_eq!(mkt.spot, Some(1.10));
        assert_eq!(mkt.vol, Some(0.1));
        assert!(is_close(
            mkt.discount.discount_factor(1.0),
            exp(-0.05),
            1e-15,
            1e-15
        ));
        assert!(is_close(
            mkt.foreign.unwrap().discount_factor(1.0),
            exp(-0.02),
            1e-15,
            1e-15
        ));
    }
}
