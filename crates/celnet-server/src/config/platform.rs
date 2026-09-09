//! Unified declarative platform configuration (`PlatformConfig`).
//!
//! Provides a centralized, typed, declarative configuration model for all Celnet
//! subsystems: consistency tiers, GPU batch sizing, fuel-metered plugins,
//! high-throughput fleet topology, risk limits, SBE shared-memory IPC, and
//! real-time telemetry SLOs.
//!
//! Replaces ad-hoc environment variable proliferation while preserving 100% backward
//! compatibility via graceful environment fallback ([`PlatformConfig::from_env`]).

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::consistency::{ConsistencyLevel, ConsistencyPolicy};

/// Environment variable pointing to the declarative configuration file.
pub const CELNET_CONFIG_ENV: &str = "CELNET_CONFIG_PATH";
/// Default path for the configuration file if not specified.
pub const DEFAULT_CONFIG_FILE: &str = "celnet.json";

/// Declarative configuration for the configurable-consistency tier (ADR-0015).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsistencyConfig {
    /// Platform-wide default consistency level (`local` or `strong`).
    #[serde(default)]
    pub platform_default: ConsistencyLevel,
    /// Per-tenant consistency overrides.
    #[serde(default)]
    pub tenants: HashMap<String, ConsistencyLevel>,
    /// Per-desk consistency overrides.
    #[serde(default)]
    pub desks: HashMap<String, ConsistencyLevel>,
    /// Per-FX-book consistency overrides.
    #[serde(default)]
    pub books: HashMap<String, ConsistencyLevel>,
    /// Per-rates-book consistency overrides (keyed by numeric cell id).
    #[serde(default)]
    pub rates_books: HashMap<u32, ConsistencyLevel>,
}

impl Default for ConsistencyConfig {
    fn default() -> Self {
        Self {
            platform_default: ConsistencyLevel::Local,
            tenants: HashMap::new(),
            desks: HashMap::new(),
            books: HashMap::new(),
            rates_books: HashMap::new(),
        }
    }
}

/// Declarative configuration for hardware-accelerated GPU risk batching.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuConfig {
    /// Whether GPU compute acceleration is enabled when supported hardware is present.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Minimum portfolio batch size before offloading computation from CPU SIMD to GPU.
    #[serde(default = "default_gpu_batch_size")]
    pub min_batch_size: usize,
    /// Preferred GPU backend/device descriptor ("auto", "high_performance", "low_power").
    #[serde(default = "default_device_pref")]
    pub device_preference: String,
}

fn default_gpu_batch_size() -> usize {
    1024
}

fn default_device_pref() -> String {
    "auto".to_string()
}

impl Default for GpuConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_batch_size: default_gpu_batch_size(),
            device_preference: default_device_pref(),
        }
    }
}

/// Declarative configuration for the Wasm fuel-metered plugin host sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Whether user-defined pricing plugins are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Maximum execution fuel per invocation before termination.
    #[serde(default = "default_fuel_limit")]
    pub fuel_limit: u64,
    /// Directories searched for compiled `.wasm` plugins.
    #[serde(default = "default_plugin_dirs")]
    pub plugin_dirs: Vec<PathBuf>,
}

fn default_fuel_limit() -> u64 {
    10_000_000
}

fn default_plugin_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from("plugins")]
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fuel_limit: default_fuel_limit(),
            plugin_dirs: default_plugin_dirs(),
        }
    }
}

/// Declarative configuration for distributed risk fleet and partition routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetConfig {
    /// Local node identifier in the cluster.
    #[serde(default = "default_node_id")]
    pub node_id: u64,
    /// Total expected cluster size for quorum calculation.
    #[serde(default = "default_cluster_size")]
    pub cluster_size: usize,
    /// Network addresses of peer nodes in the fleet.
    #[serde(default)]
    pub peers: Vec<String>,
    /// Partitioning algorithm ("hrw" / rendezvous hashing).
    #[serde(default = "default_partition_strategy")]
    pub partition_strategy: String,
}

fn default_node_id() -> u64 {
    1
}

fn default_cluster_size() -> usize {
    1
}

fn default_partition_strategy() -> String {
    "hrw".to_string()
}

impl Default for FleetConfig {
    fn default() -> Self {
        Self {
            node_id: default_node_id(),
            cluster_size: default_cluster_size(),
            peers: Vec::new(),
            partition_strategy: default_partition_strategy(),
        }
    }
}

/// Declarative configuration for real-time pre-trade risk and credit limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitsConfig {
    /// Whether pre-trade limit checks are strictly enforced on RFQ/order ingress.
    #[serde(default = "default_true")]
    pub enforce_pre_trade: bool,
    /// Default risk headroom in base currency when unconfigured.
    #[serde(default = "default_headroom")]
    pub default_risk_headroom: f64,
    /// Maximum notional allowed per individual order/quote.
    #[serde(default = "default_max_notional")]
    pub max_notional_per_order: f64,
}

fn default_headroom() -> f64 {
    1_000_000.0
}

fn default_max_notional() -> f64 {
    100_000_000.0
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            enforce_pre_trade: true,
            default_risk_headroom: default_headroom(),
            max_notional_per_order: default_max_notional(),
        }
    }
}

/// Declarative configuration for ultra-low-latency SBE shared-memory IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SbeIpcConfig {
    /// Whether SBE SHM ring buffer IPC is active for inter-process communication.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Path to the shared memory backing file.
    #[serde(default = "default_shm_path")]
    pub shm_path: String,
    /// Ring buffer capacity (number of slots, power of two).
    #[serde(default = "default_ring_capacity")]
    pub ring_capacity: usize,
}

fn default_shm_path() -> String {
    "/tmp/celnet-sbe-ring".to_string()
}

fn default_ring_capacity() -> usize {
    65536
}

impl Default for SbeIpcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            shm_path: default_shm_path(),
            ring_capacity: default_ring_capacity(),
        }
    }
}

/// Declarative configuration for real-time telemetry, tracing, and latency SLOs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Telemetry sampling rate (0.0 to 1.0).
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
    /// Service Level Objective for P99 pricing tick latency in microseconds.
    #[serde(default = "default_slo_p99")]
    pub latency_slo_p99_micros: u64,
    /// Whether structured audit logging is enabled for every booking and entitlement check.
    #[serde(default = "default_true")]
    pub structured_audit_log: bool,
}

fn default_true() -> bool {
    true
}

fn default_sample_rate() -> f64 {
    1.0
}

fn default_slo_p99() -> u64 {
    100
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            sample_rate: default_sample_rate(),
            latency_slo_p99_micros: default_slo_p99(),
            structured_audit_log: true,
        }
    }
}

/// The top-level Celnet platform configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlatformConfig {
    /// Consistency tiering settings.
    #[serde(default)]
    pub consistency: ConsistencyConfig,
    /// GPU acceleration settings.
    #[serde(default)]
    pub gpu: GpuConfig,
    /// Plugin sandbox settings.
    #[serde(default)]
    pub plugins: PluginConfig,
    /// Cluster fleet topology settings.
    #[serde(default)]
    pub fleet: FleetConfig,
    /// Pre-trade limit controls.
    #[serde(default)]
    pub limits: LimitsConfig,
    /// Shared memory SBE IPC settings.
    #[serde(default)]
    pub sbe_ipc: SbeIpcConfig,
    /// Observability and SLO telemetry.
    #[serde(default)]
    pub telemetry: TelemetryConfig,
}

impl PlatformConfig {
    /// Create a new platform config with all defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse configuration from a JSON string.
    ///
    /// # Errors
    /// Returns error message if parsing fails.
    pub fn from_json_str(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("failed to parse platform config JSON: {e}"))
    }

    /// Serialize configuration to a pretty JSON string.
    ///
    /// # Errors
    /// Returns error message if serialization fails.
    pub fn to_json_string(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize platform config: {e}"))
    }

    /// Load configuration from a file path.
    ///
    /// # Errors
    /// Returns error message if reading or parsing fails.
    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read config file at {}: {e}", path.display()))?;
        Self::from_json_str(&content)
    }

    /// Load configuration from the specified path, or check [`CELNET_CONFIG_ENV`],
    /// or fallback to [`DEFAULT_CONFIG_FILE`], or construct from environment variables.
    #[must_use]
    pub fn load_or_default(explicit_path: Option<&Path>) -> Self {
        if let Some(path) = explicit_path {
            if let Ok(cfg) = Self::load_from_file(path) {
                return cfg;
            }
        }

        if let Ok(env_path) = std::env::var(CELNET_CONFIG_ENV) {
            let path = Path::new(&env_path);
            if path.exists() {
                if let Ok(cfg) = Self::load_from_file(path) {
                    return cfg;
                }
            }
        }

        let default_path = Path::new(DEFAULT_CONFIG_FILE);
        if default_path.exists() {
            if let Ok(cfg) = Self::load_from_file(default_path) {
                return cfg;
            }
        }

        Self::from_env()
    }

    /// Construct configuration from environment variables, preserving full
    /// compatibility with existing deployment scripts.
    #[must_use]
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        let env_policy = ConsistencyPolicy::from_env();

        cfg.consistency.platform_default = env_policy.default_level();
        cfg
    }

    /// Project the consistency section into an active [`ConsistencyPolicy`].
    #[must_use]
    pub fn to_consistency_policy(&self) -> ConsistencyPolicy {
        let mut policy = ConsistencyPolicy::new();
        policy.set_default(self.consistency.platform_default);

        for (tenant, &level) in &self.consistency.tenants {
            policy.set_tenant(tenant.clone(), level);
        }
        for (desk, &level) in &self.consistency.desks {
            policy.set_desk(desk.clone(), level);
        }
        for (book, &level) in &self.consistency.books {
            policy.set_book(book.clone(), level);
        }
        for (&rates_book, &level) in &self.consistency.rates_books {
            policy.set_rates_book(rates_book, level);
        }

        policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_invariants() {
        let cfg = PlatformConfig::default();
        assert_eq!(cfg.consistency.platform_default, ConsistencyLevel::Local);
        assert!(cfg.gpu.enabled);
        assert_eq!(cfg.gpu.min_batch_size, 1024);
        assert!(cfg.plugins.enabled);
        assert_eq!(cfg.plugins.fuel_limit, 10_000_000);
        assert_eq!(cfg.fleet.node_id, 1);
        assert!(cfg.limits.enforce_pre_trade);
        assert!(cfg.sbe_ipc.enabled);
        assert_eq!(cfg.telemetry.latency_slo_p99_micros, 100);
    }

    #[test]
    fn json_round_trip() {
        let mut cfg = PlatformConfig::default();
        cfg.consistency
            .books
            .insert("FX-EXOTICS-NY".to_string(), ConsistencyLevel::Strong);
        cfg.gpu.min_batch_size = 2048;
        cfg.fleet.peers.push("10.0.0.2:9000".to_string());

        let json = cfg.to_json_string().expect("serializes cleanly");
        let parsed = PlatformConfig::from_json_str(&json).expect("deserializes cleanly");

        assert_eq!(cfg, parsed);
        assert_eq!(
            parsed.consistency.books.get("FX-EXOTICS-NY"),
            Some(&ConsistencyLevel::Strong)
        );
    }

    #[test]
    fn projects_to_consistency_policy() {
        let mut cfg = PlatformConfig::default();
        cfg.consistency
            .books
            .insert("BOOK-A".to_string(), ConsistencyLevel::Strong);
        cfg.consistency
            .desks
            .insert("DESK-NY".to_string(), ConsistencyLevel::Strong);
        cfg.consistency.rates_books.insert(42, ConsistencyLevel::Strong);

        let policy = cfg.to_consistency_policy();
        assert_eq!(policy.resolve("BOOK-A", None, None), ConsistencyLevel::Strong);
        assert_eq!(
            policy.resolve("BOOK-OTHER", Some("DESK-NY"), None),
            ConsistencyLevel::Strong
        );
        assert_eq!(
            policy.resolve("BOOK-OTHER", Some("DESK-LON"), None),
            ConsistencyLevel::Local
        );
        assert_eq!(policy.resolve_rates(42), ConsistencyLevel::Strong);
        assert_eq!(policy.resolve_rates(99), ConsistencyLevel::Local);
    }
}
