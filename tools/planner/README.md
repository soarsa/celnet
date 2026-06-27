# Planner: graph-derived decomposition + ranked blast radius (R3)

The **host-side** planning helper of the v0.5.0 Verified Requirements layer
(`docs/design/v0.5.0-requirements-plan.md` §3 R3). "Define FX Options" →
lodestar proposes the per-service requirement skeleton from the **real
dependency graph**, not a human guess. Like its sibling host-side drivers under
[`tools/`](../README.md) — [`requirements/`](../requirements/README.md),
[`visual/`](../visual/README.md), and `judge/` — it lives entirely outside the
engine:
**zero engine edits, zero new engine surface, no build target** — pure
orchestration over the shipped `lodestar cli <tool> <json>` seam.

## What it does

```
planner.sh plan <seed_qualified_name> [--deliverable <slug>] [--emit-stubs]
```

1. **Enumerate reachability** from the seed: one `trace_path`
   (`mode=cross_service`) with **explicit** `edge_types` — the CALLS family
   plus the `pass_cross_repo` contract edges (`CROSS_HTTP_CALLS`,
   `CROSS_ASYNC_CALLS`, `CROSS_CHANNEL`, `CROSS_CONSUMES_TOKEN`) — giving the
   visited symbol set with hop counts.
2. **Type the spine**: `query_graph` lists each `CROSS_*` edge family with
   exact `qualified_name`s (also the safety net: a cross row whose near side
   is reachable pulls its far side in at hop+1, horizon-bounded), plus the
   `TESTS` and `IMPORTS` overlays.
3. **Rank** (`tdad-rank.sh`): the TDAD closed form (arXiv:2603.17973) over the
   deterministic edge set —

   | strategy   | condition                                | score |
   |------------|------------------------------------------|-------|
   | Direct     | hop 1, CALLS family or `CROSS_*`         | 0.95  |
   | Coverage   | `TESTS` edge into the impacted set       | 0.80  |
   | Transitive | hop 2..horizon (default 3), same families| 0.70  |
   | Imports    | `IMPORTS` edge into the impacted set     | 0.50  |

   Tiers: `high >= 0.80`, `medium >= 0.60`, `low > 0`. Max strategy score
   wins per symbol; ties break by edge class then hop — byte-deterministic.
4. **Propose**: one requirement proposal per **reachable** service (service =
   the leading `qualified_name` segment(s), `service_segs`), anchored to that
   service's ranked symbols (seed first for its own service, capped at
   `max_anchors`).
5. **Author draft stubs** (`--emit-stubs`): one `spec:satisfies` claim per
   proposal via `knowledge_put`, anchored to the impacted `qualified_name`s.
   A stub carries **no acceptance target**, so the R1 Stage-1 gate defers
   (`LODE_VERIFY_NOT_APPLICABLE`) and the claim **stays `draft`** — by
   construction, not by convention. The human/agent then commits the
   `*.acceptance.json` target, anchors it via the `design-target:<ref>`
   sentinel, and re-verifies (R1); the R2 deliverable roll-up takes it from
   there.

## The honesty contract (what the selftest proves)

- **The graph decides WHAT; any judgement is host-side IF (R3-b).** The
  candidate set is graph reachability only. `--services` can only **narrow**
  the graph-fixed set; asking for an unreachable service is refused (exit 4).
- **The score is a ranking signal, never a gate (R3-a).** It orders draft
  stubs for curation. It cannot flip a claim, mint a verdict, or block
  anything.
- **Draft-only, abort on anything else.** If a stub ever comes back
  non-`draft` from `knowledge_put`, the planner aborts (exit 4) rather than
  proceed past an auto-activation.
- **Nothing outside the closed form is guessed.** Unknown edge classes and
  beyond-horizon hops are emitted under `"unscored"` with a reason — reported,
  never tiered.
- **The `detect_changes` overlay is display-only.** Its symbols carry no
  `qualified_name`, so folding them into the ranking would be a guess; the
  ranked set is byte-identical with and without `--changes`.
- **Deterministic.** No timestamps, fully sorted output: identical graph
  state ⇒ byte-identical plan.
- **Honest failure.** An unresolvable seed or failed engine call exits
  non-zero with no plan fabricated.

## Usage

```sh
# what would FX Options touch, ranked?
sh tools/planner/planner.sh plan capture.client.submit_trade --deliverable fx-options

# narrow to two services and author the draft stubs
sh tools/planner/planner.sh plan capture.client.submit_trade \
    --deliverable fx-options --services capture,pricing --emit-stubs

# fold in the working-tree change overlay (display only)
sh tools/planner/planner.sh plan capture.client.submit_trade --changes main

# the ranking core, standalone (stdin: qn<TAB>via<TAB>hop)
printf 'svc.mod.fn\tCALLS\t1\n' | sh tools/planner/tdad-rank.sh

# wiring check (no engine call): 0 ready, 2 engine missing
sh tools/planner/planner.sh --self-test

# the recorded-replay gate (no engine, no graph, no network)
sh tools/planner/planner-selftest.sh
```

## Configuration (env > `lodestar.planner.toml` > default)

Copy `lodestar.planner.toml.example` next to where you run, or point
`LODESTAR_PLANNER_CONFIG` at it. Every key has an env twin:

| Setting | env | default |
|---|---|---|
| engine binary | `LODESTAR_BIN` | `lodestar` on PATH |
| project key | `LODESTAR_PROJECT` | first segment of the seed |
| trace depth | `LODESTAR_PLANNER_DEPTH` | `3` |
| transitive horizon | `LODESTAR_PLANNER_HORIZON` | `3` |
| service = leading QN segments | `LODESTAR_PLANNER_SERVICE_SEGS` | `1` |
| anchors per stub | `LODESTAR_PLANNER_MAX_ANCHORS` | `5` |
| stub author tag | `LODESTAR_PLANNER_AUTHOR` | `lodestar-planner@0.5.0` |
| put project (`service` or fixed key) | `LODESTAR_PLANNER_PUT_PROJECT` | `service` |
| stub confidence | `LODESTAR_PLANNER_CONFIDENCE` | `low` |

`put_project=service` suits cross-repo estates where each service is its own
indexed project (the leading `qualified_name` segment); pin a fixed key for a
monorepo indexed as one project (and raise `service_segs` so services map to
top-level folders).

## Exit codes

`0` ok · `1` usage · `2` engine binary missing · `3` engine/tool failure
(incl. seed not found) · `4` refused (honesty violation / non-narrowing
`--services`).

## Testing

`planner-selftest.sh` is the §3 R3 gate as a runnable harness, in the
`tools/visual/fixtures/recorded` discipline: real `lodestar cli` output
shapes recorded once under `fixtures/recorded/` and replayed byte-identically
by `lodestar-replay.sh` — no engine, no model, no network, no store write.
The recorded estate is the fx-options vocabulary (capture → pricing → risk
reachable; settlement/frontend present in the whole-graph recordings but
unreachable). It proves: reachable-only proposal, the exact tier ordering,
×2 byte-identical determinism, draft-only stub authoring (and the abort on a
non-draft response), narrow-only `--services`, honest failure on a missing
seed, and the display-only `detect_changes` overlay.
