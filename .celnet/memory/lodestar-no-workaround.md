---
name: lodestar-no-workaround
description: "Never replicate or work around lodestar's capabilities; if lodestar needs enhancing, raise a GitHub issue on soarsa/lodestar so it's fixed upstream."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b757c952-cc98-456a-9272-5cb8b360235d
---

Operator directive (2026-06-22): lodestar should protect us automatically. **Do NOT build parallel
machinery that replicates or works around a lodestar capability.** If lodestar can't do something we
need, that is a lodestar gap → **raise a ticket on `github.com/soarsa/lodestar`** (issues enabled) and
let it be resolved upstream; in the meantime accept lodestar's honest DEFER (the present-guard never
false-passes) rather than papering over it.

**Why:** lodestar is the deterministic substrate; bolting on our own checkers fragments the source of
truth, drifts from lodestar's model, and hides the real gap. Upstream fixes benefit every project +
keep one mechanism.

**Corollary — DOCS too (2026-06-22):** do NOT write a local repo doc that re-describes lodestar's
generic capabilities (e.g. a "capability-coverage" doc). That fragments knowledge, is invisible to
every other session/project (they stay blind), and goes stale. If we were blind to a capability, that
is a lodestar **discoverability** gap → fix it **upstream** (a `soarsa/lodestar` docs issue/PR so the
capability surface is discoverable for ALL sessions). Keep only CelNet-SPECIFIC outputs local — and
prefer expressing them IN lodestar (claims, `spec:satisfies` acceptance targets, tasks/ledger) over
prose, since those are themselves graph-discoverable.

**How to apply:** when a gate/claim/derive-rule doesn't fire, first diagnose via the graph (is the
extraction present? `get_graph_schema`, `query_graph` for the needed nodes/edges). If the capability
exists but our setup is wrong → fix our usage the lodestar-native way. If lodestar genuinely doesn't
extract/support it → GitHub issue with evidence (the graph queries), don't work around.

**First case (2026-06-22):** `design:token` DEFERs for all CSS-module-styled GUI components because
lodestar emits 0 `component→DesignToken` `CONSUMES_TOKEN` edges — it doesn't trace the
`className → .module.css → var(--token) → DesignToken` chain (only intra-DTCG aliases). Filed
**lodestar#7** (https://github.com/soarsa/lodestar/issues/7). No workaround built; design:token DEFERs
honestly until #7 lands. See [[lodestar-first]], [[lodestar-migration]].
