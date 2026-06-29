//! Persisted inbound FIX-acceptor connection definitions.
//!
//! The edge can bind many inbound FIX acceptors; their definitions are stored as
//! a small JSON document so they survive restarts and auto-load on boot. This
//! module owns only the *data + persistence* — the live bind/stop lifecycle is the
//! [`crate::services::fix_registry`]'s job, and the management API is
//! `FixAdminService`.
//!
//! The schema is deliberately **kind-generic**: [`AcceptorKind`] starts with one
//! variant (`Options`, the existing FX-options dialect) and the spot dialect adds a
//! variant in phase 2 without reshaping the store, the API, or the UI.

use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Env var naming the JSON config file. Absent ⇒ [`DEFAULT_CONFIG_PATH`].
pub const CONFIG_ENV: &str = "CELNET_FIX_CONFIG";
/// Default config path (repo-/cwd-local, human-editable) when the env is unset.
pub const DEFAULT_CONFIG_PATH: &str = "fix-connections.json";

/// The dialect an inbound acceptor speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptorKind {
    /// The FX-options dialect served by the existing `celnet-fix` acceptor.
    Options,
    /// The fixed-income (linear-rates / OIS) one-shot RFQ dialect: an inbound
    /// `QuoteRequest(R)` carrying `SubscriptionRequestType(263)=0` is priced to a
    /// one-shot `Quote(S)` through the shared rates path
    /// ([`crate::services::fix`] dispatches it to `celnet_fix::dialect_rates`).
    FixedIncomeQuote,
    /// The fixed-income (linear-rates / OIS) streaming RFS dialect: an inbound
    /// `QuoteRequest(R)` carrying `SubscriptionRequestType(263)=1` subscribes to a
    /// streamed request-for-stream, served by the same rates dialect.
    FixedIncomeStream,
    // Phase 2: `SpotFx` — a strike/expiry-less spot two-way dialect.
}

impl AcceptorKind {
    /// A stable lowercase wire/display token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AcceptorKind::Options => "options",
            AcceptorKind::FixedIncomeQuote => "fixed_income_quote",
            AcceptorKind::FixedIncomeStream => "fixed_income_stream",
        }
    }

    /// Parse the wire/display token (case-insensitive).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "options" => Some(AcceptorKind::Options),
            "fixed_income_quote" => Some(AcceptorKind::FixedIncomeQuote),
            "fixed_income_stream" => Some(AcceptorKind::FixedIncomeStream),
            _ => None,
        }
    }
}

/// One persisted inbound FIX-acceptor definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixConnectionDef {
    /// Stable identifier (the registry/API key). Never reused.
    pub id: String,
    /// Human-friendly label shown in the UI.
    pub name: String,
    /// Which dialect the acceptor speaks.
    pub kind: AcceptorKind,
    /// The `host:port` to bind, e.g. `127.0.0.1:9099`.
    pub bind_addr: String,
    /// Our venue `SenderCompID`.
    pub sender_comp_id: String,
    /// The expected counterparty `SenderCompID` (the FSM rejects any other peer).
    pub target_comp_id: String,
    /// Whether the acceptor should be (and stay) bound.
    pub enabled: bool,
    /// The owning desk id (`DeskDef::id`). Every connection created through the
    /// admin API belongs to a desk (`fix_admin::def_from_spec` rejects a blank
    /// desk); a desk-scoped (non-admin) session sees only connections whose
    /// `desk` matches its own, while an admin sees all. `#[serde(default)]` keeps
    /// configs written before desk ownership (no `desk` key) loading with an empty
    /// desk rather than failing to parse.
    #[serde(default)]
    pub desk: String,
}

impl FixConnectionDef {
    /// Structural validation independent of any other connection. Cross-cutting
    /// checks (unique id/name, address conflicts vs other enabled acceptors) live
    /// in the registry, which knows the live set.
    ///
    /// # Errors
    /// Returns a human-readable message naming the first failing field.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("id must not be empty".to_string());
        }
        if self.name.trim().is_empty() {
            return Err("name must not be empty".to_string());
        }
        if self.sender_comp_id.trim().is_empty() {
            return Err("sender_comp_id must not be empty".to_string());
        }
        if self.target_comp_id.trim().is_empty() {
            return Err("target_comp_id must not be empty".to_string());
        }
        self.socket_addr()
            .map(|_| ())
            .ok_or_else(|| format!("bind_addr `{}` is not a valid host:port", self.bind_addr))
    }

    /// The parsed bind address, or `None` if `bind_addr` is malformed.
    #[must_use]
    pub fn socket_addr(&self) -> Option<SocketAddr> {
        self.bind_addr.parse().ok()
    }
}

/// The persisted document: an ordered list of connection definitions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixConnectionStore {
    /// The ordered set of inbound FIX-acceptor definitions.
    #[serde(default)]
    pub connections: Vec<FixConnectionDef>,
}

impl FixConnectionStore {
    /// Resolve the config path from [`CONFIG_ENV`], falling back to
    /// [`DEFAULT_CONFIG_PATH`].
    #[must_use]
    pub fn config_path() -> PathBuf {
        std::env::var_os(CONFIG_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH))
    }

    /// Load from `path`. A **missing** file is a first run ⇒ empty store (not an
    /// error); a present-but-corrupt file is an `InvalidData` error so a broken
    /// config fails loudly rather than silently dropping saved acceptors.
    ///
    /// # Errors
    /// Propagates IO errors other than not-found, and JSON parse failures.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Persist **atomically**: pretty-print to a sibling `*.tmp` file then rename
    /// over `path`, so a crash mid-write can never leave a half-written config
    /// (the same durability discipline as `celnet_fix::session::FileStore`).
    ///
    /// # Errors
    /// Propagates IO/serialization failures.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = tmp_sibling(path);
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Borrow a connection by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&FixConnectionDef> {
        self.connections.iter().find(|c| c.id == id)
    }

    /// Insert a new connection or replace the existing one with the same id
    /// (preserving position on replace).
    pub fn upsert(&mut self, def: FixConnectionDef) {
        if let Some(slot) = self.connections.iter_mut().find(|c| c.id == def.id) {
            *slot = def;
        } else {
            self.connections.push(def);
        }
    }

    /// Remove the connection with `id`; returns whether one was removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.connections.len();
        self.connections.retain(|c| c.id != id);
        self.connections.len() != before
    }
}

/// `path` with `.tmp` appended to its file name (a sibling temp for atomic save).
fn tmp_sibling(path: &Path) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FixConnectionDef {
        FixConnectionDef {
            id: "opt-1".to_string(),
            name: "Bank A options".to_string(),
            kind: AcceptorKind::Options,
            bind_addr: "127.0.0.1:9099".to_string(),
            sender_comp_id: "CELNET".to_string(),
            target_comp_id: "CELNET-CPTY".to_string(),
            enabled: true,
            desk: "g10".to_string(),
        }
    }

    #[test]
    fn json_round_trips_through_store() {
        let mut store = FixConnectionStore::default();
        store.upsert(sample());
        let bytes = serde_json::to_vec(&store).unwrap();
        let back: FixConnectionStore = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(store, back);
        assert_eq!(back.get("opt-1").unwrap().kind, AcceptorKind::Options);
        assert_eq!(back.get("opt-1").unwrap().desk, "g10");
    }

    #[test]
    fn legacy_config_without_desk_loads_as_unowned() {
        // A config written before desk ownership has no `desk` key; serde's
        // default must fill it as an empty (unowned) string, not fail to parse.
        let legacy = r#"{"connections":[{"id":"opt-1","name":"Bank A","kind":"options",
            "bind_addr":"127.0.0.1:9099","sender_comp_id":"CELNET",
            "target_comp_id":"CELNET-CPTY","enabled":true}]}"#;
        let store: FixConnectionStore = serde_json::from_str(legacy).unwrap();
        assert_eq!(store.get("opt-1").unwrap().desk, "");
    }

    #[test]
    fn missing_file_is_an_empty_store() {
        let dir = std::env::temp_dir().join("celnet-fixcfg-missing");
        let path = dir.join("does-not-exist.json");
        let _ = std::fs::remove_file(&path);
        let store = FixConnectionStore::load(&path).unwrap();
        assert!(store.connections.is_empty());
    }

    #[test]
    fn save_is_atomic_and_reloads_equal() {
        // A unique path per run keeps the test isolated and parallel-safe.
        let dir = std::env::temp_dir().join("celnet-fixcfg-save");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("conns-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut store = FixConnectionStore::default();
        store.upsert(sample());
        store.save(&path).unwrap();

        // The temp sibling must not linger after a successful rename.
        assert!(!tmp_sibling(&path).exists(), "atomic temp left behind");

        let reloaded = FixConnectionStore::load(&path).unwrap();
        assert_eq!(store, reloaded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upsert_replaces_in_place_and_remove_works() {
        let mut store = FixConnectionStore::default();
        store.upsert(sample());
        let mut renamed = sample();
        renamed.name = "Bank A (renamed)".to_string();
        store.upsert(renamed);
        assert_eq!(
            store.connections.len(),
            1,
            "upsert must replace, not append"
        );
        assert_eq!(store.get("opt-1").unwrap().name, "Bank A (renamed)");
        assert!(store.remove("opt-1"));
        assert!(!store.remove("opt-1"));
        assert!(store.connections.is_empty());
    }

    #[test]
    fn validate_rejects_empty_fields_and_bad_addr() {
        assert!(sample().validate().is_ok());

        let mut empty_name = sample();
        empty_name.name = "  ".to_string();
        assert!(empty_name.validate().is_err());

        let mut bad_addr = sample();
        bad_addr.bind_addr = "not-an-addr".to_string();
        assert!(bad_addr.validate().is_err());
        assert!(bad_addr.socket_addr().is_none());
    }

    #[test]
    fn kind_token_round_trips() {
        assert_eq!(AcceptorKind::parse("OPTIONS"), Some(AcceptorKind::Options));
        assert_eq!(AcceptorKind::Options.as_str(), "options");
        assert_eq!(AcceptorKind::parse("spot"), None);
        // The two fixed-income dialects round-trip through their stable tokens.
        assert_eq!(
            AcceptorKind::parse("Fixed_Income_Quote"),
            Some(AcceptorKind::FixedIncomeQuote)
        );
        assert_eq!(
            AcceptorKind::FixedIncomeQuote.as_str(),
            "fixed_income_quote"
        );
        assert_eq!(
            AcceptorKind::parse("fixed_income_stream"),
            Some(AcceptorKind::FixedIncomeStream)
        );
        assert_eq!(
            AcceptorKind::FixedIncomeStream.as_str(),
            "fixed_income_stream"
        );
    }
}
