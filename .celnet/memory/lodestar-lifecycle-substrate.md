---
name: lodestar-lifecycle-substrate
description: "Use lodestar across the WHOLE dev lifecycle — plan, refactor, optimise, deterministically deliver, test, accelerate — not just code discovery."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b757c952-cc98-456a-9272-5cb8b360235d
---

Operator directive (2026-06-22): **lodestar is the substrate for the entire development lifecycle**,
not just discovery. Drive every phase through it the native way (raise a [[lodestar-no-workaround]]
ticket for genuine gaps; never replicate it):

- **Plan** — `spec:satisfies` + committed `*.acceptance.json` targets, `tools/planner`, `get_architecture`
  (Leiden seams), `detect_changes` (scope before building), `knowledge_get deliverable=<slug>` (done-ness), `manage_adr`.
- **Refactor** — `trace_path` (impact), `detect_changes` (blast radius), `search_graph` (dead code
  `max_degree=0`, high fan-in/out candidates), `SIMILAR_TO`/`MotifCandidate` (duplication→DRY); the
  staleness loop flips affected invariants `stale` so re-verify they held.
- **Optimise** — the extracted hot-path metrics (`alloc_in_loop`, `linear_scan_in_loop`,
  `transitive_loop_depth`) + derive-rules that flag violations every index; `query_graph` for hidden
  O(n²); `RUNTIME_CALLS` (after `ingest_traces`).
- **Deterministically deliver** — byte-identical index, Stage-1 constraint gate (precision 1.0),
  `spec:satisfies` decidable gating, `knowledge_export` (SARIF/conformance) for CI, set-union shared knowledge.
- **Test** — `knowledge_coverage` / Trust Map (untested-high-impact ranked), `TESTS`/`TESTS_FILE` edges,
  the `verification-loop` skill, execute-verify.
- **Accelerate** — `evidence_pack` (~21× fewer tokens), `get_code_snippet`/`search_graph` over file
  reads, `knowledge_get` to cite the "why", the agent fleet + workflows wired on all of it.

**State (2026-06-22):** structurally fully indexed (20,931 nodes) but THINLY leveraged — ~0.2% anchored,
no spec:satisfies/acceptance targets, ~0 TESTS edges, no runtime traces. The capability-coverage research
(`docs/plan/LODESTAR-CAPABILITY-COVERAGE.md`, in progress) quantifies the gap per phase + the wire-native
vs lodestar-ticket remediation. See [[lodestar-first]], [[lodestar-no-workaround]], [[lodestar-migration]].
