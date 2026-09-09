import hashlib
import tempfile
import unittest
from celnet import DynamicRuntimeManager, LocalCapabilityCache


class TestRuntimeHydration(unittest.TestCase):
    def test_local_capability_cache(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            cache = LocalCapabilityCache(cache_dir=tmpdir)

            payload = b"wasm_bytecode_sample_v1"
            digest = hashlib.sha256(payload).hexdigest()

            self.assertFalse(cache.contains("feature.pricer.exotic", "1.0.0"))

            cache.store("feature.pricer.exotic", "1.0.0", payload, digest)
            self.assertTrue(cache.contains("feature.pricer.exotic", "1.0.0"))

            loaded = cache.load("feature.pricer.exotic", "1.0.0")
            self.assertEqual(loaded, payload)

            # Corrupted digest should raise ValueError
            with self.assertRaises(ValueError):
                cache.store("feature.pricer.exotic", "2.0.0", payload, "bad_digest_hex")

    def test_dynamic_runtime_manager_sync(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            cache = LocalCapabilityCache(cache_dir=tmpdir)
            mgr = DynamicRuntimeManager(cache=cache)

            manifest = {
                "manifest_version": 1,
                "targets": {
                    "feature.rates.multicurve": {
                        "version": "1.0.0",
                        "required_tier": "RATES_AND_BONDS",
                    },
                    "feature.margin.simm": {
                        "version": "2.6.0",
                        "required_tier": "PORTFOLIO_MARGIN",
                    },
                },
            }

            # Sync with license only authorizing RATES_AND_BONDS
            report = mgr.sync_with_manifest(manifest, licensed_tiers=["RATES_AND_BONDS"])
            self.assertEqual(report["hydrated"], ["feature.rates.multicurve"])
            self.assertEqual(report["skipped_unlicensed"], ["feature.margin.simm"])
            self.assertTrue(mgr.is_capability_active("feature.rates.multicurve"))
            self.assertFalse(mgr.is_capability_active("feature.margin.simm"))

            # Second sync reports up_to_date
            report2 = mgr.sync_with_manifest(manifest, licensed_tiers=["RATES_AND_BONDS"])
            self.assertEqual(report2["up_to_date"], ["feature.rates.multicurve"])


if __name__ == "__main__":
    unittest.main()
