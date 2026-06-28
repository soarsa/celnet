# Item B — Auth §2/§3 follow-up (quote-accept binding + desk-identity bridge)

> Implementation spec for master-plan item B (`docs/plan/NEXT-ARCHITECTURE-IMPLEMENTATION.md` §2).
> Closes the two lower-severity halves of the verified security finding
> (`docs/SECURITY-AUTHZ-FINDING.md` §2/§3). Branch: `arch/b-auth-quote-risk`. Authored from a
> cited code scout 2026-06-27; every edit site below is a real file:line, no guesses.
> ONE T2 (full check + live GUI/Excel e2e under `CELNET_ACCESS_MODE=enforce`) at the end.

## Design decisions (operator-confirmed)
- **§3 = full desk-identity bridge** (not a deny-by-default clamp). The session's desk is a `String`
  slug (`config::identity::DeskDef.id`); risk facts key by numeric `risk_cube::DeskId`. **There is no
  bridge today AND the desk dimension is unpopulated in production**: the live attribution path
  (`risk/store.rs:428 book_from_attribution`) hardcodes `desk: DeskId(0)` (resolve-via-parent), and
  `set_book_desk` (the only book→desk wiring) is **test-only**. So the bridge must (a) give desks a
  canonical numeric id, (b) declare book→desk membership in config, (c) populate the hierarchy at boot,
  (d) narrow a non-admin session's effective principal to its desk. Admin / no-session / federation
  paths stay byte-identical (`DeskScope::All` ⇒ no narrowing).
- **§2 WS path is unary (token rides in the body)** — unlike the stream cross-cut, no router-class
  change: add fields to the proto message, decode them in the WS codec, the service reads them. The
  risk service already does exactly this (`risk/mod.rs:701 req.session_token`).
- **No `schema_version`** (guardrail 9): the new proto fields are additive `optional`; `DeskDef.books`
  is an additive serde-default field. Absent principal/token ⇒ the audited grant-all default, exactly
  as the risk path and the stream cross-cut already default.

## Phase 1 — Contract (proto + WS codec)
- `crates/celnet-proto/proto/celnet.proto`:
  - `QuoteRequest` (1546): add `optional string session_token` + `optional EntitlementPrincipal principal`.
  - `QuoteAccept` (1667): add `optional string session_token` + `optional EntitlementPrincipal principal`.
  - (RejectQuote uses `QuoteReject`? confirm; gate it too if it has its own message.)
- `crates/celnet-server/src/ws/codec.rs`:
  - `quote_request_from_json` (952): decode `session_token` (`opt_string`) + `principal`
    (`entitlement_principal_from_json`, reuse the risk path's decoder).
  - `quote_accept_from_json` (1048): same.
  - The `*_to_json` encoders (for the SDK/clients’ outbound frames) gain the two fields.

## Phase 2 — Server §2 (gate + bind)
- `QuoteEdge` (`services/quote.rs:235`) gains `sessions: Arc<SessionRegistry>` + `store: Arc<PositionStore>`
  (the shared access-mode authority — `access_mode()` is runtime-settable, store-held). Add a
  `.with_sessions(...)` builder mirroring `RiskEdge::with_sessions` (risk/mod.rs:205); default to
  `default_sessions()` so existing `QuoteEdge::new`/`with_fleet` test constructors keep compiling.
- Boot (`lib.rs:371`): create `sessions` BEFORE the quote edge (move line 385 up), build the quote edge
  `.with_sessions(Arc::clone(&sessions))` and pass `Arc::clone(&store)`.
- Gate the 4 RPCs — `request_quote` (408), `request_multi_dealer_quote` (544), `accept_quote` (684),
  `reject_quote` (866): `resolve_caller(&self.sessions, req.session_token.as_deref(), req.principal.clone())`
  + `authorize_caller(self.store.access_mode(), &caller, "QuoteService/<rpc>", ReadAny, correlation_id)`.
- **Bind accept to the requester principal**: on `request_quote`/`request_multi_dealer_quote`, record the
  resolved caller's identity (user email when authenticated, else the asserted principal fingerprint) on
  the stored `QuoteRecord`. On `accept_quote`, after resolving the accepting caller, **require it match
  the recording caller** (a different authenticated user, or a different asserted principal, is refused
  `permission_denied`) — closing "any party that learns a quote_id books another requester's quote",
  beyond the idempotency-key match. Absent-caller (permissive/no-token) keeps the legacy idempotency-only
  behaviour so the demo edge is unchanged.

## Phase 3 — Server §3 (desk bridge + risk narrowing)
- `DeskDef` (`config/identity.rs:113`): add `#[serde(default)] books: Vec<String>` (book NAMES owned by
  the desk; matches the attribution book strings the interner sees).
- `PositionStore` (`risk/store.rs`): expose `intern(&str) -> u32` (through `g.interner`) + a
  `configure_desk(desk_slug, &[book_name])` that interns the slug → `DeskId`, interns each book → BookId,
  and `set_book_desk(book, desk)`. Canonical desk id = the interned handle of the slug (stable/idempotent
  within a run; interning is idempotent so the live path’s lazy book interning yields the same handles).
- Boot (`lib.rs` after identity_store load ~470): for each `DeskDef`, `store.configure_desk(&d.id, &d.books)`.
- `ResolvedCaller` (`access.rs:259`): add `risk_desk_scope(&self, store) -> Option<DeskId>` OR expose the
  desk slug; the risk impls resolve slug→DeskId via `store.intern`. Admin/no-session ⇒ `None` (no narrowing).
- `aggregate_risk_impl` (risk/mod.rs:344) + `drill_risk_impl` (401): take `&ResolvedCaller` (handlers
  already resolve it at 699/725/751 — thread it in). Compute `effective = principal_of(req.principal)`
  then, if the caller is a desk-bound non-admin, narrow: grant-all ⇒ `scoped().grant(Rule::on(Desk, d))`;
  scoped ⇒ each grant `.and(Desk, d)`; denies carried (deny-wins). `limit_status_impl`/`list_positions_impl`
  if they prune by principal too — audit + apply the same narrowing.
- Independent check (oracle): a desk-D trader asserting grant-all sees EXACTLY the same nodes as the same
  trader asserting `scoped().grant(Desk=D)`, and STRICTLY FEWER than an admin — a property test over a
  two-desk seeded cube (mirror `risk/mod.rs:923` seeding).

## Phase 4 — Clients (after the contract settles)
- Rust SDK `crates/celnet-client`: the quote builders (`Rfq`, `request_quote` lib.rs:285, `Rfq::accept`
  748, the multi-dealer path) put `self.session_token` + `self.principal` into the QuoteRequest/QuoteAccept
  body (mirroring how risk requests already carry them). Default principal = `principal_or_grant_all`.
- CLI `crates/celnet-cli/src/rfq.rs`: inherits the SDK; add `--session-token` thread like `stream` has.
- GUI `gui/src/data/wsTransport.ts` (requestQuote 769, acceptQuote 815) + `wsCodec.ts`: frames carry
  `session_token` (from `WsConnection`) + `principal` (`principalOrGrantAllToWire`). `mockSource.ts` mirrors.
- Excel `excel/src/transport/connection.ts` (requestQuote 785, acceptQuote 835): same.

## Gate
- T1 on changed crates as the batch settles (celnet-proto, celnet-server, celnet-client, celnet-cli,
  celnet-entitlements if touched). **T2 once**: full `just check` + live GUI/Excel e2e under
  `CELNET_ACCESS_MODE=enforce` (the non-negotiable for a gui/excel/wire item).
- FX byte-identity unaffected (no carry/pricing change). Adversarial verify (celnet-verifier): hidden
  mocks, the desk-narrowing oracle independence, contract drift across the 5 clients, "is it done".
