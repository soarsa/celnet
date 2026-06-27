# Architectural review — fixed-income/rates consolidation (post-merge)

**Substance lives in lodestar, not here.** This doc is the thin process narrative; the
captured knowledge and the dedup verdicts are stored as graph-anchored lodestar claims
(durable in `.lodestar/knowledge/claims-mirror.json`, queryable via `knowledge_get` /
`DELIVERABLES.md`). Query the review:

```
knowledge_get <symbol>        # the verified "why" on the new celnet-rates surface
knowledge_get deliverable=…   # capability roll-ups incl. fixed income
```

## Context
`feature/fixedincome` merged a new `celnet-rates` crate (OIS/SOFR curves, PV01/DV01,
Brent solver, schedule/bootstrap) + GUI Excel workspace, alongside three rigor lanes
(w6-exotics, w6-fuzz, crypto-surface). It reuses `celnet-types` + `celnet-calendar` only —
not the carry seam (`celnet-core`/`Carry`), `celnet-conventions`, or any solver crate.

## Review dimensions (each → a stored lodestar verdict claim)
- **F1 discounting seam** — does `celnet-rates::Curve` duplicate / bypass `Carry`'s
  `forward()/discount()` producer (ADR-0008), or is it a `Carry` producer? (the no-workaround test)
- **F2 conventions** — day-count/calendar/schedule reuse vs reimplementation vs `celnet-conventions`.
- **F3 numerics** — `solver::brent_root` vs `celnet-surface` fitmath / `celnet-vanilla` root-finders.
- **F4 risk layering** — `rates::risk` PV01/DV01/key-rate vs `celnet-risk-cube` (ADR-0008 risk layer).
- **F5 completeness** — any other new overlap (GUI Excel workspace, schedule, bootstrap).

## Process
1. **Capture** (Sonnet author → Opus verify) — source-grounded claims on the new code.
2. **Review** (Opus, refute-default) — a verdict per dimension: real_overlap / acceptable_for_now /
   already_integrated, with cited evidence + an ADR-aligned recommendation.
3. **Store in lodestar** — author capture claims + review verdict claims centrally
   (author `celnet-knowledge`, idempotent), refresh the mirror, commit.
4. **Remediate** confirmed overlaps (ADR-aligned), each gated; record decisions as ADR claims.
