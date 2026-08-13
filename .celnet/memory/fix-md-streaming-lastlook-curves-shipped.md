---
name: fix-md-streaming-lastlook-curves-shipped
description: FI streaming venue is now FIX Market Data (35=V/W/D) off the agg book; configurable Sync/Async last-look on the pricing group; multi-curve registry. Live UAT 4375541 (2026-08-09).
metadata: 
  node_type: memory
  type: project
  originSessionId: b69ceb7d-66d6-4bc7-ac17-96e24adc4293
---

Big batch shipped + deployed to UAT (release **`4375541c`**, host `celnet-app-3` = external `136.107.165.2` / internal `8.234.191.9`; domain `celnetapp.uat.celnet.uk`). Key durable facts + gotchas:

**FIX FI streaming venue is now real Market Data, not quote-streaming.** `AcceptorKind::FixedIncomeStream` (`CELER_RATES_STREAM`, port 56003) speaks `35=V` MarketDataRequest (subscribe) → `35=W` MarketDataSnapshotFullRefresh → `35=D` NewOrderSingle → `35=8` ExecutionReport. Priced off the aggregated-book composite + pricing-group tiering + the [[fi-pricing-groups-shipped]] pricing-source mode. The RFQ venue (`CELER_RATES`, 56002) is UNCHANGED on `35=R`/`35=S`. **Deploy gotcha (fixed in `deploy/start-fix-sim.sh`):** the streaming sim leg MUST run `--asset esp` (Market Data), NOT `--intent rfs` (old quote dialect the MD venue ignores → BrokenPipe); one client per CompID/port; the ESP `--seed` must be decimal not hex; sim gRPC login reads the pw file `/home/celnet/.lpsim_pw`.

**MD instruments are liftable via a per-symbol token ledger.** `emit_md_snapshot` mints a liftable token per streamed symbol/side using `mint_two_way_no_clear` + `retire` — NOT `mint_two_way` (whose `clear_live()` wipes the whole ledger; that's for the single-quote RFQ/WS click-to-trade path only). `on_new_order` resolves the symbol's token when there's no QuoteID. Without this, ≥2 concurrent MD subscriptions reject all-but-last as "unknown or forged quote".

**Configurable dealer last-look on the pricing group** (per FIX session), wire tags on `PricingGroupDesc`/`Spec`: `last_look_mode`=13 (enum `Sync`=0/`Async`=1), `last_look_tolerance_bps`=14 (opt double), `async_giveback_pct`=15 (opt double). Defaults Sync / 1.0bp / 50%. Fill = `q + f·(c−q)` where on the favorable side f=giveback% under Async else 0; adverse-within-tol fills @ q; adverse-beyond-tol rejects (token preserved). Fixed the reject churn on fast-moving bonds. GUI selector in Pricing Groups editor (`manage_pricing`).

**Multi-curve registry** ("Market Data" surface renamed → **Curves** under the FI domain only; FX keeps vol-surface label via `railRowPresentation(row, domain)`). New `CurveDefinition` (curve_id slug + display_name + index_label/day_count/calendar + `interpolation` enum 0=log-linear-df/1=monotone-convex-forward + `pillars` CurveSet + `primary` bool). 4 RPCs `List/Create/Update/DeleteCurveDefinition` (WS verbs `*_curve_definition`; write needs `refdata`×FI cap). Seeded default `usd-sofr` primary is byte-identical to the old hardcoded curve so FIX pricing is unchanged; bare-currency resolution = currency→primary curve id. `GetCurve` gained optional `curve_id`; `MarkCurve` unchanged (Excel-compatible). Also wired the live curve editor into FIX auto-quotes (`SurfaceBook::live_curve("USD")`), so pillar edits move OIS quotes.

**Also in this batch:** FIX Session Monitor search bar (free-text / `tag=value` / MsgType / direction / time, 5000-frame buffer); reliable deploy auto-refresh (version.json buildTime poll + cache-clear + one-shot reload — [[gui-release-auto-reload]] superseded); risk dashboard focal roster + sort + By-product breakdown (decodes the RatesInstrument oneof arm → productKind OIS/IRS/FRA/BOND); hedge-by-counterparty (`HedgeField::Counterparty` wire tag 18) + right-click "Change hedging strategy" deal→hedge-rule seed; dark-theme readability lift; status-ribbon de-jump. Every-feature-needs-in-app-help recorded → [[feature-needs-in-app-help]].

Still local-only unless the user OKs a push to `origin`. Deploy = `deploy/celnet-deploy.sh -t uat release`. Detail → [[risk-routing-build-state]], [[fi-agg-book-rfq-gap-and-tiering-backlog]].
