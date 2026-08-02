//! Runtime registry of managed inbound FIX acceptors.
//!
//! Owns the live bind/stop lifecycle for the persisted connection definitions
//! ([`crate::config::fix_connections`]): it loads the JSON store, binds every
//! enabled acceptor on boot, and applies create/update/delete/enable mutations at
//! runtime — persisting after every change so the set survives a restart.
//!
//! Each acceptor reuses the existing single-acceptor machinery
//! ([`crate::services::fix::FixAcceptor`] + [`FixContext`]); the registry is just
//! the many-of map plus persistence and validation. The legacy `CELNET_FIX_ADDR`
//! env seed is bound separately by [`crate::Edge`] and is independent of this
//! registry (a dev affordance, not a managed/persisted connection).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use tokio::sync::Mutex;

use crate::clock::Clock;
use crate::config::fix_connections::{FixConnectionDef, FixConnectionStore};
use crate::config::identity::IdentityStore;
use crate::core_link::CoreLink;
use crate::services::desk::RfqDeskEdge;
use crate::services::fix::{FixAcceptor, FixContext, RatesAutoQuotePolicy};
use crate::services::fix_monitor::FixMonitor;
use crate::services::risk::store::PositionStore;
use crate::spread::SpreadModel;
use crate::surface_book::SurfaceBook;

/// Resolves whether a routing-desk id names a currently-defined desk.
///
/// This is the narrow seam the FIX acceptor registry uses to (a) validate that a
/// connection's routing desk references a real desk on the admin create/update
/// path and (b) warn loudly at bind time when a connection's routing desk does not
/// resolve (an unrouted venue whose inbound RFQ/deal notifications would be
/// silently dropped). It deliberately depends only on desk existence, not on the
/// whole identity store, so the registry stays decoupled from user/auth concerns.
pub trait DeskDirectory: Send + Sync {
    /// Whether `desk_id` names a currently-defined desk.
    fn desk_exists(&self, desk_id: &str) -> bool;
}

/// The live identity store is the source of truth for defined desks. A poisoned
/// lock resolves as "not found", which fails safe toward the loud warn/reject
/// path rather than silently treating an unknown desk as valid.
impl DeskDirectory for std::sync::Mutex<IdentityStore> {
    fn desk_exists(&self, desk_id: &str) -> bool {
        self.lock().is_ok_and(|store| store.desk(desk_id).is_some())
    }
}

/// A connection's routing-desk resolution state against the wired desk directory.
/// Drives both the admin-path rejection and the bind-time log level, and is the
/// deterministic decision the guard tests assert on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutingDeskState {
    /// No routing desk set — an intentionally-unrouted venue (a quiet debug line).
    Unrouted,
    /// The routing desk resolves to a defined desk (nothing to warn about).
    Resolved,
    /// The routing desk is set but resolves to no defined desk — the silent-drop
    /// failure mode (a loud warn at bind, a hard reject on the admin path).
    Undefined,
    /// No desk directory is wired, so existence cannot be decided (skip checks).
    Undecidable,
}

/// A connection definition plus its live runtime status (for the admin API/UI).
#[derive(Debug, Clone)]
pub struct ConnectionStatus {
    /// The persisted definition.
    pub def: FixConnectionDef,
    /// Whether an acceptor is currently bound and listening.
    pub running: bool,
    /// The actually-bound address (resolves an ephemeral `:0`), when running.
    pub bound_addr: Option<SocketAddr>,
}

/// The mutable interior, guarded by one async mutex so all admin ops serialize.
struct Inner {
    store: FixConnectionStore,
    running: HashMap<String, FixAcceptor>,
}

/// Manages the set of inbound FIX acceptors and their persistence.
pub struct FixAcceptorRegistry {
    inner: Mutex<Inner>,
    link: Arc<CoreLink>,
    spread: SpreadModel,
    clock: Clock,
    surface_book: Arc<SurfaceBook>,
    monitor: Arc<FixMonitor>,
    /// The shared live position book every managed acceptor gates against (ADR-0016 A1),
    /// so a FIX lift on any managed connection sees the same pre-trade limit tree.
    store: Arc<PositionStore>,
    /// The dealer-quoting desk inbox a managed fixed-income acceptor records inbound
    /// RFQs into (so the GUI desk shows what a FIX venue received). `None` on a build
    /// that doesn't wire a desk (never, in the live edge).
    desk_edge: Option<Arc<RfqDeskEdge>>,
    /// The auto-quote admission policy every fixed-income acceptor applies (notional
    /// cap + on-the-run tenors): admitted ⇒ auto-quoted, declined ⇒ routed to a desk.
    auto_quote: RatesAutoQuotePolicy,
    /// Resolves whether a connection's routing desk names a defined desk. Injected
    /// once after boot (the identity store is constructed after this registry), so
    /// it is a set-once [`OnceLock`]. When present, the admin create/update path
    /// rejects a routing desk that does not resolve, and [`Self::bind_acceptor`]
    /// emits a loud unresolved-desk warning. Absent (e.g. in unit tests that never
    /// wire it) ⇒ desk-resolution checks are skipped best-effort.
    desk_directory: OnceLock<Arc<dyn DeskDirectory>>,
    /// The live aggregated-book composite and pricing-group registry a managed
    /// fixed-income **stream** acceptor prices its outbound RFS/ESP off (composite-based
    /// and tiered per the connection's pricing group — see
    /// [`FixContext::with_aggregation`]). Injected once after boot (the hub is constructed
    /// alongside this registry but wired here through a set-once handle to keep
    /// [`Self::load`]'s signature stable), so it is a set-once [`OnceLock`]. Absent (unit
    /// tests that never wire it) ⇒ the stream keeps the standalone P0 demo re-price,
    /// byte-identical.
    aggregation_hub: OnceLock<Arc<crate::services::aggregation::AggregationHub>>,
    /// The firm-wide runtime **pricing kill-switch** each managed acceptor's context is
    /// wired with so its outbound RFQ auto-quotes + RFS/ESP streams honour
    /// `SetPricingControl`. Injected once at boot (set-once), like `aggregation_hub`.
    /// Absent (unit tests) ⇒ the acceptor keeps `FixContext`'s default both-enabled
    /// control, byte-identical to before.
    pricing_control: OnceLock<Arc<crate::services::pricing_control::PricingControl>>,
    config_path: PathBuf,
}

impl std::fmt::Debug for FixAcceptorRegistry {
    /// A shape-only `Debug` (the interior engine handles — `CoreLink`, the live
    /// acceptors — are not `Debug`; the async mutex is not locked here). Exists so the
    /// owning [`crate::Edge`] can derive `Debug`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FixAcceptorRegistry")
            .field("config_path", &self.config_path)
            .finish_non_exhaustive()
    }
}

impl FixAcceptorRegistry {
    /// Build the registry from the persisted store at `config_path`. Does **not**
    /// bind anything yet — call [`Self::start_enabled`] once the edge is up. A
    /// corrupt store file is surfaced as an error so a broken config fails loudly.
    ///
    /// # Errors
    /// Propagates a corrupt-config load error.
    // Constructor-style loader wiring the registry's collaborators; the house
    // convention allows the arg count rather than boxing indirection into every caller.
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        link: Arc<CoreLink>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
        monitor: Arc<FixMonitor>,
        store: Arc<PositionStore>,
        desk_edge: Option<Arc<RfqDeskEdge>>,
        config_path: PathBuf,
    ) -> std::io::Result<Self> {
        let connections = FixConnectionStore::load(&config_path)?;
        Ok(Self {
            inner: Mutex::new(Inner {
                store: connections,
                running: HashMap::new(),
            }),
            link,
            spread,
            clock,
            surface_book,
            monitor,
            store,
            desk_edge,
            auto_quote: RatesAutoQuotePolicy::default(),
            desk_directory: OnceLock::new(),
            aggregation_hub: OnceLock::new(),
            pricing_control: OnceLock::new(),
            config_path,
        })
    }

    /// Inject the aggregated-book composite and pricing-group registry a managed
    /// fixed-income **stream** acceptor prices its outbound RFS/ESP off (composite-based
    /// and tiered per the connection's pricing group). Called once at boot after the hub
    /// is built, before [`Self::start_enabled`]. Idempotent: a second call is ignored
    /// (set-once). Absent ⇒ streams keep the standalone P0 demo re-price.
    pub fn set_aggregation_hub(&self, hub: Arc<crate::services::aggregation::AggregationHub>) {
        let _ = self.aggregation_hub.set(hub);
    }

    /// Inject the firm-wide runtime pricing kill-switch each managed acceptor's outbound
    /// pricing is gated by. Called once at boot after the control is built, before
    /// [`Self::start_enabled`]. Idempotent (set-once); absent ⇒ acceptors keep the
    /// default both-enabled control.
    pub fn set_pricing_control(
        &self,
        control: Arc<crate::services::pricing_control::PricingControl>,
    ) {
        let _ = self.pricing_control.set(control);
    }

    /// Inject the desk directory used to validate connection routing desks and to
    /// warn on an unresolved one. Called once at boot after the identity store is
    /// built (which happens after this registry is loaded), before
    /// [`Self::start_enabled`]. Idempotent: a second call is ignored (set-once).
    pub fn set_desk_directory(&self, directory: Arc<dyn DeskDirectory>) {
        let _ = self.desk_directory.set(directory);
    }

    /// Resolve a connection's routing-desk state against the wired desk directory.
    /// This is the single decision both the admin-path rejection
    /// ([`Self::validate_routing_desk`]) and the bind-time warning
    /// ([`Self::warn_on_unrouted_desk`]) act on, so it is deterministic and
    /// unit-testable without capturing tracing output.
    fn routing_desk_state(&self, def: &FixConnectionDef) -> RoutingDeskState {
        resolve_routing_desk_state(self.desk_directory.get(), def)
    }

    /// Reject a routing desk that is set but does not resolve to a defined desk on
    /// the admin create/update path. A blank routing desk is accepted here (its
    /// unrouted-but-valid state is surfaced by the bind-time warning); an
    /// undecidable state (no directory wired) is accepted best-effort.
    ///
    /// # Errors
    /// The connection's non-blank `desk` does not name a defined desk.
    fn validate_routing_desk(&self, def: &FixConnectionDef) -> Result<(), String> {
        if self.routing_desk_state(def) == RoutingDeskState::Undefined {
            return Err(format!(
                "routing desk `{}` is not a defined desk — create the desk first \
                 or choose an existing one",
                def.desk.trim()
            ));
        }
        Ok(())
    }

    /// Emit a structured warning at bind time when a connection's routing desk is
    /// set but does not resolve to a defined desk — a silent-drop failure mode, as
    /// its inbound RFQ/deal notifications route to a desk no user is on and are
    /// dropped. Also emits a quiet debug line when the routing desk is unset (an
    /// intentionally-unrouted venue). Never fails the bind: desks may be created
    /// after a connection binds, and the connection re-warns on its next bind.
    /// A no-op when the state is undecidable (no desk directory wired).
    fn warn_on_unrouted_desk(&self, def: &FixConnectionDef) {
        match self.routing_desk_state(def) {
            RoutingDeskState::Undefined => tracing::warn!(
                connection = %def.id,
                name = %def.name,
                desk = %def.desk.trim(),
                "FIX connection routes to an UNDEFINED desk — RFQ/deal notifications \
                 for this venue will be dropped (no user is on this desk); create the \
                 desk or reassign the connection"
            ),
            RoutingDeskState::Unrouted => tracing::debug!(
                connection = %def.id,
                name = %def.name,
                "FIX connection has no routing desk — inbound RFQ/deal notifications \
                 for this venue are not routed to any desk"
            ),
            RoutingDeskState::Resolved | RoutingDeskState::Undecidable => {}
        }
    }

    /// Bind every **enabled** connection. Called once at boot; a per-connection
    /// bind failure is logged and skipped so one bad address can't block the rest.
    pub async fn start_enabled(&self) {
        let mut g = self.inner.lock().await;
        let defs: Vec<FixConnectionDef> = g
            .store
            .connections
            .iter()
            .filter(|c| c.enabled)
            .cloned()
            .collect();
        for def in defs {
            match self.bind_acceptor(&def).await {
                Ok(acc) => {
                    g.running.insert(def.id.clone(), acc);
                }
                Err(e) => {
                    eprintln!(
                        "[fix-registry] could not start `{}` ({}): {e}",
                        def.id, def.bind_addr
                    );
                }
            }
        }
    }

    /// Stop every running acceptor (in-flight sessions run to their own close).
    /// Used on edge shutdown.
    pub async fn abort_all(&self) {
        let mut g = self.inner.lock().await;
        for (_, acc) in g.running.drain() {
            acc.abort();
        }
    }

    /// The current connections and their live status.
    pub async fn list(&self) -> Vec<ConnectionStatus> {
        let g = self.inner.lock().await;
        g.store
            .connections
            .iter()
            .map(|def| status_of(&g, def))
            .collect()
    }

    /// Define a new connection. Validates structurally and for uniqueness, binds
    /// it if enabled (a bind failure aborts the whole op — nothing is persisted),
    /// then persists.
    ///
    /// # Errors
    /// Validation, uniqueness, address-conflict, bind or persist failure.
    pub async fn create(&self, def: FixConnectionDef) -> Result<ConnectionStatus, String> {
        def.validate()?;
        self.validate_routing_desk(&def)?;
        let mut g = self.inner.lock().await;
        if g.store.get(&def.id).is_some() {
            return Err(format!("a connection with id `{}` already exists", def.id));
        }
        if g.store.connections.iter().any(|c| c.name == def.name) {
            return Err(format!("a connection named `{}` already exists", def.name));
        }
        self.commit(&mut g, def).await
    }

    /// Replace an existing connection's definition (same id). Restarts the
    /// acceptor to reflect any address/CompID/enabled change.
    ///
    /// # Errors
    /// Not-found, validation, name conflict, address conflict, bind or persist failure.
    pub async fn update(
        &self,
        id: &str,
        mut def: FixConnectionDef,
    ) -> Result<ConnectionStatus, String> {
        def.id = id.to_string();
        def.validate()?;
        self.validate_routing_desk(&def)?;
        let mut g = self.inner.lock().await;
        if g.store.get(id).is_none() {
            return Err(format!("no connection with id `{id}`"));
        }
        if g.store
            .connections
            .iter()
            .any(|c| c.id != id && c.name == def.name)
        {
            return Err(format!("a connection named `{}` already exists", def.name));
        }
        // Drop any currently-running acceptor for this id before re-binding.
        if let Some(acc) = g.running.remove(id) {
            acc.abort();
        }
        self.commit(&mut g, def).await
    }

    /// Enable or disable a connection: bind/abort the acceptor and persist.
    ///
    /// # Errors
    /// Not-found, address conflict, bind or persist failure.
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<ConnectionStatus, String> {
        let mut g = self.inner.lock().await;
        let mut def = g
            .store
            .get(id)
            .cloned()
            .ok_or_else(|| format!("no connection with id `{id}`"))?;
        if def.enabled == enabled && g.running.contains_key(id) == enabled {
            return Ok(status_of(&g, &def)); // already in the requested state
        }
        if !enabled && let Some(acc) = g.running.remove(id) {
            acc.abort();
        }
        def.enabled = enabled;
        self.commit(&mut g, def).await
    }

    /// Delete a connection: abort its acceptor (if running), drop it, and persist.
    ///
    /// # Errors
    /// Not-found or persist failure.
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        let mut g = self.inner.lock().await;
        if g.store.get(id).is_none() {
            return Err(format!("no connection with id `{id}`"));
        }
        if let Some(acc) = g.running.remove(id) {
            acc.abort();
        }
        g.store.remove(id);
        self.persist(&g.store)?;
        Ok(())
    }

    // --- internals ---------------------------------------------------------

    /// Apply (bind-if-enabled + store-upsert + persist) for `def`, with the guard
    /// held so the whole op is atomic w.r.t. other admin calls. On enable, an
    /// address already used by another enabled connection is rejected.
    async fn commit(
        &self,
        g: &mut Inner,
        def: FixConnectionDef,
    ) -> Result<ConnectionStatus, String> {
        if def.enabled {
            if let Some(other) = enabled_addr_conflict(g, &def) {
                return Err(format!(
                    "address `{}` is already used by enabled connection `{}`",
                    def.bind_addr, other
                ));
            }
            // Re-bind from scratch (callers remove any prior acceptor first).
            let acc = self.bind_acceptor(&def).await?;
            g.running.insert(def.id.clone(), acc);
        }
        g.store.upsert(def.clone());
        self.persist(&g.store)?;
        Ok(status_of(g, &def))
    }

    /// Bind one acceptor with the connection's own CompIDs over the shared engine
    /// services. Does not touch the registry map (so it never re-locks).
    async fn bind_acceptor(&self, def: &FixConnectionDef) -> Result<FixAcceptor, String> {
        let addr = def
            .socket_addr()
            .ok_or_else(|| format!("bind_addr `{}` is not a valid host:port", def.bind_addr))?;
        self.warn_on_unrouted_desk(def);
        let ctx = FixContext::with_comp_ids(
            Arc::clone(&self.link),
            self.spread,
            self.clock.clone(),
            Arc::clone(&self.surface_book),
            def.sender_comp_id.clone().into_bytes(),
            def.target_comp_id.clone().into_bytes(),
            Arc::clone(&self.monitor),
            def.id.clone(),
            def.kind,
            Arc::clone(&self.store),
        )
        // A fixed-income venue records inbound RFQs into the desk inbox under its
        // configured desk and applies the shared auto-quote policy; an FX-options
        // acceptor ignores this (its path never records to a desk).
        .with_desk_routing(
            self.desk_edge.clone(),
            def.desk.clone(),
            self.auto_quote.clone(),
        )
        // Price a fixed-income STREAM venue's outbound RFS/ESP off the aggregated-book
        // composite through the connection's pricing group when a book covers the streamed
        // instrument (design §5); absent ⇒ the standalone P0 demo re-price.
        .with_aggregation(self.aggregation_hub.get().cloned())
        // Gate this venue's outbound RFQ auto-quotes + RFS/ESP streams on the firm-wide
        // kill-switch; absent ⇒ the default both-enabled control (byte-identical).
        .with_pricing_control(self.pricing_control.get().cloned());
        FixAcceptor::start(addr, ctx)
            .await
            .map_err(|e| format!("could not bind {addr}: {e}"))
    }

    fn persist(&self, store: &FixConnectionStore) -> Result<(), String> {
        store
            .save(&self.config_path)
            .map_err(|e| format!("could not persist FIX connection config: {e}"))
    }
}

/// The runtime status for `def` given the current interior.
fn status_of(g: &Inner, def: &FixConnectionDef) -> ConnectionStatus {
    let acc = g.running.get(&def.id);
    ConnectionStatus {
        def: def.clone(),
        running: acc.is_some(),
        bound_addr: acc.map(FixAcceptor::local_addr),
    }
}

/// Decide a connection's [`RoutingDeskState`] from its routing desk and the wired
/// desk directory. A free function (no registry collaborators) so the guard's
/// decision is unit-testable directly: the bind-time warn fires **iff** this
/// returns [`RoutingDeskState::Undefined`], and never for a resolved desk.
fn resolve_routing_desk_state(
    directory: Option<&Arc<dyn DeskDirectory>>,
    def: &FixConnectionDef,
) -> RoutingDeskState {
    let desk = def.desk.trim();
    if desk.is_empty() {
        return RoutingDeskState::Unrouted;
    }
    match directory {
        None => RoutingDeskState::Undecidable,
        Some(dir) if dir.desk_exists(desk) => RoutingDeskState::Resolved,
        Some(_) => RoutingDeskState::Undefined,
    }
}

/// The id of another **enabled** connection bound to the same socket address as
/// `def`, if any (a self-id match is ignored).
fn enabled_addr_conflict(g: &Inner, def: &FixConnectionDef) -> Option<String> {
    let want = def.socket_addr()?;
    g.store
        .connections
        .iter()
        .find(|c| c.id != def.id && c.enabled && c.socket_addr() == Some(want))
        .map(|c| c.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::fix_connections::AcceptorKind;
    use crate::config::identity::DeskDef;

    fn conn_routed_to(desk: &str) -> FixConnectionDef {
        FixConnectionDef {
            id: "cpty-1".into(),
            name: "Counterparty One".into(),
            kind: AcceptorKind::FixedIncomeQuote,
            bind_addr: "127.0.0.1:0".into(),
            sender_comp_id: "CELNET".into(),
            target_comp_id: "CPTY".into(),
            enabled: true,
            desk: desk.into(),
        }
    }

    /// A desk directory backed by the real identity store, seeded with `desk_ids`.
    fn directory_with(desk_ids: &[&str]) -> Arc<dyn DeskDirectory> {
        let mut store = IdentityStore::default();
        for id in desk_ids {
            store.desks.push(DeskDef {
                id: (*id).into(),
                name: (*id).into(),
                books: Vec::new(),
            });
        }
        Arc::new(std::sync::Mutex::new(store))
    }

    /// The bind-time warn fires **iff** the routing desk is set-but-undefined, and
    /// never for a resolved desk — the silent-drop guard's core decision. An unset
    /// desk is the intentionally-unrouted state; no directory ⇒ undecidable (skip).
    #[test]
    fn routing_desk_state_flags_undefined_and_clears_resolved() {
        let dir = directory_with(&["g10"]);
        assert_eq!(
            resolve_routing_desk_state(Some(&dir), &conn_routed_to("em-vol")),
            RoutingDeskState::Undefined,
            "a set-but-undefined routing desk must be flagged (warn fires)"
        );
        assert_eq!(
            resolve_routing_desk_state(Some(&dir), &conn_routed_to("g10")),
            RoutingDeskState::Resolved,
            "a defined routing desk must resolve cleanly (no warn)"
        );
        assert_eq!(
            resolve_routing_desk_state(Some(&dir), &conn_routed_to("   ")),
            RoutingDeskState::Unrouted,
            "a blank routing desk is the intentionally-unrouted state"
        );
        assert_eq!(
            resolve_routing_desk_state(None, &conn_routed_to("em-vol")),
            RoutingDeskState::Undecidable,
            "no directory wired ⇒ existence is undecidable, checks skipped"
        );
    }

    /// The real `IdentityStore`-backed directory resolves desks by their stable id,
    /// so a rename of a desk's display name (which never touches the id) can never
    /// turn a resolved routing desk into an undefined one.
    #[test]
    fn identity_store_directory_resolves_by_stable_id() {
        let dir = directory_with(&["g10", "credit"]);
        assert!(dir.desk_exists("g10"));
        assert!(dir.desk_exists("credit"));
        assert!(!dir.desk_exists("g10-options"));
    }

    /// `validate_routing_desk`'s decision is the same predicate: an undefined desk
    /// is the only rejectable state; resolved / unrouted / undecidable all pass.
    #[test]
    fn only_undefined_routing_desk_is_rejectable() {
        let dir = directory_with(&["g10"]);
        for (desk, directory, rejectable) in [
            ("em-vol", Some(&dir), true),
            ("g10", Some(&dir), false),
            ("   ", Some(&dir), false),
            ("em-vol", None, false),
        ] {
            let state = resolve_routing_desk_state(directory, &conn_routed_to(desk));
            assert_eq!(
                state == RoutingDeskState::Undefined,
                rejectable,
                "desk {desk:?} with directory={} rejectable mismatch",
                directory.is_some()
            );
        }
    }
}
