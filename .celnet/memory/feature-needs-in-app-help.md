---
name: feature-needs-in-app-help
description: Every new trader-facing feature MUST ship an in-app help guide so traders can understand how to configure it (like the pricing-source setting).
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b69ceb7d-66d6-4bc7-ac17-96e24adc4293
---

Every new trader-facing feature we build MUST ship with an **in-app help guide** that
explains what it is and **how to configure it** — traders should be able to learn a
feature from inside the application, not from tribal knowledge or external docs. The
canonical example the user pointed to is the **pricing-source setting** (internal curve
vs aggregated book) on the Pricing Groups editor — a feature like that must not land
without its help.

Concretely, a feature is not "done" until it also delivers, as appropriate to its size:
- an entry in the trader **Help center**,
- a **rich in-editor help panel** on the surface where it's configured (the established
  pattern — see commit `2ad2468` "trader Help center + rich in-editor help panels +
  guided tutorials"),
- a **guided tutorial** for anything multi-step to set up,
- and the `docs/` guide kept in sync (e.g. `docs/TRADER-RULE-ENGINE-SETUP.md` for the
  acceptance/routing/hedging engines).

**Why:** the platform is trader-centric ([[CLAUDE.md]] guardrail 11) and configuration
surfaces (pricing groups, acceptance/risk/hedging rule engines, pricing source, tiering,
aggregated book, notifications, permissions) are dense and non-obvious. The user has
repeatedly had to *ask* what a control does (e.g. "where is the pricing-source setting",
"what do the internalise badge colors mean") — that's the signal the feature shipped
without adequate self-documenting help. Discoverable in-app help is a product requirement,
not a nice-to-have.

**How to apply:** when scoping/implementing any new trader-facing feature (or a new
config control on an existing surface), include the in-app help as part of the same
deliverable and verify it live — do not treat help as a follow-up. When delegating GUI
work, put "add the Help-center entry + in-editor help panel (+ tutorial if multi-step),
matching the existing help pattern" into the agent brief. Retrofit help for recently
shipped controls that lack it — starting with the **pricing-source mode** selector
(`cf2cfff`/`d452497`), which currently has no dedicated help entry.

Related: [[fi-pricing-groups-shipped]], [[session-pivoted-tiering-shipped]],
[[risk-routing-build-state]].
