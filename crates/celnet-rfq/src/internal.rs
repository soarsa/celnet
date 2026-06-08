//! [`InternalPricerSource`] — the native Celnet dealer on the panel.
//!
//! A [`crate::panel::QuoteSource`] that produces a two-way quote **in process**:
//! it wraps the client's resolved mid premium with a symmetric half-spread to
//! form `bid = mid − hs`, `offer = mid + hs`. Keeping it native guarantees the
//! panel always carries ≥ 1 dealer (a market always exists), so the SDK / CLI /
//! GUI see a ranked panel even with no external LPs connected.
//!
//! The mid is *supplied* (resolved by the edge from the live surface +
//! `celnet-vanilla` engine before the panel is run) rather than computed here:
//! `celnet-rfq` is an aggregation leaf and does not depend on the pricing engine
//! (one-way deps). This mirrors the `celnet-fix` dialect's injected-pricer
//! design — the workflow crate owns the *aggregation*, the edge owns the
//! *model*. The quote is fully deterministic: same `(mid, half_spread)` ⇒ same
//! two-way, bit-for-bit.

use std::time::Duration;

use crate::panel::{QuoteSource, QuoteSourceReply, RfqRequest, TwoWay};

/// A native in-process dealer: a fixed mid + half-spread producing one
/// deterministic two-way quote, stamped with a configurable validity window.
#[derive(Debug, Clone)]
pub struct InternalPricerSource {
    lp_id: String,
    mid: f64,
    half_spread: f64,
    epoch_nanos: u64,
    valid_for_nanos: u64,
}

impl InternalPricerSource {
    /// Build a native source.
    ///
    /// * `lp_id` — the stable audit identifier (e.g. `"CELNET-NATIVE"`).
    /// * `mid` — the resolved mid premium (domestic per unit base).
    /// * `half_spread` — the symmetric half-spread applied around the mid
    ///   (`bid = mid − hs`, `offer = mid + hs`). Must be ≥ 0; a negative value
    ///   would invert the two-way, so it is clamped to 0.
    /// * `epoch_nanos` — the logical timestamp stamped on the quote.
    /// * `valid_for_nanos` — how long (from `epoch_nanos`) the quote stays
    ///   liftable; `valid_until_nanos = epoch_nanos + valid_for_nanos`.
    #[must_use]
    pub fn new(
        lp_id: impl Into<String>,
        mid: f64,
        half_spread: f64,
        epoch_nanos: u64,
        valid_for_nanos: u64,
    ) -> Self {
        Self {
            lp_id: lp_id.into(),
            mid,
            half_spread: half_spread.max(0.0),
            epoch_nanos,
            valid_for_nanos,
        }
    }

    /// The two-way this source will quote (exposed for tests / introspection).
    #[must_use]
    pub fn two_way(&self) -> TwoWay {
        TwoWay {
            bid: self.mid - self.half_spread,
            offer: self.mid + self.half_spread,
        }
    }
}

impl QuoteSource for InternalPricerSource {
    fn lp_id(&self) -> &str {
        &self.lp_id
    }

    fn request<'a>(
        &'a self,
        _request: &'a RfqRequest,
        _deadline: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = QuoteSourceReply> + Send + 'a>> {
        let reply = QuoteSourceReply::Quote {
            price: self.two_way(),
            epoch_nanos: self.epoch_nanos,
            valid_until_nanos: self.epoch_nanos.saturating_add(self.valid_for_nanos),
        };
        Box::pin(async move { reply })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::{Ccy, CcyPair, OptionType, Tenor};

    fn req() -> RfqRequest {
        RfqRequest::new(
            "R",
            CcyPair::new(Ccy::EUR, Ccy::USD),
            OptionType::Call,
            1.10,
            Tenor::Months(3),
        )
    }

    #[test]
    fn two_way_brackets_the_mid_symmetrically() {
        let src = InternalPricerSource::new("NATIVE", 0.0085, 0.0005, 10, 1_000);
        let tw = src.two_way();
        assert!((tw.bid - 0.0080).abs() < 1e-12);
        assert!((tw.offer - 0.0090).abs() < 1e-12);
        // Symmetric around the mid.
        assert!(((tw.bid + tw.offer) / 2.0 - 0.0085).abs() < 1e-12);
    }

    #[test]
    fn negative_half_spread_clamped_to_zero() {
        let src = InternalPricerSource::new("NATIVE", 0.01, -1.0, 0, 0);
        let tw = src.two_way();
        // Clamped ⇒ bid == offer == mid (never an inverted two-way).
        assert!((tw.bid - tw.offer).abs() < 1e-15);
        assert!((tw.bid - 0.01).abs() < 1e-15);
    }

    #[tokio::test]
    async fn always_quotes_deterministically() {
        let src = InternalPricerSource::new("NATIVE", 0.0085, 0.0005, 42, 1_000);
        let r1 = src.request(&req(), Duration::from_secs(1)).await;
        let r2 = src.request(&req(), Duration::from_secs(1)).await;
        assert_eq!(r1, r2);
        match r1 {
            QuoteSourceReply::Quote {
                price,
                epoch_nanos,
                valid_until_nanos,
            } => {
                assert_eq!(epoch_nanos, 42);
                assert_eq!(valid_until_nanos, 1_042);
                assert!((price.bid - 0.0080).abs() < 1e-12);
            }
            QuoteSourceReply::NoQuote => panic!("native dealer must always quote"),
        }
    }
}
