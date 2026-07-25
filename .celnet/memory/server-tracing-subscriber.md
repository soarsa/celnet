---
name: server-tracing-subscriber
description: celnet-server had NO tracing subscriber installed — all tracing::* events were dropped; fixed + added detailed login/quote logging
metadata: 
  node_type: memory
  type: project
  originSessionId: 5ff9525a-81fa-41e7-a4f8-745f35d76a80
---

**Root cause (fixed 2026-07-01, commit 43b27c6):** `crates/celnet-server/src/main.rs` never
installed a `tracing` subscriber, so EVERY `tracing::*` event in the server was silently dropped in
production — login, quotes, and even the seed-admin `SECURITY:` warning (which is why that warning
never appeared in the log during the admin-reset investigation, and why the API log looked thin).
The `ready —`/`draining` lines that DID appear are `println!`, not tracing. Fix: `main()` now calls
`celnet_observability::init_json_subscriber(&LogConfig::default())` as its first line (INFO default,
honours `RUST_LOG=info` from the deploy); a failed install is non-fatal so tests keep their own.

**Detailed logging added** (edge handlers only — NOT the zero-alloc hot core; no secrets):
- `services/auth.rs` (was zero tracing): login ok/fail (`email`, `role`, or `reason`
  ∈ unknown_user|bad_password|disabled|rate_limited — external `unauthenticated` still opaque),
  logout, create/update/delete user, reset password (actor + target_email).
- `services/quote.rs` (shared layer → both gRPC + WS mirror): quote requested/returned/rejected
  (requester, idempotency_key, instrument summary, quote_id, bid/offer/reason) + multi-dealer panel.

**Effect:** ships with the NEXT release — redeploy to get INFO-level structured login/quote logs in
`/opt/celnet/shared/logs/celnet-server.out.log`. **Pre-existing unrelated test failure**
`forwarding::quote_request_then_accept_routes_to_issuer` (AcceptQuote capability gate, no session) —
fails identically on clean tree (verified via git stash), untouched by this change.
