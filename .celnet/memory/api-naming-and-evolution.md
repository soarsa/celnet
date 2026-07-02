---
name: api-naming-and-evolution
description: "Celnet naming is celnet-logical & vendor-neutral; no versioned APIs; always refactor, zero legacy."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b315eccc-f521-4987-b5b5-1a21d5710edb
---

Binding rules for Celnet's API surface and code evolution (user directive, 30 May 2026):

- **Naming is `celnet`-logical & vendor-neutral.** Crates/modules/types/traits/fns are named for their **purpose** under the `celnet-` namespace (crate prefix is `celnet-`, NOT `celer-`). No commercial-product / competitor / vendor names and no person/paper/framework names in API identifiers (e.g. `VanillaInputs`, not `GkInputs`). Method provenance only in doc comments. The parent firm **Celer** / `celertech` estate (our own systems) may be named in integration/docs context, never in core product identifiers.
- **No versioned APIs.** Exactly one clean current contract — no `schema_version`, no N/N-1 negotiation, no back-compat shims (no external users). Zero-downtime upgrades use **blue-green / full cutover**, not mixed-version windows.
- **Always refactor to cleanest; zero legacy.** Delete dead code, keep files in correct dirs, keep ALL docs/guides/references in sync, no stale/duplicate references. After structural changes, re-`index_repository` so the codebase-memory graph always covers the full scope.

**How to apply:** When touching any area, opportunistically clean it. Renames cascade through docs + memory + the graph in the same change. See [[no-commercial-products]], [[scale-and-performance]], [[celnet-mission]].
