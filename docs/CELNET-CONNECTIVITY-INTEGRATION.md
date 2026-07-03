# Celnet — Connectivity Extension Integration (design)

> Status: **design doc**, gates the `celnet-connectivity` extension build. Board lane
> `conn-design`…`conn-gui-console` on `coord/board`. Authored from a 3-agent read-only
> review of `github.com/soarsa/celnet-connectivity` @ `bb97347` (2026-07-02), cross-checked
> against CelNet's existing connectivity surface (`celnet-fix`, `celnet-integration`,
> `celnet-rfq`, `celnet-server`). See `docs/CELER-INTEGRATION.md` (WS-H) for the estate map.

---

## 0. TL;DR

`celnet-connectivity` is a competent **cash-FX vendor-FIX onboarding/certification
control-plane** (a Java→Rust port in progress). It is **not** an FX-options system and it is
**~98.5% scaffold**. We therefore **do not** adopt the tree wholesale — doing so would import
~200 mocks, a duplicate FIX engine, vendor-named identifiers, and a Postgres/identity stack,
violating guardrails #2 (no placeholders), #8 (vendor-neutral naming) and #10 (zero legacy /
no duplication).

Instead we build a **CelNet-native connectivity extension**: a new `celnet-connectivity`
crate that adds a *vendor-adapter descriptor framework* + *certification harness* **on top of
the existing real `celnet-fix` engine**, seeded with the 3 genuinely-real adapters
(vendor-neutralised), fed by the imported vendor **intelligence** + **spec tooling**, exposed
through the **one canonical API** and CelNet's **existing GUI**. We drop everything else.

**Non-scope flag:** the single most important FX-options connectivity input — a **vol-surface
(ATM/25Δ&10Δ RR/BF) feed** — is *absent* from `celnet-connectivity`. That gap (the Fenics-FMD
adapter on `celnet-integration`'s `MarketDataSource` seam) is tracked as `conn-vol-feed` and
is arguably higher priority than any LP FIX venue.

---

## 1. What the source repo actually is (review verdict)

| Dimension | Finding (cited from the review) |
|---|---|
| Real vs. scaffold | **3 of 201** adapter modules fleshed out (`bloomberg_fxgo_order`, `bloomberg_fxgo_price`, `rabofx_esp_order`); **198 carry the `SCAFFOLDED` marker**. |
| Adapter model | `FixAdapter` (`src/adapter/mod.rs:278`) is a **metadata/spec descriptor, not a network client** — identity, embedded QuickFIX XML, config template, cert-checks, wire-event→cert mapping. Object-safe, `Arc<dyn>`, explicit `vec!` registry (`registry.rs:20`). |
| The real engine | Hand-written **FIX 4.4 wire engine** in `service/` (`fix_message.rs` correct BodyLength/checksum, `fix_session.rs:188` real initiator, `inbound_fix_session.rs` acceptor) + a loopback conformance harness (`service/simulator/counterparty.rs`). |
| `todo!()` count | **17 live `todo!()`/`unimplemented!()`** (`lifecycle_service.rs:66-74`, `credential_service.rs:33`…). 200/202 conformance tests are registration smoke-tests vs a loopback stub. |
| Naming | Every module/type/dir is a **commercial vendor product name** (`BloombergFxgoPrice`, `citi_esp_price`, `jpm_algo_order`, …). |
| Footprint | axum 0.8 + **sqlx dual SQLite/Postgres** (31+7 migrations), JWT/tenant/TOTP auth, AES-256-GCM, **OpenSSL/native-tls** (vs CelNet rustls), **edition 2021 / rustc 1.78** (vs 2024/1.96). Standalone Vite/React SPA + Ansible/nginx deploy. |
| Dependencies | **Clean OSS** — no commercial/proprietary crates or vendor SDKs. |
| Catalog | `adapters.csv` = **225 tuples / 113 vendors / 4 tiers**, ~100% FIX, dominantly FX spot/fwd/NDF/swap. ~3-4 FX-**options** rows, all scaffold. Genuinely valuable as an *inventory*. |
| "GAN harness" | A **frontend UI-design** iteration loop (grades Tailwind), unrelated to adapters, self-scores without rendering. Irrelevant. |
| Vol surface | **Absent everywhere.** No ATM/RR/BF / smile / implied-vol content in the whole repo. |

## 2. Guardrail conflicts (why not adopt wholesale)

- **#2 (no mocks/placeholders):** 198 scaffold adapters + 17 `todo!()` fail a strict bar.
- **#8 (vendor-neutral identifiers):** ~200 modules/types are vendor product names.
- **#10 (zero legacy / no duplication):** the ported FIX engine **duplicates real `celnet-fix`**
  (framing, session recovery, initiator+acceptor, dialects); the SPA duplicates the GUI; the
  JWT/tenant/auth stack duplicates `celnet-entitlements` + `celnet-server` auth.
- **Posture:** OpenSSL/native-tls vs rustls; edition 2021/1.78 vs 2024/1.96; Ansible/nginx vs
  the justfile/sccache/cargo-workspace deploy; a Postgres dependency a stateless pricer lacks.

## 3. What we take vs. drop

**Take (the genuine, non-duplicative value):**
1. **Ideas/design** of the `FixAdapter` *descriptor* pattern and the **certification harness**
   (cert-checks, auto-pass on wire events, promote/demote gating) — CelNet lacks this; it is the
   real IP.
2. The **3 real adapters** as reference specs (Bloomberg FXGO order/price, Rabobank ESP),
   re-expressed vendor-neutral.
3. The **vendor intelligence** (`adapters.csv` + `VENDOR_ADAPTERS_INVENTORY.md`) as a
   prioritized *target catalog* (docs/data), and the **`spec-extract` PDF→QuickFIX-XML tooling**
   (honest pdfplumber scaffolding) under `tools/connectivity/`.
4. The **ops-console concept** (connection dashboard / connection-detail-with-live-FIX-log /
   certification checklist) — rebuilt in CelNet's GUI.
5. Operational *ideas*: HA connection-lease port arbitration, `/metrics` health-gate, MiFID-II
   clock-sync, systemd hardening — folded into CelNet's deploy where relevant.

**Drop:** the 198 scaffolds, the 17 `todo!()`, the duplicate FIX engine, the standalone SPA,
the Ansible/nginx pipeline, the GAN harness, and the Postgres/JWT/tenant/TOTP stack.

## 4. Target architecture

```
            ┌──────────────── celnet-connectivity (NEW crate) ────────────────┐
 celnet-fix │  VendorAdapter (descriptor): identity + Counterparty(data) +    │
 (REUSE ────►│  embedded spec dict + config template + CertCheck set +        │
  engine)   │  event→cert mapping;  AdapterRegistry (explicit, vendor-neutral) │
            │  CertHarness: seed → auto-pass on wire events → promote/demote   │
            └───────────────┬───────────────────────────────┬────────────────┘
                            │ drives sessions via            │ conformance vs
                            ▼ celnet-fix Session/Initiator    ▼ in-proc counterparty
                    ┌───────────────┐                 ┌──────────────────────┐
                    │  celnet-rfq   │  (FixLpAdapter  │  loopback stub (test)│
                    │  multi-dealer │   already real) └──────────────────────┘
                    └───────────────┘
   celnet-server (ONE API) ── ConnectivityService (proto+WS): list/create/start/stop/
        kill/test session · live FIX-log stream · cert status   ──► GUI ops-console screens
```

- **Reuse `celnet-fix`** for all wire/session work. `celnet-connectivity` never re-implements FIX.
- **Vendor identity is data, not identifiers.** The adapter type is purpose-named
  (`FixVenueAdapter`, `VendorAdapter`); the counterparty ("Bloomberg FXGO") lives in a
  `Counterparty`/`VendorId` value. Naming a counterparty you connect to is legitimate integration
  context (guardrail #8 covers *product* identifiers), so this satisfies #8.
- **No new datastore or identity system.** Connection config/cert state persists through CelNet's
  existing mechanisms; auth/entitlements reuse `celnet-entitlements` + `celnet-server`.
- **One API, one GUI.** The control-plane is `celnet-server` RPCs (coordinator-owned proto
  window); the ops screens are rebuilt in `gui/` against that contract (API-first parity).

## 5. Phased plan (board lanes)

| Lane | Gate | Deliverable |
|---|---|---|
| `conn-design` (this doc) | T1 | ADR + design + lodestar knowledge. **← in progress** |
| `conn-framework` | T2 | `celnet-connectivity` crate: descriptor trait + registry + cert-harness on `celnet-fix`; 3 real adapters vendor-neutral; loopback conformance gate. |
| `conn-catalog` | T1 | Vendor intelligence catalog (docs/data) + `spec-extract` tooling under `tools/connectivity/`. |
| `conn-vol-feed` | T2 | **Priority gap:** FMD-shaped vol-surface adapter on `celnet-integration` `MarketDataSource` → canonical surface; golden convention round-trip. |
| `conn-server-api` | T2 | `ConnectivityService` (proto + WS): connection lifecycle + live FIX-log + cert status. |
| `conn-gui-console` | T2 | Ops-console screens in the existing GUI; live Playwright+axe e2e. |

Ordering: design → (framework ∥ catalog ∥ vol-feed) → server-api → gui-console. Serial cargo on
the M4; T2 landing batched through the coordinator window.

## 6. Open decisions (defaulted; redirectable)

1. **Vol-feed priority.** Default: treat `conn-vol-feed` as the highest-value connectivity work
   (an options desk needs vol surfaces before 200 LP venues). Building the framework first
   (`conn-framework`) still comes first because it's self-contained and the repo's real asset.
2. **Persistence.** Default: **no Postgres.** Connection/cert state stays in-process +
   CelNet's existing journal where durability is needed. Revisit only if a real multi-node
   onboarding console demands it.
3. **Adapter breadth.** Default: the 200-venue catalog is a **demand-driven backlog**, not a
   thing to fully build. Seed with the 3 real adapters + whatever an actual desk asks for.
