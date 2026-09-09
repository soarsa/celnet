---
name: uat-sim-all-ois-rfs-esp-wedged
description: "UAT sim blotter was 100% OIS because the RFS+ESP legs spun forever (fixed, a4f85332) AND stream registry-order instruments the agg book cannot price (still open)."
metadata: 
  node_type: memory
  type: project
  originSessionId: fb8b11c8-9538-4fdf-b75a-042dd7481848
  modified: 2026-08-17T13:40:23.916Z
---

Investigated 2026-08-17. The UAT client blotter showed nothing but OIS. Measured, not
assumed — `pricing.2026-08-17.log` by connection:

| connection | events | symbols |
|---|---|---|
| `celer-rates-celnet` (RFQ :56002) | **4662** | `USD-OIS` |
| `celer-rates-stream-celnet` (RFS :56003) | **2** | `ust-2y-note`, `acme-5y-corp` |
| `celer-rates-esp-celnet` (ESP :56004) | **2** | `ust-2y-note`, `acme-5y-corp` |

RFS/ESP stopped dead at 10:57:46 and produced nothing for 2h35m.

**Cause A — spin bug. FIXED in `a4f85332`.** `collect_md_stream`
(`crates/celnet-fix/src/initiator.rs`) picked `lift_at` as its wake instant whenever a lift
was pending, without requiring the instant be in the FUTURE. On a silent venue no snapshot
arrives → `last_symbol` stays `None` → the lift cannot fire → `pending_lift` stays true →
wake pinned to a PAST instant → `timeout_at` returns `Elapsed` instantly → and the
`wake >= deadline` break cannot fire because `lift_at < deadline`. Hot loop forever.
Live proof: `ps` showed both legs at **87% CPU** for 2h35m (the healthy `fi` leg: 0.0%) —
~175% of a 2-vCPU box, a likely contributor to the chronic UAT CPU/memory pressure in
[[uat-oom-root-cause-and-swap-fix]]. Fix = only wake on `lift_at` while `t > now`; the lift
still fires from the `lift_due` check when any snapshot lands, so a slow venue keeps its
lift. `md_stream` had **zero** test coverage — that is how it shipped. Regression added in
`crates/celnet-fix/tests/session_loopback.rs`
(`md_stream_with_a_pending_lift_ends_on_a_silent_venue`); verified it TIMES OUT against the
old code and passes against the new.

**Cause B — wrong instruments. STILL OPEN.** `download_top_bonds` in
`crates/celnet-fix/examples/fix_rfq_client.rs` takes the first N bond-family instruments in
**registry order** — its doc comment claims "the most liquid/relevant set", which is false.
On UAT that yields exactly: `ust-2y-note` + `acme-5y-corp` (both of which the server itself
logs as `TRADEABLE BUT UNQUOTABLE`, `unquotable:2`) then `912797TS6/UA3/UU9/UE5/SW8/UK1` —
**all zero-coupon BILLS**. lp-sim's quotable set is *"141 cash bonds + 6 OIS points + 8
STIR"* with `include_bills=0`, so **none of the 8 streamed names are in it**. The agg-book
legs can never contribute. Planned fix: probe adaptively — download a larger candidate pool,
drop names that publish no snapshot during their hold, re-probe parked ones periodically.
Do NOT hardcode an instrument list (guardrail 2/8).

**Cause C — mix doesn't match the spec.** Operator's intent: RFQ (:56002) = **manual curve
requests only** (today `--manual-every 3`, so 2/3 is auto-quoted OIS); RFS (:56003) =
agg-book bonds via RFQ at **odd sizes** (today `RATES_NOTIONAL_LADDER` is round clips
100k/1m/2m/10m/20m/30m); ESP (:56004) = **agg book only**.

Also unexplained: the GUI FIX admin shows `⚠ no order route` on all three FI connections —
gates whether a lift can book at all.

Useful facts: UAT box `celnet@34.26.86.222`, key-based SSH works. Live sim log is
`/opt/celnet/shared/logs/fix-sim.out.log` (NOT `/home/celnet/fix-sim-run/...`, stale since
Jul 10). Launch env lives in `/opt/celnet/shared/bin/fixsimctl`, whose defaults come from
`deploy/roles/celnet_provision/templates/fixsimctl.sh.j2` + `deploy/group_vars/all.yml`.
Curves on the box are `usd-sofr`/`usd-sor`/`sob`/`usdsob` (primary) — there is **no
`USD-OIS` curve**, yet the RFQ leg requests `--curve USD-OIS` and the server still
auto-quotes it off the primary USD curve, so this is NOT a pricing failure. Aggregated book
`ust` is enabled with members `marketaccess-sim, traderweb-sim, citigroup-sim, jpm-sim,
cme-sim`.
