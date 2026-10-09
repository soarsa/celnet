# Memory Bridge: Native Memory ↔ Lodestar Claims

The document establishes a single-source-of-truth architecture where durable codebase knowledge lives in lodestar (a graph-anchored knowledge system) as verified claims, while native agent memory maintains only *pointers* to these claims—never duplicate copies.

## Core Principle

"When a fact about the codebase is durable and anchored to a symbol, it belongs in lodestar" with native memory holding lightweight references via claim IDs rather than restating content.

## Key Benefits of Pointers Over Copies

The pointer approach prevents three failure modes:

1. **Drift prevention**: Lodestar automatically flags claims as stale when anchored symbols change; copied facts in memory deteriorate invisibly without such signals.

2. **No parallel wikis**: The system explicitly rejects maintaining duplicate stores of the same information—the defining weakness it aims to solve in LLM-generated code documentation.

3. **Auditability**: Claim IDs provide provenance threading (anchors, evidence, verdicts, events) that prose memory lines cannot offer.

## What Memory Retains

Native memory preserves non-code facts: user preferences, workflow feedback, project goals, and the citation references themselves. The boundary is clean: if something describes a code symbol, it belongs anchored in lodestar.

## Maintenance Stability

Claim IDs remain stable across re-authoring within maintenance loops, so memory references require updates only when claims are retired—making the system largely self-maintaining.
