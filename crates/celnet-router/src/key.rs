//! Partition keys — the unit of shardable pricing work.
//!
//! Per `docs/SCALE-OUT.md` §2 the **primary** partition key is the
//! currency-pair (all tenors of a pair stay co-resident so surface rebuild
//! never crosses a node); a hot pair may be **sub-sharded** by tenant and/or
//! book. A [`PartitionKey`] captures that hierarchy as a small `Copy` value and
//! folds to a single stable 64-bit digest for rendezvous routing.

use celnet_types::CcyPair;

use crate::hash::{fold64, mix64};

/// An opaque tenant identifier (the owning desk / client entity). Stable across
/// the fleet; only its bits matter for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TenantId(pub u64);

/// An opaque book identifier (a risk-netting book within a tenant). Stable
/// across the fleet; only its bits matter for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BookId(pub u64);

/// The shardable unit of pricing work: a currency-pair, optionally sub-sharded
/// by tenant and/or book.
///
/// Constructed via [`PartitionKey::pair`] then refined with
/// [`PartitionKey::with_tenant`] / [`PartitionKey::with_book`]. Two keys route
/// to the same replica iff their [`digest`](PartitionKey::digest)s match, so the
/// granularity you build is exactly the granularity that shards: a bare pair
/// key keeps *all* of a pair's flow on one shard, while adding a book splits a
/// hot pair's flow across shards by book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PartitionKey {
    pair: CcyPair,
    tenant: Option<TenantId>,
    book: Option<BookId>,
}

impl PartitionKey {
    /// A pair-granularity key — all tenors and books of the pair co-resident.
    #[must_use]
    pub const fn pair(pair: CcyPair) -> Self {
        Self {
            pair,
            tenant: None,
            book: None,
        }
    }

    /// Sub-shard this key by tenant.
    #[must_use]
    pub const fn with_tenant(mut self, tenant: TenantId) -> Self {
        self.tenant = Some(tenant);
        self
    }

    /// Sub-shard this key by book.
    #[must_use]
    pub const fn with_book(mut self, book: BookId) -> Self {
        self.book = Some(book);
        self
    }

    /// The currency-pair this key belongs to.
    #[must_use]
    pub const fn ccy_pair(&self) -> CcyPair {
        self.pair
    }

    /// The tenant sub-key, if any.
    #[must_use]
    pub const fn tenant(&self) -> Option<TenantId> {
        self.tenant
    }

    /// The book sub-key, if any.
    #[must_use]
    pub const fn book(&self) -> Option<BookId> {
        self.book
    }

    /// Fold the key into a stable 64-bit digest for rendezvous routing.
    ///
    /// Deterministic and process-independent: the same key produces the same
    /// digest on every node, which is what lets stateless routers agree on
    /// ownership with no shared state. The pair is encoded from its six
    /// validated ASCII bytes; absent sub-keys fold a fixed sentinel so a bare
    /// pair key and a `(pair, tenant=0)` key remain distinct.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let b = self.pair.base.as_str().as_bytes();
        let q = self.pair.quote.as_str().as_bytes();
        // Pack the six ASCII currency bytes into one lane.
        let pair_lane = (u64::from(b[0]) << 40)
            | (u64::from(b[1]) << 32)
            | (u64::from(b[2]) << 24)
            | (u64::from(q[0]) << 16)
            | (u64::from(q[1]) << 8)
            | u64::from(q[2]);

        let mut acc = mix64(pair_lane);
        // Distinguish "no sub-key" from any real id by tagging presence in a
        // high bit that no plausible id occupies, then folding.
        acc = fold64(acc, tagged(self.tenant.map(|t| t.0)));
        acc = fold64(acc, tagged(self.book.map(|b| b.0)));
        acc
    }
}

/// Encode an optional id lane: `None` → a fixed sentinel; `Some(v)` → `v` with
/// a presence tag so a present-zero id never collides with the sentinel.
#[inline]
const fn tagged(id: Option<u64>) -> u64 {
    match id {
        // Sentinel chosen to be unreachable by `tagged(Some(_))`'s mixing.
        None => 0xA5A5_A5A5_A5A5_A5A5,
        Some(v) => mix64(v ^ 0x5555_5555_5555_5555),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::Ccy;

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }
    fn usdjpy() -> CcyPair {
        CcyPair::new(Ccy::USD, Ccy::JPY)
    }

    #[test]
    fn digest_is_deterministic() {
        let k = PartitionKey::pair(eurusd()).with_tenant(TenantId(7));
        assert_eq!(k.digest(), k.digest());
    }

    #[test]
    fn distinct_pairs_distinct_digests() {
        assert_ne!(
            PartitionKey::pair(eurusd()).digest(),
            PartitionKey::pair(usdjpy()).digest()
        );
    }

    #[test]
    fn subkeys_change_digest() {
        let base = PartitionKey::pair(eurusd());
        let with_t = base.with_tenant(TenantId(0));
        let with_b = base.with_book(BookId(0));
        // present-zero must differ from absent (sentinel) and from each other.
        assert_ne!(base.digest(), with_t.digest());
        assert_ne!(base.digest(), with_b.digest());
        assert_ne!(with_t.digest(), with_b.digest());
    }

    #[test]
    fn accessors_round_trip() {
        let k = PartitionKey::pair(eurusd())
            .with_tenant(TenantId(3))
            .with_book(BookId(9));
        assert_eq!(k.ccy_pair(), eurusd());
        assert_eq!(k.tenant(), Some(TenantId(3)));
        assert_eq!(k.book(), Some(BookId(9)));
    }
}
