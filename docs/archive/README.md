# Retired documents

Documents that are fully superseded and referenced by nothing. They are kept — not deleted —
so the decision trail survives, but **nothing should link to them and nothing should plan
against them**.

Retired 2026-08-17 (estate cleanup):

| Document | Why retired |
|---|---|
| `DOCS-EVOLUTION-PLAN.md` | A 2026-06-08 meta-plan for how the docs corpus should evolve. Superseded by [`../README.md`](../README.md), which does the job the plan described. |
| `POST-W2-EXECUTION-CHECKLIST.md` | Execution checklist for the W2 wave. The wave completed; `celnet-linear` and the metals breadth are on `main`. |
| `RESUME-ANCHOR.md` | A standalone session resume anchor. Superseded by the single-line anchor kept in `CLAUDE.md`, which is the one the session actually reads. |

## Criterion

A document is retired when **both** hold:

1. it has no inbound reference from any `.md`, source file, `justfile`, or tooling script, and
2. it describes a completed wave or a process that has been replaced.

Staleness alone is not sufficient — much of `docs/` is stable reference material that is
correct precisely because it has not needed to change.

## Not archived here

**Client-confidential material was removed from the repository entirely**, not archived. Two
customer capability responses naming a real institution were relocated outside the repo on
2026-08-17; customer material does not belong in a product repository at any path.
