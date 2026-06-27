# Security finding — caller authorization is not a cross-cut (VERIFIED, open)

Surfaced by the architecture determination, then **adversarially verified** (refute-default,
source-cited) 2026-06-27. Status: **CONFIRMED**. Also recorded as an `adr` claim in lodestar
(anchored to `Session::handle_execute` / `handle_subscribe`).

## The exposure
Any client that completes the **WS handshake** — or opens a gRPC **StreamSession** — gets a
fully-driven RFS session with **no authentication and no entitlement check**: it can subscribe
to any instrument's stream and **execute a click-to-trade**. The execute path is gated only by a
server-minted last-look token, which the same unauthenticated session is handed on subscribe.
`QuoteService::accept_quote` is a second, lower-volume booking path with the same gap.

## Per-surface verdict

| Surface | Verdict | Evidence |
|---|---|---|
| gRPC FixAdminService | **ENFORCED** (genuine baseline) | every RPC: `resolve_caller` + `authorize_caller(Admin)` + server-side `desk_scope` (`services/fix_admin.rs`) |
| gRPC RiskService | **ENFORCED (role) — entitlement caller-asserted** | `risk/mod.rs:718` gates role; but `aggregate_risk_impl:344`/`drill_risk_impl:401` prune by **body `req.principal`**, and `convert::principal_of:93` maps an omitted principal → `grant_all` (whole-firm) |
| gRPC StreamService | **UNGATED (fully open)** | `stream_session:496` gates only `is_ready()`; `Session:834` has no principal/token field; `handle_subscribe:929` / `handle_execute:1135` perform no caller check (last-look token only) |
| WS RFS stream (`dispatch`→`rfs_in_tx`) | **UNGATED (fully open)** | `ws/serve_connection:269` accepts the socket with no auth handshake; same principal-free `Session` |
| WS unary RPCs (`handle_unary`) | **mirrors the service** | `ws/mod.rs:538` — token/principal ride in the body, so risk/fix-admin stay gated, pricing/quote stay ungated (no token-stripping bug) |
| PricingService (`price`) | **UNGATED — plausibly by-design (public)** | `services/pricing.rs:84` — readiness only; pricing a hypothetical with client-supplied market is reasonably public |
| QuoteService (`accept_quote`) | **UNGATED** | `quote.rs:684` — idempotency-key + last-look + panel-line integrity, but no caller/desk entitlement |

## Minimal fix (one seam, not per-handler)
1. **Stream/WS:** authenticate the session open — give `Session` a `caller: ResolvedCaller` set
   once (validate a `session_token` at `stream_session`/`serve_connection` before `session_driver`
   runs); have `handle_subscribe`/`handle_execute`/`handle_modify` check the caller's `desk_scope`
   against the instrument's desk; reject anonymous sessions under `Enforce`.
2. **Pricing/Quote:** if not intentionally public, add the same `resolve_caller`+`authorize_caller(ReadAny)`
   two-liner; bind `accept_quote` to the requester's resolved principal, not just the idempotency key.
3. **Risk hardening:** intersect body `req.principal` with the session-derived `desk_scope` in
   `aggregate_risk_impl`/`drill_risk_impl` so an omitted principal cannot widen to `grant_all`.

A single tonic interceptor + a WS pre-handshake auth check producing a `ResolvedCaller` threaded
into the session is the uniform cross-cut the codebase currently lacks.

## Caveat (threat model)
This is a static-architecture verdict on the server code. If the WS/stream endpoints sit behind
an authenticating gateway in deployment (the Celer estate edge), the *exploitability* is reduced —
but the server itself enforces nothing, so defense-in-depth is absent. Confirm the deployment
posture before sizing the remediation.
