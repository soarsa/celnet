# Knowledge tool contracts

The exact request/response JSON the `knowledge-maintenance` skill calls. These are the
shapes for `knowledge_todo`, `knowledge_get`, `knowledge_put` — scoped to the
`invariant:pure` claim kind. They are additive to the inherited 14 structural/navigation
MCP tools, bringing the total to 22 (14 structural + 8 knowledge); these three are part of the
8-tool knowledge surface. They use the same JSON-RPC-over-stdio transport; the CLI exposes the
identical payloads via `lodestar(.exe) cli <tool> '<json>'`.

> **Scope note.** This file documents these three knowledge tools in full request/response
> detail. The five further knowledge tools — `knowledge_check`, `knowledge_review`,
> `knowledge_config`, `evidence_pack`, `knowledge_export` — complete the 8-tool knowledge
> surface; their contracts live in [`../SKILL.md`](../SKILL.md) and the knowledge-layer table
> in the README. `knowledge_review` is covered in SKILL.md.

> This file is also the **integration contract** for the server code in `src/mcp/mcp.c`. The
> skill is written against exactly these field names and states; if the server diverges, update
> this file and the skill together. Project key in every request below is the path-independent
> key `"project": "lodestar"` (use your own key from `list_projects` for another repo).

Lifecycle states referenced below: `draft | active | stale | contradicted | retired`. The only
transition the skill drives is `draft → active` (gate pass); it reads `stale`/`contradicted`
items produced by the engine, and `retired` is set on retire/anchor deletion.

---

## knowledge_todo — list change-scoped work + graph facts

**Request**

```json
{ "project": "lodestar" }
```

Optional filters (all default to "everything in scope"):

```json
{ "project": "lodestar",
  "states": ["draft", "stale", "contradicted"],
  "kind": "invariant",
  "limit": 50 }
```

**Response** — claims needing work, **each with the graph facts for its anchor already
attached** so the skill needs zero extra calls to verify purity:

```json
{
  "todo": [
    {
      "id": "clm_0001",
      "kind": "invariant",
      "constraint": "pure",
      "state": "stale",
      "text": "lode_is_keyword has no side effects; result depends only on its arguments.",
      "anchors": [
        {
          "qualified_name": "lodestar.internal.lode.helpers.lode_is_keyword",
          "node_content_hash": "0032de7d007614db...",   // current hash from the graph
          "resolved": true,                              // anchor maps to a live node
          "writes": []                                   // the WRITES set, graph-derived
        }
      ],
      "reason": "anchor content hash changed (detect_changes)"
    }
  ],
  "count": 1
}
```

- `anchors[].writes` is the deterministic `WRITES(node)` set the skill reasons over — empty
  array means the gate will pass an `invariant:pure` claim.
- `reason` explains why the item is in the list (`authored` for `draft`, the staleness trigger
  for `stale`, the refuting claim/evidence for `contradicted`).
- Empty `todo` ⇒ nothing to maintain.

---

## knowledge_get — fetch claims for a symbol/subsystem

**Request** (one of `qualified_name` or `subsystem`):

```json
{ "project": "lodestar",
  "qualified_name": "lodestar.internal.lode.helpers.lode_is_keyword",
  "states": ["active"] }
```

**Response**

```json
{
  "claims": [
    {
      "id": "clm_0001",
      "kind": "invariant",
      "constraint": "pure",
      "state": "active",
      "text": "lode_is_keyword has no side effects; result depends only on its arguments.",
      "confidence": "high",
      "anchors": [
        { "qualified_name": "lodestar.internal.lode.helpers.lode_is_keyword",
          "node_content_hash": "0032de7d007614db..." }
      ],
      "author": "agent-code",
      "created_at": "2026-06-06T00:00:00Z",
      "updated_at": "2026-06-06T00:00:00Z"
    }
  ],
  "count": 1
}
```

`states` defaults to `["active"]` so a caller assembling an answer never sees a non-active
claim unless it asks. The skill uses `states:["active"]` when citing to the user.

---

## knowledge_put — create / revise a claim (validates anchors + runs the gate)

**Request**

```json
{
  "project": "lodestar",
  "kind": "invariant",
  "constraint": "pure",
  "text": "lode_is_keyword has no side effects; result depends only on its arguments.",
  "confidence": "high",
  "anchors": [
    { "qualified_name": "lodestar.internal.lode.helpers.lode_is_keyword" }
  ],
  "supersedes": null
}
```

- Omit `id` to create; pass an existing `id` to revise (revision preserves history via
  `supersedes`, never overwrites — design §4).
- The skill does **not** send `node_content_hash`; lodestar reads the current hash from the live
  node at write time (P2). It must not be agent-supplied.

**Response — accepted, gate passed (`WRITES = ∅`):**

```json
{
  "id": "clm_0001",
  "state": "active",
  "gate": { "constraint": "pure", "passed": true, "writes": [] },
  "anchors": [
    { "qualified_name": "lodestar.internal.lode.helpers.lode_is_keyword",
      "node_content_hash": "0032de7d007614db...", "resolved": true }
  ],
  "event_id": 1
}
```

**Response — accepted as draft, gate hard-rejected (`WRITES ≠ ∅`):**

```json
{
  "id": "clm_0002",
  "state": "draft",
  "gate": {
    "constraint": "pure",
    "passed": false,
    "writes": ["lodestar.internal.lode.helpers.g_keyword_call_count"],
    "verdict": "claim asserts purity but node writes global state"
  }
}
```

A failed gate keeps the claim out of `active` and **names the write target** so the skill can
regenerate/replan. The skill must never present such a claim as fact.

**Response — rejected, unresolved anchor (error, not stored):**

```json
{
  "error": "unresolved_anchor",
  "anchor": "lodestar.internal.lode.helpers.lode_is_keywrd",
  "message": "qualified_name does not resolve to a live graph node"
}
```

`put` **rejects** (does not store) a claim whose anchor does not resolve. The skill treats it
as "you named the wrong symbol" and corrects the `qualified_name`.

**Durability.** A successful `put` (and every state transition / review verdict) does more than
write a `knowledge_*` row: it **appends an immutable, content-addressed event** to the durable log
at `.lodestar/knowledge/events/<id>.json`. The `event_id` in the response is the DB-row id of that
transition; the durable record is the on-disk event file, named by content hash. The SQLite tables
are a derived projection re-folded from the log on the next index (`lode_kn_reconcile` freshness
check), so claims are durable across cache wipes and **merge by set-union** across branches — a
teammate who pulls the committed `events/` sees the claim with no conflict. Commit
`.lodestar/knowledge/` as text; never gitignore it. (Earlier this contract noted only "does not
persist" on the *error* path — that is unchanged: a rejected anchor logs nothing.) See
[SKILL.md → Durability](../SKILL.md) and
[knowledge durability layout](../../../../docs/guides/deployment-and-capabilities.md#46-knowledge-durability--on-disk-layout-what-to-commit-freshness).

---

## The server behaviour this contract relies on (precise)

The skill depends on behaviour wired into the engine's **shared** files
(`src/mcp/mcp.c`, `src/store/store.c`, `src/knowledge/*`). Stated precisely so the skill and
the server stay in lockstep:

1. **Three MCP tools** `knowledge_todo`, `knowledge_get`, `knowledge_put` are registered in
   `src/mcp/mcp.c`, with the request/response JSON above, validated against a schema.
2. **`knowledge_put` runs the Stage-1 constraint gate** for `invariant:pure` before deciding
   state: it reads the anchor node's outgoing `WRITES` edges from the graph; `∅ ⇒ active`,
   non-empty ⇒ stays `draft` with `gate.passed=false` and the `writes` list. The gate logic
   lives in `src/knowledge/verify.c`; `mcp.c` only calls it.
3. **`knowledge_put` rejects unresolved anchors** with the `unresolved_anchor` error and does
   not persist.
4. **`knowledge_todo` attaches graph facts** (`node_content_hash`, `resolved`, `writes`) to
   each anchor so the skill needs no second round-trip to know the `WRITES` set.
5. **Staleness hook:** after each incremental re-index, for every node whose content
   hash changed, the engine flips its anchored `active` claims to `stale` and logs a
   `knowledge_event` (`triggered_by="detect_changes"`). This is what surfaces `stale` items
   into `knowledge_todo`. The hook lives in the pipeline/watcher path calling the knowledge
   store; the skill only observes the result.

If any field name here changes, change it **here and in `SKILL.md`
together** so the agent-as-LLM provider stays in lockstep with the real tools.
