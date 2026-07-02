# Decision — license gating, scope bar, rail badges (GUI redesign)

Status: Accepted (2026-07-01). To promote to `docs/adr/` at landing.

## Context
The mockups show, on the capability rail + top bar: unlicensed asset classes **greyed with a lock +
upsell**, a compact **Desk ▸ Book scope pill**, and no "NEW" badges. The shipped app instead **HIDES**
gated workspaces (`Shell.tsx`: "hide, never grey" — documented), and `ScopeControl.tsx` is a
breadcrumb-drill with group-by. This is a genuine UX-policy conflict, not a drop-in.

## Decision
1. **Distinguish "not LICENSED" from "not ENTITLED".**
   - **Not licensed (commercial, per asset class):** **PRESENT + GATED + UPSELL** — show the class/
     surface greyed with a lock and a "request access / license" affordance. Rationale: per-class
     licensing is a first-class product primitive; showing it drives discoverability + upsell, and a
     firm that *could* license Crypto should see it exists. This **supersedes "hide"** for commercial
     license gating.
   - **Not entitled (deny-wins capability / information barrier):** **KEEP HIDE.** Rationale:
     security/info-barrier — a user denied a capability (e.g. another desk's book) must not even see
     it. Deny-wins hiding is unchanged.
   - So the rail computes a **three-state**: `present` · `gated-upsell` (license) · `hidden`
     (entitlement-deny), driven by `celnet-entitlements` (entitlement→hide) + a new per-class
     **license** flag (license→gated-upsell).
2. **Scope bar:** evolve `ScopeControl` into the redesign's compact **Desk ▸ Book selector** — keep
   the drill + group-by capability, restyle to the pill/selector form; it remains the one scope
   primitive that governs every surface.
3. **Drop the "NEW" badges** from the rail (pro-tool noise; per the critique).

## Consequences
- `Shell.tsx` rail gains the license-vs-entitlement three-state; needs a `licensed(assetClass)` input
  (design primitive) distinct from the entitlement check.
- `ScopeControl` restyled; existing tests updated.
- Never fabricate license/entitlement flags — read the real `celnet-entitlements` kernel
  (`lib/capabilityMatrix.ts`); the license flag is a new commercial-tier input, initially config-driven.
