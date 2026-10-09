# Security finding — caller authorization is not a cross-cut (VERIFIED)

Surfaced by the architecture determination, then **adversarially verified** (refute-default,
source-cited) 2026-06-27. Status: **CONFIRMED → FULLY REMEDIATED 2026-06-27**. The headline
stream/WS hole was closed first (`8e0ee48`); the §2 quote-accept gating/binding and the §3 risk
`principal∩desk_scope` hardening (via a full desk-identity bridge) landed second (item B). Also
recorded as an `adr` claim in lodestar (anchored to `Session::handle_execute` / `handle_subscribe`).

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
| gRPC RiskService | **ENFORCED (role) + desk-scoped (REMEDIATED §3)** | role gated as before; the reads now narrow the effective principal to the **session's desk** (`effective_principal`/`narrow_to_desk`) via the desk-identity bridge, so an omitted/grant-all body principal can no longer widen a non-admin trader to firm-wide (admin/no-session byte-identical) |
| gRPC StreamService | **ENFORCED (REMEDIATED)** | `Session` now holds `caller: ResolvedCaller` pinned from the `Authenticate` frame; one gate `authorize_caller(self.access_mode, &self.caller, ReadAny)` runs before subscribe/modify/execute/resync; anonymous rejected under `Enforce` |
| WS RFS stream (`dispatch`→`rfs_in_tx`) | **ENFORCED (REMEDIATED)** | `decode_stream_control` decodes `authenticate`; `is_stream_control` routes it to the SAME `Session` gate; the WS mirror enforces identically to gRPC |
| WS unary RPCs (`handle_unary`) | **mirrors the service** | `ws/mod.rs:538` — token/principal ride in the body, so risk/fix-admin stay gated, pricing/quote stay ungated (no token-stripping bug) |
| PricingService (`price`) | **UNGATED — plausibly by-design (public)** | `services/pricing.rs:84` — readiness only; pricing a hypothetical with client-supplied market is reasonably public |
| QuoteService (all 4 RPCs) | **GATED + bound (REMEDIATED §2)** | `request_quote`/`request_multi_dealer_quote`/`accept_quote`/`reject_quote` now `resolve_caller`+`authorize_caller(ReadAny)` before the forward branch; `accept_quote` binds to the **authenticated requester** (`RequesterBinding`) on top of the idempotency-key + last-look + panel-line integrity |

## Minimal fix (one seam, not per-handler)
1. ✅ **DONE — Stream/WS:** authenticate the session open — `Session` carries a `caller: ResolvedCaller`
   pinned once from the `Authenticate` frame; one `authorize_caller(ReadAny)` gate runs before
   subscribe/modify/execute/resync on the gRPC stream AND the WS mirror; anonymous rejected under
   `Enforce`. All five clients authenticate-first (grant-all default). Live-validated under Enforce.
2. ✅ **DONE (item B §2) — Quote:** all four QuoteService RPCs now `resolve_caller`+`authorize_caller(ReadAny)`
   (PricingService stays intentionally public); `accept_quote` is **bound to the authenticated requester**
   (`RequesterBinding::Authenticated(user_id)`), refusing a different authenticated caller on top of the
   idempotency-key match. Pricing left public by design. Adversarially verified + live-e2e under Enforce.
3. ✅ **DONE (item B §3) — Risk hardening:** body `req.principal` is intersected with the session-derived
   desk in `aggregate_risk_impl`/`drill_risk_impl` (+ `list_positions`/`limit_status`) via the desk-identity
   bridge, so an omitted/grant-all principal cannot widen a non-admin trader to `grant_all`.

   **Design decision (2026-06-27, operator-chosen): build the desk-identity bridge.** The session's
   desk is a `String` slug (`config::identity::DeskDef.id`) but risk facts are keyed by numeric
   `risk_cube::DeskId(u64)` (set from wire `OrgKey.desk` at `risk/convert.rs:265`) — there is **no**
   slug↔u64 bridge today, so a *silent narrowing* of a trader to their desk is impossible without one.
   Rather than a deny-by-default clamp, the operator chose the complete fix: a **canonical desk
   registry** mapping each identity desk slug → a stable risk `DeskId(u64)`, populated through the
   book/attribution path so a logged-in trader's desk reconciles with their facts' `OrgKey.desk`; then
   `aggregate_risk_impl`/`drill_risk_impl` narrow a non-admin session's effective principal to
   `Rule::on(DimensionId::Desk, their_desk_u64) ∩ body_principal`. Admin / no-session / federation
   paths stay byte-identical (`DeskScope::All` ⇒ no narrowing). This is item B §3 in
   `docs/plan/NEXT-ARCHITECTURE-IMPLEMENTATION.md`.

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
an authenticating gateway in deployment (the CelNet estate edge), the *exploitability* is reduced —
but the server itself enforces nothing, so defense-in-depth is absent. Confirm the deployment
posture before sizing the remediation.
