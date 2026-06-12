# Standing workflow-agent preamble (source of truth — paste-by-reference into briefs)

Every Celnet workflow brief includes these three blocks. They encode the operator
directives: graph-first token economy, right-sized contexts, optimal model per task.

## 1. Graph-first discovery (MCP via CLI — connection-independent)

Discovery goes through the codebase-memory graph BEFORE any file reading, via Bash
(works whether or not the MCP server is attached to the session):

```bash
BIN="$HOME/.local/bin/codebase-memory-mcp"
"$BIN" cli search_graph '{"project":"Users-adrian-code-celeroption","name_pattern":"^price_instrument$","limit":5}'
"$BIN" cli search_graph '{"project":"Users-adrian-code-celeroption","query":"one touch first passage","limit":8}'   # BM25
"$BIN" cli get_code_snippet '{"project":"Users-adrian-code-celeroption","qualified_name":"<qn from search>"}'
"$BIN" cli trace_path '{"project":"Users-adrian-code-celeroption","function_name":"price_instrument","mode":"calls"}'
```

Health probe before trusting it (the corruption lesson): a hub symbol returning 0 or a
node count ≪17k ⇒ report it; the coordinator rebuilds (3s). Grep/Read are the fallback
for non-code text only.

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
