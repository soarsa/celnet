---
name: parallel-session-model
description: How multiple agent sessions develop Celnet concurrently without conflict.
metadata: 
  node_type: memory
  type: project
  originSessionId: b315eccc-f521-4987-b5b5-1a21d5710edb
---

Celnet is structured so multiple agent sessions work in parallel without disrupting each other or losing progress:

- **Crate-per-workstream**: independent sessions own disjoint crates → no merge conflicts. Shared interface crates (core types, traits, wire schemas) are stabilized FIRST, then frozen-ish; changes to them are coordinated.
- **Ledger in GUIDE.md**: the implementation ledger records what's done / in-progress / claimed, so a fresh session orients instantly. Update it before/after a unit of work.
- **Knowledge base in `docs/`** (architecture, analytics-spec, competitive, integration, roadmap) is the source of truth for design; the roadmap defines workstreams and their crate ownership + test gates.
- **codebase-memory-mcp** indexes the workspace for structural queries (re-index as code grows) — see [[cbm-mcp-first]].
- **Test gates**: a workstream is "done" only when its crate's tests + clippy + fmt pass. No mocks ([[no-mocks-policy]]).
- Git is **local-only** ([[git-local-only]]).
