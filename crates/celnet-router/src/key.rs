//! Partition keys — the unit of shardable pricing work.
//!
//! Per `docs/SCALE-OUT.md` §2 the **primary** partition key is the
//! currency-pair (all tenors of a pair stay co-resident so surface rebuild
//! never crosses a node) for FX/options work, or — for fixed-income/rates work
//! — a single **settlement currency** (all tenors of a rates book stay
//! co-resident so curve rebuild never crosses a node). Either subject may be
//! **sub-sharded** by tenant and/or book. A [`PartitionKey`] captures that
//! hierarchy as a small `Copy` value and folds to a single stable 64-bit digest
//! for rendezvous routing.

use celnet_types::{Ccy, CcyPair};

use crate::hash::{fold64, mix64};

/// The shardable **subject** of pricing/risk work: an FX/metal currency-pair
/// (the options/surface primary key) or a single currency (the
/// fixed-income/rates primary key). Modelling the subject as a sum type keeps a
/// currency key honestly distinct from a pair key — a rates `USD` book is *not*
/// a degenerate `USDUSD` pair, and its [`digest`](PartitionKey::digest) lives in
/// a disjoint lane so the two never alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Subject {
    /// An FX/metal currency-pair (all tenors of the pair co-resident).
    Pair(CcyPair),
    /// A single settlement currency (all tenors of the rates book co-resident).
    Currency(Ccy),
    /// A cross-currency basis book (e.g. EUR/USD basis vs USD SOFR reference ccy).
    Basis {
        base_pair: CcyPair,
        reference_ccy: Ccy,
    },
    /// A multi-asset risk netting group identifier.
    NettingGroup(u64),
}

impl Subject {
    /// Pack the subject's currency bytes into a single 64-bit routing lane.
    ///
    /// A pair packs its six validated ASCII bytes into bits `0..48` exactly as
    /// the pre-currency-subject encoding did, so **every existing pair digest is
    /// byte-identical** and no FX key reshuffles. A currency packs its three
    /// ASCII bytes into bits `0..24` and sets bit 63 — a region no pair lane can
    /// reach (a pair uses at most bit 47) — so a currency subject never collides
    /// with any pair, in particular `USD` never aliases the self-pair `USDUSD`.
    ///
    /// Cross-currency basis sets bit 62, and multi-asset netting groups set bit 61,
    /// ensuring non-overlapping bit-regions across all subject types.
    fn lane(self) -> u64 {
        match self {
            Subject::Pair(pair) => {
                let b = pair.base.as_str().as_bytes();
                let q = pair.quote.as_str().as_bytes();
                (u64::from(b[0]) << 40)
                    | (u64::from(b[1]) << 32)
                    | (u64::from(b[2]) << 24)
                    | (u64::from(q[0]) << 16)
                    | (u64::from(q[1]) << 8)
                    | u64::from(q[2])
            }
            Subject::Currency(ccy) => {
                let c = ccy.as_str().as_bytes();
                (1u64 << 63) | (u64::from(c[0]) << 16) | (u64::from(c[1]) << 8) | u64::from(c[2])
            }
            Subject::Basis {
                base_pair,
                reference_ccy,
            } => {
                let b = base_pair.base.as_str().as_bytes();
                let q = base_pair.quote.as_str().as_bytes();
                let r = reference_ccy.as_str().as_bytes();
                (1u64 << 62)
                    | (u64::from(b[0]) << 40)
                    | (u64::from(b[1]) << 32)
                    | (u64::from(q[0]) << 24)
                    | (u64::from(q[1]) << 16)
                    | (u64::from(r[0]) << 8)
                    | u64::from(r[1])
            }
            Subject::NettingGroup(group_id) => {
                (1u64 << 61) | (group_id & 0x1FFF_FFFF_FFFF_FFFF)
            }
        }
    }
}

/// An opaque tenant identifier (the owning desk / client entity). Stable across
/// the fleet; only its bits matter for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TenantId(pub u64);

/// An opaque book identifier (a risk-netting book within a tenant). Stable
/// across the fleet; only its bits matter for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BookId(pub u64);

/// The shardable unit of pricing work: a currency-pair *or* a single currency,
/// optionally sub-sharded by tenant and/or book.
///
/// Constructed via [`PartitionKey::pair`] (FX/options) or
/// [`PartitionKey::currency`] (fixed-income/rates), then refined with
/// [`PartitionKey::with_tenant`] / [`PartitionKey::with_book`]. Two keys route
/// to the same replica iff their [`digest`](PartitionKey::digest)s match, so the
/// granularity you build is exactly the granularity that shards: a bare pair (or
/// currency) key keeps *all* of that subject's flow on one shard, while adding a
/// book splits a hot subject's flow across shards by book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PartitionKey {
    subject: Subject,
    tenant: Option<TenantId>,
    book: Option<BookId>,
}

impl PartitionKey {
    /// A pair-granularity key — all tenors and books of the pair co-resident.
    #[must_use]
    pub const fn pair(pair: CcyPair) -> Self {
        Self {
            subject: Subject::Pair(pair),
            tenant: None,
            book: None,
        }
    }

    /// A currency-granularity key — all tenors and books of the rates book in
    /// this settlement currency co-resident (the fixed-income primary key).
    #[must_use]
    pub const fn currency(ccy: Ccy) -> Self {
        Self {
            subject: Subject::Currency(ccy),
            tenant: None,
            book: None,
        }
    }

    /// A cross-currency basis key — co-locating basis curves and legs.
    #[must_use]
    pub const fn basis(base_pair: CcyPair, reference_ccy: Ccy) -> Self {
        Self {
            subject: Subject::Basis {
                base_pair,
                reference_ccy,
            },
            tenant: None,
            book: None,
        }
    }

    /// A multi-asset risk netting group key.
    #[must_use]
    pub const fn netting_group(group_id: u64) -> Self {
        Self {
            subject: Subject::NettingGroup(group_id),
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

    /// The currency-pair this key belongs to, or `None` for a currency-subject
    /// (fixed-income/rates) key.
    #[must_use]
    pub const fn ccy_pair(&self) -> Option<CcyPair> {
        match self.subject {
            Subject::Pair(pair) => Some(pair),
            _ => None,
        }
    }

    /// The settlement currency this key belongs to, or `None` for a
    /// pair-subject (FX/options) key.
    #[must_use]
    pub const fn ccy(&self) -> Option<Ccy> {
        match self.subject {
            Subject::Currency(ccy) => Some(ccy),
            _ => None,
        }
    }

    /// The cross-currency basis pair and reference currency, if applicable.
    #[must_use]
    pub const fn basis_info(&self) -> Option<(CcyPair, Ccy)> {
        match self.subject {
            Subject::Basis {
                base_pair,
                reference_ccy,
            } => Some((base_pair, reference_ccy)),
            _ => None,
        }
    }

    /// The netting group identifier, if applicable.
    #[must_use]
    pub const fn netting_group_id(&self) -> Option<u64> {
        match self.subject {
            Subject::NettingGroup(id) => Some(id),
            _ => None,
        }
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
    /// ownership with no shared state. The subject is encoded into a single lane
    /// from its validated ASCII bytes ([`Subject::lane`]); absent sub-keys fold
    /// a fixed sentinel so a bare subject key and a `(subject, tenant=0)` key
    /// remain distinct.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut acc = mix64(self.subject.lane());
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
        assert_eq!(k.ccy_pair(), Some(eurusd()));
        assert_eq!(k.ccy(), None);
        assert_eq!(k.tenant(), Some(TenantId(3)));
        assert_eq!(k.book(), Some(BookId(9)));
    }

    #[test]
    fn currency_subject_accessors_round_trip() {
        let k = PartitionKey::currency(Ccy::USD).with_tenant(TenantId(5));
        assert_eq!(k.ccy(), Some(Ccy::USD));
        assert_eq!(k.ccy_pair(), None);
        assert_eq!(k.tenant(), Some(TenantId(5)));
        assert_eq!(k.book(), None);
    }

    #[test]
    fn currency_subject_is_deterministic_and_distinct() {
        // Deterministic.
        assert_eq!(
            PartitionKey::currency(Ccy::USD).digest(),
            PartitionKey::currency(Ccy::USD).digest()
        );
        // Distinct currencies route distinctly.
        assert_ne!(
            PartitionKey::currency(Ccy::USD).digest(),
            PartitionKey::currency(Ccy::EUR).digest()
        );
        // A currency subject must NOT alias the degenerate self-pair (the whole
        // reason `currency` is its own subject rather than a faked `CcyPair`).
        assert_ne!(
            PartitionKey::currency(Ccy::USD).digest(),
            PartitionKey::pair(CcyPair::new(Ccy::USD, Ccy::USD)).digest()
        );
    }

    #[test]
    fn pair_digests_are_unchanged_by_the_currency_subject() {
        // The pair lane is byte-identical to the pre-currency-subject encoding,
        // so existing FX routing never reshuffles. Pin the EURUSD digest to a
        // recomputed-from-first-principles reference.
        let b = Ccy::EUR.as_str().as_bytes();
        let q = Ccy::USD.as_str().as_bytes();
        let pair_lane = (u64::from(b[0]) << 40)
            | (u64::from(b[1]) << 32)
            | (u64::from(b[2]) << 24)
            | (u64::from(q[0]) << 16)
            | (u64::from(q[1]) << 8)
            | u64::from(q[2]);
        let mut acc = mix64(pair_lane);
        acc = fold64(acc, tagged(None));
        acc = fold64(acc, tagged(None));
        assert_eq!(PartitionKey::pair(eurusd()).digest(), acc);
    }

    #[test]
    fn basis_and_netting_group_round_trip_and_distinction() {
        let basis_k = PartitionKey::basis(eurusd(), Ccy::USD).with_tenant(TenantId(1));
        assert_eq!(basis_k.basis_info(), Some((eurusd(), Ccy::USD)));
        assert_eq!(basis_k.ccy_pair(), None);
        assert_eq!(basis_k.ccy(), None);
        assert_eq!(basis_k.netting_group_id(), None);

        let net_k = PartitionKey::netting_group(42).with_book(BookId(10));
        assert_eq!(net_k.netting_group_id(), Some(42));
        assert_eq!(net_k.basis_info(), None);

        // Deterministic and non-colliding across all variants
        assert_eq!(basis_k.digest(), basis_k.digest());
        assert_eq!(net_k.digest(), net_k.digest());
        assert_ne!(basis_k.digest(), net_k.digest());
        assert_ne!(basis_k.digest(), PartitionKey::pair(eurusd()).digest());
        assert_ne!(basis_k.digest(), PartitionKey::currency(Ccy::USD).digest());
    }
}
