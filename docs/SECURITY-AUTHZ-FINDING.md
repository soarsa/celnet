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

## Client-side counterpart (SDK + CLI)
The server seam above only enforces if clients actually authenticate. The Rust SDK
(`celnet-client`) now sends a `StreamAuth` **`Authenticate` frame as the FIRST control frame** on
every `StreamSession` open (and re-sends it first on a drain-cutover reconnect, since the re-dialed
stream is anonymous), so the server pins the caller before any subscribe/execute. It carries the
client's `session_token` when one is attached (`Client::with_session_token`, the real
`AuthService.Login` bearer) and an entitlement `principal` (`Client::with_principal`), defaulting —
exactly like the gated risk requests (`risk::principal_or_grant_all`) — to the audited **explicit
grant-all** so the headline streaming workflow is admitted under `Enforce` without relying on the
server granting an absent caller. The CLI `stream` command threads an optional `--session-token`
onto the SDK; otherwise it inherits the grant-all default. Sites: `Client::open_session` →
`rfs::SessionAuth` (`crates/celnet-client/src/{lib.rs,rfs.rs}`); CLI `run_stream`
(`crates/celnet-cli/src/risk.rs`). Covered end-to-end by
`rfs_workflow::stream_authenticates_under_enforce_with_login_token_and_grant_all_default` (real
Login token + grant-all default, both admitted under `Enforce`).

## Caveat (threat model)
This is a static-architecture verdict on the server code. If the WS/stream endpoints sit behind
an authenticating gateway in deployment (the Celer estate edge), the *exploitability* is reduced —
but the server itself enforces nothing, so defense-in-depth is absent. Confirm the deployment
posture before sizing the remediation.
