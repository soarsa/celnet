Wrote `/Users/adrian/code/celeroption/docs/API-CLIENTS.md` — the trader-centric Celnet API + client-SDK design.

Contents:
- **§0 Gap analysis** — why the current `celnet.edge` (single broadcast `RfsServer`, echoed `uint64 request_id`, point-read `SurfaceVol`, single `PriceBarrier`) is not yet trader-shaped, framed as exposure gaps over `celnet-surface`/`celnet-vanilla`/`celnet-exotics`.
- **§1 One contract, two transports** — the five-file `.proto` family (`celnet.instrument`/`quote`/`stream`/`surface`/`position`), no version field, gRPC primary + WS tagged-JSON mirror, and the unified `Instrument` oneof (vanilla | strategy | barrier | digital | touch | tarf | accumulator) with `Quantity`/`Solve`/`FixingSchedule`.
- **§2 Message set** — proto for Price, RFQ lifecycle (`RequestQuote`/`AcceptQuote`/`RejectQuote` with idempotency + `valid_until`/last-look), bidirectional RFS (`Subscribe`/`Modify`/`Unsubscribe`/`Resync` ↔ `Snapshot`/`Delta`/`Heartbeat`/`Lagged`/`StreamEnd`), surface (`GetSmile`/`MarkSurface`/`StreamSurface`/`Scenario` with `ArbStatus` + conventions), and position/`AttributePnl`.
- **§3 `celnet-client` SDK shape** — ergonomic async API with a responsibilities table (SubscriptionId, snapshot+delta merge, heartbeat, reconnect+resync, idempotency, per-subscriber conflation).
- **§4 Twelve numbered real-like trader workflow scenarios** (plus an optional 13th blue-green cutover test), each with actor / steps / asserts, framed as defects-drive-the-proto.
- **§5 Competitor-API comparison table** (SynOption, Fenics, Bloomberg, 360T, Digital Vega) + "how Celnet out-designs them".
- **§6 Build & freeze sequencing** respecting the single-current-contract / re-index discipline.

All identifiers are vendor-neutral and purpose-named (`VanillaInputs`, not `GkInputs`), mirroring the existing `edge.proto` style; competitor specifics, crate names, methods (HRW/disruptor/conflation/HdrHistogram), and ~5-minute RFS window are preserved from the research bundle.