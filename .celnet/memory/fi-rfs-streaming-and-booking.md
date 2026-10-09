---
name: fi-rfs-streaming-and-booking
description: True RFS continuous streaming + auto-quote lifts book deals + livelier mixed sim — shipped & live on UAT (commit 2529408)
metadata: 
  node_type: memory
  type: project
  originSessionId: 5ff9525a-81fa-41e7-a4f8-745f35d76a80
---

Shipped 2026-07-05 (commit `2529408` on main, released `2529408-20260705T154904Z` to UAT). Four FI features, all verified live on the box:

1. **True RFS continuous streaming** — `celnet-server/src/services/fix.rs`: a `fixed_income_stream` venue now serves real RFS. `FixSession::serve` became a `tokio::select!` over the frame reader + a 5s ticker (`RFS_STREAM_INTERVAL`); on a Subscribe it registers an `RfsSubscription` (keyed by `QuoteReqID`), and `tick_rfs_stream` re-prices each off the P0 curve with a small deterministic triangle-wave (`rfs_streamed_rate`, ±2bp) and pushes fresh `Quote(S)` until Unsubscribe/close. `next_frame` is cancellation-safe so the ticker never drops bytes.
2. **Auto-quote lift books a deal** — the desk request id an auto-quote/stream created rides the live `FixQuote` (`rates_request_id`); on a `NewOrderSingle` fill the venue calls new `RfqDeskEdge::book_fix_lift` (mirrors `accept_desk_quote`'s booking body minus RPC auth) → books position + `Deal`, moves the request QUOTED→ACCEPTED, closes any matching RFS stream.
3. **Client** — `celnet-fix/initiator.rs`: `InitiatorSession::stream` (subscribe + hold + optional lift) and `set_policy`. Example `fix_rfq_client.rs`: `--intent rfq|rfs`, `--lift-every N`, `--stream-hold`.
4. **Sim/deploy** — `start-fix-sim.sh`: optional preflight-gated RFS leg + lifts every 3rd auto-quote; cadence 180s→20s (also `fixsimctl.sh.j2`).

**Box wiring (the registry is GUI-managed on the box, NOT in the repo):** the FIX acceptor registry lives at `/home/celnet/fix-connections.json` (server CWD = `/home/celnet`); ansible user is `celnet` (owns everything, **no sudo** needed). Added a `fixed_income_stream` acceptor `celnet-rates-stream-celnet` @127.0.0.1:56003 (sender `CELNET`, target `CELNET_RATES_STREAM`, desk `celnet`) directly to that JSON + restarted server. `release` (deploy.yml) ships binaries + restarts server but does NOT re-template `fixsimctl` (that's provision/`full`, which needs `--ask-become-pass`); patched the box's `fixsimctl` in place instead (period 20 + `FIXSIM_STREAM_PORT=56003`). Deploy: `cd deploy && ./celnet-deploy.sh -t uat release`; box ops via `ansible uat -i inventory/hosts.ini -m shell -a '…'`.

**Verified live:** all 3 acceptors bind (56001/56002/56003); RFQ leg `[2] ✓ auto-quote LIFTED & FILLED — deal booked` + `[3] 15y → rates desk`; RFS leg `streamed 4/7/10/13 update(s)`; deals booked climbing.

**Two polish items — FIXED & verified live (commit `004fcfd`, released `004fcfd-20260706T101029Z`):** (a) RFS-leg *self*-lift now FILLS — the server keeps each RFS subscription's immediately-superseded quote liftable for one extra tick (2-deep `prev_quote_id`/`last_quote_id` in `RfsSubscription`/`tick_rfs_stream`), so a lift racing the 5s re-price still fills (`[5] ✓ streamed quote LIFTED & FILLED — deal booked`); (b) the RFS sim client uses a STABLE `QuoteReqID` (`{req_id}-RFS`) across cycles so each re-subscribe REPLACES the one live stream (keyed by QuoteReqID) instead of piling up — stream counts now steady (~4/cycle) not climbing 4/7/10/13.

**UAT host IP changed** 2026-07-06 after a host re-provision: `136.115.32.199` → `136.114.170.5` (updated in `deploy/inventory/hosts.ini`, commit `9bdb19b`). New IP ⇒ unknown SSH host key: run ansible/deploy with `ANSIBLE_HOST_KEY_CHECKING=False`. The registry (`/home/celnet/fix-connections.json` incl. the RFS acceptor) and the in-place `fixsimctl` patch survived the reboot; the sim must be restarted after a reboot (`fixsimctl start`). Related: [[fix-sim-and-gui-keepalive]], [[fi-dealer-quoting-shipped]].
