# Requirement — Permissions & Administration (action capabilities across FX options + fixed income)

Status: **REQUIREMENT (not yet implemented)** · Authored 2026-06-28 · Owner: auth/identity lane
Supersedes nothing; **extends** the existing identity/entitlement model. This is the
capability layer the `accept`/quote/execute paths in
[`SECURITY-AUTHZ-FINDING.md`](../archive/audits/SECURITY-AUTHZ-FINDING.md) §"Minimal fix" #2/#3 are missing.

---

## 1. Why

The platform today authenticates a caller and answers exactly **two** authorization
questions — *"can this caller read?"* (`RequiredAuthority::ReadAny`) and *"is this caller an
admin?"* (`RequiredAuthority::Admin`) — plus a read-side **risk** entitlement `Principal`
(who *sees* which risk subtree). What it cannot yet answer is the question a trading desk
actually asks:

> *May **this user** **quote** / **respond to an RFQ** / **respond to an IOI** / **stream prices** /
> **execute (deal)** / **book** — on **FX options** vs **fixed income** — for **this desk**?*

Role is binary (`Role::{Admin, Trader}`, `config/identity.rs:47`). A "Trader" can do
everything a desk member can do, on every asset class, with no separation between *pricing*
and *dealing*, and no per-asset-class grant. Several booking paths are entirely ungated
(`QuoteService.AcceptQuote`, `celnet.proto:2990`, confirmed ungated in the security finding).
This requirement defines a **capability model** that closes that gap and the **administration
surface** to manage it, and binds the **UI** so a user only sees and can only invoke what they
are entitled to.

## 2. Current state (what we build on — do not duplicate)

| Concern | Where it lives today | Keep / extend |
|---|---|---|
| Authentication (login → session token → caller) | `AuthService.Login`, `SessionRegistry`, `AuthenticatedUser{user_id,email,role,desk_id}` (`services/sessions.rs`) | **Keep** — the capability set is resolved from the authenticated user |
| Coarse authority gate | `authorize_caller(mode, caller, resource, RequiredAuthority::{ReadAny,Admin})` (`services/access.rs`) | **Extend** — add a capability-typed authority |
| Enforce vs Permissive trust mode | `AccessMode::{Enforce,Permissive}` (`celnet-entitlements::decision`) | **Keep** — capabilities honour the same dev-mode escape hatch |
| Desk ownership / visibility | `DeskScope::{All,Desk(id),Deskless}` (`services/access.rs`) | **Keep** — capabilities are *scoped by desk* through the same type |
| Read-side risk entitlement | `Principal` grant/deny over the risk cube (`celnet-entitlements`) | **Keep, orthogonal** — "who sees which risk" stays separate from "who may act" |
| User/desk admin | `IdentityAdminService.{CreateUser,UpdateUser,DeleteUser,CreateDesk,DeleteDesk}` (`celnet.proto:3534`) | **Extend** — assign roles/capabilities/desk |
| Seed admin | `admin@celnet.com` / `password` (`config/identity.rs:39`) | **Keep** |

**Design invariant carried forward:** authorization is a **server-side seam**, not a
per-handler sprinkle. Capabilities resolve from the *session-authenticated* user, never from a
client-asserted body field (the security finding's #3 lesson: an omitted body principal must
never widen authority).

## 3. The capability model

### 3.1 Capability = (Action × AssetClass), scoped by Desk

A **capability** is the right to perform one **action** on one **asset class**:

```
Capability := Action × AssetClass
Action     := View | Price | QuoteRespond | RfqRespond | IoiRespond | Stream | Execute | Book | Administer
AssetClass := FxOptions | FixedIncome      (Rates is part of FixedIncome)
```

| Action | Meaning | Gates (server) | Gates (UI) |
|---|---|---|---|
| `View` | see blotters/positions/curves for the asset class | risk/list/curve reads | workspace visible |
| `Price` | price a hypothetical (no market commitment) | `Price`, `PriceRates` | Pricer inputs enabled |
| `QuoteRespond` | publish a tradeable quote on a panel/RFS line | quote publish | "Quote" action |
| `RfqRespond` | respond to a counterparty **RFQ** | `RfqDeskService.RespondDeskRequest` (kind=RFQ) | Quoting "Respond" on RFQ rows |
| `IoiRespond` | respond to / action an **IOI** | `RespondDeskRequest` (kind=IOI) | Quoting "Respond" on IOI rows |
| `Stream` | open an RFS price stream (subscribe) | `StreamService.StreamSession` subscribe | live-stream tiles |
| `Execute` | **deal** — click-to-trade / accept a quote | `StreamSession` execute, `QuoteService.AcceptQuote`, `RfqDeskService.AcceptDeskQuote` | "Deal"/"Accept" buttons |
| `Book` | book a resulting position to a desk book | `RiskService.BookRatesPosition` | "Book" action |
| `Administer` | user/desk/connection/permission admin | `IdentityAdminService.*`, `FixAdminService.*` | Administration workspace |

> **Separation of duties is now expressible.** A junior trader can hold
> `Price+Stream+View` on `FxOptions` but *not* `Execute` — pricing and dealing are distinct
> capabilities, which the binary role cannot represent today. A FI sales user can hold
> `RfqRespond+IoiRespond` on `FixedIncome` without `FxOptions` rights at all.

### 3.2 Desk scope on every capability

Each granted capability carries a **desk scope** resolved through the existing
`DeskScope`:

- `DeskScope::All` — firm-wide (admins, central risk).
- `DeskScope::Desk(id)` — only requests/streams/books **owned by that desk** (the common
  trader grant; mirrors how `RfqDeskService` already filters inbound traffic by `desk_id`).

The gate is therefore **two-dimensional**: *does the user hold the capability* **and** *does
the target resource fall inside the capability's desk scope*. This reuses the desk-ownership
filter `FixAdminService` already applies (`services/fix_admin.rs`) — no new scoping primitive.

### 3.3 Roles are named capability bundles (not a parallel system)

To keep administration ergonomic, a **Role** becomes a *named, editable bundle* of
capabilities — the binary enum is replaced by a small set of seeded roles plus per-user
overrides:

| Seeded role | Capability bundle (illustrative default; editable by admin) |
|---|---|
| `Administrator` | `Administer` + `View(all)` firm-wide |
| `FX Options Trader` | `View,Price,QuoteRespond,Stream,Execute,Book` on `FxOptions`, `DeskScope::Desk(self)` |
| `FI / Rates Trader` | `View,Price,QuoteRespond,RfqRespond,IoiRespond,Stream,Execute,Book` on `FixedIncome`, `DeskScope::Desk(self)` |
| `Sales` | `View,RfqRespond,IoiRespond` on both asset classes, `DeskScope::Desk(self)` |
| `Risk / Read-only` | `View` on both, `DeskScope::All`, **no** action capabilities |

**Resolution:** a user's effective capability set = (role bundle) **∪** (explicit per-user
grants) **∖** (explicit per-user denials). **Deny wins**, mirroring the entitlement filter's
information-barrier semantics (`celnet-entitlements` deny-by-default + deny-wins). Default for
a user with no role and no grants = **nothing** (deny-by-default, the existing
`Principal::default` posture).

## 4. Enforcement architecture (one seam)

1. **Extend `RequiredAuthority`** with a capability variant:
   `RequiredAuthority::Capability(Action, AssetClass)` alongside the existing `ReadAny`/`Admin`.
2. **Resolve once per request** at the service edge: from the `ResolvedCaller`
   (session-authenticated, already threaded for stream/WS in the security-finding remediation),
   load the user's effective capability set + desk scope.
3. **`authorize_caller`** answers *capability ∧ desk-scope ∧ AccessMode* and emits the **same
   one structured audit record per decision** the access seam already writes — every
   deal/quote/respond decision is audited (allow **and** deny), no new logging path.
4. **Close the ungated paths** named in the security finding using this seam:
   - `QuoteService.AcceptQuote` → `Capability(Execute, FxOptions)`.
   - `RfqDeskService.AcceptDeskQuote` → `Capability(Execute, <class of request>)`.
   - `RfqDeskService.RespondDeskRequest` → `Capability(RfqRespond|IoiRespond, class)` by `kind`.
   - `StreamService.StreamSession` execute frame → `Capability(Execute, class)`; subscribe →
     `Capability(Stream, class)`.
   - `RiskService.BookRatesPosition` → `Capability(Book, FixedIncome)`.
5. **No new transport for the check** — capabilities ride on the resolved caller, never on a
   request body. (Closes finding #3: body principal can only ever *narrow*, never widen.)

## 5. UI permissions (derived, never authoritative)

The GUI/Excel surfaces must reflect capabilities so users are not shown actions they cannot
perform — but the **server remains the sole authority** (UI gating is UX, not security):

- **Workspace visibility**: a workspace renders only if the user holds at least `View` on its
  asset class (`Quoting`, `Deals`, `Rates Book`, `Administration`, …).
- **Action affordances**: "Quote", "Respond", "Deal/Accept", "Book" buttons are **disabled +
  tooltip-explained** when the matching capability/desk-scope is absent — they are never merely
  hidden silently where a user might expect them (discoverable, explained denial).
- **Capability delivery**: the authenticated session response carries the resolved capability
  set (a flat, current contract — no versioning) so the client can render without a second
  round-trip. The client treats it as advisory; the server re-checks on every action.
- **Live revocation**: an admin permission change pushes through the existing notification
  stream so an open client re-resolves capabilities without a reload (consistent with the
  release-reload watcher already shipped).

## 6. Administration surface (requirements)

Extends `IdentityAdminService`. All admin RPCs require `Capability(Administer, *)`.

1. **Users**: create/update/delete (exists) **+ assign role, add/remove explicit capability
   grants & denials, set desk membership, reset password, enable/disable**.
2. **Roles**: list/create/update/delete a named capability bundle; edit the capability matrix
   per role (the §3.3 table is the *seed*, not hard-coded policy).
3. **Desks**: create/delete (exists) + list members.
4. **Audit view**: read back the authorization decision log (the records the access seam
   already emits) filtered by user/action/outcome — admins can answer "who dealt what, and was
   anyone denied".
5. **Admin GUI**: an `Administration` workspace (users table → user detail with role +
   capability matrix editor + desk picker; roles editor; audit pane). Anti-template, consistent
   with the existing design system; capability matrix is a real grid, not a wall of checkboxes
   with no hierarchy.

## 7. Data & contract changes (single current contract — no versioning)

- **Wire (proto)**: add `Capability`, `Action`, `AssetClass` messages/enums; extend the
  authenticated-session response with the resolved capability set; extend
  `UpdateUserRequest` / add role+grant RPCs to `IdentityAdminService`. All new fields/variants
  carry **leading** `//` doc comments (prost only propagates leading comments → missing_docs).
- **Identity store**: persist roles, per-user grants/denials, and desk membership in the
  identity config/store (`config/identity.rs`) — the seed admin keeps full `Administer`.
- **Server**: `RequiredAuthority::Capability(..)`; capability resolution from `ResolvedCaller`;
  gate the six paths in §4; one audit record per decision.
- **Clients**: GUI `Administration` workspace + capability-aware affordance gating in
  `Quoting`/`Deals`/`Rates Book`; Excel mirrors the same capability checks on its action paths.

## 8. Acceptance criteria

- [ ] A user with `Price` but not `Execute` on `FxOptions` can price and stream but receives a
  **denied (audited)** response from `AcceptQuote`/stream-execute — verified live under
  `AccessMode::Enforce`.
- [ ] A FI desk user with `DeskScope::Desk(7)` cannot `RespondDeskRequest`/`AcceptDeskQuote`
  for a request owned by desk 9 (cross-desk denial), even holding the capability.
- [ ] An omitted/forged body principal cannot widen authority beyond the session-resolved
  capability set (finding #3 regression test).
- [ ] Deny wins: an explicit per-user denial overrides a role grant.
- [ ] Every deal/quote/respond/book decision (allow **and** deny) emits exactly one structured
  audit record.
- [ ] The Administration GUI can create a user, assign a role, add/remove a capability, set a
  desk, and the change takes effect on the user's **open** session via the notification stream.
- [ ] GUI/Excel disable (with explanation) actions the user lacks; server re-checks regardless.
- [ ] Gates: T2 full set + live GUI/Excel e2e under `Enforce`; no `match`-on-asset-class in any
  hot payoff/stream path (capability resolve happens at the request edge, not the hot core).

## 9. Non-goals

- Not a new authentication mechanism — login/session/token stay as-is.
- Not replacing the read-side risk `Principal` — capabilities (may *act*) and entitlements
  (may *see* risk) remain orthogonal layers.
- No API versioning / N-1 negotiation (guardrail #9): one current contract, evolved in place.
- No external IdP/SSO integration in this slice (deployment gateway concern; can follow).

## 10. Suggested delivery slices (each its own gated workflow)

1. **Kernel**: `Capability/Action/AssetClass` types + resolution + `RequiredAuthority::Capability`
   in `celnet-entitlements`/server access seam, with unit tests (deny-wins, desk-scope ∩).
2. **Close ungated paths**: gate the six §4 RPCs; finding #2/#3 regression tests; live Enforce.
3. **Admin contract + store**: roles/grants/denials persistence + `IdentityAdminService`
   extensions; audit read-back.
4. **Admin GUI**: `Administration` workspace (users/roles/desks/audit) + live-revocation.
5. **Affordance gating**: capability-aware enable/disable across Quoting/Deals/Rates Book in
   GUI + Excel; e2e.

---

*Provenance/why is recorded here and (to be) anchored as a lodestar `adr` claim against the
access seam once slice 1 lands. Cross-references: [`SECURITY-AUTHZ-FINDING.md`](../archive/audits/SECURITY-AUTHZ-FINDING.md),
`docs/RISK-HIERARCHY.md` §2.6/§4, `docs/EXPERIENCE-ARCHITECTURE.md` P2-8.*
