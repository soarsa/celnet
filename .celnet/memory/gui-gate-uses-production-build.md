---
name: gui-gate-uses-production-build
description: "The GUI's real gate is `npm run build` (tsc -b), not `npm run typecheck` (tsc --noEmit) — the production build is stricter"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 3759f335-a305-4b8e-b3aa-727dea5a173c
---

For the celnet `gui/` crate, the authoritative typecheck gate is `npm run build`
(`tsc -b && vite build`), NOT `npm run typecheck` (`tsc --noEmit`).

**Why:** `tsc -b` uses the composite/project-references tsconfig with stricter
flags (notably `noUncheckedIndexedAccess`) that the flat `--noEmit` config does
not apply. On 2026-06-27 a UAT `release` deploy (deploy/celnet-deploy.sh option 2)
failed at the on-box `npm run build` step with two TS errors (`mockSource.ts`
index-possibly-undefined; `RatesRiskWorkspace.tsx` readonly-array→mutable setState)
that BOTH local `npm run typecheck` and every subagent gate had passed clean.

**How to apply:** Before committing/deploying GUI work, run `npm run build` (with
`set -o pipefail`), not just `npm run typecheck`. Tell subagents building GUI code
to gate on `npm run build`. The deploy's GUI step IS `npm run build`, so that is
the only gate that matches production.
