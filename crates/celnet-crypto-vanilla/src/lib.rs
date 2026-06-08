//! Digital-asset (crypto) vanilla option pricing on the cost-of-carry seam — the
//! linear (USD-margined) and inverse (coin-margined) European vanilla, with the full
//! desk Greek strip.
//!
//! This is the crypto sibling of the FX (`celnet-vanilla`), equity
//! (`celnet-equity-vanilla`), and commodity (`celnet-commodity-vanilla`) leaves.
//! Like them it prices off the asset-class-agnostic cost-of-carry seam
//! ([`celnet_types::Carry::CostOfCarry`]): a forward `F = S·e^{b·t}` and a numeraire
//! discount `e^{−r·t}`, where for a digital asset the net carry is
//!
//! ```text
//! b = r − funding         (funding = the coin's perpetual-funding / lease / staking yield)
//! ```
//!
//! ([`funding::funding_carry`]). Crypto splits into TWO settlement styles
//! ([`settlement::SettlementStyle`]):
//!
//! * **Linear / USD(T)-margined** ([`linear`]) — premium and payoff in USD(T). This
//!   is *exactly* the shared asset-class-agnostic generalized-BSM payoff math (zero
//!   new math); the leaf reads forward/discount **only** through the carry seam and
//!   never matches on a [`celnet_types::Carry`] or [`celnet_types::Underlying`]
//!   variant (ADR-0008 "no-match-carry").
//! * **Inverse / coin-margined** ([`inverse`]) — premium and payoff in the **base
//!   coin**: per USD-notional-1 the contract pays `max(φ(S_T − K), 0) / S_T` coins.
//!   The `1/S_T` factor is a genuine non-linear convexity transform — **not** a
//!   rescaled vanilla. The exact coin price is derived from the USD risk-neutral
//!   expectation of the `1/S_T`-weighted payoff (full derivation in [`inverse`]):
//!
//!   ```text
//!   V_coin = φ·df·[ Φ(φ·d2) − (K/F)·e^{σ²t}·Φ(φ·d3) ]   coins,
//!   ```
//!
//!   with `d2 = d1 − σ√t`, `d3 = d1 − 2σ√t`. The naive `V_lin / S₀` is WRONG (it
//!   drops the material `Cov(1/S_T, payoff)` convexity term and assumes
//!   `E^Q[1/S_T] = 1/S₀`, when in fact `E^Q[1/S_T] = e^{−bt}·e^{σ²t}/S₀`). The
//!   coin-measure Greeks ([`inverse::InverseGreeks`]) are what a coin-margined desk
//!   hedges with; the USD-equivalent is exposed as a derived field.
//!
//! Every Greek in both leaves is cross-validated against central finite differences,
//! and the prices are gated three independent ways for the inverse (a code-disjoint
//! splitmix64 + Box-Muller MC within its reported standard error, a high-resolution
//! deterministic quadrature of the raw `1/S_T`-weighted payoff, and the SIGNED
//! structural convexity sandwich — `V_coin·S₀ < V_lin` for calls, `> V_lin` for puts,
//! strictly, which a naive `V_lin/S₀` rescale violates) and via the `funding → r_for`
//! identity through the already-golden FX leaf for the linear.
//!
//! ## Honest boundary (W3 §9 — deploy-bound carve-out)
//!
//! In-repo this crate proves the **payoff math, the measure correctness, the carry
//! assembly, and the convention identity**. The following are **ENV** (exchange/venue
//! data), seamed but never fabricated in-repo: live perpetual-funding VALUES, live
//! crypto vol surfaces / option-chain quote VALUES, and live index-price settlement
//! fixing VALUES. Only the fixing/convention *identity* (which coin, which UTC cut,
//! which index definition — pinned in [`deribit`] from the published spec) is in-repo.

#![forbid(unsafe_code)]

pub mod funding;
pub mod inverse;
pub mod linear;
pub mod settlement;

pub use funding::funding_carry;
pub use inverse::{InverseGreeks, InverseInputs};
pub use linear::LinearInputs;
pub use settlement::{SettlementStyle, route_price};

/// Published Deribit BTC/ETH option **contract conventions** — identity/structure
/// only, hand-pinned from the primary venue specification with the citation.
///
/// Source: **Deribit "Options" / "Option specification" knowledge-base pages**
/// (deribit.com/kb), the canonical European BTC/ETH options venue. These are the
/// settlement/convention *identities* the platform must encode; the live fixing
/// **values** are ENV (the honest boundary above). Re-pinned here so a mis-encoded
/// contract unit, settlement coin, or expiry cut fails a test, not production.
pub mod deribit {
    /// Contract style: **European**, cash-settled, exercised automatically at expiry.
    pub const STYLE_EUROPEAN: &str = "European";

    /// Settlement is **inverse / coin-margined**: the option is denominated and
    /// settled in the **base coin** (BTC for BTC options, ETH for ETH options), not
    /// in USD. One contract corresponds to 1 unit of the underlying coin of
    /// USD-index exposure; P&L accrues in the coin. (Deribit KB "Option
    /// specification": "Options are … settled in … the base currency".)
    pub const SETTLEMENT_IS_INVERSE_COIN: bool = true;

    /// Contract unit: **1 coin** of underlying per contract (BTC or ETH). (Deribit KB:
    /// "Contract size: 1 BTC" / "1 ETH".)
    pub const CONTRACT_UNIT_COINS: f64 = 1.0;

    /// Expiry cut: **08:00 UTC** on the expiration date. (Deribit KB "Expiration":
    /// settlement at 08:00 UTC.) Stored as seconds past midnight UTC.
    pub const EXPIRY_CUT_UTC_SECONDS: u32 = 8 * 3_600;

    /// Settlement / index fixing: the **Deribit BTC (resp. ETH) index** — a
    /// volume/sanity-filtered average of constituent spot venues, sampled into the
    /// expiry; the option settles against this index print at the 08:00 UTC cut.
    /// (Deribit KB "Deribit BTC Index / ETH Index".) The numeric index VALUE is ENV.
    pub const SETTLEMENT_FIXING: &str = "Deribit index (08:00 UTC print)";

    /// Premium tick / quotation: option **premium is quoted in the base coin** (e.g.
    /// BTC), with a venue tick of **0.0005** coin. (Deribit KB "Tick size".) The tick
    /// is a market-microstructure identity, not a pricing input.
    pub const PREMIUM_TICK_COINS: f64 = 0.0005;
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::OptionType;

    /// The Deribit convention identities are the published spec values — a guard so a
    /// mis-encoded contract unit / settlement coin / expiry cut fails here, not in
    /// production. (Identity/structure; the live fixing VALUE is ENV.)
    #[test]
    fn deribit_conventions_match_published_spec() {
        assert_eq!(deribit::STYLE_EUROPEAN, "European");
        // Deribit BTC/ETH options settle in the base coin (inverse), not USD — the
        // published settlement flag must map to the coin-margined routing style.
        let style = if deribit::SETTLEMENT_IS_INVERSE_COIN {
            SettlementStyle::InverseCoin
        } else {
            SettlementStyle::Linear
        };
        assert_eq!(style, SettlementStyle::InverseCoin);
        assert_eq!(deribit::CONTRACT_UNIT_COINS.to_bits(), 1.0_f64.to_bits());
        // 08:00 UTC = 28 800 seconds past midnight.
        assert_eq!(deribit::EXPIRY_CUT_UTC_SECONDS, 28_800);
        assert_eq!(
            deribit::SETTLEMENT_FIXING,
            "Deribit index (08:00 UTC print)"
        );
        assert_eq!(deribit::PREMIUM_TICK_COINS.to_bits(), 0.0005_f64.to_bits());
    }

    /// Smoke: the crate's public surface prices both styles end-to-end through the
    /// re-exports.
    #[test]
    fn public_surface_prices_both_styles() {
        let carry = funding_carry(0.05, 0.02);
        let lin = route_price(
            SettlementStyle::Linear,
            OptionType::Call,
            30_000.0,
            31_000.0,
            0.65,
            0.5,
            carry,
        );
        let inv = route_price(
            SettlementStyle::InverseCoin,
            OptionType::Call,
            30_000.0,
            31_000.0,
            0.65,
            0.5,
            carry,
        );
        assert!(lin > 0.0);
        assert!(inv > 0.0);
        // The inverse premium is in coins (small); the linear in USD (large).
        assert!(inv < 1.0);
        assert!(lin > 100.0);
    }
}
