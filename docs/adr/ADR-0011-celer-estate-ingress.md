# ADR-0011 — Celer-estate ingress + external vendor FX-options market-data ingestion

- **Status:** Accepted as the **integration architecture** (2026-06-28). The in-repo seams
  (vendor-feed adapter, resilient subscriber, deployment-mode gating, egress governor) are
  **built and gated**; the live JVM-estate transport (distributor connectivity) and the four
  inferred trade-lifecycle hops remain **deploy/estate-gated and explicitly NOT claimed as
  runtime-verified in-repo** (see "Honest scope"). Records the decision; the concrete Celer
  trade-lifecycle transport adapter is a deploy-time/environment concern, not an in-repo type.
- **Honours:** ADR-0007 (one unversioned contract — no `schema_version`), ADR-0008 (the
  asset-class-agnostic carry seam the ingested market data feeds), and CLAUDE.md guardrails
  #7 (open-source/free only), #8 (vendor-neutral product identifiers — the vendor name appears
  only as a documented external feed *source*, never in a core product identifier), and #11.
- **Source map:** `docs/CELER-INTEGRATION.md` (the integration research findings — the
  authoritative narrative for the Celer-side service names, transports and the still-inferred
  hops). This ADR records the *Celnet-side* decision grounded in the built code.

## Context (grounded in the code + the integration research)

Celnet must (a) **consume** external vendor FX-options market data (ATM + 25Δ/10Δ RR + BF
wing quotes per tenor per pair, plus spot / forward points / NDF fixings) to build its
arbitrage-free surface, and (b) **integrate into the Celer trade-lifecycle estate** (front
end / order routing / risk / position management / clearing) as a net-new, opt-in
option-product producer/consumer. Hard constraints from the research
(`docs/CELER-INTEGRATION.md` §0):

- The Celer **distributor is an in-process JVM disruptor mailbox** with *skip-while-full*
  back-pressure — a Rust process cannot natively join it; it speaks the socket protocol
  (`DistributorProducerChannelHandler`) or runs a JVM adapter. A high-frequency pricer can
  cause **silent price drops** unless mailboxes are sized and egress is rate-limited.
- **Cross-service edges are invisible to automated tooling** (the estate uses the in-proc
  distributor + Protobuf + FIX, not HTTP/gRPC) — the dependency map is maintained manually.
- Several lifecycle hops (`orderrouting → risk`, `risk → destination`,
  `destination → clearing`, `clearing → positionmanager`) are **inferred, not runtime-traced**.
- Spot-only `marketdata` may not supply vol — an **external vendor FX-options feed** is the
  designated vol-surface source; Celnet normalizes its (under-documented) delta/ATM
  conventions into the canonical surface on ingest.

What is **already built in-repo** (verified against the graph):

- `celnet-integration::vendor` — the vendor-feed wire layer: `VendorSmileMessage`
  (`from_json`/`to_json`), `WireForward` (`outright()` = points + spot), and the
  convention round-trips `WireDeltaConvention`/`WireAtmConvention`
  (`canonical`/`from_canonical`) that map the vendor's delta/ATM conventions to Celnet's
  canonical surface. `normalize()` enforces a three-layer convention contract before any
  quote is accepted.
- `celnet-integration::subscriber` — the source-agnostic ingestion seam: the `FeedTransport`
  trait, the `FeedFrame` (Snapshot/Delta) canonical message, and `ResilientSubscriber`
  (`ingest_frame` strict-monotone per-key sequencing; `run_until` two-level reconnect — on a
  sequence gap it drops to awaiting-snapshot and resyncs, never silently fills).
- `celnet-integration::deployment::DeploymentMode` — `{ CelerIntegrated, Hybrid, Standalone,
  ExternalFeedOnly }`; `uses_celer_feed()` / `uses_external_feed()` gate which feeds attach.
- `celnet-integration::egress::EgressGovernor` — the bounded, conflating egress rate-limiter
  that respects the distributor's skip-while-full mailbox (the silent-drop guard).
- `celnet-server::Edge::attach_vendor_feed` — binds a `MarketDataSource` + `PriceSink` +
  subscription keys + `VendorFeedConfig` and starts the `VendorFeed` task that drives the
  server's `surface_book`; gated by `DeployMode::binds_vendor_feed`. Proven end-to-end by
  `vendor_feed_with_gap_resyncs_and_surface_matches_direct_pipeline` (a vendor feed, including
  a gap+resync, drives the surface identically to the direct pipeline).

## Decision

1. **Vendor market-data INGRESS is a normalizing, resilient, source-agnostic adapter.**
   A vendor FX-options feed enters through the `FeedTransport` trait → `ResilientSubscriber`
   (sequenced, gap-detecting, self-resyncing) → `vendor::normalize` (convention round-trip to
   the canonical surface) → the server `surface_book` via `Edge::attach_vendor_feed`. The
   adapter is **source-agnostic**: the concrete vendor is a `FeedTransport` impl behind the
   trait, so the vendor name never reaches a core product identifier (guardrail #8) and a
   second source is a second impl, not a fork.

2. **Estate integration is DEPLOYMENT-MODE gated, additive, and non-disruptive.**
   `DeploymentMode` selects which feeds bind: `ExternalFeedOnly` (vendor only),
   `CelerIntegrated`/`Hybrid` (Celer feed ± vendor, with dual-feed divergence detection in
   `celnet-integration::divergence`), `Standalone` (neither). Celnet is added as a **net-new,
   opt-in option-product producer/consumer** that does not alter existing spot/FX flows; per
   the phased plan (`docs/CELER-INTEGRATION.md` §4), ingress (read-only) and shadow pricing
   precede any estate egress.

3. **Egress to the estate is bounded + conflating to respect the distributor.**
   All publication to the distributor / `MarketMerchantPriceService` flows through
   `EgressGovernor` (bounded, conflating, rate-limited) so a high-frequency pricer cannot
   overflow the skip-while-full mailbox and silently drop prices. The browser/webtrader edge
   is the same one WS contract, second encoding (ADR-0009 codec) — not a separate surface.

4. **The Celer-facing boundary speaks Protobuf + FIX; the JVM-estate transport is
   deploy-gated.** Internal Celnet wire formats stay internal; only the Celer boundary uses
   Protobuf/FIX/distributor. The distributor connectivity choice (JVM adapter vs the
   `DistributorProducerChannelHandler` socket protocol) and the four inferred lifecycle hops
   are **environment/staging decisions validated against the real estate**, not asserted in
   the repo.

## Consequences

- A new market-data source is a new `FeedTransport` impl + (if its conventions differ) a new
  `WireDeltaConvention`/`WireAtmConvention` mapping — no change to the surface, the pricer, or
  any downstream crate.
- The ingested surface is the **same** `celnet-surface` object the carry-seam pricers
  (ADR-0008) consume — so cross-asset/FX pricing, Greeks, and risk all sit on the one
  ingested, arbitrage-free surface with no parallel data path.
- **Invariant — no fabricated market state.** A sequence gap resyncs from a fresh snapshot;
  an underivable observable is skipped, never invented; a lagging egress consumer conflates
  (drops a stale tick) rather than back-pressuring the pricer or fabricating a value.
- Estate egress stays **off** until a deployment mode + tenant overlay enables it; existing
  Celer product flows are untouched until the option product is explicitly turned on.

## Honest scope (what is NOT claimed)

- The concrete **Celer trade-lifecycle transport adapter** (a live distributor/JVM bridge) is
  **not** an in-repo type — `DeploymentMode` is the gate; the live wiring is deploy/estate-
  gated. The graph shows no CALLS path from `celnet-server` into a named "Celer order/position
  adapter".
- The four lifecycle hops (`orderrouting → risk → destination → clearing → positionmanager`)
  are **inferred** (`docs/CELER-INTEGRATION.md` §5) and must be runtime-traced against the
  real services before being relied on.
- Adding an FX-**option** product type to the estate (`celertech-type`, `staticdata`, every
  API proto enum, `positionmanager` netting keys, `risk` exposure models, `destination` FIX
  dialect) is a broad, cross-cutting estate change concentrated at the `celnet-integration` +
  `celnet-proto` boundary — scoped, not yet executed in the estate.

## Alternatives rejected

- **Join the JVM distributor mailbox natively from Rust** — impossible (in-process JVM
  disruptor); a socket-protocol client or JVM adapter is required.
- **Publish to the estate without an egress governor** — rejected: the skip-while-full
  mailbox would silently drop a high-frequency pricer's prices.
- **A vendor-named ingest path baked into the core** — rejected (guardrail #8): the vendor is
  a `FeedTransport` impl behind a source-agnostic trait; the vendor name lives only in
  integration/docs context, never in a core product identifier.
- **A parallel market-data path for ingested vs Celer-native data** — rejected: both
  normalize to the one `celnet-surface` object; in `Hybrid` mode they are reconciled by
  divergence detection, not duplicated.

## Supporting verified claims (lodestar knowledge layer)

Authored graph-anchored (lifecycle `draft` — promotion to `active` awaits a cross-family
Stage-2 review). Anchors are real `qualified_name`s verified against the graph:

- `decision` — vendor MD ingress is a source-agnostic normalizing+resilient adapter onto the
  one canonical surface. Anchors: `celnet-integration::vendor::VendorSmileMessage`,
  `celnet-integration::subscriber::ResilientSubscriber`,
  `celnet-server::lib::Edge::attach_vendor_feed`.
- `decision` — estate integration is deployment-mode gated and additive; the live Celer
  transport is deploy-gated, not an in-repo adapter. Anchors:
  `celnet-integration::deployment::DeploymentMode`,
  `celnet-integration::egress::EgressGovernor`,
  `celnet-server::services::deploy::DeployMode::binds_vendor_feed`.
