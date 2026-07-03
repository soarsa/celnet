//! The venue-adapter registry — a compile-time lookup over every seeded
//! [`VendorAdapterSpec`]. No allocation, no IO: the whole table is `&'static`.

use crate::adapters;
use crate::descriptor::VendorAdapterSpec;

/// Lookup over the seeded venue adapters, backing the control-plane's
/// `GET /adapters` surface.
#[derive(Debug, Clone, Copy)]
pub struct AdapterRegistry {
    specs: &'static [VendorAdapterSpec],
}

impl AdapterRegistry {
    /// The registry over the crate's seeded adapters.
    pub fn new() -> Self {
        Self {
            specs: adapters::ALL,
        }
    }

    /// Look up an adapter by its stable `code`.
    pub fn find(&self, code: &str) -> Option<&'static VendorAdapterSpec> {
        self.specs.iter().find(|s| s.code == code)
    }

    /// Every seeded adapter.
    pub fn list(&self) -> &'static [VendorAdapterSpec] {
        self.specs
    }

    /// Number of seeded adapters.
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_every_spec_validates() {
        let reg = AdapterRegistry::new();
        assert!(!reg.is_empty());
        let mut codes: Vec<&str> = reg.list().iter().map(|s| s.code).collect();
        let n = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), n, "adapter codes must be unique");
        for s in reg.list() {
            s.validate().unwrap_or_else(|e| panic!("{}: {e}", s.code));
        }
    }

    #[test]
    fn find_hits_and_misses() {
        let reg = AdapterRegistry::new();
        assert!(reg.find("bloomberg_fxgo_price").is_some());
        assert_eq!(
            reg.find("bloomberg_fxgo_price").unwrap().role.as_str(),
            "PRICE"
        );
        assert!(reg.find("no_such_venue").is_none());
    }
}
