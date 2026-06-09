//! Common-numeraire conversion (`docs/RISK-HIERARCHY.md` §2.3).
//!
//! Delta is a **currency amount**, not a scalar. A EURUSD position's canonical
//! delta is a EUR (CCY1) hedge amount; a USDJPY position's is a USD amount. You
//! cannot add "0.5 of EURUSD delta" to "0.5 of USDJPY delta" as bare numbers — you
//! resolve each into a **per-currency exposure vector** and convert to a single
//! **reporting numeraire** at spot.
//!
//! Each canonical leaf decomposes into **two currency legs**:
//!
//! - `+delta_base` in the **base** (CCY1) currency — the hedge you hold;
//! - `−delta_base · spot` in the **quote** (CCY2) currency — the funding leg you
//!   sell to hold it.
//!
//! These net at the **currency node** (§2.3): a EURUSD book's EUR leg and a
//! EURJPY book's EUR leg net into a single EUR exposure; the USD legs net
//! separately; etc. The firm view is a vector over currencies, then converted to
//! the reporting currency.
//!
//! **Vega** is normalized to a 1-vol-point (1 %) move and measured in the position's
//! **premium currency** (§2.3); converting it to the reporting numeraire therefore
//! routes through that premium currency, *not* the pair's base — this is the
//! coupling between §2.2 and §2.3 that a naive "just sum the vegas" implementation
//! gets wrong.
//!
//! Conversion rates are supplied by a caller-provided [`SpotResolver`] (the cube
//! wires it to the live or IPV-pinned surface) so this module is free of any
//! market-data dependency and stays deterministic for a given resolver.

use celnet_types::Ccy;

use crate::leaf::CanonicalLeaf;

/// Error converting a currency amount into the reporting numeraire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumeraireError {
    /// No conversion rate is available for this currency into the numeraire.
    /// Carries the offending currency. The conversion fails loudly rather than
    /// dropping the leg, so a missing cross can never silently understate firm
    /// exposure.
    MissingRate(Ccy),
    /// A supplied conversion rate was non-finite or non-positive (a spot rate must
    /// be a finite positive number). Carries the offending currency.
    InvalidRate(Ccy),
}

impl core::fmt::Display for NumeraireError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            NumeraireError::MissingRate(c) => {
                write!(
                    f,
                    "no conversion rate from {c} into the reporting numeraire"
                )
            }
            NumeraireError::InvalidRate(c) => {
                write!(f, "non-finite or non-positive conversion rate for {c}")
            }
        }
    }
}

impl core::error::Error for NumeraireError {}

/// Resolves the spot rate to convert **1 unit of a currency into the reporting
/// numeraire**.
///
/// Implementations are the caller's window onto the surface/market (live or
/// IPV-pinned). The contract: `rate_into_numeraire(ccy)` returns how many units
/// of the numeraire one unit of `ccy` is worth at spot. For `ccy == numeraire`
/// it must return `1.0`. Conversion is deterministic for a fixed resolver.
pub trait SpotResolver {
    /// The reporting numeraire all amounts are converted into.
    fn numeraire(&self) -> Ccy;

    /// Units of the numeraire per 1 unit of `ccy` at spot. Returns `None` when no
    /// rate is available (→ [`NumeraireError::MissingRate`]).
    fn rate_into_numeraire(&self, ccy: Ccy) -> Option<f64>;
}

/// A small immutable [`SpotResolver`] backed by a fixed table of `(ccy → rate)`
/// pairs.
///
/// Suitable for a snapshot conversion (the cube builds one per valuation from the
/// current surface). The numeraire's own rate is implicitly `1.0` and need not be
/// listed. Lookups are linear over the slice — the table is the small set of
/// currencies live in a portfolio, so this is allocation-free and cache-friendly.
#[derive(Debug, Clone, Copy)]
pub struct StaticSpotResolver<'a> {
    numeraire: Ccy,
    rates: &'a [(Ccy, f64)],
}

impl<'a> StaticSpotResolver<'a> {
    /// Build a resolver for `numeraire` from a table of `(ccy, units-of-numeraire-
    /// per-unit-ccy)` pairs. The numeraire itself need not appear in `rates`.
    #[must_use]
    pub const fn new(numeraire: Ccy, rates: &'a [(Ccy, f64)]) -> Self {
        Self { numeraire, rates }
    }
}

impl SpotResolver for StaticSpotResolver<'_> {
    fn numeraire(&self) -> Ccy {
        self.numeraire
    }

    fn rate_into_numeraire(&self, ccy: Ccy) -> Option<f64> {
        if ccy == self.numeraire {
            return Some(1.0);
        }
        self.rates.iter().find(|(c, _)| *c == ccy).map(|(_, r)| *r)
    }
}

/// A signed exposure amount in a single named currency (a leg of the exposure
/// vector). The `amount` is in units of `ccy`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CcyExposure {
    /// The currency this exposure is denominated in.
    pub ccy: Ccy,
    /// Signed amount in units of `ccy` (positive = long that currency).
    pub amount: f64,
}

/// The delta exposure of one or more positions as a **per-currency vector** —
/// the §2.3 currency-node netting representation.
///
/// Built from canonical leaves; legs in the same currency are netted (a EUR leg
/// from a EURUSD book and a EUR leg from a EURJPY book combine into one EUR
/// entry). A vector can then be reported leg-by-leg or collapsed to a single
/// reporting-numeraire scalar via [`CurrencyExposure::in_numeraire`].
///
/// The backing store is a small fixed-capacity inline set keyed by currency, so
/// building the vector is allocation-free (a portfolio touches a bounded set of
/// currencies; the canonical FX universe is far under [`CurrencyExposure::CAP`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurrencyExposure {
    legs: [Option<CcyExposure>; Self::CAP],
}

impl Default for CurrencyExposure {
    fn default() -> Self {
        Self::new()
    }
}

impl CurrencyExposure {
    /// Maximum number of distinct currencies an exposure vector holds. Generous
    /// for FX (the deliverable G10 + the liquid EM set sit well under this); a
    /// caller that overflows it gets a panic-free [`Self::add`] that returns an
    /// out-of-capacity signal rather than silently dropping a currency.
    pub const CAP: usize = 32;

    /// An empty exposure vector.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            legs: [None; Self::CAP],
        }
    }

    /// Net a signed `amount` of `ccy` into the vector. Zero amounts are ignored
    /// (they would only clutter the vector). Returns `false` if a *new* currency
    /// would exceed [`Self::CAP`] (the amount is then not recorded); netting into
    /// an existing currency always succeeds.
    pub fn add(&mut self, ccy: Ccy, amount: f64) -> bool {
        if amount == 0.0 {
            return true;
        }
        // Net into an existing leg if present.
        for leg in self.legs.iter_mut().flatten() {
            if leg.ccy == ccy {
                leg.amount += amount;
                return true;
            }
        }
        // Otherwise occupy the first free slot.
        for slot in &mut self.legs {
            if slot.is_none() {
                *slot = Some(CcyExposure { ccy, amount });
                return true;
            }
        }
        false
    }

    /// Accumulate a **fiat-quoted** canonical leaf's **delta** into the vector as
    /// its two currency legs: `+delta_base` in the base currency, `−delta_base ·
    /// spot` in the quote currency. This is the §2.3 currency-node netting and is
    /// byte-identical to the FX path for FX/metal underlyings (whose
    /// [`Underlying::as_ccy_pair`] projects to a fiat-quoted [`CcyPair`]).
    ///
    /// Returns `false` if a new currency overflowed [`Self::CAP`], **or** if the
    /// leaf's underlying does not project to a fiat [`CcyPair`] (the cross-asset
    /// equity/commodity/crypto base leg is an *asset unit*, not a currency, so it
    /// cannot be added to a `Ccy`-keyed vector — that asset-leg netting is the
    /// deferred follow-up documented at the crate level). The leaf is then **not**
    /// silently dropped into the wrong currency; the caller must route its delta
    /// through the asset-leg path instead.
    pub fn add_leaf_delta(&mut self, leaf: &CanonicalLeaf) -> bool {
        let Some(pair) = leaf.underlying.as_ccy_pair() else {
            return false;
        };
        let base_ok = self.add(pair.base, leaf.greeks.delta_base);
        let quote_ok = self.add(pair.quote, -leaf.greeks.delta_base * leaf.spot);
        base_ok && quote_ok
    }

    /// The non-zero currency legs, in insertion order.
    pub fn legs(&self) -> impl Iterator<Item = CcyExposure> + '_ {
        self.legs.iter().filter_map(|s| *s)
    }

    /// The signed exposure in a single named currency (`0.0` if absent).
    #[must_use]
    pub fn amount_in(&self, ccy: Ccy) -> f64 {
        self.legs().find(|l| l.ccy == ccy).map_or(0.0, |l| l.amount)
    }

    /// Collapse the whole vector into a single scalar in the reporting numeraire,
    /// converting each leg at the resolver's spot rate and summing.
    ///
    /// # Errors
    /// [`NumeraireError::MissingRate`] if any non-zero leg's currency has no
    /// conversion rate; [`NumeraireError::InvalidRate`] if a supplied rate is
    /// non-finite or non-positive. The conversion fails rather than dropping a
    /// leg, so a firm total can never silently omit an unconvertible currency.
    pub fn in_numeraire<R: SpotResolver>(&self, resolver: &R) -> Result<f64, NumeraireError> {
        let mut total = 0.0;
        for leg in self.legs() {
            total += convert(leg.amount, leg.ccy, resolver)?;
        }
        Ok(total)
    }
}

/// A single monetary amount with its currency — a premium or a converted vega.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonetaryAmount {
    /// The currency the amount is denominated in.
    pub ccy: Ccy,
    /// Signed amount in units of `ccy`.
    pub amount: f64,
}

impl MonetaryAmount {
    /// Construct an amount in a currency.
    #[must_use]
    pub const fn new(ccy: Ccy, amount: f64) -> Self {
        Self { ccy, amount }
    }

    /// Convert this amount into the resolver's reporting numeraire.
    ///
    /// # Errors
    /// [`NumeraireError::MissingRate`] / [`NumeraireError::InvalidRate`] as for
    /// [`CurrencyExposure::in_numeraire`].
    pub fn in_numeraire<R: SpotResolver>(&self, resolver: &R) -> Result<f64, NumeraireError> {
        convert(self.amount, self.ccy, resolver)
    }
}

/// A reporting-numeraire view over a set of canonical leaves: the netted delta
/// currency vector, the numeraire-converted total delta, the premium, and the
/// premium-currency-correct vega.
///
/// This is the §2.3 deliverable the cube presents at any node: "express
/// delta-by-ccy-leg and vega in a chosen reporting numeraire."
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Numeraire {
    /// The reporting currency everything is expressed in.
    pub numeraire: Ccy,
    /// The netted per-currency delta vector (legs are in their own currencies).
    pub delta_vector: CurrencyExposure,
    /// Total delta collapsed to a single scalar in `numeraire`.
    pub delta_numeraire: f64,
    /// Total premium in `numeraire`.
    pub premium_numeraire: f64,
    /// Total vega (per 1.0 absolute vol) in `numeraire`, converted through each
    /// leaf's **premium** currency (not its pair base) — the §2.2/§2.3 coupling.
    pub vega_numeraire: f64,
}

impl Numeraire {
    /// Build the reporting-numeraire view of a slice of canonical leaves.
    ///
    /// Delta legs are netted per currency, then collapsed to the numeraire; premium
    /// (quote-currency) and vega (premium-currency) are each converted per leaf and
    /// summed. Deterministic for a fixed `resolver`.
    ///
    /// # Errors
    /// [`NumeraireError::MissingRate`] if any currency that appears (a pair leg, a
    /// premium currency, or a vega currency) has no conversion rate;
    /// [`NumeraireError::InvalidRate`] for a non-finite/non-positive rate.
    ///
    /// Currency-vector capacity ([`CurrencyExposure::CAP`] = 32 distinct
    /// currencies) is far above any FX-sized universe; an overflow would be a
    /// programming error in a synthetic portfolio, so it trips a `debug_assert`
    /// rather than being a recoverable error on this path.
    pub fn from_leaves<R: SpotResolver>(
        leaves: &[CanonicalLeaf],
        resolver: &R,
    ) -> Result<Self, NumeraireError> {
        let mut delta_vector = CurrencyExposure::new();
        let mut premium_numeraire = 0.0;
        let mut vega_numeraire = 0.0;

        for leaf in leaves {
            // Delta → two currency legs, netted in the vector. For a fiat-quoted
            // underlying (FX/metal — the §1.5 first-landing scope) both legs are
            // currencies. A non-projecting cross-asset underlying's base leg is an
            // asset unit, not a currency, so it is not nettable here; its quote
            // (numeraire) funding leg is still currency-nettable and is added
            // explicitly below. (FX universes are far under CAP=32 distinct
            // currencies; a CAP overflow would be a programming error in an enormous
            // synthetic portfolio, so it trips a debug_assert. We never silently
            // drop into the wrong currency: see the tests.)
            match leaf.underlying.as_ccy_pair() {
                Some(_) => {
                    let ok = delta_vector.add_leaf_delta(leaf);
                    debug_assert!(ok, "currency-exposure vector exceeded CAP");
                }
                None => {
                    // Cross-asset leaf: net only the numeraire-currency funding leg
                    // here; the asset-unit base leg netting is the documented
                    // deferred follow-up (§1.5).
                    let ok = delta_vector
                        .add(leaf.vega_premium_ccy, -leaf.greeks.delta_base * leaf.spot);
                    debug_assert!(ok, "currency-exposure vector exceeded CAP");
                }
            }
            // Premium is in the numeraire / quote currency (= the leaf's premium
            // currency for every fiat-quoted underlying).
            premium_numeraire += convert(leaf.premium_quote, leaf.vega_premium_ccy, resolver)?;
            // Vega is in the premium currency (§2.3 coupling).
            vega_numeraire += convert(leaf.greeks.vega, leaf.vega_premium_ccy, resolver)?;
        }

        let delta_numeraire = delta_vector.in_numeraire(resolver)?;

        Ok(Self {
            numeraire: resolver.numeraire(),
            delta_vector,
            delta_numeraire,
            premium_numeraire,
            vega_numeraire,
        })
    }
}

/// Convert `amount` of `ccy` into the resolver's numeraire, validating the rate.
fn convert<R: SpotResolver>(amount: f64, ccy: Ccy, resolver: &R) -> Result<f64, NumeraireError> {
    let rate = resolver
        .rate_into_numeraire(ccy)
        .ok_or(NumeraireError::MissingRate(ccy))?;
    if !rate.is_finite() || rate <= 0.0 {
        return Err(NumeraireError::InvalidRate(ccy));
    }
    Ok(amount * rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PositionRisk;
    use crate::leaf::{CanonicalGreeks, canonicalize};
    use celnet_core::is_close;
    use celnet_types::{
        CcyPair, DeltaConvention, OptionType, PremiumStyle, Underlying, VanillaInputs,
    };

    fn leaf(pair: CcyPair, delta_base: f64, spot: f64) -> CanonicalLeaf {
        CanonicalLeaf {
            underlying: Underlying::Fx(pair),
            spot,
            greeks: CanonicalGreeks {
                delta_base,
                gamma: 0.0,
                vega: 0.0,
                theta: 0.0,
                vanna: 0.0,
                volga: 0.0,
                charm: 0.0,
                speed: 0.0,
                zomma: 0.0,
                color: 0.0,
            },
            premium_quote: 0.0,
            vega_premium_ccy: pair.quote,
            quoted_was_premium_adjusted: false,
        }
    }

    /// USD legs from a EURUSD book and a USDJPY book net at the USD currency node;
    /// the EUR and JPY legs stand alone. This is the §2.3 cross-pair currency-node
    /// netting worked example.
    #[test]
    fn usd_legs_net_across_pairs() {
        let eurusd = CcyPair::new(Ccy::EUR, Ccy::USD);
        let usdjpy = CcyPair::new(Ccy::USD, Ccy::JPY);
        let leaves = [
            // +1.0mm EUR base hedge at 1.10 → −1.10mm USD funding leg.
            leaf(eurusd, 1_000_000.0, 1.10),
            // +2.0mm USD base hedge at 156 → −312.0mm JPY funding leg.
            leaf(usdjpy, 2_000_000.0, 156.0),
        ];
        let mut v = CurrencyExposure::new();
        for l in &leaves {
            assert!(v.add_leaf_delta(l));
        }
        // EUR leg: +1.0mm.
        assert!(is_close(v.amount_in(Ccy::EUR), 1_000_000.0, 1e-12, 1e-6));
        // USD leg: −1.10mm (from EURUSD) + 2.0mm (from USDJPY) = +0.90mm.
        assert!(
            is_close(v.amount_in(Ccy::USD), 900_000.0, 1e-12, 1e-3),
            "USD net leg wrong: {}",
            v.amount_in(Ccy::USD)
        );
        // JPY leg: −312.0mm.
        assert!(is_close(v.amount_in(Ccy::JPY), -312_000_000.0, 1e-12, 1.0));
    }

    /// Collapsing the vector to a numeraire scalar converts each leg at spot and
    /// sums. A round-trip through USD as numeraire: the EURUSD leaf's two legs
    /// (+1mm EUR, −1.10mm USD) net to exactly zero USD (delta is a self-funding
    /// spot hedge), which is the sanity check that the two-leg decomposition is
    /// internally consistent.
    #[test]
    fn single_pair_delta_is_self_funding_in_either_leg_currency() {
        let eurusd = CcyPair::new(Ccy::EUR, Ccy::USD);
        let mut v = CurrencyExposure::new();
        assert!(v.add_leaf_delta(&leaf(eurusd, 1_000_000.0, 1.10)));
        // Convert to USD: EUR leg ×1.10 + USD leg ×1.0 = 1.10mm − 1.10mm = 0.
        let usd = StaticSpotResolver::new(Ccy::USD, &[(Ccy::EUR, 1.10)]);
        let total = v.in_numeraire(&usd).unwrap();
        assert!(
            is_close(total, 0.0, 0.0, 1e-3),
            "EURUSD spot-delta hedge must be self-funding in USD: {total}"
        );
    }

    /// A missing cross rate fails loudly (no silent drop); an invalid rate is
    /// rejected; the numeraire's own rate is implicitly 1.0.
    #[test]
    fn missing_and_invalid_rates_error() {
        let usdjpy = CcyPair::new(Ccy::USD, Ccy::JPY);
        let mut v = CurrencyExposure::new();
        assert!(v.add_leaf_delta(&leaf(usdjpy, 1_000_000.0, 156.0)));
        // Numeraire EUR, but only USD rate supplied → JPY leg has no rate.
        let r = StaticSpotResolver::new(Ccy::EUR, &[(Ccy::USD, 0.92)]);
        assert_eq!(
            v.in_numeraire(&r),
            Err(NumeraireError::MissingRate(Ccy::JPY))
        );
        // Invalid (negative) rate.
        let bad = StaticSpotResolver::new(Ccy::EUR, &[(Ccy::USD, -1.0), (Ccy::JPY, 0.006)]);
        assert_eq!(
            v.in_numeraire(&bad),
            Err(NumeraireError::InvalidRate(Ccy::USD))
        );
        // Numeraire's own rate is 1.0 without being listed.
        assert!(
            StaticSpotResolver::new(Ccy::USD, &[])
                .rate_into_numeraire(Ccy::USD)
                .is_some()
        );
    }

    /// Vega converts through the PREMIUM currency, not the pair base. A USDJPY
    /// position's vega is in JPY; reported in USD it must be divided by ~156, not
    /// left in JPY-sized units. This is the §2.2/§2.3 coupling that a naive sum
    /// gets wrong by a factor of spot.
    #[test]
    fn vega_converts_through_premium_currency() {
        let usdjpy = CcyPair::new(Ccy::USD, Ccy::JPY);
        let mut l = leaf(usdjpy, 0.0, 156.0);
        // 1.0mm JPY of vega per 1.0 vol.
        l.greeks.vega = 1_000_000.0;
        assert_eq!(l.vega_premium_ccy, Ccy::JPY);
        // Report in USD: JPY→USD at 1/156 ≈ 0.00641.
        let usd = StaticSpotResolver::new(Ccy::USD, &[(Ccy::JPY, 1.0 / 156.0)]);
        let view = Numeraire::from_leaves(&[l], &usd).unwrap();
        assert!(
            is_close(view.vega_numeraire, 1_000_000.0 / 156.0, 1e-9, 1e-6),
            "vega must convert JPY→USD through premium ccy: {}",
            view.vega_numeraire
        );
    }

    /// End-to-end: canonicalize two real positions in different conventions and
    /// pairs, then express the book in a common numeraire. Premium/vega/delta all
    /// land in USD and the delta vector nets correctly.
    #[test]
    fn end_to_end_two_pair_book_in_usd() {
        let eurusd = CcyPair::new(Ccy::EUR, Ccy::USD);
        let usdjpy = CcyPair::new(Ccy::USD, Ccy::JPY);
        let p1 = canonicalize(&PositionRisk::fx(
            eurusd,
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            DeltaConvention::SpotPremiumAdjusted,
            PremiumStyle::PercentForeign,
        ))
        .unwrap();
        let p2 = canonicalize(&PositionRisk::fx(
            usdjpy,
            OptionType::Put,
            8_000_000.0,
            VanillaInputs::new(156.0, 154.0, 0.11, 0.5, 0.01, 0.05),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        ))
        .unwrap();
        // Numeraire USD; EUR worth 1.10 USD, JPY worth 1/156 USD.
        let usd = StaticSpotResolver::new(Ccy::USD, &[(Ccy::EUR, 1.10), (Ccy::JPY, 1.0 / 156.0)]);
        let view = Numeraire::from_leaves(&[p1.clone(), p2.clone()], &usd).unwrap();
        assert_eq!(view.numeraire, Ccy::USD);
        // Premium of a long call + long put is positive in USD.
        assert!(view.premium_numeraire > 0.0);
        // Long options → positive vega in USD.
        assert!(view.vega_numeraire > 0.0);
        // The delta vector has EUR, USD, JPY legs; collapsing equals the scalar.
        let recomputed = view.delta_vector.in_numeraire(&usd).unwrap();
        assert!(is_close(recomputed, view.delta_numeraire, 1e-9, 1e-3));
        // USD leg is the sum of EURUSD's funding leg and USDJPY's base hedge.
        let usd_leg = view.delta_vector.amount_in(Ccy::USD);
        let expected_usd = -p1.greeks.delta_base * p1.spot + p2.greeks.delta_base;
        assert!(is_close(usd_leg, expected_usd, 1e-9, 1e-3));
    }

    /// Capacity guard: adding more distinct currencies than CAP returns false for
    /// the overflowing currency rather than panicking or silently overwriting.
    #[test]
    fn capacity_overflow_is_reported_not_silent() {
        let mut v = CurrencyExposure::new();
        // Fill all CAP slots with distinct synthetic currencies.
        let mut count = 0;
        for a in b'A'..=b'Z' {
            for b in b'A'..=b'Z' {
                if count >= CurrencyExposure::CAP {
                    break;
                }
                let c = Ccy::new([a, b, b'A']).unwrap();
                assert!(v.add(c, 1.0));
                count += 1;
            }
            if count >= CurrencyExposure::CAP {
                break;
            }
        }
        // One more distinct currency overflows.
        let overflow = Ccy::new([b'Z', b'Z', b'Z']).unwrap();
        assert!(!v.add(overflow, 1.0), "overflow must be reported as false");
        // But netting into an existing currency still works.
        let existing = Ccy::new([b'A', b'A', b'A']).unwrap();
        assert!(v.add(existing, 5.0));
        assert!(is_close(v.amount_in(existing), 6.0, 0.0, 1e-12));
    }
}
