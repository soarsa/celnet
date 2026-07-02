---
name: model-selection-policy
description: Operator directive (2026-06-11) — pick the optimal model per task weighing RESULTS vs TOKEN USAGE; measured outcomes from the W6/crypto-leaf fleet inform the tiers.
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 022c5024-0efd-40df-b066-eb5f16656b52
---

Operator: "leverage the optimal model for the tasks considering results and token usage."
Measured this program (Celnet mesh, 2026-06-10/11):

- **sonnet** delivered the entire fuzz lane flawlessly (5 targets + mirrors + a real Cargo.toml
  fix) at a fraction of the cost — *mechanical, pattern-following, house-style work does NOT
  need a frontier model.*
- **fable** delivered the hard numerics (zero-missed mutation gates incl. risk-cube 956; the
  crypto leaf; found the Ikeda-Kunitomo double-KO blowup + the charm/color sign bug) — worth
  the 2× price *only where math judgment is the task*. Caveat: long fable runs were also the
  ones dropping StructuredOutput and burning session caps — keep fable agents to SMALL units.
- **opus** for adversarial verification of fable-built work — cross-model diversity catches
  what self-review misses, at lower cost than fable.

**Policy (workflow `agent(..., {model})` selection):**
1. Math/numerics judgment (oracle derivation, equivalence adjudication, pricing engines) → `fable`, smallest bankable units.
2. Mechanical/pattern work (fuzz targets, config plumbing, call-site sweeps, doc sync, test
   scaffolds from a written spec) → `sonnet`.
3. Adversarial verifiers → `opus` (diversity from the fable builder); spot-check scope only
   (verification economy [[planned-builds-no-blocking]]).
4. Trivial single-file lookups/edits inside a workflow → `haiku` if isolated, else fold into
   the nearest agent.
5. Default-omit (inherit) ONLY when the task profile is genuinely mixed.

Related: [[planned-builds-no-blocking]], [[token-and-context-discipline]], [[cbm-mcp-first]].
