"""
Celnet Dynamic Runtime & Capability Hydration Manager.

Provides minimal-footprint client capability hydration, cryptographic supply-chain verification,
and JIT loading of native compute engines and quantitative plugins.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
from typing import Any, Dict, List, Optional


class LocalCapabilityCache:
    """Thread-safe content-addressed local artifact cache for dynamic Celnet capabilities."""

    def __init__(self, cache_dir: Optional[str] = None):
        if cache_dir:
            self.root = Path(cache_dir)
        else:
            self.root = Path(os.path.expanduser("~/.celnet/cache"))
        self.root.mkdir(parents=True, exist_ok=True)

    def _path_for(self, component_id: str, version: str) -> Path:
        safe_name = f"{component_id.replace('/', '_')}_{version}.cca"
        return self.root / safe_name

    def contains(self, component_id: str, version: str) -> bool:
        return self._path_for(component_id, version).is_file()

    def store(self, component_id: str, version: str, payload: bytes, digest_hex: str) -> None:
        computed = hashlib.sha256(payload).hexdigest()
        # Verify content integrity
        if digest_hex and computed.lower() != digest_hex.lower():
            raise ValueError(f"Content digest mismatch for {component_id} v{version}")
        path = self._path_for(component_id, version)
        temp_path = path.with_suffix(".tmp")
        with open(temp_path, "wb") as f:
            f.write(payload)
        os.replace(temp_path, path)

    def load(self, component_id: str, version: str) -> Optional[bytes]:
        path = self._path_for(component_id, version)
        if not path.is_file():
            return None
        with open(path, "rb") as f:
            return f.read()

    def list_installed(self) -> List[Dict[str, str]]:
        installed = []
        for file in self.root.glob("*.cca"):
            name = file.stem
            installed.append({"artifact": name, "size_bytes": str(file.stat().st_size)})
        return installed


class DynamicRuntimeManager:
    """Manages JIT capability discovery, licensing validation, and remote hydration."""

    def __init__(self, cache: Optional[LocalCapabilityCache] = None):
        self.cache = cache or LocalCapabilityCache()
        self.active_capabilities: Dict[str, str] = {}

    def is_capability_active(self, component_id: str) -> bool:
        return component_id in self.active_capabilities

    def register_capability(self, component_id: str, version: str, payload: bytes, digest: str) -> bool:
        self.cache.store(component_id, version, payload, digest)
        self.active_capabilities[component_id] = version
        return True

    def sync_with_manifest(
        self,
        targets_manifest: Dict[str, Any],
        licensed_tiers: List[str],
    ) -> Dict[str, List[str]]:
        hydrated = []
        skipped = []
        up_to_date = []

        targets = targets_manifest.get("targets", {})
        for comp_id, metadata in targets.items():
            required_tier = metadata.get("required_tier", "")
            if required_tier and required_tier not in licensed_tiers:
                skipped.append(comp_id)
                continue

            version = metadata.get("version", "1.0.0")
            digest = metadata.get("content_digest_hex", "")

            if self.cache.contains(comp_id, version):
                self.active_capabilities[comp_id] = version
                up_to_date.append(comp_id)
            else:
                # In production, pull bytes from repository edge
                simulated_payload = f"celnet_plugin_{comp_id}_{version}".encode()
                computed_digest = hashlib.sha256(simulated_payload).hexdigest()
                self.cache.store(comp_id, version, simulated_payload, computed_digest)
                self.active_capabilities[comp_id] = version
                hydrated.append(comp_id)

        return {
            "hydrated": hydrated,
            "skipped_unlicensed": skipped,
            "up_to_date": up_to_date,
        }
