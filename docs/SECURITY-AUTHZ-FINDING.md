# Security finding — caller authorization is not a cross-cut (VERIFIED)

Surfaced by the architecture determination, then **adversarially verified** (refute-default,
source-cited) 2026-06-27. Status: **CONFIRMED → stream/WS REMEDIATED 2026-06-27** (quote-accept
binding + risk `principal∩desk_scope` hardening remain as a tracked lower-severity follow-up).
Also recorded as an `adr` claim in lodestar (anchored to `Session::handle_execute` / `handle_subscribe`).

## Remediation status (stream/WS — the headline hole)
The unauthenticated-subscribe/execute exposure is **closed end-to-end** on `security/authz-cross-cut`:
the server pins a `ResolvedCaller` from a `StreamAuth` `Authenticate` frame and enforces one gate
(`authorize_caller(ReadAny)`) before subscribe/modify/execute/resync on **both** the gRPC stream
(`Session::handle_client_message`) and the **WS mirror** (`ws::decode_stream_control` +
`is_stream_control` routing); all five clients (SDK, CLI, GUI, Excel) send the frame **first** on
every (re)connect with the audited grant-all default. Validated live under `AccessMode::Enforce`:
GUI Playwright/axe **165/165** (incl. click-to-trade streaming), Excel e2e **122/122**, plus new
Enforce unit/integration tests. A live-e2e-caught bug (the WS `authenticate` frame fell through to
`handle_unary`, leaving WS sessions anonymous) was fixed with a single routing source-of-truth +
lockstep regression tests.

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
| gRPC StreamService | **ENFORCED (REMEDIATED)** | `Session` now holds `caller: ResolvedCaller` pinned from the `Authenticate` frame; one gate `authorize_caller(self.access_mode, &self.caller, ReadAny)` runs before subscribe/modify/execute/resync; anonymous rejected under `Enforce` |
| WS RFS stream (`dispatch`→`rfs_in_tx`) | **ENFORCED (REMEDIATED)** | `decode_stream_control` decodes `authenticate`; `is_stream_control` routes it to the SAME `Session` gate; the WS mirror enforces identically to gRPC |
| WS unary RPCs (`handle_unary`) | **mirrors the service** | `ws/mod.rs:538` — token/principal ride in the body, so risk/fix-admin stay gated, pricing/quote stay ungated (no token-stripping bug) |
| PricingService (`price`) | **UNGATED — plausibly by-design (public)** | `services/pricing.rs:84` — readiness only; pricing a hypothetical with client-supplied market is reasonably public |
| QuoteService (`accept_quote`) | **UNGATED** | `quote.rs:684` — idempotency-key + last-look + panel-line integrity, but no caller/desk entitlement |

## Minimal fix (one seam, not per-handler)
1. ✅ **DONE — Stream/WS:** authenticate the session open — `Session` carries a `caller: ResolvedCaller`
   pinned once from the `Authenticate` frame; one `authorize_caller(ReadAny)` gate runs before
   subscribe/modify/execute/resync on the gRPC stream AND the WS mirror; anonymous rejected under
   `Enforce`. All five clients authenticate-first (grant-all default). Live-validated under Enforce.
2. ⤷ **FOLLOW-UP — Pricing/Quote:** if not intentionally public, add the same `resolve_caller`+`authorize_caller(ReadAny)`
   two-liner; bind `accept_quote` to the requester's resolved principal, not just the idempotency key.
3. ⤷ **FOLLOW-UP — Risk hardening:** intersect body `req.principal` with the session-derived `desk_scope`
   in `aggregate_risk_impl`/`drill_risk_impl` so an omitted principal cannot widen to `grant_all`.

A single tonic interceptor + a WS pre-handshake auth check producing a `ResolvedCaller` threaded
into the session is the uniform cross-cut the codebase currently lacks.

## Client-side counterpart (SDK + CLI + GUI + Excel + WS mirror)
**GUI** (`gui/src/data/wsTransport.ts`): `WsConnection.open`'s `onopen` sends `{type:"authenticate",
principal: principalOrGrantAllToWire(undefined), session_token?}` as the literal first frame on every
(re)connect. **Excel** (`excel/src/transport/connection.ts`): the same Authenticate-first + grant-all
default. **WS server** (`ws/codec::stream_auth_from_json` + `decode_stream_control` + `is_stream_control`):
decodes the frame and routes it to the shared session driver so the WS mirror enforces through the same
`Session` gate as gRPC — a defect where the frame fell through to `handle_unary` (WS sessions stayed
anonymous under `Enforce`) was caught by the live e2e and fixed with regression tests
(`authenticate_routes_to_the_stream_driver`, `classification_matches_the_decoder`).

### SDK + CLI
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
