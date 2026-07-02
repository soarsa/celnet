//! The typed **configurable-consistency policy** (ADR-0015 §2.1 / §2.1.1).
//!
//! Consistency is a per-granularity operator choice, **wired everywhere but forced
//! nowhere**: a book / desk / tenant that needs linearizable, replicated,
//! zero-data-loss state opts into the [`ConsistencyLevel::Strong`] (Raft-quorum) tier;
//! everything else stays on the [`ConsistencyLevel::Local`] ultra-low-latency default.
//! The level is resolved by a **most-specific-wins cascade** —
//! `book → desk → tenant → platform-default` — with the platform default `Local`, so a
//! fleet that configures nothing keeps today's single-node ultra-low-latency behaviour
//! byte-for-byte and `Strong` is purely additive, opt-in state.
//!
//! This module is a **pure config type** (no I/O beyond [`ConsistencyPolicy::from_env`],
//! which reads the deploy-time knobs once at boot). The resolved level is read **once,
//! off the hot path**, at the booking sink — never per tick — so the lookup itself never
//! touches the price / streaming / market-data path (the ADR-0015 §4.3 hard invariant).

use std::collections::HashMap;

/// The consistency tier a book's authoritative state write runs at (ADR-0015 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConsistencyLevel {
    /// **Opt-in.** The authoritative book write becomes a `celnet_replog::BookUpdate`
    /// quorum-committed through Raft — linearizable, quorum-replicated, zero-data-loss,
    /// ms-scale, and (per §4.3) always on the async booking / state tier, never the
    /// pinned pricing thread.
    Strong,
    /// **Default.** The single-node ultra-low-latency fast path (`celnet-journal`
    /// `sync_data` fsync + a light non-quorum peer broadcast) — µs, no quorum round-trip.
    #[default]
    Local,
}

impl ConsistencyLevel {
    /// Parse an operator token: `"strong"` (any case, trimmed) ⇒ [`Self::Strong`];
    /// anything else — including the empty string — ⇒ the [`Self::Local`] default.
    #[must_use]
    pub fn parse(token: &str) -> Self {
        if token.trim().eq_ignore_ascii_case("strong") {
            Self::Strong
        } else {
            Self::Local
        }
    }

    /// Whether this is the opt-in [`Self::Strong`] quorum tier.
    #[must_use]
    pub fn is_strong(self) -> bool {
        matches!(self, Self::Strong)
    }
}

/// The resolved consistency policy: the `book → desk → tenant → platform-default`
/// most-specific-wins cascade (ADR-0015 §2.1.1). The platform default is
/// [`ConsistencyLevel::Local`], so an unconfigured fleet is byte-identical to the
/// single-node fast path. FX books resolve by name (from the booking's attribution);
/// linear-rates books resolve by their numeric cell id (the only identifier a rates
/// cell carries).
#[derive(Debug, Clone, Default)]
pub struct ConsistencyPolicy {
    /// The platform default (`Local` unless an operator raises it).
    default: ConsistencyLevel,
    /// Per-tenant overrides (least specific of the named layers).
    tenants: HashMap<String, ConsistencyLevel>,
    /// Per-desk overrides.
    desks: HashMap<String, ConsistencyLevel>,
    /// Per-FX-book overrides (most specific).
    books: HashMap<String, ConsistencyLevel>,
    /// The `book → desk` membership (populated at boot from the identity desk defs), so
    /// a book with no direct override inherits its desk's level in the cascade.
    book_to_desk: HashMap<String, String>,
    /// Per-linear-rates-book overrides, keyed by the numeric cell book id.
    rates_books: HashMap<u32, ConsistencyLevel>,
}

impl ConsistencyPolicy {
    /// An empty policy: the platform default is [`ConsistencyLevel::Local`] and no
    /// overrides — byte-identical to today.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve the deploy-time policy from the environment, mirroring the
    /// `CELNET_FLEET_*` knob discipline (read once, at boot):
    ///
    /// * `CELNET_CONSISTENCY_DEFAULT` — `strong | local` platform default (default `local`);
    /// * `CELNET_CONSISTENCY_STRONG_BOOKS` — comma-separated FX book names run `Strong`;
    /// * `CELNET_CONSISTENCY_STRONG_DESKS` — comma-separated desk names run `Strong`;
    /// * `CELNET_CONSISTENCY_STRONG_TENANTS` — comma-separated tenant names run `Strong`;
    /// * `CELNET_CONSISTENCY_STRONG_RATES_BOOKS` — comma-separated numeric rates book ids
    ///   run `Strong`.
    ///
    /// All absent ⇒ the empty, `Local`-default policy ([`Self::new`]).
    #[must_use]
    pub fn from_env() -> Self {
        let mut policy = Self::new();
        if let Ok(default) = std::env::var("CELNET_CONSISTENCY_DEFAULT") {
            policy.default = ConsistencyLevel::parse(&default);
        }
        for book in split_env("CELNET_CONSISTENCY_STRONG_BOOKS") {
            policy.books.insert(book, ConsistencyLevel::Strong);
        }
        for desk in split_env("CELNET_CONSISTENCY_STRONG_DESKS") {
            policy.desks.insert(desk, ConsistencyLevel::Strong);
        }
        for tenant in split_env("CELNET_CONSISTENCY_STRONG_TENANTS") {
            policy.tenants.insert(tenant, ConsistencyLevel::Strong);
        }
        for id in split_env("CELNET_CONSISTENCY_STRONG_RATES_BOOKS") {
            if let Ok(book) = id.parse::<u32>() {
                policy.rates_books.insert(book, ConsistencyLevel::Strong);
            }
        }
        policy
    }

    /// Set the platform-default level (the cascade floor).
    pub fn set_default(&mut self, level: ConsistencyLevel) {
        self.default = level;
    }

    /// Override an FX book's level (the most-specific layer).
    pub fn set_book(&mut self, book: impl Into<String>, level: ConsistencyLevel) {
        self.books.insert(book.into(), level);
    }

    /// Override a desk's level.
    pub fn set_desk(&mut self, desk: impl Into<String>, level: ConsistencyLevel) {
        self.desks.insert(desk.into(), level);
    }

    /// Override a tenant's level.
    pub fn set_tenant(&mut self, tenant: impl Into<String>, level: ConsistencyLevel) {
        self.tenants.insert(tenant.into(), level);
    }

    /// Override a linear-rates book's level (keyed by the numeric cell book id).
    pub fn set_rates_book(&mut self, book: u32, level: ConsistencyLevel) {
        self.rates_books.insert(book, level);
    }

    /// Record that `book` belongs to `desk`, so the book inherits its desk's level when
    /// it carries no direct override. Populated at boot from the identity desk defs.
    pub fn map_book_to_desk(&mut self, book: impl Into<String>, desk: impl Into<String>) {
        self.book_to_desk.insert(book.into(), desk.into());
    }

    /// The platform-default level.
    #[must_use]
    pub fn default_level(&self) -> ConsistencyLevel {
        self.default
    }

    /// Resolve the level for an FX book with an explicitly-known desk / tenant, applying
    /// the full `book → desk → tenant → default` most-specific-wins cascade. A `None`
    /// desk falls back to the recorded `book → desk` membership ([`Self::map_book_to_desk`]).
    #[must_use]
    pub fn resolve(
        &self,
        book: &str,
        desk: Option<&str>,
        tenant: Option<&str>,
    ) -> ConsistencyLevel {
        if let Some(&level) = self.books.get(book) {
            return level;
        }
        let desk = desk.or_else(|| self.book_to_desk.get(book).map(String::as_str));
        if let Some(desk) = desk
            && let Some(&level) = self.desks.get(desk)
        {
            return level;
        }
        if let Some(tenant) = tenant
            && let Some(&level) = self.tenants.get(tenant)
        {
            return level;
        }
        self.default
    }

    /// Resolve the level for an FX book by name — the booking-sink convenience: the desk
    /// is taken from the recorded `book → desk` membership and the tenant from the
    /// platform layer, so the full cascade still applies.
    #[must_use]
    pub fn resolve_book(&self, book: &str) -> ConsistencyLevel {
        self.resolve(book, None, None)
    }

    /// Resolve the level for a linear-rates book by its numeric cell id (falling back to
    /// the platform default).
    #[must_use]
    pub fn resolve_rates(&self, book: u32) -> ConsistencyLevel {
        self.rates_books.get(&book).copied().unwrap_or(self.default)
    }

    /// Whether **any** book — via the default, an FX/desk/tenant name, or a rates id —
    /// opts into [`ConsistencyLevel::Strong`]. The edge boots the Raft consensus tier
    /// **iff** this holds (else zero overhead: a pure-`Local` fleet never binds a node).
    #[must_use]
    pub fn any_strong(&self) -> bool {
        self.default.is_strong()
            || self.books.values().any(|l| l.is_strong())
            || self.desks.values().any(|l| l.is_strong())
            || self.tenants.values().any(|l| l.is_strong())
            || self.rates_books.values().any(|l| l.is_strong())
    }
}

/// Split a comma-separated environment variable into trimmed, non-empty tokens.
fn split_env(var: &str) -> Vec<String> {
    std::env::var(var)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_maps_only_strong_token_to_strong() {
        assert_eq!(ConsistencyLevel::parse("strong"), ConsistencyLevel::Strong);
        assert_eq!(ConsistencyLevel::parse("STRONG"), ConsistencyLevel::Strong);
        assert_eq!(
            ConsistencyLevel::parse("  Strong "),
            ConsistencyLevel::Strong
        );
        assert_eq!(ConsistencyLevel::parse("local"), ConsistencyLevel::Local);
        assert_eq!(ConsistencyLevel::parse(""), ConsistencyLevel::Local);
        assert_eq!(ConsistencyLevel::parse("quorum"), ConsistencyLevel::Local);
    }

    #[test]
    fn default_is_local_and_unconfigured_is_all_local() {
        let policy = ConsistencyPolicy::new();
        assert_eq!(policy.default_level(), ConsistencyLevel::Local);
        assert_eq!(policy.resolve_book("ANY-BOOK"), ConsistencyLevel::Local);
        assert_eq!(policy.resolve_rates(42), ConsistencyLevel::Local);
        assert!(!policy.any_strong());
    }

    #[test]
    fn book_override_wins_over_desk_and_default() {
        let mut policy = ConsistencyPolicy::new();
        policy.map_book_to_desk("EM-VOL-1", "EM-DESK");
        policy.set_desk("EM-DESK", ConsistencyLevel::Strong);
        // The book has no direct override, so it inherits its desk's Strong level.
        assert_eq!(policy.resolve_book("EM-VOL-1"), ConsistencyLevel::Strong);
        // A direct book override (Local) beats the desk's Strong.
        policy.set_book("EM-VOL-1", ConsistencyLevel::Local);
        assert_eq!(policy.resolve_book("EM-VOL-1"), ConsistencyLevel::Local);
        assert!(
            policy.any_strong(),
            "the desk override still makes some book Strong"
        );
    }

    #[test]
    fn desk_then_tenant_then_default_cascade() {
        let mut policy = ConsistencyPolicy::new();
        policy.set_tenant("ACME", ConsistencyLevel::Strong);
        // No book / desk match ⇒ the tenant layer resolves Strong.
        assert_eq!(
            policy.resolve("BOOK-X", Some("DESK-Y"), Some("ACME")),
            ConsistencyLevel::Strong
        );
        // A desk override is more specific than the tenant.
        policy.set_desk("DESK-Y", ConsistencyLevel::Local);
        assert_eq!(
            policy.resolve("BOOK-X", Some("DESK-Y"), Some("ACME")),
            ConsistencyLevel::Local
        );
    }

    #[test]
    fn rates_books_resolve_by_numeric_id() {
        let mut policy = ConsistencyPolicy::new();
        policy.set_rates_book(7, ConsistencyLevel::Strong);
        assert_eq!(policy.resolve_rates(7), ConsistencyLevel::Strong);
        assert_eq!(policy.resolve_rates(8), ConsistencyLevel::Local);
        assert!(policy.any_strong());
    }
}
