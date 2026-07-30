# Granular Per-Feature Permissions — Review & Target Design

Status: **DESIGN / REVIEW (not yet implemented)** · Authored 2026-07-30 · Owner: auth/identity lane
Read-only audit produced against `main`; **no code was changed** by this document.
Extends [`PERMISSIONS-ADMINISTRATION-REQUIREMENT.md`](plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md)
(the `Action × AssetClass` capability kernel, already shipped) — this is the follow-on that
replaces the remaining **coarse** `isAdmin` / blanket-`Administer` gates on the surfaces built
this session with **fine-grained, per-feature** capabilities, and adds **rail-entry visibility
control** (a user without the capability does not even see the entry).

---

## 0. The ask, in one line

The user wants to control precisely **who SEES** and **who can EDIT** each surface built this
session, replacing binary `isAdmin` / `Administer` gates with fine-grained capabilities —
**including rail visibility** (not just disabling controls inside a pane).

---

## 1. The model we build on (cite file:line)

The kernel is **already capability-based** — this is a refinement, not a rewrite.

- **`Action` × `AssetClass`** capability atom — `crates/celnet-entitlements/src/capability.rs:47`
  (`Action`: View, Price, QuoteRespond, RfqRespond, IoiRespond, Stream, Execute, Book,
  **RiskTransfer**, Simulate, Administer) `× AssetClass{FxOptions, FixedIncome}`
  (`capability.rs:129`). Deny-by-default, **deny-wins** algebra (`CapabilitySet::allows`,
  `capability.rs:236`). Actions carry stable snake_case **labels** (`Action::label`,
  `capability.rs:101`) and round-trip via `from_label` (`:121`) — **the wire is label-strings,
  not a renumbered enum**, so adding an action is purely additive.
- **Effective set** = `role bundle ∪ per-user grants ∖ per-user denies`, deny-wins
  (`AuthenticatedUser::capabilities`, `crates/celnet-server/src/services/sessions.rs:161`).
  `Role::Admin ⇒ CapabilitySet::grant_all()` (never narrowable, auto-holds every action incl.
  future ones); `Role::Trader ⇒ default_trader_bundle` (`config/identity.rs:104`) = **every
  action except `Administer` and `RiskTransfer`, both assets**, then the admin-editable
  per-role bundle + per-user overlay (`UserDef.capability_grants`/`capability_denies`,
  `identity.rs:161`/`:165`).
- **Server gates** — two seams, same algebra:
  - `RequiredAuthority::{ReadAny, Admin, Capability(Action, AssetClass)}` via
    `authorize_caller` (`services/access.rs:204`, `:395`) — used by `RiskService`/`FixAdminService`.
  - `AuthEdge::require_admin` (`services/auth.rs:346`) and `require_capability`
    (`services/auth.rs:364`) — token→identity checks used by the `AuthService`/`IdentityAdmin`
    RPC handlers (users, desks, risk books, pricing groups, agg books, routing, transfers).
- **Login projects the effective set to the client** — `LoginResult.capabilities`
  (`login_returns_caller_effective_capabilities`, `auth.rs:4012`); admin gets 11×2=22, a fresh
  trader 9×2=18.
- **GUI capability layer** — `gui/src/lib/capabilityMatrix.ts`:
  - `can(caps, action, asset)` exact-tuple membership (`:167`); `capabilityDenialTitle` (`:202`).
  - `COMPONENT_ACCESS` (`:272`) — a component→capability bridge (each UI component declares its
    Read = `view` and Write = its own actions); the **grant-admin matrix is a projection over
    it**. **The session's new management surfaces are NOT in this table** (only `administration`
    exists as one blanket `administer` row, `:373`).
  - `roleAllows` (`:57`) — **STALE**: returns `action !== "administer"`, i.e. it still hands a
    trader `risk_transfer` by role, diverging from the server's `default_trader_bundle` (which
    also holds back `risk_transfer`). Affordance gating uses the server-projected set so live
    gating is correct, but the **admin editor's role-baseline preview over-grants** (also
    `roleBaselineSummary`, `:509`). Fix in Phase B.
- **GUI auth context** — `gui/src/hooks/useAuth.ts`: `isAdmin = user?.role === "ADMIN"` (`~127`);
  `can` is **permissive when signed out** (`~114`), else `capabilitySetHas(capabilities,…)`.
- **GUI rail / nav gating** — `gui/src/lib/commands.ts`:
  - `RAIL` (`:106`) — each row declares `assets: CapabilityAsset[]` (admin/ops rows = `[]`).
  - `ADMIN_ONLY_WORKSPACES` (`:212`) = `{connections, admin, permissions, pricinggroups, refdata}`.
  - `workspaceAccessible(id, auth)` (`:232`): admin-only ⇒ `auth.isAdmin`; else visible iff
    `auth.can("view", asset)` for **any** served asset. `domainAccessible` (`:299`) mirrors it.
  - **Enforced, not cosmetic**: `AppContext` re-homes to `firstAccessibleWorkspace` on every
    render (`gui/src/app/AppContext.tsx ~526`), so an inaccessible workspace can't be reached by
    URL/deep-link either.
- **Grant UI already exists** — `SetUserCapabilities` / `SetRoleCapabilities`
  (`auth.rs:807` / `:891`, both `require_admin`), driven by `useCapabilityEditor` /
  `useRoleCapabilityEditor` over the `capabilityMatrix` projection. **It iterates
  `CAPABILITY_ACTIONS × CAPABILITY_ASSETS`** (`gui/src/data/contract.ts:2254`), so **new
  actions surface as new matrix rows automatically** once added to `contract.ts`.

**The core smell:** the kernel is granular, but the session's surfaces gate on only three things —
raw **`isAdmin`**, blanket **`Administer`**, or an **overloaded `QuoteRespond·FixedIncome`**
reused as a stand-in for "FI risk manager" (the server code itself flags this: *"A finer
`risk_manage·fixed_income` capability … is a later refinement"*, `auth.rs:1668`). And **rail
visibility has no per-feature dimension at all** — every FI management surface is visible to
anyone holding `view·fixed_income`.

---

## 2. Current-state audit (per session feature)

Gating columns: **Rail visibility** · **View** · **Edit/action (GUI)** · **Server RPC** ·
**Verdict**.

| # | Feature | Rail visibility | View (GUI) | Edit / action (GUI) | Server RPC gate | Verdict |
|---|---|---|---|---|---|---|
| 1 | **Risk Routing** (decision-graph editor) | `riskrouting`, `view·FI` (`commands.ts:145`) | signed-in | `canEdit = can("quote_respond","fixed_income"); readOnly=!canEdit` (`RiskRoutingWorkspace.tsx:63`) | `get/update_risk_routing_graph`: `require_capability(QuoteRespond, FI)` (`auth.rs:1793`,`1811`) | **Overloaded** `quote_respond·FI` = "FI risk mgr" |
| 2 | **Risk Portfolios** (`RiskBookDef` CRUD) | `riskbooks`, `view·FI` (`commands.ts:141`) | signed-in | `readOnly = !isAdmin` (`RiskBooksWorkspace.tsx:119`); intro says *"Trader-defined risk portfolios"* (`~358`) | CRUD `require_admin` (`auth.rs:1709`,`1736`,`1771`); `list_risk_books` `require_capability(QuoteRespond, FI)` (`:1686`) | **Coarse admin** + the "trader-defined" **contradiction** |
| 3 | **Risk Dashboard** (per-portfolio + global roll-up) | `riskdashboard`, `view·FI` (`commands.ts:142`) | signed-in, **no gate** (`RiskDashboardWorkspace.tsx`) | none (read-only) | `list_risk_book_risk`: `require_capability(QuoteRespond, FI)` (`auth.rs:1847`); `RiskService` reads `ReadAny` | **Overloaded**; firm-wide roll-up visible to any FI quoter |
| 4 | **Risk Transfer** (initiate/accept/reject/cancel/list) | **no rail entry yet** (transport+types only; `wsTransport.ts`) | n/a (no UI) | n/a | mutating: `require_capability(RiskTransfer, asset)` (`auth.rs:1903`,`1923`,`1943`,`1963`); `list`: `authenticate` (`:1984`) | **Already granular** ✓ (GUI pending) |
| 5 | **Configurable Notifications** (per-event, sounds, channels) | `SettingsPanel` in TitleBar, ungated | anon-ok | none | **none** — client-local `localStorage` (`settings/settingsSchema.ts`) | No server surface; **client-only per-user** |
| 6 | **Deals blotter / Book / Agg Book** | `book` `view·(all)` (`commands.ts:152`); `aggbook` `view·FI` (`:133`) | `view·asset` | Deals: none; Agg Book **Manage toggle hidden unless `isAdmin`** (`AggregatedBookWorkspace.tsx:229`) | agg-book CRUD `require_admin` (`auth.rs:1428`,`1459`,`1492`) | Blotter fine; **agg-book manage = coarse admin** |
| 7 | **Tiering** (session-pivoted) **+ Pricing Groups** | `tiering` `view·FI` (`commands.ts:134`); `pricinggroups` **ADMIN_ONLY** (`:160`,`:212`) | tiering: `view·FI`; PG: `isAdmin` (whole pane) | Tiering reassign `disabled={!isAdmin}` (`TieringWorkspace.tsx:514`); PG structure `readOnly=!isAdmin`, pipeline `!(isAdmin||canRetune)` (`PricingGroupsWorkspace.tsx`) | PG structure CRUD `require_admin` (`auth.rs:1542`,`1574`,`1607`); **pipeline** `require_capability(QuoteRespond, FI)` (`:1635`); tiering assign = `UpdateConnection` `RequiredAuthority::Admin` (`fix_admin.rs`) | **Coarse admin** on structure/assign (pipeline gate is legit) |
| 8 | **Sim counterparties** | top-bar tool, not a rail row | `can("simulate","fixed_income")` (`Shell.tsx` TitleBar `454`) | disabled + denial tooltip | **none** (client sandbox) | **Already granular** ✓ |
| 9 | **Display fixes** (FI risk-vector/greeks removal, FX scope bar) | pure display | n/a | n/a | n/a | **No gating needed** |

**GUI↔server drift found:** the `riskrouting` rail comment (`commands.ts:137`) claims *"every
underlying RPC is admin-gated (edit affordances gate on `isAdmin`)"*, but the server gates
routing/dashboard/risk-book-list on `QuoteRespond·FI`, and `RiskRoutingWorkspace` actually gates
edit on `quote_respond·FI` — the comment is stale. `PricingGroupsWorkspace` contains a non-admin
`canRetune` pipeline-edit branch that is **unreachable** because the whole workspace is in
`ADMIN_ONLY_WORKSPACES` (dead branch until the pane's visibility moves to a capability — see §4).

---

## 3. Proposed capability model (minimal new set)

Principle: **reuse `Action × AssetClass`**; add a **new `Action` variant only where a real
management distinction is missing**. Because the wire is label-strings and the grant matrix
enumerates `Action::ALL`, each new variant flows automatically through bundles, overlay
parsing, login projection, wire, and the admin matrix.

### 3.1 Recommended — three new actions

| New `Action` | Meaning (what it authorizes) | Replaces today's | Why a distinct capability |
|---|---|---|---|
| **`RiskManage`** | Manage the **risk-portfolio tree** (`RiskBookDef` CRUD), the **risk-routing decision graph**, and **view the firm-wide routed-risk roll-up** (dashboard). | `isAdmin` (book CRUD) + overloaded `QuoteRespond·FI` (routing, dashboard, book list) | A firm **risk-control** function distinct from *publishing a quote* and from *super-admin*. Exactly the `risk_manage·fixed_income` the server code names as the intended refinement (`auth.rs:1668`,`1837`). Fixes the "trader-defined portfolios" contradiction: grantable to a desk/risk lead **without** full `Administer`. |
| **`ManagePricing`** | **Pricing-group structure/membership** CRUD and **session-pivoted tiering assignment** (which group is bound to a FIX session). | `isAdmin`/`Administer` (PG CRUD; tiering assign via `UpdateConnection`) | FI **client-pricing desk** control, separable from user/desk/permission admin. (The per-group **pipeline retune** stays on `QuoteRespond·FI` — that is a *legitimate* quoting-trader knob, not a smell.) |
| **`ManageLiquidity`** | **FIX connection** admin (create/update/delete/enable) and **aggregated-book** config. | `RequiredAuthority::Admin` (connections) + `require_admin` (agg-book CRUD) | Inbound **liquidity / venue ops**, distinct from identity admin. Optional (see §3.3): if not adopted, connections + agg-book stay under `Administer`. |

Kept as-is: **`Administer`** narrows to its true meaning — **users · desks · permissions ·
reference-data** (the super-admin seat, and the *only* thing that can grant capabilities).
**`RiskTransfer`** (already granular, gates the transfer RPCs). **`Simulate`** (already granular).
All trading actions (View/Price/Quote/RFQ/IOI/Stream/Execute/Book) unchanged.

Asset scoping: `RiskManage`, `ManagePricing`, `ManageLiquidity` are exercised on
**`FixedIncome`** today (all the surfaces are FI). Keeping them `Action × AssetClass` means an FX
risk-manager / FX liquidity seat is expressible later with **zero** new type work — grant the
same action on `FxOptions`.

### 3.2 The view-only-risk vs act-on-risk split (the user's explicit ask)

Three orthogonal tiers over risk, all representable in the kernel:

- **See** firm risk → `RiskManage·FI` view path (dashboard roll-up). *Optional finer split:* add
  **`RiskView`** for a read-only risk/compliance seat that can see the firm roll-up but cannot
  edit routing/portfolios (then dashboard read = `RiskView·FI`, routing/portfolio edit =
  `RiskManage·FI`, and `RiskManage` implies `RiskView` only by also granting it). Adopt only if a
  read-only-risk seat is a real role; otherwise `RiskManage` covers both and is simpler.
- **Configure** risk (routing rules, portfolio buckets/limits) → `RiskManage·FI`.
- **Move** existing risk across the desk boundary → `RiskTransfer·asset` (unchanged).

### 3.3 Optional finer splits (only if a firm actually wants them)

- **`ManageRouting` vs `ManageRiskPortfolios`** — split `RiskManage` if routing (a firm-wide
  control graph) must be held tighter than portfolio-bucket editing. Criterion: adopt only if
  those two authorities go to *different* people; otherwise one `RiskManage` avoids matrix bloat.
- **`ManageLiquidity`** itself is optional (§3.1) — fold into `Administer` if venue ops is always
  a super-admin task at this firm.

Recommendation: ship **`RiskManage` + `ManagePricing`** first (they map to the rails the user is
actually hitting), add **`ManageLiquidity`** in the same phase if venue-ops separation is wanted,
and defer the §3.2/§3.3 finer splits until a role demands them.

---

## 4. Per-feature target gating

`viewCap` = the capability that makes the **rail entry appear**; `editCap` = the capability the
mutating affordance + server RPC demand. Admin (`grant_all`) auto-holds every `*Manage*` cap, so
**admins see and edit everything with zero migration**.

| Feature | Rail entry | Target **rail-visibility** cap | Target **view** cap | Target **edit/action** cap | Server RPC → `RequiredAuthority` / `require_*` |
|---|---|---|---|---|---|
| Risk Routing | `riskrouting` | `RiskManage·FI` | `RiskManage·FI` | `RiskManage·FI` | `get/update_risk_routing_graph` → `Capability(RiskManage, FI)` (was `QuoteRespond·FI`) |
| Risk Portfolios | `riskbooks` | `RiskManage·FI` | `RiskManage·FI` | `RiskManage·FI` | `create/update/delete_risk_book` → `Capability(RiskManage, FI)` (was `require_admin`); `list_risk_books` → same (was `QuoteRespond·FI`) |
| Risk Dashboard | `riskdashboard` | `RiskManage·FI` (firm roll-up) | `RiskManage·FI` | n/a (read-only) | `list_risk_book_risk` → `Capability(RiskManage, FI)` (was `QuoteRespond·FI`) |
| Risk Transfer | **new** `risktransfer` | `RiskTransfer·(FI∨FX)` | same | `RiskTransfer·asset` | already `require_capability(RiskTransfer, asset)` — **no change** |
| Notifications | (settings panel) | ungated (per-user client pref) | ungated | ungated | **none** — leave client-local; **no server gating needed** |
| Deals blotter / Book | `book` | `view·asset` (unchanged) | `view·asset` | n/a | unchanged (reads) |
| Agg Book | `aggbook` | `view·FI` (View mode is a trader read) | `view·FI` | `ManageLiquidity·FI` (Manage toggle) | `create/update/delete_aggregated_book` → `Capability(ManageLiquidity, FI)` (was `require_admin`) |
| Tiering (assign) | `tiering` | `ManagePricing·FI` | `view·FI` (read roster) | `ManagePricing·FI` | tiering assign rides `UpdateConnection` → `Capability(ManageLiquidity, *)` **and** the PG binding is `ManagePricing·FI`; recommend a dedicated `AssignPricingGroup` path gated `ManagePricing·FI` rather than overloading `UpdateConnection` |
| Pricing Groups | `pricinggroups` (move OUT of `ADMIN_ONLY`) | `ManagePricing·FI` | `ManagePricing·FI` | structure `ManagePricing·FI`; pipeline retune `QuoteRespond·FI` (keep) | `create/update/delete_pricing_group` → `Capability(ManagePricing, FI)` (was `require_admin`); `update_pricing_group_pipeline` → **keep** `QuoteRespond·FI`; `list_pricing_groups` → keep `authenticate` |
| Connections | `connections` (out of `ADMIN_ONLY` if adopting `ManageLiquidity`) | `ManageLiquidity·*` (else `Administer`) | same | same | `Create/Update/Delete/SetEnabled Connection` → `Capability(ManageLiquidity, asset)` (was `Admin`) |
| Sim counterparties | (top-bar) | `Simulate·FI` (unchanged) | — | `Simulate·FI` | none — **no change** |
| Users / Desks / Permissions / Ref-data | `admin`,`permissions`,`refdata` | `Administer` | `Administer` | `Administer` | unchanged (`require_admin`); **SetUser/RoleCapabilities stay `require_admin`** |

### 4.1 Rail-visibility rule change (the "control who sees it" ask)

Today `workspaceAccessible` knows only *admin-only ⇒ isAdmin* vs *trading ⇒ view·asset*. Give
each `RAIL` row an **optional `viewCap: {action, asset}`** (or a small predicate) and rewrite:

```
workspaceAccessible(id, auth):
  cap = RAIL[id].viewCap
  if cap:  return auth.can(cap.action, cap.asset)   // NEW: per-feature visibility
  if ADMIN_ONLY_WORKSPACES.has(id):  return auth.isAdmin
  return workspaceAssets(id).some(a => auth.can("view", a))
```

Assign `viewCap`: `riskrouting/riskbooks/riskdashboard → RiskManage·FI`;
`tiering/pricinggroups → ManagePricing·FI`; `connections → ManageLiquidity·*` (or leave
`ADMIN_ONLY`); `aggbook` keeps `view·FI` (its View mode is a legitimate trader read; only the
Manage panel needs `ManageLiquidity·FI`). Because `can` is permissive when signed-out and admin
holds `grant_all`, signed-out discovery and admin visibility are unchanged. The `AppContext`
re-home effect already consumes `workspaceAccessible`, so deep-links are covered for free.

### 4.2 GUI affordance gates (replace `isAdmin`)

Swap the in-pane `isAdmin` checks for `auth.can(newCap,…)`:
`RiskBooksWorkspace` `readOnly = !isAdmin` → `!can("risk_manage","fixed_income")`;
`TieringWorkspace` reassign `disabled={!isAdmin}` → `!can("manage_pricing","fixed_income")`;
`AggregatedBookWorkspace` Manage toggle `isAdmin` → `can("manage_liquidity","fixed_income")`;
`PricingGroupsWorkspace` `readOnlyStructure = !isAdmin` → `!can("manage_pricing","fixed_income")`
(and the pane leaves `ADMIN_ONLY`, making its existing `canRetune` pipeline branch reachable).
`RiskRoutingWorkspace` `canEdit` moves `quote_respond` → `risk_manage`. Keep the **disable +
`capabilityDenialTitle` tooltip** discipline (never silent hide) for *controls*; use rail
**hide** for *whole surfaces* the user has no `viewCap` for.

### 4.3 How an admin grants these

The grant path **already supports arbitrary per-capability grant/deny** — `SetUserCapabilities` /
`SetRoleCapabilities` (`auth.rs:807`/`:891`, `require_admin`) take label lists, and the matrix
UI enumerates `CAPABILITY_ACTIONS × CAPABILITY_ASSETS`. So the admin UI needs only:
1. the new action labels in `contract.ts` `CAPABILITY_ACTIONS` + `ACTION_LABELS` +
   `ACTION_PHRASE` (they then appear as new matrix rows/tooltips automatically);
2. new **`COMPONENT_ACCESS`** rows so the trader-friendly *component* grid can toggle them
   (`riskrouting/riskbooks/riskdashboard → readActions:[view], writeActions:[risk_manage]` on FI;
   `tiering/pricinggroups → [manage_pricing]`; `connections/aggbook-manage → [manage_liquidity]`).
No new grant RPC is required.

---

## 5. Migration & back-compat (guardrail 9 — one current contract)

- **Additive, no version bump.** Capabilities ride the wire as **snake_case labels**
  (`PermissionGrant`, `caps_from_wire`), so new `Action` variants need **no proto enum renumber**
  and **no N/N-1 negotiation**. `Action::ALL` grows (11→13/14) → update the login-projection
  count assertions (`auth.rs:4012`) and exhaustiveness tests.
- **Existing `identity.json` overlays unaffected** — a new label is simply absent (ungranted)
  until an admin grants it; deny-by-default keeps them closed.
- **Default bundles:**
  - **Admin** = `grant_all` — unchanged, auto-holds every new manage cap (zero admin disruption).
  - **`default_trader_bundle`** (`identity.rs:104`) — add `RiskManage`, `ManagePricing`,
    `ManageLiquidity` to the **held-back** set alongside `Administer`/`RiskTransfer`. A default
    trader then keeps all trading caps but **loses the Risk Routing / Risk Portfolios / Risk
    Dashboard / Tiering rails** (they now require an explicit grant). **This is the intended
    tightening**, but it is a behavioral change: during rollout, grant `RiskManage` /
    `ManagePricing` to the specific users who legitimately manage those surfaces (per-user
    overlay or an editable role bundle). Recommend seeding a **"FI Risk Manager"** and **"FI
    Pricing Desk"** role bundle (§3.3 of the parent requirement makes role bundles editable).
- **Fix the stale GUI role preview** — `capabilityMatrix.roleAllows` / `roleBaselineSummary` must
  mirror the real held-back set (Administer + RiskTransfer + the new manage caps), else the admin
  matrix keeps over-representing a trader's baseline.

---

## 6. Implementation plan (server first, GUI second)

**Phase A — server entitlements + RPC gates (lands first, GUI-independent).**
1. `crates/celnet-entitlements/src/capability.rs` — add `RiskManage`, `ManagePricing`,
   `ManageLiquidity` to `Action` (+ `label`/`from_label` + `Action::ALL` + tests).
2. `config/identity.rs::default_trader_bundle` — add the three to the held-back set; seed the
   optional FI-Risk-Manager / FI-Pricing-Desk role bundles.
3. Re-gate handlers in `services/auth.rs`: `get/update_risk_routing_graph` (`:1793`,`:1811`),
   `list/create/update/delete_risk_book` (`:1686`,`:1709`,`:1736`,`:1771`),
   `list_risk_book_risk` (`:1847`) → `RiskManage·FI`; `create/update/delete_pricing_group`
   (`:1542`,`:1574`,`:1607`) → `ManagePricing·FI` (keep `update_pricing_group_pipeline`
   `QuoteRespond·FI`); `create/update/delete_aggregated_book` (`:1428`,`:1459`,`:1492`) →
   `ManageLiquidity·FI`. `services/fix_admin.rs` connection RPCs → `Capability(ManageLiquidity,…)`
   (or leave `Admin`); add an `AssignPricingGroup`-shaped path gated `ManagePricing·FI` rather
   than overloading `UpdateConnection` for tiering assignment.
4. Update the login-projection count + boundary tests; `just t1` on
   `celnet-entitlements`/`celnet-server`.

**Phase B — GUI rail + affordance gates.**
1. `gui/src/data/contract.ts` — add the three labels to `CapabilityAction` + `CAPABILITY_ACTIONS`.
2. `gui/src/lib/capabilityMatrix.ts` — `ACTION_LABELS` + `ACTION_PHRASE` rows; **fix `roleAllows`
   / `roleBaselineSummary`** to the true held-back set; add `COMPONENT_ACCESS` rows (§4.3).
3. `gui/src/lib/commands.ts` — add `viewCap` to `RAIL`, rewrite `workspaceAccessible` (§4.1),
   move `pricinggroups` (and optionally `connections`) out of `ADMIN_ONLY_WORKSPACES`.
4. Swap in-pane `isAdmin` → `can(newCap,…)` in `RiskBooksWorkspace`, `TieringWorkspace`,
   `AggregatedBookWorkspace`, `PricingGroupsWorkspace`, `RiskRoutingWorkspace` (§4.2); fix the
   stale `riskrouting` rail comment. `npm run build` + vitest.

**Phase C — Risk Transfer GUI (phase-3, in progress by another session).**
- New `risktransfer` rail row with `viewCap = RiskTransfer` (or `View` + inbox visibility);
  actions already server-gated on `RiskTransfer·asset` — GUI gates the initiate/accept/reject/
  cancel affordances on `can("risk_transfer", asset)`.

---

## 7. Non-goals / notes

- **Notifications and the Sim sandbox stay client-only / already-granular** — no server gating is
  added; notifications are a per-user localStorage preference, `Simulate·FI` already gates the tool.
- Display-only fixes (FI risk-vector/greeks removal, FX scope bar) need **no** gating.
- No new authentication, no API versioning, no read-side risk `Principal` change — this is purely
  the *may-act / may-see-the-surface* layer, orthogonal to the risk-cube read predicate.
