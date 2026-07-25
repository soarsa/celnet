---
name: fi-desk-routing-and-asset-separation
description: "FI quote/deal routing is desk-name based (user.desk_id must match a FIX connection's routing desk or the user gets NOTHING); plus the domain-tab breakout, admin desk-config (UpdateDesk), hard asset separation, FI Streaming tab, exception-only notifications, and 120s mixed sim — all shipped to UAT 2026-07-08."
metadata:
  node_type: memory
  type: project
  originSessionId: 24534380-8fb6-4089-a501-c7168ebd8613
---

Shipped + deployed to UAT 2026-07-08 (release `67d5263`, host 136.114.170.5). Local `main` only (not pushed to origin).

**Desk-based routing (the load-bearing, non-obvious rule).** Inbound FIX RFQ/deal
notifications fan out by DESK NAME only: `NotificationBroker.publish` →
`DeskFilter.allows(desk)`. A user with no `desk_id` (or one that doesn't match the FIX
connection's routing desk) resolves to `DeskScope::Deskless` → EMPTY filter → receives
NOTHING. Capabilities gate *actions*, not *reception*. Root-caused a "FI users get no
quotes" bug to exactly this: all traders were on desk `marex`, but every connection in
`/home/celnet/fix-connections.json` routed to desk `celnet` (zero users). Fixed on UAT by
repointing all 3 connections' `desk` → `marex` (backup kept). See [[fi-rfs-streaming-and-booking]].

**Desk config is now admin-managed (vendor-neutral).** `DeskDesc` = `{ id (stable routing
key, never changes), name (editable label) }`. New `UpdateDesk` RPC (proto + server
`auth.rs` + WS codec byte-identical + `celnet-client`) renames the label only — routing
stays on `id`. `AdminWorkspace` (Administration tab) does inline desk assignment
(optimistic+rollback), desk create/rename/delete, capability matrix (deny-wins). A FIX
connection's routing desk is now an OPTIONAL admin-set desk id (blank = unrouted). Startup
guard in `fix_registry.rs bind_acceptor` emits a LOUD warn when a connection's desk doesn't
resolve to a defined desk (kills the silent-drop class). NOTE: UAT desk still has internal
id `marex`; rename the *display label* in Administration → Desks (routing unaffected).
Supersedes/delivers much of [[user-admin-desk-feature-plan]] and [[permissions-capability-track]].

**Hard vertical asset separation** (supersedes the earlier Model-A cross-asset lens): Market
Data / Risk / Quotes now show ONLY the active domain's asset (no in-screen FX-vs-FI toggle;
lens derived from `activeDomain`). Domain deep-links (`?dom=fixed_income`) seed `activeDomain`
+ the shared-screen lens on FIRST paint (was a post-mount flash bug). Combined-tail Risk lens
removed from per-domain screens.

**New FI → Streaming tab** (`FiStreamingWorkspace`, `assets:["fixed_income"]`): live bond/swap
grid over `app.stream.ratesRows`/`subscribeRates` + right-hand RFS sidebar (request price →
streamed indicative two-way). GAP: rates have no stream-token execute, so Execute books via
`transport.submitDeskRequest` — enabled for OIS, disabled-with-reason for IRS/bond.

**Exception-only notifications.** WS Notification frame now carries `alert_worthy` (bool —
GATE popups on this) + `reason` (`ManualInterventionReason`: 1=UnconfiguredTenor,
2=CreditRiskBreak, 3=UnknownSecurity, 4=PricingFailure); alert kind = `ManualInterventionRequired`
(ordinal 7). Only `alert_worthy` fires desktop (`Notification` API) + growl popout + sound;
auto-priced RFQs land quietly in the blotter. `CreditRiskBreak` reason exists but has no
detector wired yet (no credit path).

**Sim (`deploy/start-fix-sim.sh` + `fix-rfq-client`).** Now every 120s (`FIXSIM_PERIOD=120`),
mixed: ~2/3 auto-priced+lifted→booked, ~1/3 manual-intervention (`--manual-every 3`,
`--manual-tenor 15` unconfigured, `--manual-security XXX-UNKNOWN`). Run daemonized on the box:
`FIXSIM_DAEMON=1 FIXSIM_RUN_DIR=/home/celnet/fix-sim-run FIXSIM_RFQ_BIN=/opt/celnet/current/bin/fix-rfq-client /opt/celnet/shared/bin/start-fix-sim.sh`.

**celnet-aggregation (A+B) built** (`ed7f798`): asset-agnostic `VenueFeed` trait + deterministic
`SimVenue`, `ConsolidatedBook` (BBO/depth/staleness/divergence-exclusion), `RiskPricer`
(directional skew + size tiers). 25 tests. NOT yet wired to server/GUI (lanes C/D/E pending) —
intended to feed the FI Streaming tab / a venue-liquidity aggregation screen later.

**Ops gotchas:** (1) run `celnet-deploy.sh` FROM `deploy/` — cwd drift silently makes
`./celnet-deploy.sh` "No such file" and the deploy no-ops (verify served `version.json`
buildTime changed). (2) `pkill -f start-fix-sim.sh`/`fix-rfq-client` MATCHES YOUR OWN SSH
command line and kills the session (EXIT 255) — kill sim procs by PID excluding `$$`, or match
`"fix-rfq-client --"`. (3) UAT box disk ~90% (9.7G) — prune old `/opt/celnet/releases/*` before
a release build. (4) small box OOMs npm ci — deploy is idempotent, just re-run.
