//! Ergonomic constructors and accessors over the generalized wire vocabulary.
//!
//! W1 generalized the contract from FX-only scalars (`r_dom`/`r_for`,
//! `rho_dom`/`rho_for`, a bare `CcyPair pair`) to asset-class-tagged messages
//! ([`Underlying`], [`CarryModel`], [`RateSensitivities`]). The FX arm is still
//! the overwhelmingly common case, so these helpers let the many FX call sites
//! (server pricers, the SDK, the CLI, benches) build and read the FX projection
//! with one call — and, crucially, **bit-for-bit identically** to the former
//! flat-scalar form: [`MarketContext::r_dom`] returns the very `f64` that used to
//! live in the `r_dom` field, [`MarketContext::r_for`] the former `r_for`, and
//! the FX constructors round-trip those exact bits. No new arithmetic is
//! introduced; this is pure (de)structuring of the wire message.

use crate::{
    CarryModel, CcyPair, CommodityRef, CryptoPair, EquityRef, FxRates, Greeks, MarketContext, Metal,
    MetalPair, RateSensitivities, Symbol, Underlying, VanillaInputs, carry_model,
    rate_sensitivities, underlying,
};

impl Underlying {
    /// An FX underlying for `pair`, stamping the settlement (numeraire) currency
    /// as the pair's quote (domestic) leg — the FX convention.
    #[must_use]
    pub fn fx(pair: CcyPair) -> Self {
        let settlement_ccy = pair.quote.clone();
        Underlying {
            r#ref: Some(underlying::Ref::Fx(pair)),
            settlement_ccy,
        }
    }

    /// A metal underlying for `pair`, stamping the settlement (numeraire)
    /// currency as the metal pair's fiat quote leg.
    #[must_use]
    pub fn metal(pair: MetalPair) -> Self {
        let settlement_ccy = pair.quote.clone();
        Underlying {
            r#ref: Some(underlying::Ref::Metal(pair)),
            settlement_ccy,
        }
    }

    /// An equity underlying for `equity`, stamping the settlement (numeraire)
    /// currency as the equity's quote/settlement currency.
    #[must_use]
    pub fn equity(equity: EquityRef) -> Self {
        let settlement_ccy = equity.currency.clone();
        Underlying {
            r#ref: Some(underlying::Ref::Equity(equity)),
            settlement_ccy,
        }
    }

    /// A commodity underlying for `commodity`, stamping the settlement
    /// (numeraire) currency as the commodity's quote/settlement currency.
    #[must_use]
    pub fn commodity(commodity: CommodityRef) -> Self {
        let settlement_ccy = commodity.currency.clone();
        Underlying {
            r#ref: Some(underlying::Ref::Commodity(commodity)),
            settlement_ccy,
        }
    }

    /// A digital-asset (crypto) underlying for `pair`, stamping the settlement
    /// (numeraire) currency as the crypto pair's quote leg (fiat or stablecoin).
    #[must_use]
    pub fn digital_asset(pair: CryptoPair) -> Self {
        let settlement_ccy = pair.quote.clone();
        Underlying {
            r#ref: Some(underlying::Ref::DigitalAsset(pair)),
            settlement_ccy,
        }
    }

    /// The FX pair if this underlying is the FX arm, else `None`.
    #[must_use]
    pub fn as_fx(&self) -> Option<&CcyPair> {
        match &self.r#ref {
            Some(underlying::Ref::Fx(p)) => Some(p),
            Some(
                underlying::Ref::Metal(_)
                | underlying::Ref::Equity(_)
                | underlying::Ref::Commodity(_)
                | underlying::Ref::DigitalAsset(_),
            )
            | None => None,
        }
    }

    /// The metal pair if this underlying is the metal arm, else `None`.
    #[must_use]
    pub fn as_metal(&self) -> Option<&MetalPair> {
        match &self.r#ref {
            Some(underlying::Ref::Metal(m)) => Some(m),
            Some(
                underlying::Ref::Fx(_)
                | underlying::Ref::Equity(_)
                | underlying::Ref::Commodity(_)
                | underlying::Ref::DigitalAsset(_),
            )
            | None => None,
        }
    }

    /// The equity reference if this underlying is the equity arm, else `None`.
    #[must_use]
    pub fn as_equity(&self) -> Option<&EquityRef> {
        match &self.r#ref {
            Some(underlying::Ref::Equity(e)) => Some(e),
            Some(
                underlying::Ref::Fx(_)
                | underlying::Ref::Metal(_)
                | underlying::Ref::Commodity(_)
                | underlying::Ref::DigitalAsset(_),
            )
            | None => None,
        }
    }

    /// The commodity reference if this underlying is the commodity arm, else
    /// `None`.
    #[must_use]
    pub fn as_commodity(&self) -> Option<&CommodityRef> {
        match &self.r#ref {
            Some(underlying::Ref::Commodity(c)) => Some(c),
            Some(
                underlying::Ref::Fx(_)
                | underlying::Ref::Metal(_)
                | underlying::Ref::Equity(_)
                | underlying::Ref::DigitalAsset(_),
            )
            | None => None,
        }
    }

    /// The crypto pair if this underlying is the digital-asset arm, else `None`.
    #[must_use]
    pub fn as_digital_asset(&self) -> Option<&CryptoPair> {
        match &self.r#ref {
            Some(underlying::Ref::DigitalAsset(p)) => Some(p),
            Some(
                underlying::Ref::Fx(_)
                | underlying::Ref::Metal(_)
                | underlying::Ref::Equity(_)
                | underlying::Ref::Commodity(_),
            )
            | None => None,
        }
    }
}

impl MetalPair {
    /// A metal pair from a domain [`Metal`] and its fiat quote leg code.
    #[must_use]
    pub fn new(metal: Metal, quote: impl Into<String>) -> Self {
        MetalPair {
            metal: metal as i32,
            quote: quote.into(),
        }
    }
}

impl Symbol {
    /// A symbol from its ticker and listing venue (pass an empty venue when the
    /// ticker is globally unambiguous).
    #[must_use]
    pub fn new(ticker: impl Into<String>, venue: impl Into<String>) -> Self {
        Symbol {
            ticker: ticker.into(),
            venue: venue.into(),
        }
    }
}

impl EquityRef {
    /// An equity reference from its listed symbol and quote/settlement currency.
    #[must_use]
    pub fn new(symbol: Symbol, currency: impl Into<String>) -> Self {
        EquityRef {
            symbol: Some(symbol),
            currency: currency.into(),
        }
    }
}

impl CommodityRef {
    /// A commodity reference from its symbol and quote/settlement currency.
    #[must_use]
    pub fn new(symbol: Symbol, currency: impl Into<String>) -> Self {
        CommodityRef {
            symbol: Some(symbol),
            currency: currency.into(),
        }
    }
}

impl CryptoPair {
    /// A crypto pair from its coin (base) leg and its numeraire (quote) leg.
    #[must_use]
    pub fn new(base: impl Into<String>, quote: impl Into<String>) -> Self {
        CryptoPair {
            base: base.into(),
            quote: quote.into(),
        }
    }
}

impl CarryModel {
    /// The FX two-rate carry arm carrying the foreign (base) rate `r_for`.
    #[must_use]
    pub fn fx(r_for: f64) -> Self {
        CarryModel {
            model: Some(carry_model::Model::Fx(FxRates { r_for })),
        }
    }

    /// The foreign (base) rate if this is the FX carry arm, else `None`.
    #[must_use]
    pub fn fx_r_for(&self) -> Option<f64> {
        match &self.model {
            Some(carry_model::Model::Fx(fx)) => Some(fx.r_for),
            _ => None,
        }
    }
}

impl MarketContext {
    /// Build an FX market context from spot, vol and the two FX rates — the FX
    /// projection of the generalized `{discount_rate, carry}` form. `discount_rate`
    /// is set to `r_dom` and `carry` to the FX arm carrying `r_for`, so
    /// [`MarketContext::r_dom`]/[`MarketContext::r_for`] return these exact bits.
    #[must_use]
    pub fn fx(spot: f64, vol: f64, r_dom: f64, r_for: f64) -> Self {
        MarketContext {
            spot,
            vol,
            discount_rate: r_dom,
            carry: Some(CarryModel::fx(r_for)),
        }
    }

    /// The domestic (quote) rate `r_dom` — exactly the `discount_rate` field. For
    /// FX the discount rate *is* `r_dom`, so this is the former flat scalar.
    #[must_use]
    pub fn r_dom(&self) -> f64 {
        self.discount_rate
    }

    /// The foreign (base) rate `r_for` — the FX carry arm's `r_for`, or `0.0` if
    /// the carry is absent / not the FX arm (a non-FX context never reaches the
    /// FX leaf; the validity guard rejects that before pricing).
    #[must_use]
    pub fn r_for(&self) -> f64 {
        self.carry
            .as_ref()
            .and_then(CarryModel::fx_r_for)
            .unwrap_or(0.0)
    }

    /// This context with `spot` replaced (used by finite-difference spot bumps);
    /// the carry/vol/discount are cloned unchanged.
    #[must_use]
    pub fn with_spot(&self, spot: f64) -> Self {
        MarketContext { spot, ..*self }
    }

    /// This context with `vol` replaced (used by finite-difference vega bumps).
    #[must_use]
    pub fn with_vol(&self, vol: f64) -> Self {
        MarketContext { vol, ..*self }
    }

    /// This context with the domestic rate `r_dom` (the `discount_rate`) replaced
    /// (used by finite-difference rho-domestic bumps); the carry is unchanged.
    #[must_use]
    pub fn with_r_dom(&self, r_dom: f64) -> Self {
        MarketContext {
            discount_rate: r_dom,
            ..*self
        }
    }

    /// This context with the foreign rate `r_for` replaced in the FX carry arm
    /// (used by finite-difference rho-foreign bumps); spot/vol/discount unchanged.
    #[must_use]
    pub fn with_r_for(&self, r_for: f64) -> Self {
        MarketContext {
            carry: Some(CarryModel::fx(r_for)),
            ..*self
        }
    }
}

impl VanillaInputs {
    /// Build FX vanilla inputs — the FX projection of the generalized
    /// `{discount_rate, carry}` form (`discount_rate = r_dom`, `carry.fx.r_for =
    /// r_for`). Round-trips the two rates' exact bits via [`VanillaInputs::r_dom`]/
    /// [`VanillaInputs::r_for`].
    #[must_use]
    pub fn fx(spot: f64, strike: f64, vol: f64, t: f64, r_dom: f64, r_for: f64) -> Self {
        VanillaInputs {
            spot,
            strike,
            vol,
            t,
            discount_rate: r_dom,
            carry: Some(CarryModel::fx(r_for)),
        }
    }

    /// The domestic (quote) rate `r_dom` — exactly the `discount_rate` field.
    #[must_use]
    pub fn r_dom(&self) -> f64 {
        self.discount_rate
    }

    /// The foreign (base) rate `r_for` — the FX carry arm's `r_for`, or `0.0` if
    /// the carry is absent / not the FX arm.
    #[must_use]
    pub fn r_for(&self) -> f64 {
        self.carry
            .as_ref()
            .and_then(CarryModel::fx_r_for)
            .unwrap_or(0.0)
    }
}

impl RateSensitivities {
    /// The FX two-rho arm (`rho_dom`, `rho_for`) — the FX projection of the
    /// generalized rate Greeks; round-trips both rhos' exact bits.
    #[must_use]
    pub fn fx(rho_dom: f64, rho_for: f64) -> Self {
        RateSensitivities {
            sensitivities: Some(rate_sensitivities::Sensitivities::Fx(
                rate_sensitivities::FxRho { rho_dom, rho_for },
            )),
        }
    }

    /// The FX rhos `(rho_dom, rho_for)` if this is the FX arm, else `None`.
    #[must_use]
    pub fn fx_rhos(&self) -> Option<(f64, f64)> {
        match &self.sensitivities {
            Some(rate_sensitivities::Sensitivities::Fx(fx)) => Some((fx.rho_dom, fx.rho_for)),
            _ => None,
        }
    }
}

impl Greeks {
    /// The domestic rho if these Greeks carry the FX rate-sensitivity arm, else
    /// `0.0` (the price-only / non-FX strip).
    #[must_use]
    pub fn rho_dom(&self) -> f64 {
        self.rate_sensitivities
            .as_ref()
            .and_then(RateSensitivities::fx_rhos)
            .map_or(0.0, |(d, _)| d)
    }

    /// The foreign rho if these Greeks carry the FX rate-sensitivity arm, else
    /// `0.0`.
    #[must_use]
    pub fn rho_for(&self) -> f64 {
        self.rate_sensitivities
            .as_ref()
            .and_then(RateSensitivities::fx_rhos)
            .map_or(0.0, |(_, f)| f)
    }
}
