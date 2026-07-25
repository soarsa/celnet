---
name: fix-sim-and-gui-keepalive
description: GUI reconnect-flicker root cause+fix (client WS keepalive) and how the FIX quote simulator is deployed/logged on the server; credit designs written
metadata: 
  node_type: memory
  type: project
  originSessionId: 5ff9525a-81fa-41e7-a4f8-745f35d76a80
---

Delivered 2026-07-01 on branch `feature/fi-reference-data`.

**GUI reconnect "connection lost" flash — the REAL cause (2026-07-01):** NOT idle-reaping and NOT
an LB. The UAT `celnet-server` was FLAPPING (log: 27 `ready` / 24 `draining`, ended wedged STOPPED
on `EADDRINUSE`). Every drain dropped all WebSockets → the flash; every restart → "goes back".
**Root cause = self-inflicted:** the `CELNET_FIX_ADDR` I added to `celnet_env` bound a LEGACY FIX
acceptor that COLLIDES with the acceptors the server already binds from the MANAGED registry
(`/home/celnet/fix-connections.json`: "celer" FX/options @127.0.0.1:51001, FI quote venue @9100).
With `CELNET_FIX_ADDR` set the process aborts at boot; unset → boots clean (verified foreground on
the box). **Fix (commit 74e424d):** removed `CELNET_FIX_ADDR/SENDER/TARGET` from `group_vars/all.yml`
— FIX is configured via the managed registry ONLY; re-enabling the legacy env aborts boot. Live
remediation: commented FIX out of `/opt/celnet/shared/config/celnet.env` + `celnetctl start`
(RUNNING). **The server only drains on SIGINT** (`main.rs` `ctrl_c()`); no cron/systemd supervises
it, so once wedged it stayed down. Diagnose via `ssh celnet@136.115.32.199` (key-based) →
`celnetctl status`, `ss -ltnp | grep -E '50051|50061'`, `tail /opt/celnet/shared/logs/celnet-server.out.log`.

Two supporting client mitigations (still valid, but were NOT the cause): (1) WS keepalive —
`WsConnection` sends `{"type":"heartbeat"}` every 20s (`gui/src/data/wsTransport.ts`); server no-op
in `services/stream.rs`. (2) `OVERLAY_GRACE_MS` widened 1.2s→4s (`useConnectionStatus.ts`) so a
transient reconnect doesn't flash the modal. Both help mask brief drops but don't fix a flapping
server.

**FIX quote simulator deploy:** `deploy/start-fix-sim.sh` "did nothing" because `cargo --quiet` hid
build+connect errors and foreground mode never logged to a file. Fixed the launcher: prebuild
visibly, drop `--quiet`, ALWAYS tee every RFQ sent + Quote/fill received to
`deploy/fix-sim-run/log/fix-sim.log` (all modes), loud preflight, clean daemon re-exec + pidfile.
It now dials the EXISTING managed "celer" acceptor: **127.0.0.1:51001, sim SENDER=CELNET-CPTY,
TARGET=CELNET** (sim SENDER == venue target_comp_id; sim TARGET == venue sender_comp_id; FI venue is
9100). Run: `FIXSIM_DAEMON=1 deploy/start-fix-sim.sh` then `tail -f deploy/fix-sim-run/log/fix-sim.log`.
App-level RFQ/Quote logs there; raw FIX WIRE in/out is server-side (FixAdminService / GUI FIX
monitor). Full `fix-sim` bot binary still design-only (`docs/FIX-SIM-DESIGN.md` step 2).

**Asset selector — FI default (commit e29d98b):** the sim drives fixed income OR FX options.
`fix_rfq_client --asset fi|fx` (default **fi**): FI builds an OIS rates RFQ via
`dialect_rates::build_rates_quote_request` (`--curve USD-OIS --tenor 5 --notional 10000000 --side
pay|receive|two-way`), Observe only. `start-fix-sim.sh`/`fixsimctl` derive per-asset acceptor:
**fi → 127.0.0.1:56002 (sim SENDER=CELER_RATES, TARGET=CELNET)**, fx → :56001 (CELER_FXO). A
**`fixed_income_quote` acceptor `celer-rates-celnet` @56002** was added to the box registry
(`/home/celnet/fix-connections.json`, enabled, sender_comp_id=CELNET/target_comp_id=CELER_RATES,
desk=celnet) + server restarted (both 56001/56002 listening). Registry acceptors bind at boot and a
bad bind is SKIPPED (never aborts boot — unlike the legacy CELNET_FIX_ADDR). Verified end-to-end: FI
RFQ → **Quote 0.04045/0.04055 (~4.05% 5y OIS)**. **The box still runs the OLD client (no `--asset`)
until a REDEPLOY ships the new fix-rfq-client + scripts** — redeploy, then `fixsimctl start` runs FI
by default. The manual registry add is runtime state (persists across deploys); the canonical way to
add an acceptor is the GUI FIX admin.

**Shipped with the release (commit 7086de1):** `celnet_fixsim_enabled: true` (group_vars) makes the
deploy build `celnet-fix --example fix_rfq_client` (release toolchain at `/opt/celnet/.cargo`), ship
it as `<release>/bin/fix-rfq-client`, install `start-fix-sim.sh` to `shared/bin`, and template a
`fixsimctl` control script (start|once|stop|status|tail) — mirrors `celnetctl`. The celnet user has
NO cargo at runtime, so the launcher uses the prebuilt binary via `FIXSIM_RFQ_BIN` (skips cargo).
**On the box after a redeploy:** `fixsimctl start` then `fixsimctl tail` (log
`/opt/celnet/shared/logs/fix-sim.out.log`). Verified the RFQ→Quote→FILL flow against the live UAT
acceptor via SSH tunnel (bid 0.0672/offer 0.0686, FILLED). The FIX acceptor is loopback-only
(127.0.0.1:56001), so off-box clients need an SSH tunnel; on-box `fixsimctl` needs none.

**SecurityListRequest(35=x)/SecurityList(35=y)** in `celnet-fix` — DONE (commit 2750edf, pushed).
`MsgType` x/y + tags 320/559/560/393/146/893; `dialect_fx::{SecurityDef, build_security_list_request,
build_security_list}`; `QuoteSource` gained `securities()->Vec<SecurityDef>`; `on_security_list_request`
emits one SecurityList via the same send/seq path as `on_quote_request`; loopback round-trip test
(`tests/security_list.rs`) over a real socket. 47 lib + 1 green, clippy -D clean, rfq+server compile.
**HONEST GAP:** this is the celnet-fix LIBRARY acceptor (loopback test + example client). The
PRODUCTION server FIX edge is a SEPARATE path (`celnet-server/services/fix.rs` + `fix_registry.rs`,
its own `FixContext`/`FixAcceptor`) and does NOT yet answer SecurityList — wiring its universe
projection from pairs/registry is the next step. No server-side `celnet_fix::acceptor::QuoteSource`
impl exists (server quoting runs through `celnet_rfq::QuoteSource`, a different trait — not fabricated).

**FIX RFQs now land in the desk inbox (commit `aa88b22`, branch feature/fi-reference-data):**
ROOT CAUSE the inbox was empty on login — a dedicated FI acceptor auto-quoted every
inbound RFQ INSIDE the pricing edge (`services/fix.rs::on_quote_request`→`rates_line`,
instant two-way `35=S`) and NEVER inserted into `DeskRequestStore`; the human desk inbox
(`RfqDeskEdge`/`DeskGateway`) is a SEPARATE path the server doesn't bind at boot. Fix:
`FixContext` gained `desk_edge`+`desk`+`auto_quote` (via non-breaking `with_desk_routing`
builder; threaded from `fix_registry::load(desk_edge=Some(rfq_desk_edge))` in lib.rs). New
`RfqDeskEdge::ingest_fix_rfq` (auth-free — FIX authed at CompID) inserts + publishes a
notification. A `RatesAutoQuotePolicy{max_notional:25mm, tenors:{1,2,3,5,7,10}}` decides
per-RFQ: admitted⇒auto-quote+store QUOTED history; declined (>25mm or off-the-run e.g. 30y)
⇒store PENDING, NO auto `35=S` (human prices it; taker runs Observe). FX + legacy
content-OIS paths byte-identical (only FixedIncomeQuote/Stream intercepted); Quote mint/send
extracted to shared `emit_two_way_quote`. **GUI needed ZERO change** —
`QuotingWorkspace.tsx` already loads on login AND refreshes on every push notification
(line 91), no state filter, renders all 4 states. Sim (`start-fix-sim.sh`) now rotates FI
tenor∈{2,5,10,30}/notional∈{10mm,50mm} (`FIXSIM_VARY=1` default) so both branches fire.
**Monitor "no fix message" was just the Inbound filter** — outbound `35=S` IS recorded
(fix.rs:345); click "All". **Desk visibility:** admin@celnet.com = DeskScope::All sees any
desk; a trader sees only their desk — so "map to real rates desk" = set the acceptor's
`desk` (GUI FIX admin) AND the trader's desk to the same id. **NOT YET DEPLOYED** — box runs
old server; redeploy needed for the feature to show. Gated locally: cargo check + clippy -D
+ 26 fix/desk tests + 3 new (ingest_fix_rfq, auto_quote_policy, rates_side) green.

**Client-hang on a routed RFQ FIXED (commit `dfcb1e2`, on main):** a manual (desk-routed)
RFQ gets no auto `35=S`, so `Initiator::request_and_lift` step 3 blocked forever on
`reader.next_frame()` (only heartbeats arrive; they don't break the loop) → the sim STOPPED
sending after the first manual request. Fix: bound step 3 with a deadline
(`Initiator::quote_timeout`, default `DEFAULT_QUOTE_TIMEOUT`=5s, `.with_quote_timeout()` to
override) via `tokio::time::timeout_at`; on no reply it returns no-quote, the FI client prints
"submitted to the rates desk", the loop continues. Also: the server auto branch
(`on_rates_quote_request`) now routes an admitted-but-UNPRICEABLE RFQ to the desk (PENDING)
instead of dropping (`rates_line(...).ok()` → `match`), so "a tenor that doesn't exist" always
becomes a desk ticket. NB `par_rate_for` interpolates ANY tenor≥1, so the manual trigger is the
auto-quote tenor SET {1,2,3,5,7,10}, NOT a pricing failure — a non-standard tenor (e.g. 15y) is
"the tenor that doesn't exist" for the auto venue. Sim now deterministic: on-the-run tenors auto,
every Nth (`FIXSIM_MANUAL_EVERY`=4) a `FIXSIM_MANUAL_TENOR`=15y manual, fixed small notional so
tenor is the sole trigger. Still needs UAT redeploy to take effect on the box.

**Persistent session + desktop notifs + quotes blotter DEPLOYED to UAT (release `ef54f42`, 2026-07-04):**
- **Persistent FIX session (#1, commit `dcaeb7e`):** the sim logged on/off PER RFQ (bash spawned a
  one-shot client per request). Fixed: `Initiator::open` → `InitiatorSession::request` (extracted
  `do_logon`/`send_request`/`collect`; `request_and_lift` unchanged) logs on ONCE and streams many
  RFQs over one session. Client got `--repeat N`(0=forever)/`--interval S`; FI loop rotates on-the-run
  tenors (auto-quoted) + injects `--manual-tenor`(15y) every `--manual-every`(4)th → desk. Logs
  `[i] Ny OIS auto-quoted` / `submitted to the rates desk` / `auto-accepted & FILLED @px` (auto-accept
  logging). `start-fix-sim.sh` now runs ONE persistent client under a supervisor. Verified locally
  ([0][1][2] auto + [3]15y desk, no re-logon) AND on box (log: "streaming persistent"→connect→[0]).
  **NB cadence = `FIXSIM_PERIOD` 180s** (hardcoded in fixsimctl.sh.j2 from `celnet_fixsim_period`); lower
  it there + redeploy for a faster demo.
- **Desktop "growl" notifs + Quotes blotter (GUI, commit `ef54f42`):** `useDesktopNotifications` hook
  (Web Notifications API) fires a native OS notification for a desk notification ONLY when the tab is
  hidden/unfocused (no double-notify), permission+mute toggle in NotificationCenter (localStorage).
  New `QuotesBlotterWorkspace` = a **"Quotes" lens in BookWorkspace** (next to Deals; the fe-fi-migration
  folded standalone Deals into Book lenses) listing every QUOTED/ACCEPTED quote, load-on-mount + stream
  refresh, desk-scoped. Build clean, 1013 tests, axe 0.
- **Deploy:** `cd deploy && ./celnet-deploy.sh -t uat release` — NO vault/become password needed for the
  `release` play (runs as celnet); the box is key-SSH reachable. Restart sim: on box
  `cd /opt/celnet && ./shared/bin/fixsimctl stop; ./shared/bin/fixsimctl start`.

**Credit designs DONE (docs):** `docs/FI-CREDIT-ENGINE-DESIGN.md` + `docs/adr/ADR-0013-credit-
pricing-leaf.md` — `celnet-credit` as a pure reduced-form hazard-rate leaf (SurvivalCurve, CDS
par-spread/MtM/upfront, credit-risky bond, CR01+JTD) consuming celnet-rates + [[fi-platform-branch-program]]'s
celnet-bond; CR01/JTD as new risk-cube dimensions. Design only — no credit code yet.
