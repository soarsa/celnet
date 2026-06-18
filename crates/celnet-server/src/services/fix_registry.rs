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
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::clock::Clock;
use crate::config::fix_connections::{FixConnectionDef, FixConnectionStore};
use crate::core_link::CoreLink;
use crate::services::fix::{FixAcceptor, FixContext};
use crate::spread::SpreadModel;
use crate::surface_book::SurfaceBook;

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
    pub fn load(
        link: Arc<CoreLink>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
        config_path: PathBuf,
    ) -> std::io::Result<Self> {
        let store = FixConnectionStore::load(&config_path)?;
        Ok(Self {
            inner: Mutex::new(Inner {
                store,
                running: HashMap::new(),
            }),
            link,
            spread,
            clock,
            surface_book,
            config_path,
        })
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
                    eprintln!("[fix-registry] could not start `{}` ({}): {e}", def.id, def.bind_addr);
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
    pub async fn update(&self, id: &str, mut def: FixConnectionDef) -> Result<ConnectionStatus, String> {
        def.id = id.to_string();
        def.validate()?;
        let mut g = self.inner.lock().await;
        if g.store.get(id).is_none() {
            return Err(format!("no connection with id `{id}`"));
        }
        if g.store.connections.iter().any(|c| c.id != id && c.name == def.name) {
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
    async fn commit(&self, g: &mut Inner, def: FixConnectionDef) -> Result<ConnectionStatus, String> {
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
        let ctx = FixContext::with_comp_ids(
            Arc::clone(&self.link),
            self.spread,
            self.clock.clone(),
            Arc::clone(&self.surface_book),
            def.sender_comp_id.clone().into_bytes(),
            def.target_comp_id.clone().into_bytes(),
        );
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
