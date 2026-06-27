# Requirements bridge (R5): tracker import / verified-status push-back

The **optional, default-off, host-side** PM bridge between lodestar's verified
requirements layer (v0.5.0 `spec:satisfies` / `Deliverable` roll-up) and the
team's tracker — **JIRA, Linear, Plane, GitHub Issues** — via a vendor-neutral
adapter contract. It is one of the host-side drivers under
[`tools/`](../README.md), alongside [`planner/`](../planner/README.md) (which
authors the draft `spec:satisfies` stubs this bridge imports against) and the
[`judge/`](../README.md) review drivers it routes claims through. It augments the tracker; it never replaces it: lodestar is
the source of *verified truth*, the tracker stays the *human workflow surface*.

The engine is untouched: pure-C, zero-dep, air-gapped, **no tracker code, no
network call** (machine-checked by the selftest's nm/strings gate). The bridge
talks to the engine exclusively through the public `lodestar cli` MCP seam
(`knowledge_put` / `knowledge_review` / `knowledge_get` / `knowledge_export`)
and to the tracker exclusively through an adapter. With no config the tracker
is `off`, both drivers print `absent`, and the default build is byte-identical.

## Layout

```
tools/requirements/
  README.md                          this file
  lib.sh                             shared helpers + the ADAPTER CONTRACT
  requirements-import.sh             tracker epic -> draft spec:satisfies claims (testimony)
  requirements-push.sh               verified roll-up -> tracker status, receipt-first
  requirements-selftest.sh           the R5 gate harness (sandbox round-trip)
  lodestar.requirements.toml.example config template (every key has an env twin)
  adapters/
    stub.sh                          LOCAL file-based sandbox tracker (no network; the tested reference)
    jira.sh  linear.sh  plane.sh  github.sh   vendor reference transports (key-gated)
```

## The two directions

### Import (`requirements-import.sh import <epic_id>`)

Reads an epic's child requirements from the tracker and authors each as a
**draft `spec:satisfies` claim** via `knowledge_put`, anchored to the graph
symbols the ticket names (and, when the ticket names a committed acceptance
target, to the `design-target:<ref>` sentinel — the R1 content-addressed
carrier, swept by the existing staleness resolver).

- **Epistemic source = testimony (R5-a).** Imported text is the tracker's
  word, not knowledge. Every claim carries a machine-readable
  `[epistemic_source=testimony tracker=... epic=... ticket=...]` tag, is
  authored as `requirements-bridge`, and the engine's own **ASSERT event**
  (append-only event log, `.lodestar/knowledge/events/`) is the import
  receipt — the event is already emitted; the bridge tags its source.
- **Cross-family review before trust (R5-b).** Every authored claim is routed
  through the shipped `knowledge_review` seam. With `review_cmd` configured
  (e.g. `sh tools/judge/judge-ollama.sh judge`) the review is driven
  immediately; without it the claim stays draft testimony and the exact review
  command is printed. The engine enforces never-self either way.
- **VeriTrans authoring-confidence hint — DRAFT-ONLY (the §6.4 FLAG).**
  `confidence_cmd` may annotate each draft claim with a round-trip NL↔PL
  confidence (`authoring-hint:roundtrip=<v>` in the claim's confidence field)
  to help a human prioritize review. **It never gates**: there is deliberately
  no threshold branch in the bridge — a 0.01-confidence requirement is
  authored exactly like a 0.99 one (the selftest asserts this). The
  coverage-threshold acceptance use is permanently rejected (plan §5).
- A requirement with no anchors is reported **SKIP** — a claim must anchor to
  live graph symbols, never guessed.

### Push-back (`requirements-push.sh push <ticket> --anchor "<qn>..." [--deliverable <slug>]`)

Projects lodestar's **verified roll-up** onto the ticket. "Done" is a
projection of verification, never a human button:

| verified roll-up | ticket status |
|---|---|
| `active` (all children verified) | **done** |
| `contradicted` | blocked |
| `stale` / `draft` | in-progress |
| unknown (no claims found) | **nothing pushed** (exit 3, never guessed) |

- **Roll-up source.** Preferred: the R2 `deliverable=` filter on
  `knowledge_get` (the engine's own weakest-child-state projection). Fallback
  on engines predating R2: the weakest state across the anchors'
  `spec:satisfies` claims, computed only from engine-reported states.
- **Receipt-first (R5-a).** Before the tracker is touched, the round-trip
  receipt is recorded in the engine's append-only event log: a
  `requirements:receipt` claim through `knowledge_put` whose text is the
  canonical sorted-key receipt JSON —

  ```json
  {"deliverable":"fx-options","direction":"status-push",
   "epistemic_source":"tool-output","receipt":"tracker-roundtrip",
   "rollup_state":"active","target_hash":"<16-hex>","ticket":"FXO-1",
   "tool_version":"requirements-bridge@0.5.0","tracker":"jira"}
  ```

  The engine emits the ASSERT event (`lode_kn_event_append`) carrying that
  payload with author `requirements-bridge`. The receipt claim is an
  unrecognized kind at the deterministic gate, so it **defers and stays draft
  forever** — a ledger entry, never trusted knowledge, structurally incapable
  of minting a PASS. `lodestar-syncd` gossips it like any event.
- **"Done requires a roll-up receipt"** is enforced twice: the bridge records
  the receipt before any push, and **every adapter refuses
  `set-status done` with an empty receipt id**. "Claimed verified but no
  receipt" is a detectable absence. The failure direction is safe by
  construction: a receipt without Done can happen (tracker down — honest); a
  Done without receipt cannot.
- **Machine-readable feed.** Every push exports the `knowledge_export` SARIF
  (active→pass, never a non-active pass) + conformance projections into
  `out_dir` and hands the SARIF path to the adapter as an attachment.
- `target_hash` is read from the claim's stored design-target anchor hash —
  the engine's own FNV-1a content id. The bridge never re-derives a hash
  (one hasher, the engine's).

## Adapter contract (vendor-neutral)

Documented normatively in `lib.sh`. Summary:

```
adapter capabilities                              -> {"tracker":...,"ready":bool,...}
adapter fetch-epic <epic_id>                      -> canonical requirement stream:
    line 1 : {"epic":"...","deliverable":"<slug>","title":"..."}
    line 2+: {"ticket":"...","title":"...","text":"...",
              "anchors":"<qn> <qn> ...","target":"<repo-rel path or empty>"}
adapter get-status <ticket>                       -> {"ticket":...,"status":...}
adapter set-status <ticket> <status> <receipt_id> [feed]
    -> ack JSON; MUST refuse status=done with an empty receipt_id (exit 1)
```

The stream is line-oriented flat JSON so POSIX `sh` parses it without `jq`.
Anchors/targets come from a fenced ` ```lodestar ` block in the ticket
description (`anchors:`, `target:`, `deliverable:` lines) — vendor adapters
extract it; the stub stores streams verbatim. Degrade legend (the judge-api
convention): exit 4 no API key · 5 no curl/gh · 3 missing host tool (`jq`) —
always honestly, never a fabricated result. A custom/in-house tracker plugs in
via `adapter_cmd` (any command speaking this contract).

`adapters/stub.sh` is a *real* adapter over local files (no network) — the
selftest's sandbox tracker and a working surface for air-gapped demos. The
vendor adapters are reference transports validated against the contract;
their `capabilities` probes run offline.

## Configuration

Copy `lodestar.requirements.toml.example` into `tools/lodestar.judge.toml`
(the single host-driver config surface) or set the env twins. Key knobs:

| Setting | env | default |
|---|---|---|
| tracker | `LODESTAR_REQ_TRACKER` | `off` |
| adapter override | `LODESTAR_REQ_ADAPTER_CMD` | `""` (adapters/<tracker>.sh) |
| review routing | `LODESTAR_REQ_REVIEW_CMD` | `""` (print command, stay draft) |
| confidence hint | `LODESTAR_REQ_CONFIDENCE_CMD` | `""` (no hint; never gates) |
| author identity | `LODESTAR_REQ_AUTHOR` | `requirements-bridge` |
| feed out dir | `LODESTAR_REQ_OUT_DIR` | `.lodestar/requirements` |
| engine binary / project | `LODESTAR_BIN` / `LODESTAR_PROJECT` | `lodestar` / `lodestar` (your key from `list_projects`) |

## Quick start

```sh
# 0. Nothing configured? Everything is off and absent:
sh tools/requirements/requirements-import.sh --self-test     # exit 0, 'absent'

# 1. Prove the whole round-trip on this machine (sandbox tracker + REAL engine):
sh tools/requirements/requirements-selftest.sh

# 2. Wire a real tracker, then import an epic as draft testimony:
LODESTAR_REQ_TRACKER=jira sh tools/requirements/requirements-import.sh import FXO-1

# 3. After verification, project the verified status back (receipt-first):
LODESTAR_REQ_TRACKER=jira sh tools/requirements/requirements-push.sh \
    push FXO-1 --anchor "pricing.core.price_fx_option" --deliverable fx-options
```

## The R5 gate this satisfies (plan §3 R5)

`requirements-selftest.sh` is the machine check: a sandbox round-trip — epic
imported → draft claims authored (tagged testimony, routed through the
cross-family review) → roll-up verified `active` → ticket flipped via the
adapter **with** a roll-up receipt in the event log; a negative case proving
an unreviewed deliverable never flips Done; the nm/strings purity gate on the
built binary; and the structural byte-identity precondition (zero build-input
references to this directory). Exit 0 all green · 1 failure · 2 engine binary
absent (pure checks green; re-run post-build).
