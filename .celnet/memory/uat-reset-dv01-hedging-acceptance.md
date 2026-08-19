---
name: uat-reset-dv01-hedging-acceptance
description: DONE 2026-08-14 — UAT reset + DV01 hedging/acceptance reconfigured via the WS admin API; a real DV01 breach fired and the residual fell to target. Includes the admin-client recipe.
metadata: 
  node_type: memory
  type: project
  originSessionId: 0472eb2d-0caa-4c06-9a09-ecc39b4681c0
  modified: 2026-08-14T14:18:13.936Z
---

All four operator-approved steps completed on UAT (34.26.86.222) 2026-08-14.

**1. Deploy** — `19a5d3fe` is live (`/opt/celnet/releases/19a5d3fe-20260814T121339Z`), server + lp-sim + 3 fix-sims running under `/opt/celnet/shared/bin/celnetctl`.

**2. Wipe** — trading state is **in-memory only**: `IdentityStore` (`crates/celnet-server/src/config/identity.rs`) persists CONFIG ONLY (no deals/positions/blotters), so a `celnetctl stop/start` purges trading state. The 9 config graphs were removed from `identity.json` while stopped; every field is `#[serde(default)]`, so deletion yields pristine defaults. Backup: `/home/celnet/PRE-WIPE-BACKUP-20260814T140414Z` (+ the earlier `PRE-RESET-BACKUP-20260814T121617`). `aggregated_books` (ust + LP-SIM-01..04) and `pricing_groups` (gold) were then RESTORED verbatim from that backup — known-good plumbing, not rebuilt.

**A pristine store is NOT empty — the server SEEDS it:** risk book `warehouse`, routing → warehouse, firm hedge policy `breached=="false" ? Warehouse : SubmitMarketOrder`, accept-all acceptance, and a DV01 threshold on `warehouse` with cap 1e6 (unreachable ⇒ looks like "hedging is off" when it is merely un-tuned).

**3. Reconfigure — driven over the WS admin API, not by hand-editing JSON.** Recipe (reusable): SSH-tunnel `-L 50061:127.0.0.1:50061`, speak the GUI's own envelope `{type, ...body, session_token, correlation_id}` → reply `{type:<expect>, correlation_id, ...}`; open with `{type:"authenticate", principal:{grant_all:true,grants:[],denies:[]}}`, then `login`. Admin password is on the box at `/home/celnet/.celnet_admin_pw` (`admin@celnet.com`). Enum ordinals come from `crates/celnet-proto/proto/celnet.proto`; the RPC `type` strings and reply names from `gui/src/data/wsTransport.ts`. Scripts kept in the session scratchpad as `celnet-admin.mjs` / `celnet-watch.mjs`.

Installed: books `rates-usd` (DV01 cap 5000) + `wash-book` (500) under `warehouse`; routing notional ≤250k → wash, else rates-usd; DV01 threshold amber 60% / red 80% / hedge back to 50%; a 6-row futures vehicle registry (ZT/ZF/ZN/TN/ZB/UB) keyed on the PRODUCT symbol carrying the registry's real `dv01_per_contract_at_notional_yield`; a book-scoped policy `breached=="false" → WAREHOUSE, else SUBMIT_MARKET_ORDER(overflow) into future ZF`; acceptance rejecting >250m notional and quotes >5s old, else Accept.

**ORDERING CONSTRAINT (the server enforces it):** `set_hedge_config` (the vehicle registry) must be written BEFORE a policy graph that names a vehicle — otherwise `update_hedge_policy_graph` fails `InvalidArgument: hedge vehicle "ZF" is not in the vehicle registry, so its DV01 per unit is unknown and the hedge could not be sized`. Good guardrail, easy to trip.

**4. Real breach VERIFIED** (`list_hedge_provenance`, HDG-1): `net_risk -4000` vs `threshold 5000` ⇒ `utilization 0.8`, `band "red"`, `policy_path [0,2]` (condition → SubmitMarketOrder). `internal_crossed 2500` + `external_hedged 1500`, `residual 0`, `lp_won "COMPOSITE"`. Book DV01 then sat at **-2500 = exactly target_fraction 0.5 × cap** — the residual genuinely fell to the band target.

**CAVEAT — that hedge used the SEEDED firm policy (SelfInstrument), not the ZF-future book policy.** The provenance shows `vehicle_kind: 0`, `vehicle_instrument: ""` because it fired between the threshold write and the vehicle/policy write (the first script run aborted on the ordering constraint above). The ZF front-month auto-roll path is therefore configured but **not yet proven end-to-end**; the next breach on `rates-usd` exercises it. Re-check `list_hedge_provenance` for `vehicle_kind:3` / `vehicle_instrument:"ZF"` and a `ZFU26`-style contract id.

Related: [[open-items-dropdowns-esp-tag-futures-roll]], [[bond-hedge-books-no-offsetting-leg]], [[hedging-execution-and-buckets-shipped]], [[uat-login-identity-store-and-reset]].
