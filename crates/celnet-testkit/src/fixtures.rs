//! Named, deterministic reference market states for regression and golden tests.
//!
//! These fixtures pin a small set of economically-distinct
//! [`celnet_types::VanillaInputs`] regimes — the textbook Black-Scholes
//! benchmark plus representative G10, high-vol, short-dated and inverted-carry
//! states — so that regression tests across crates anchor to the *same* known
//! market points instead of inventing ad-hoc numbers. Each carries a stable
//! [`ReferenceMarket::name`] usable as a golden-file key.
//!
//! The set is intentionally a compiled-in constant table (zero IO, fully
//! deterministic, available in `no_std`-style contexts) rather than a file
//! loader: there is exactly one current contract for these reference states and
//! no version negotiation. Callers select by name via [`reference_market`] or
//! iterate the whole curated set via [`reference_markets`].

use celnet_types::VanillaInputs;

/// A named reference market state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceMarket {
    /// Stable identifier (golden-file key); never reused for a different state.
    pub name: &'static str,
    /// Human-readable description of the regime this fixture represents.
    pub description: &'static str,
    /// The market inputs.
    pub inputs: VanillaInputs,
}

/// The curated reference market set.
///
/// Ordered from the canonical Black-Scholes benchmark outward through the FX
/// regimes; the spot/strike relationships span ATM, ITM and OTM, and the carry
/// covers positive, near-zero and inverted (`r_for > r_dom`) cases so consumers
/// can pick a regime or sweep them all.
pub const REFERENCE_MARKETS: &[ReferenceMarket] = &[
    ReferenceMarket {
        name: "bs-textbook-atm",
        description: "Black-Scholes textbook benchmark: S=K=100, σ=20%, T=1, r_d=5%, r_f=0%.",
        inputs: VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.0),
    },
    ReferenceMarket {
        name: "eurusd-1y-atm",
        description: "EURUSD-like 1Y ATM: spot 1.10, 10 vol, positive carry (r_d=2%, r_f=1%).",
        inputs: VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.02, 0.01),
    },
    ReferenceMarket {
        name: "eurusd-6m-otm-call",
        description: "EURUSD-like 6M OTM call: spot 1.10, strike 1.25, 9 vol.",
        inputs: VanillaInputs::new(1.10, 1.25, 0.09, 0.5, 0.02, 0.01),
    },
    ReferenceMarket {
        name: "gbpusd-2y-itm-call",
        description: "GBPUSD-like 2Y ITM call: spot 1.35, strike 1.20, 14 vol, positive carry.",
        inputs: VanillaInputs::new(1.35, 1.20, 0.14, 2.0, 0.04, 0.015),
    },
    ReferenceMarket {
        name: "usdjpy-3m-high-vol-inverted",
        description: "USDJPY-like 3M, high 30 vol, inverted carry (r_for > r_dom): \
                      spot 110, strike 95.",
        inputs: VanillaInputs::new(110.0, 95.0, 0.30, 0.25, 0.01, 0.03),
    },
    ReferenceMarket {
        name: "short-dated-2w-atm",
        description: "Very short-dated 2W ATM: spot 1.30, 12 vol, ~zero carry.",
        inputs: VanillaInputs::new(1.30, 1.30, 0.12, 2.0 / 52.0, 0.015, 0.015),
    },
    ReferenceMarket {
        name: "long-dated-5y-otm-put",
        description: "Long-dated 5Y OTM put: spot 1.20, strike 1.05, 11 vol, positive carry.",
        inputs: VanillaInputs::new(1.20, 1.05, 0.11, 5.0, 0.03, 0.01),
    },
];

/// Iterate the full curated reference market set.
pub fn reference_markets() -> impl Iterator<Item = ReferenceMarket> {
    REFERENCE_MARKETS.iter().copied()
}

/// Look up a reference market by its stable [`ReferenceMarket::name`].
///
/// Returns `None` if no fixture carries that name.
#[must_use]
pub fn reference_market(name: &str) -> Option<ReferenceMarket> {
    REFERENCE_MARKETS.iter().copied().find(|m| m.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every fixture is economically valid (positive levels, vol, maturity) and
    /// has finite derived forward / discount factors.
    #[test]
    fn fixtures_are_valid_markets() {
        for m in reference_markets() {
            let i = m.inputs;
            assert!(
                i.spot > 0.0 && i.strike > 0.0,
                "{}: levels must be positive",
                m.name
            );
            assert!(i.vol > 0.0, "{}: vol must be positive", m.name);
            assert!(i.t > 0.0, "{}: maturity must be positive", m.name);
            assert!(
                i.forward().is_finite() && i.forward() > 0.0,
                "{}: forward",
                m.name
            );
            assert!(
                i.df_dom() > 0.0 && i.df_for() > 0.0,
                "{}: discount factors",
                m.name
            );
        }
    }

    /// Fixture names are unique (so they are safe golden-file keys) and
    /// resolvable through the lookup.
    #[test]
    fn names_are_unique_and_resolvable() {
        let mut seen = std::collections::BTreeSet::new();
        for m in reference_markets() {
            assert!(seen.insert(m.name), "duplicate fixture name {}", m.name);
            assert_eq!(reference_market(m.name), Some(m));
        }
        assert_eq!(reference_market("does-not-exist"), None);
    }

    /// The canonical benchmark is present and exact.
    #[test]
    fn textbook_benchmark_present() {
        let m = reference_market("bs-textbook-atm").expect("benchmark fixture");
        assert_eq!(
            m.inputs,
            VanillaInputs::new(100.0, 100.0, 0.20, 1.0, 0.05, 0.0)
        );
    }
}
