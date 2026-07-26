---
name: fi-agg-book-rfq-gap-and-tiering-backlog
description: FI tiering + RFQ-against-book backlog COMPLETE + live UAT 621542d. Tiering engine (Flat/InventorySkew/SCALE_SMOOTH) + per-book config + composite apply + FI-tab config screens + per-strategy doc links + inbound RFQ now priced against the admin book. Nothing pending.
metadata: 
  node_type: memory
  type: project
  originSessionId: cf0956fd-d116-49a0-8307-d104e532a50c
---

## COMPLETE + live on UAT `621542d` (2026-07-26). The whole FI outbound tiering + RFQ backlog.
Design/math: `docs/FI-TIERING-RESEARCH.md`. Built engine → server → GUI → RFQ, all gated.

- **`celnet-tiering`** crate (`297781c`): pure `TieringStrategy` + `FlatMarkup`, `InventorySkew`,
  `ScaledSmoothedSpread` (SCALE_SMOOTH — EWMA-smoothed spread from the PDF); `quote()` pipeline;
  anti-cross via `offer−bid=2h` skew-invariance; `SpreadUnit` + DV01 conversion.
- **Server tiering** (`67e4bee`): `TieringConfig` on `AggregatedBookDef` (admin CRUD + validate);
  applied on the composite publish path (`services/aggregation.rs::apply_tiering`) — DV01 from
  `celnet_bond`, inventory seam, per-book EWMA state (strategy stays pure), no-config byte-identical;
  additive proto/WS, byte-identical hand+generated codecs.
- **GUI** (`5dd9cbe`): FI-tab per-book `TieringEditor` (all 3 strategies + guardrails + validation);
  per-strategy "?" doc links → `docs/FI-TIERING-RESEARCH.md` §9. Fixed a pre-existing prod bug
  (step/min mismatch silently blocked ALL agg-book submits).
- **SCALE_SMOOTH** end-to-end (`fde21ae`): crate + proto/WS + apply + GUI + docs + doc links.
- **RFQ priced against the book** (`621542d`, Phase 2b): `QuoteEdge` gains the injected
  `aggregation_hub`; an inbound RFQ whose instrument is in an admin book's scope prices against that
  book's ALREADY-TIERED composite (+ member-LP lines for ranking), NOT the synthetic-demo panel
  (`CELNET_DEMO_LPS`). No-covering-book ⇒ fallback byte-identical. No proto change.

See [[fi-bond-terms-and-govvie-feed-shipped]], [[fi-aggregated-book-shipped]].

## UAT box deploy gotcha (IMPORTANT — this box is undersized)
`/dev/sda1` 9.7G (~7G non-reclaimable OS, celnet has NO sudo) AND only ~3G RAM, no swap. The deploy's
foreground `cargo build --release` OOM/thrashes the box into **UNREACHABLE** mid-build, and `npm ci`/
binary-copy hits **ENOSPC** at 99%. RELIABLE remedy (used for `621542d`):
1. Reclaim disk: `cd deploy && ansible uat -m shell -a` — rm old release dirs EXCEPT `readlink -f
   /opt/celnet/current`, + `/opt/celnet/build/gui/node_modules` + `~/.npm` + `~/.cache`.
2. **Pre-warm the build DETACHED on the box** (survives SSH drops): cargo lives at
   `/opt/celnet/.cargo/bin/cargo` (CARGO_HOME=`/opt/celnet/.cargo`, RUSTUP_HOME=`/opt/celnet/.rustup`
   — NOT `~/.cargo`!). `ansible uat -m shell -a 'cd /opt/celnet/build && setsid sh -c "export
   CARGO_HOME=/opt/celnet/.cargo RUSTUP_HOME=/opt/celnet/.rustup CARGO_BUILD_JOBS=2;
   /opt/celnet/.cargo/bin/cargo build --release -p celnet-server -p celnet-lp-sim && touch
   /tmp/prewarm.done || touch /tmp/prewarm.fail" >/tmp/prewarm.log 2>&1 </dev/null &'` — poll
   `/tmp/prewarm.done`.
3. Then re-run `./deploy/celnet-deploy.sh -t uat release` (its build is now an incremental no-op).
Also set `jobs = 2` in `/opt/celnet/.cargo/config.toml` on the box (done) so any future deploy build
caps parallelism. **The box genuinely needs more RAM + disk + swap.** See [[deploy-ssh-drop-on-silent-build]].
