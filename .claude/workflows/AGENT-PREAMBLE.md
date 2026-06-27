# Standing workflow-agent preamble (source of truth — paste-by-reference into briefs)

Every Celnet workflow brief includes these three blocks. They encode the operator
directives: graph-first token economy, right-sized contexts, optimal model per task.

## 1. Graph-first discovery (lodestar MCP via CLI — connection-independent)

Discovery goes through the **lodestar** code-graph BEFORE any file reading, via Bash
(works whether or not the MCP server is attached to the session). The stable project key
is the git-remote key `github.com-soarsa-celnet` (pinned in `.lodestar/project-id`,
identical on every clone):

```bash
L="$HOME/.local/bin/lodestar"   # on PATH after `lodestar install`
P=github.com-soarsa-celnet
"$L" cli search_graph     '{"project":"'$P'","name_pattern":"price_instrument","limit":5}'
"$L" cli search_graph     '{"project":"'$P'","query":"one touch first passage","limit":8}'   # BM25
"$L" cli get_code_snippet '{"project":"'$P'","qualified_name":"<qn from search>"}'
"$L" cli trace_path       '{"project":"'$P'","function_name":"price_instrument","mode":"calls"}'
"$L" cli knowledge_get    '{"project":"'$P'","qualified_name":"<qn>"}'   # the verified "why"
```

Health probe before trusting it: `lodestar doctor --json` must report `ok:true` /
`problems:0`, and `price_instrument` must resolve (graph ≈18k nodes). A sharp drop or a
missing hot symbol ⇒ `lodestar index --full .` (~2s, deterministic, byte-identical).
Grep/Read are the fallback for non-code text only. lodestar self-invalidates — claims go
stale when code changes, so re-query rather than trusting cached understanding.

## 2. Token discipline ("clear tokens")

- ONE purpose per agent; finish and return — never accumulate a second task in the
  same context. Long work = more small agents, not bigger ones.
- Pass seams by FILE REFERENCE (path + the graph qn), never by pasting file bodies
  into briefs or reports. Reports carry decisions + literal gate lines, not transcripts.
- Read narrowly: `get_code_snippet` over whole-file Reads; `offset/limit` when a file
  read is unavoidable; never dump a file to "get oriented" — that is what the graph is for.
- Commit/bank before summarizing (a wall kills ≤1 unit's tail).

## 3. Model per task (measured policy, shared memory `model-selection-policy`)

| Task class | model option | Why |
|---|---|---|
| Math/judgment-dense build (pricing, proto design, security boundaries) | inherit (opus) | error cost ≫ token cost |
| Mechanical/pattern (codec mirrors, manifest sweeps, doc sync, spec migration, test scaffolds from a given template) | `model: 'sonnet'` | ~5× cheaper, measured-equivalent |
| Bulk read-only critique lenses (evidence sweeps) | `model: 'sonnet'` | volume work |
| Numerical-correctness lens + synthesis judges + adversarial verifiers | inherit (opus) | the depth is the product; cross-model diversity vs session-A (fable) |
| Tiny mechanical probes (single-file checks) | `model: 'haiku'` | trivial units |
