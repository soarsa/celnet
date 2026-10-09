# ADR-0007 — One unversioned wire contract (evolve in place, zero legacy)

- **Status:** Accepted. Governs `celnet-proto` and every client (`celnet-client` SDK, the
  WebSocket JSON mirror, the admin CLI, Excel) — the single contract they all consume.
- **Honoured by:** ADR-0008 (multi-asset carry — additive `Underlying`/`CarryModel` oneofs),
  ADR-0009 (edge wire codec — `CELNHND1` content tag, *not* a schema version), and
  GUIDE.md guardrail #9.

## Context

Celnet has **no external users** — there is exactly one deployment, upgraded uniformly with
no mixed-version window. A versioned wire contract (`schema_version`, N/N-1 negotiation,
back-compat shims) would buy nothing and cost permanent complexity: every payoff, client,
and codec path would carry version branches that can never be exercised by a real peer.

## Decision

Ship **exactly one clean, current wire contract** and evolve it **in place**:

- **No `schema_version`, no template/version field, no `schemaId` negotiation, no N/N-1
  shims.** The proto is the contract; changing it changes the contract for everyone at once.
- **Additive, default-compatible growth.** New capability is added as new oneof arms /
  fields with sensible defaults (e.g. ADR-0008's `Underlying`/`CarryModel` oneofs default to
  FX), so existing call sites are unchanged without any version handshake.
- **Content tags, not version tags.** Where a byte stream needs a discriminator (the
  ADR-0009 hand-off codec's `CELNHND1` magic), it is a fixed *content* tag identifying the
  format, never a monotonic schema version to negotiate against.
- **API-first parity is the governing rule.** All clients consume this one contract; a
  contract change propagates to SDK, WS mirror, CLI, and Excel in lockstep — never a
  per-client version skew.

## Consequences

- Refactor and evolve the schema freely; there is no compatibility window to preserve and no
  legacy shape to keep alive (GUIDE.md guardrails #9, #10).
- Conformance is a single corpus run across all 5 clients with `to_bits` equality — there is
  one right answer, not a matrix of versions.
- A peer that needs a different contract is out of scope by construction; we do not add a
  version axis to accommodate hypothetical external integrators.

## Alternatives rejected

- **`schema_version` + N/N-1 negotiation** — pure cost with no external multi-version peer to
  serve; every codec/payoff/client path would carry dead version branches.
- **A frozen v1 contract evolved only by adding v2** — re-introduces the legacy shape and a
  mixed-version window we explicitly do not have.
