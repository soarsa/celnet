# Bond Data & Corporate-Actions Sourcing — Requirements / Design

> Status: **requirements + design** (2026-07-30) — **PARTIALLY BUILT; amended 2026-08-12.**
> Grounds the Celnet **data-sourcing / ingestion** capability on the existing reference-data,
> bond/rates pricing, booking/position and aggregation seams (file:line cited below, current as of
> the original doc). Vendor-neutral naming throughout (guardrail 8). OSS-only / free +
> academically-grounded methodology, with commercial sources named **reference-only** and a
> documented open fallback (guardrail 7). Off the pinned zero-alloc pricing hot core
> (guardrail 11). No mocks / placeholders / `todo!()` (guardrail 2).
>
> **⚠ As-built amendment (2026-08-12) — the original "No code yet" is no longer true.**
> **P1 shipped in full** (`celnet-corpactions` + `celnet-refstore` crates, plus a
> `CorporateActionsService` and a GUI workspace); **P3 shipped partially** (the
> announce→elect→confirm→apply→reverse lifecycle and the position-effect apply exist; the
> MT 564 / `seev.*` message parsers do **not**); **P2 shipped only as `GovvieSource`** (deterministic
> INTR+REDM derived from the curated universe — no pull adapters for FiscalData/DMO/H.15/ECB/GLEIF/
> OpenFIGI); **P4 did NOT ship** — an applied corporate action still never reaches the live pricer.
> The §11 open item *"crate boundary: evolve `celnet-refdata` vs a new `celnet-refstore`"* is
> **resolved as built**: both exist — `celnet-refdata` stayed the static OSS seed, `celnet-refstore`
> became the mastered golden store.
> **The as-built reconciliation, the seven remaining gaps (including the unwired P4 seam) and the
> phased plan to close them now live in
> [`CORPORATE-ACTION-MONITOR-GAP-ANALYSIS.md`](CORPORATE-ACTION-MONITOR-GAP-ANALYSIS.md).**
> This doc remains the source of truth for **sourcing** (standards, open-vs-commercial verdict,
> identifiers, canonical data model); that doc owns **as-built state and the pricing wire-up**.

## 0. Scope — the DATA layer that `ANALYTICS-REQUIREMENTS.md` §10 assumed

`docs/ANALYTICS-REQUIREMENTS.md` **§10** already specifies the bond **corporate-action (CA)
event universe** — the ISO 15022 / ISO 20022 CAEV taxonomy, what each event does to
inventory / P&L / risk / pricing / analytics, the double-count guard, and the celnet
grounding for a CA-apply path. **This doc does not repeat §10.** It specifies the layer §10
took as a given input: **where the CA and bond reference/analytics data comes from, how it is
ingested and kept as a golden source, and how it flows into the pricing and hedging engines.**

Read the two together:

| Concern | Owner doc |
|---|---|
| CA event taxonomy (CAEV), per-event effect on inventory/P&L/DV01/pricing, double-count guard, `book()`-path apply | `ANALYTICS-REQUIREMENTS.md` §10 (existing) |
| **Sourcing** the CA & reference data (standards, feeds, open-vs-commercial, the honest coverage gap) | **This doc §1–§4** |
| **Ingestion** architecture (connector/adapter, normalization, golden-source store, validation, lifecycle) | **This doc §6–§7** |
| **Feeding** hedging & pricing from the mastered data | **This doc §8** |

The sibling review `docs/CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md` §C already enumerated
the **instrument-definition field set** a reference-data store needs per family (FIX 4.4
`Instrument` block, FpML schedules, ISO identifiers, QuantLib inputs) and recommended a
vendor-neutral registry modelled on the identity store; this doc adopts that field work as the
**canonical schema** (§6) and adds the **sourcing + ingestion + mastering** it did not cover.

---

## 1. Corporate-actions data — standards, messaging & the announce→confirm lifecycle

CA data moves as a three-message **announce → instruct → confirm** flow, on the legacy SWIFT
ISO 15022 MT 56x family and its modern ISO 20022 `seev.*` replacement. Celnet adopts the
**code sets and the message semantics as the internal contract** (guardrail 8: we adopt the
open vocabulary, we do not embed any vendor product).

### 1.1 ISO 15022 — the MT 56x securities-events family

| MT | Message | Role in the lifecycle |
|---|---|---|
| **MT 564** | CA Notification | Account servicer → account owner: announces the event, eligible balance, entitlements, and the key dates. Start of the flow. |
| **MT 565** | CA Instruction | Account owner → servicer: the **election** for voluntary / choice events; refers back to the MT 564. |
| **MT 566** | CA Confirmation | Servicer → owner: confirms the actual cash / securities **movement**; refers back to the MT 565. |
| **MT 567** | Status / Processing Advice | Status of a submitted instruction (accepted/rejected/pending). |
| **MT 568** | Narrative | Free-text detail that does not fit structured fields. |

Sources: [SWIFT — Corporate Actions ISO 15022 (MT 564/565/566/567/568) training](https://www.swift.com/myswift/services/training/swift-training-catalogue/browse-swift-training-catalogue/corporate-actions-iso-15022-mt-564-mt-565-mt-566-mt-567-mt-568),
[MT 564 handbook](https://www.iso20022.org/15022/uhb/finmt564.htm),
[ISITC Corporate Actions Market Practice v8.0 (Dec 2022)](https://isitc.org/wp-content/uploads/Corporate_Actions_Market_Practice_v8.0_Dec2022.pdf),
[ECB — ISO 15022 vs ISO 20022 CA/PV messaging](https://www.ecb.europa.eu/paym/groups/shared/docs/8846a-cmh-tf-2018-01-16-presentation-swift-assetservicing-ca-pv-messages.pdf).

### 1.2 ISO 20022 — the `seev.*` (Securities Events) replacement

The modern, machine-readable XML successor (richer structured fields, less narrative). The
exact message numbers Celnet's ingest adapters must recognise:

| `seev.*` | Message | ≈ MT | Role |
|---|---|---|---|
| **seev.031** | CA Notification (CANO) | MT 564 | Announce the event. |
| **seev.033** | CA Instruction (CAIN) | MT 565 | Holder election. |
| **seev.034** | CA Instruction Status Advice | MT 567 | Status of a submitted instruction. |
| **seev.035** | CA Movement **Preliminary** Advice | — | Advance notice of expected cash/securities movement. |
| **seev.036** | CA Movement Confirmation (CACO) | MT 566 | Confirm completed movement. |
| **seev.037** | CA Movement Reversal Advice | — | Reverse/correct a prior movement. |
| **seev.038** | CA Narrative | MT 568 | Descriptive detail. |
| **seev.039** | CA Cancellation Advice | — | Cancel a previously announced event. |
| **seev.044** | CA Movement Preliminary Advice **Cancellation** | — | Withdraw a seev.035. |

Sources: [ISO 20022 `seev` (Securities Events) catalogue — all 49 messages](https://www.iotafinance.com/en/SWIFT-ISO20022-Business-area-seev-Securities-Events.html),
[seev.031 detail](https://www.iotafinance.com/en/SWIFT-ISO20022-Message-seev-031-001-Corporate-Action-Notification.html),
[ISO 20022 message archive](https://www.iso20022.org/catalogue-messages/iso-20022-messages-archive).

The **seev.037 reversal / seev.039 cancellation** semantics are why the golden-source store
must be **append-only + effective-dated** (§7.3): a confirmed movement can be reversed, and an
announced event can be cancelled — the store must supersede without destroying history so
positions and P&L can be un-applied correctly (the double-count guard, §10.2).

### 1.3 The CAEV & CAMV code sets — the internal event vocabulary

- **CAEV — CA Event Type.** The event taxonomy Celnet adopts as its internal event enum
  (INTR, REDM, MCAL, PCAL, PRED, DRAW, PPMT, BPUT, TEND, EXOF, CONV, … — the per-event
  position/pricing effect is the §10.1 table; **not repeated here**).
  [ISO 20022 CAEV code set](https://www.iso20022.org/standardsrepository/public/wqt/Description/mx/dico/codesets/_bHRNZtp-Ed-ak6NoX_4Aeg_-1565392153).
- **CAMV — Mandatory / Voluntary Indicator.** Three values that drive the **election
  lifecycle** and whether Celnet must solicit a holder decision:
  - **MAND** — mandatory (no election; applies automatically, e.g. REDM/INTR/PRED).
  - **VOLU** — voluntary (holder-optional, e.g. TEND, a discretionary buy-back).
  - **CHOS** — mandatory with options / choice (must respond, choosing among outcomes, e.g.
    an EXOF cash-or-stock election).
  [ISO 20022 CAMV code set](https://www.iso20022.org/standardsrepository/public/wqt/Description/mx/dico/codesets/_bKsU1dp-Ed-ak6NoX_4Aeg_735779459).
- **Where the code lists live.** CAEV/CAMV are **core (in-schema)** code sets; many other CA
  codes are **externalised** and published quarterly (XLSX/XSD/JSON) as the ISO 20022 External
  Code Sets — the ingest layer loads the current release, does not hard-code a stale copy.
  [ISO 20022 External Code Sets](https://www.iso20022.org/catalogue-messages/additional-content-messages/external-code-sets).

**Mandatory-vs-voluntary + the entitlement lifecycle** (record date → ex-date → response/market
deadline → payment date) is the state machine the ingestion event-application path (§7.5)
implements: a **MAND** event auto-applies at the entitlement dates; a **VOLU/CHOS** event opens
an **election** the desk must resolve before the deadline (a seev.033/MT 565 instruction), then
applies on confirmation (seev.036/MT 566). Celnet holds a proprietary book, so "election"
here is the desk's own decision on its inventory, not a custodian relay — but the same
lifecycle and deadlines govern it.

### 1.4 CSD / DTCC flows & the ISO 20022 migration

The upstream source of authoritative CA announcements is the **CSD** (DTCC in the US;
Euroclear / Clearstream / national CSDs in Europe), migrating MT 56x → `seev.*`. DTCC
publishes ISO 20022 CA user guides by event family (Distributions, Redemptions,
Reorganizations). Celnet does not connect to a CSD directly (it is not a custodian); this
matters because it fixes the **realistic ingest shape**: Celnet ingests either a **vendor
CA feed** (that has itself scrubbed CSD/issuer sources) or **open primary sources** (§2),
normalized to the same canonical schema.

Sources: [DTCC — Getting Started with ISO 20022 CA messaging](https://www.dtcc.com/-/media/Files/Downloads/issues/Corporate-Actions-Transformation/Getting_Started_CA_ISO_20022.pdf),
[DTCC — ISO 20022 Redemptions user guide](https://www.dtcc.com/-/media/Files/Downloads/issues/Corporate-Actions-Transformation/ISO_20022_RedemEntAlloc_UG_FINAL.pdf),
[DTCC CA 20022 data service](https://dtcclearning.com/products-and-services/dtcc-data-services/ca-20022-service.html),
[ECB AMI-SeCo Single Collateral Management Rulebook — CA standards](https://www.ecb.europa.eu/press/intro/publications/pdf/ecb.amiseco202312_corporateactions.en.pdf).
*(Flag: DTC migration phase dates circulated in secondary snippets are indicative — confirm
against the DTCC "Getting Started" PDF before citing a calendar.)*

---

## 2. Where the data realistically comes from — open vs commercial (the honest verdict)

Guardrail 7 forces an honest split. The blunt reality of this market:

> **Government / rates CA data is largely deterministic and derivable from OPEN issuance data.
> Comprehensive corporate / credit CA data is a COMMERCIAL, multi-source-scrubbed product with
> no free equivalent.** An OSS-only build can fully cover govvies/rates and must treat
> corporate CA as a *pluggable vendor-feed adapter* it ships but does not populate for free.

### 2.1 Open / free sources Celnet can build on

| Source | Data | License / access | Celnet use |
|---|---|---|---|
| **US Treasury FiscalData / TreasuryDirect** | Auction + issuance terms, redemption schedules for bills/notes/bonds/TIPS/FRN | Public domain, **no key** | Derive US govvie coupon/redemption schedules deterministically |
| **UK DMO — "Gilts in Issue"** | Gilt first-issue + redemption dates, nominal outstanding, issuance history, events calendar | Free access (⚠ explicit OGL grant unconfirmed — verify reuse terms) | Derive gilt schedules & redemption calendar |
| **Fed H.15 Selected Interest Rates** | Daily Treasury constant-maturity & other rates (XML/DDP) | Public domain | Curve observables (USD benchmark yields) |
| **ECB Data Portal (ex-SDW)** | Euro-area zero-coupon / forward / par yield curves (daily, TARGET calendar, since 2004) | Free access **and free reuse** (ESCB policy; subject to disclaimer) | Curve observables (EUR) |
| **FRED (St. Louis Fed)** | Yields, curves, macro series | **Free API, key required**; terms restrict to personal/non-commercial + some third-party copyright | Convenience mirror of curve observables — read the terms, not blanket-open |
| **GLEIF LEI** | Legal-entity identifiers, issuer hierarchy | **CC0 (genuinely open)**; free download + API; AWS Open Data mirror | Issuer identity / entity mastering |
| **OpenFIGI** | FIGI instrument identifiers, open symbology | **FIGI is MIT / free** to use, issue, redistribute; free API key | Vendor-neutral instrument identity (the OSS identifier of choice) |
| **FINRA TRACE** | US corporate/agency post-trade prices | **Free to *view*** on FINRA site; downloadable historic files **delayed** (≥6 months corporate/agency; longer for 144A); real-time low-latency feed is **commercial** | Delayed corporate-bond liquidity/price reference only — not a real-time feed |
| **SEC EDGAR** | US issuer filings (8-K, SC TO tender offers, prospectus supplements), bulk JSON | Free / public, full-text since 2001 | The closest free *primary* corporate-CA source — but filings, **not** normalized ISIN-keyed CA records; US-only; heavy parsing |

Sources: [FiscalData auctions](https://fiscaldata.treasury.gov/datasets/treasury-securities-auctions-data/) · [FiscalData API](https://fiscaldata.treasury.gov/api-documentation/) · [TreasuryDirect auction query](https://www.treasurydirect.gov/auctions/auction-query/auction-query-help/) · [DMO Gilts in Issue](https://www.dmo.gov.uk/data/gilt-market/gilts-in-issue/) · [Fed H.15](https://www.federalreserve.gov/releases/h15/) ([XML feed](https://www.federalreserve.gov/feeds/data/H15_H15.XML)) · [ECB euro-area yield curves](https://www.ecb.europa.eu/stats/financial_markets_and_interest_rates/euro_area_yield_curves/html/index.en.html) ([data access / SDMX](https://www.ecb.europa.eu/stats/accessing-our-data/html/index.en.html)) · [FRED API key](https://fred.stlouisfed.org/docs/api/api_key.html) ([terms](https://fred.stlouisfed.org/docs/api/terms_of_use.html)) · [GLEIF open data (CC0)](https://www.gleif.org/en/about/open-data) ([API](https://www.gleif.org/en/lei-data/gleif-api)) · [OpenFIGI features/license](https://www.openfigi.com/about/features) · [FINRA TRACE](https://www.finra.org/filing-reporting/trace) ([Rule 7730 dissemination](https://www.finra.org/rules-guidance/rulebooks/finra-rules/7730)) · [SEC EDGAR full-text search + APIs](https://www.sec.gov/edgar/search/).

### 2.2 Commercial sources — reference-only (NOT runtime dependencies)

Comprehensive, normalized, ISIN-keyed corporate-CA and security-master data is fundamentally a
**commercial, multi-source-scrubbed** product. These are named so the coverage gap is honest;
**none may be a runtime dependency in an OSS-only build** (guardrail 7). Celnet ships an
**adapter interface** (§7.1) any of them could feed via ISO 15022/20022, but populates it for
free only from §2.1:

- **ICE Data Services** — Corporate Actions + Reference Data ([product](https://www.ice.com/fixed-income-data-services/data-and-analytics/pricing-and-analytics/reference-data/corporate-actions)).
- **SIX (SIX Financial Information)** — Global Corporate Actions / market reference data ([product](https://www.six-group.com/en/products-services/financial-information/market-reference-data/corporate-actions-data.html)).
- **Bloomberg** (Data License / security master + CA), **LSEG / Refinitiv** ([reference data](https://www.lseg.com/en/data-analytics/market-data/data-analytics-pricing/reference-data)), **S&P Global Market Intelligence**, **FactSet** — the "big four" hold the bulk of the data market.
- **CSD data services** — DTCC / Euroclear / Clearstream announcements (the authoritative upstream the vendors scrub).

**Takeaway for the design:** the ingestion layer's connector interface is the value; the
corporate-CA *content* behind it is where a commercial feed is unavoidable. Celnet's OSS build
delivers full govvie/rates coverage and a **ready-to-plug** corporate-CA adapter that a
customer wires to their own licensed feed.

---

## 3. The honest OSS-only coverage gap (state it plainly)

- **Government bonds / rates — fully coverable OSS.** Coupon and redemption schedules are
  **deterministic** and derivable from official issuance terms + auction announcements (DMO,
  US Treasury), which are open. Schedules are then **generated** (QuantLib / celnet-bond),
  not fed. Redemptions (REDM), scheduled coupons (INTR), sinking/amortisation on published
  schedules (PPMT/PRED) are computable in-house. **This is the primary Celnet universe today**
  (`celnet-refdata` ships US/UK/EUR govvies).
- **Corporate / credit bonds — the gap.** Calls, partial calls, tenders, exchange offers,
  consent solicitations, restructurings are announced through issuer/paying-agent/CSD channels
  and aggregated by commercial vendors. Free primary sources (SEC EDGAR) are US-only,
  filing-shaped (not normalized CA records), and incomplete for the global universe. **No free
  equivalent to a scrubbed multi-source CA feed exists.** The ICE/SIX product pages market
  exactly this multi-source scrubbing as their value-add — corroborating the gap.
- **Practical stance.** Build the **deterministic govvie/rates CA in-house** from open issuance
  data + a schedule generator; ship corporate CA as a **pluggable adapter** (MT 564 / seev.031
  ingest) with a documented, honest statement that comprehensive free coverage does not exist.
  Every phase (§10) is gated so the OSS govvie path is complete and validated on its own before
  the corporate adapter is even wired.

---

## 4. Bond analytics + reference data — compute in-house vs feed; identifiers & mastering

### 4.1 Computable in-house (OSS) vs must-be-fed

**Computable in-house — the *math* is fully OSS-derivable** (QuantLib is the open golden
oracle; Celnet already re-implements these first-party in `celnet-bond` / `celnet-rates`):

| Analytic | In-house today / OSS-derivable |
|---|---|
| Coupon schedule generation (effective/termination/tenor/calendar/roll/EOM) | `celnet-bond` `CashflowSchedule::from_bond` (§5); QuantLib `Schedule` as oracle |
| Day-count & business-day calendars | `celnet-calendar` (`daycount.rs`, `holiday.rs`, `roll.rs`); QuantLib `DayCounter`/`Calendar` |
| YTM, DV01, duration, convexity, accrued | `celnet-bond` `risk.rs` (§5) |
| Z-spread / G-spread / asset-swap spread (given a curve + observed price) | `celnet-rates` `bond.rs` (per Curves review §0); QuantLib `zSpread`/`AssetSwap` as oracle |
| OAS (callable) | To build on the schedule model (§8) via short-rate tree; QuantLib OAS as oracle |
| Benchmark / OIS curve construction | `celnet-rates` `bootstrap.rs` (§5); QuantLib bootstrap as oracle |

**Must be fed (an external input, even when the analytics are in-house):**
- **Market observables** the curves consume — benchmark yields (open: H.15, FiscalData, DMO,
  ECB), OIS/SOFR/€STR fixings, and live bond prices/quotes. Real-time corporate prices are the
  commercial gap; TRACE is delayed/free-to-view only.
- **Corporate-bond CA events** (calls/tenders/exchanges) that change schedules — §3 gap.
- Exotic-market holiday-calendar edge cases (QuantLib ships most major-market calendars).

QuantLib licence for the oracle role: **modified / 3-clause BSD** — OSS-permissive, usable for
both free and proprietary work ([QuantLib license](https://www.quantlib.org/license.shtml)).
It is Celnet's **validation oracle** (guardrail 5), never a runtime dependency in the pinned
core.

### 4.2 Identifier strategy — ISIN / CUSIP / FIGI / LEI (guardrail-critical)

| Identifier | Standard | Licence posture | Celnet posture |
|---|---|---|---|
| **ISIN** | ISO 6166 | Allocated by NNAs (mostly free to allocate); a **consolidated ISIN database** + ISIN↔CUSIP linkage is **commercially licensed** (CGS) | **Primary external id**; validate the ISO-6166 check digit in-house (already done — `celnet-refdata` `isin.rs`, §5). Do not embed a commercial ISIN *database*. |
| **CUSIP** | CGS / ABA | **Proprietary, licensed** — any use beyond clearing/settlement needs a paid CGS licence | **Reference-only.** Carry a CUSIP field where the security natively has one (US Treasuries stream it — `GovBondSpec.cusip`), but never bulk-license CUSIP data as a runtime dep. |
| **FIGI** | Open (OpenFIGI) | **MIT / free** to use, issue, redistribute | **The OSS identifier of choice** for vendor-neutral cross-referencing. |
| **LEI** | ISO 17442 / GLEIF | **CC0 (open)** | **Issuer / entity identity** in the security master. |

Sources: [ANNA identifiers (ISO 6166)](https://anna-web.org/identifiers/) · [CGS licence fees (CUSIP is licensed)](https://www.cusip.com/services/license-fees.html) · [OpenFIGI (free/MIT)](https://www.openfigi.com/about/features) · [GLEIF LEI (CC0)](https://www.gleif.org/en/about/open-data).
**Guardrail-critical:** CUSIP data is licensed. The FIGI+LEI+(natively-present) ISIN triple is
the OSS-clean identity backbone; CUSIP is carried opportunistically, never sourced commercially.

### 4.3 Golden-source / security-master mastering approach

No single source is authoritative across all asset classes, geographies and event types, so a
**golden record** is composited across sources with:
- **Rules-based survivorship** — per-field, per-asset-class **source priority** + quality
  thresholds pick the winning value (e.g. official DMO redemption date beats a derived one).
- **Effective-dating / bitemporal history** — every field carries *(valid-from, recorded-at)*
  so a value's state "as known on date X, effective on date Y" is reconstructable — essential
  because CA reversals (seev.037) and cancellations (seev.039) rewrite prior state.
- **Durable audit lineage** — source → transform → golden value is recorded so provenance is
  explainable for risk/regulatory use (mirrors the `PricingProvenance` waterfall discipline
  already shipped for pricing).

Practitioner references (secondary — illustrative, not standards):
[securities master / golden copy](https://devancore.com/glossary/securities-master-data/),
[MDM survivorship](https://profisee.com/blog/mdm-survivorship/),
[security-master reconciliation](https://www.greshamtech.com/blog/security-master-modernization).

---

## 5. Celnet grounding — what exists vs the NEW build (cited file:line)

**Current state: Celnet has curated *static* govvie data and full bond/rates analytics, but
NO mutable registry, NO CA representation, and NO ingestion layer.** Every seam a data layer
must plug into already exists and is cited.

### 5.1 What exists

- **Curated static reference data (`celnet-refdata`).** `GovBondSpec` — a fully-specified,
  ISO-6166-check-valid govvie record (`crates/celnet-refdata/src/model.rs:65-104`: `isin`,
  `cusip`, `coupon_rate`, `coupon_type` (`fixed|zero` only), `coupon_frequency`, `day_count`,
  `dated_date`, `maturity_date`, `redemption`, `calendars`). The universe is a **stateless
  `Vec` rebuilt at boot** — `government_universe()` (`crates/celnet-refdata/src/lib.rs:52-56`)
  = `treasury_universe()` + `curated_universe()`; **US/UK/EUR** govvies. ISIN check-digit
  validated in-crate (`isin.rs`, `is_well_formed`). **No mutable insert/lookup, no versioning,
  no event apply.**
- **Bond analytics leaf (`celnet-bond`).** `Bond` (`crates/celnet-bond/src/bond.rs:18-25`) →
  the **single** cashflow-expansion point `CashflowSchedule::from_bond`
  (`crates/celnet-bond/src/schedule.rs:55-126`) — rolls regular coupon dates back from
  maturity, one bullet `redemption` at maturity, **no call/put/sink/amortisation, no floating
  reset, no event feed.** Risk is pure arithmetic over that schedule: `bond_risk`
  (`crates/celnet-bond/src/risk.rs:104-122`) / `BondRisk` (`risk.rs:79-96`: `clean_price`,
  `accrued_interest`, `dv01`, `modified_duration`, `convexity`); `dv01` (`risk.rs:35`).
- **Rates / curve leaf (`celnet-rates`).** `Curve` keyed on continuous year-fraction time
  (`crates/celnet-rates/src/curve.rs:86-96`, `discount_factor` `:318`, `zero_rate` `:330`);
  multi-instrument bootstrap `bootstrap_curve` (`crates/celnet-rates/src/bootstrap.rs:233`) and
  `bootstrap_ois` (`:273`). Key-rate / signed DV01 hedging ladder in `celnet-rates-risk`
  (`ladder.rs` `key_rate_ladder` / `signed_parallel_dv01` / `normalize_bond_dv01`).
- **Server reference-data seam.** `GovBondSpec` → `InstrumentDef` via `gov_bond_to_instrument_def`
  (`crates/celnet-server/src/config/reference_data.rs:688`), `government_bond_defs`
  (`:678-683`), seeded by `ensure_seed_instruments` (`:732`) and
  `IdentityStore::ensure_seed_government_bonds` (`identity.rs:1198-1228`).
- **The registry chassis to copy.** The identity / Entity / Book / User store
  (`crates/celnet-server/src/config/identity.rs`) — an in-process, **JSON-persisted**,
  admin-managed, deny-by-default keyed registry using **additive serde defaults, no
  `schema_version`** (guardrail 9). This is the pattern for a mutable reference store (Curves
  review §C.4).
- **Durable append-only substrate.** `celnet-journal` — an **fsync'd, append-only,
  sequence-ordered** log with CRC-32 per record and torn-tail recovery (`Journal::append`
  returns only after bytes are durable; `crates/celnet-journal/src/lib.rs`). The natural
  durability + lineage substrate for an effective-dated golden-source store.
- **The connector/adapter pattern to reuse.** `celnet-fix` — a full FIX stack (acceptor /
  initiator / session FSM / dialects) already ingests LP quotes; the aggregation engine's
  push-ingest seam `AggregationHub::ingest` (`crates/celnet-server/src/services/aggregation.rs:377`)
  is the model for a per-source push adapter that normalizes then updates a store.
- **Position / inventory sinks a CA apply drives.** `RatesPositionStore`
  (`crates/celnet-server/src/services/rates_book.rs:74`; struct doc `:53-71`), booked via the
  `book`/`book_with_routing` path; the flat `InstrumentInventory` + `InventorySource` trait
  (`crates/celnet-server/src/services/aggregation.rs:97-133`, trait `~:86`) that the
  inventory-skew pricing feature reads.

### 5.2 What is NEW (this doc)

1. A **mutable, effective-dated, mastered reference store** (evolves `celnet-refdata` from a
   static `Vec` into a golden source) — §7.3.
2. A **CA event model + schedule/position-effect functions** — the CAEV/CAMV record and the
   pure apply functions hanging off `CashflowSchedule::from_bond` — §6.2, §8.
3. An **ingestion / connector layer** — per-source adapters (open + a pluggable vendor
   interface), normalization to the canonical schema, validation/reconciliation, and the
   event-application lifecycle — §7.

---

## 6. Canonical internal data model

Two normalized records, both **vendor-neutral** (guardrail 8) and **off the hot core**
(guardrail 11). Field-level detail for the *instrument* record is the Curves review §C.3 field
tables (FIX 4.4 `Instrument` block + FpML + QuantLib inputs) — adopted, not repeated.

### 6.1 Mastered instrument reference record (`InstrumentMaster`)

The golden record per security — the Curves review §C.3 fields, plus mastering metadata:
- **Identity:** internal `instrument_id` (registry key) + `external_ids` map
  (FIGI / ISIN / LEI-issuer / opportunistic CUSIP — §4.2).
- **Static terms:** issuer (+ LEI), currency, `coupon_type` (extend to `fixed|zero|frn`),
  `coupon_rate`, `coupon_frequency`, `day_count`, `issue/dated/first-coupon/maturity` dates,
  `redemption`/face, ex-div & settlement convention, calendars, sector/rating (optional).
- **Schedules (NEW, the §10 gap):** `call_schedule`, `put_schedule`, `sink_schedule` /
  amortisation, `pool_factor` time series (pass-throughs), FRN `index + margin + reset`.
- **Mastering metadata (NEW):** per-field `(source, valid_from, recorded_at, quality)` for
  survivorship + bitemporal history + lineage (§4.3).

### 6.2 Corporate-action event record (`CorporateAction`)

The normalized CA event, keyed by ISIN, adopting the CAEV/CAMV vocabulary (§1.3) and the
lifecycle dates (§1.1):
- `caev` (event type — §10.1 semantics), `camv` (MAND/VOLU/CHOS — §1.3).
- Dates: `announcement`, `record`, `ex`, `response_deadline`, `payment`.
- Amounts / rates: entitlement per unit, call/put/tender price (or make-whole spread + curve
  ref), redemption factor / pool factor, exchange/conversion ratio + target instrument.
- Lifecycle status: `announced → (elected) → confirmed → applied`, plus `reversed` / `cancelled`
  (seev.037 / seev.039), each an **append** that supersedes — never an in-place mutation.
- Lineage: source message id (MT 564 / seev.031 reference), ingest adapter, recorded-at.

---

## 7. Ingestion architecture

A dedicated ingestion pipeline, entirely on the async edge — **never the pinned zero-alloc
pricing thread** (guardrail 11). It mirrors the existing push-ingest + normalize + store shape
(`AggregationHub::ingest`, `celnet-fix`), not a new paradigm.

```
  open sources (FiscalData / DMO / H.15 / ECB / GLEIF / OpenFIGI / EDGAR)
  + pluggable vendor CA feed (MT 564 / seev.031 — customer-licensed)
        │
        ▼   per-source CONNECTOR / ADAPTER  (§7.1)
   raw source records
        │
        ▼   NORMALIZE → canonical InstrumentMaster / CorporateAction  (§7.2)
   canonical records
        │
        ▼   VALIDATE + RECONCILE (survivorship, cross-source conflict)  (§7.4)
        │
        ▼   append to EFFECTIVE-DATED GOLDEN-SOURCE STORE (celnet-journal-backed)  (§7.3)
        │
        ▼   EVENT-APPLICATION LIFECYCLE (announce → confirm → apply)  (§7.5)
        │
        ▼   feeds PRICING + HEDGING (§8)
```

### 7.1 Connector / adapter model (per source)

One adapter per source, behind a common `RefDataSource` / `CorpActionSource` trait (the trait
discipline `InventorySource` already uses). Each adapter owns only *fetch + parse to raw*; it
never touches the store directly. Adapter families:
- **Open pull adapters** — HTTP/REST + file pull on a schedule (FiscalData, DMO, H.15, ECB
  SDMX, GLEIF, OpenFIGI, EDGAR). Deterministic, cache-friendly, no licence constraints.
- **Message adapters** — parse ISO 15022 **MT 564/565/566** and ISO 20022 **seev.031/033/036**
  (+ 037/039). Reuse the framing/dialect discipline of `celnet-fix`.
- **Pluggable vendor CA adapter** — the same trait, implemented against a customer-licensed
  feed. **Ships as an interface with no bundled commercial data** (guardrail 7). This is the
  only seam through which comprehensive corporate CA enters, and it is customer-wired.

### 7.2 Normalization

Each adapter maps its raw record onto the canonical `InstrumentMaster` / `CorporateAction`
(§6). Govvie adapters *derive* schedules from issuance terms via the existing schedule
primitives (`CashflowSchedule::from_bond`) rather than expecting a fed schedule. All
convention values land as stable snake_case labels (the discipline `GovBondSpec` +
`reference_data.rs` label helpers already use), so the store stays human-editable and the
server maps labels → its enums without the store depending on server types.

### 7.3 Effective-dated, append-only golden-source store

The mutable evolution of `celnet-refdata`: a keyed registry (by internal `instrument_id`) whose
every write is an **append** with `(source, valid_from, recorded_at, quality)`, backed by
`celnet-journal` for durability + torn-tail-safe recovery, and JSON-snapshotted like the
identity store for admin editing. Reads resolve the **effective** record for a valuation date
(bitemporal); a CA reversal/cancellation appends a superseding version rather than deleting —
so a wrongly-applied redemption can be un-applied and history stays auditable (§4.3, §10.2).

### 7.4 Validation & reconciliation

- **Structural validation** — ISO-6166 ISIN check digit (existing `isin.rs`), known day-count /
  frequency / calendar labels (existing `reference_data.rs` `*_is_known` helpers), real
  calendar dates (existing `CivilYmd::is_valid`).
- **Cross-source reconciliation** — when two sources disagree on a field, apply the §4.3
  survivorship rules (source priority + quality) and record the conflict + winner in lineage.
- **Economic validation** (guardrail 5) — a coupon-bearing record must round-trip
  price↔yield on the real `celnet-bond` leaf (the oracle test `celnet-refdata` already runs
  on its curated universe) before it is admitted.

### 7.5 Event-application lifecycle

The state machine §1.3 implies, driving the §8 sinks:
1. **Announce** (MT 564 / seev.031) → append a `CorporateAction` (status `announced`); update
   the instrument's schedules (call/put/sink) if the event carries schedule terms.
2. **Elect** (VOLU/CHOS only) → the desk resolves its election before `response_deadline`
   (MT 565 / seev.033); status `elected`.
3. **Confirm** (MT 566 / seev.036) at payment/record dates → status `confirmed`, then
   **apply** the position effect (§8) and mark `applied`.
4. **Reverse / cancel** (seev.037 / seev.039) → append a superseding record; un-apply the
   position effect via the same booking path.
MAND events skip step 2 and auto-apply at the entitlement dates.

---

## 8. How it feeds hedging & pricing — the payoff (cited seams)

The whole point: mastered data + applied CA events drive schedule-accurate pricing, curve/DV01
hedging, and position/inventory correctness.

- **Schedule-accurate bond pricing (`celnet-bond`).** Call/put/sink/amortisation + FRN reset
  schedules from §6.1 extend the **single** expansion point `CashflowSchedule::from_bond`
  (`crates/celnet-bond/src/schedule.rs:55-126`) — so clean/dirty price, accrued, YTM, DV01,
  duration, convexity (`risk.rs:79-122`) all re-derive off the **post-event** schedule
  automatically. A make-whole call is priced against the reference curve (§8 curve seam). This
  is the §10.1/§10.2 pricing correctness, now with real data behind it.
- **Curve construction & DV01 / key-rate hedging (`celnet-rates`).** Open curve observables
  (§2.1: H.15 / ECB / FiscalData / DMO) feed `bootstrap_curve`
  (`crates/celnet-rates/src/bootstrap.rs:233`); the resulting `Curve` (`curve.rs:86-96`) prices
  bonds and make-whole calls, and the signed key-rate ladder (`celnet-rates-risk` `ladder.rs`
  `key_rate_ladder` / `signed_parallel_dv01`, with `normalize_bond_dv01` reconciling the bond
  sign convention) computes the hedge. A call/put shortening effective maturity re-derives the
  ladder off the post-event schedule (§10.2 negative convexity).
- **Inventory / position adjustment on ex-date / pay-date.** A confirmed CA (§7.5) drives the
  existing booking sinks: REDM/MCAL **realise** (position → 0) and PCAL/PRED/DRAW/PPMT **scale**
  (nominal ↓ / pool factor) via `RatesPositionStore::book`
  (`crates/celnet-server/src/services/rates_book.rs:74`; book path
  `services/risk/mod.rs:1019-1046`, `:674-690` per §10.4) with a CA-sourced position delta; the
  flat `InstrumentInventory` (`aggregation.rs:97-133`) scales in lockstep, so the
  inventory-skew pricing feature reads the corrected position. No parallel store.
- **Analytics / mark-to-market (double-count guard).** The §4.2 analytics fold gains a
  **CA event class** joined by ISIN so a redemption/coupon is attributed as a CA, **not** a
  trade — the §10.2 double-count guard, now fed by real confirmed events.

---

## 9. OSS-licensing verdict table (guardrail 7)

| Item | Licence / posture | Runtime dep? | Celnet use |
|---|---|---|---|
| ISO 15022 MT 564/565/566, ISO 20022 `seev.031/033/036/037/039` | Open standard | ❌ (adopt vocabulary/parse) | CA message contract (§1) |
| CAEV / CAMV code sets, ISO 20022 External Code Sets | Open standard | ❌ (adopt code set) | Internal event enum + mandatory/voluntary lifecycle (§1.3) |
| US Treasury FiscalData / TreasuryDirect | Public domain, no key | ✅ pull | Derive US govvie schedules (§2.1) |
| UK DMO "Gilts in Issue" | Free access (⚠ OGL grant unconfirmed) | ✅ pull (verify reuse) | Derive gilt schedules (§2.1) |
| Fed H.15 | Public domain | ✅ pull | USD curve observables |
| ECB Data Portal (yield curves) | Free access **and free reuse** (disclaimer) | ✅ pull | EUR curve observables |
| FRED | Free API, **key + terms** (non-commercial / third-party © on some series) | ⚠ optional convenience | Curve-observable mirror — read terms |
| GLEIF LEI | **CC0 (open)** | ✅ | Issuer / entity identity (§4.2) |
| OpenFIGI / FIGI | **MIT / free** | ✅ | Vendor-neutral instrument identity (§4.2) |
| FINRA TRACE | Free-to-view; downloads delayed; real-time is commercial | ⚠ delayed reference only | US corp liquidity/price reference (§2.1) |
| SEC EDGAR | Free / public | ✅ pull (heavy parse) | Closest free primary US corp-CA source (§2.1) |
| ISIN (ISO 6166) allocation | NNA-issued (mostly free); consolidated DB is commercial | ✅ validate check digit; ❌ commercial DB | Primary external id (§4.2) |
| **CUSIP data** | **Proprietary / licensed (CGS)** | ❌ never bulk-license | Carry natively-present field only (§4.2) |
| QuantLib | **modified / 3-clause BSD** | ❌ (oracle, not linked in core) | Validation oracle for schedules/spreads/OAS/curves (§4.1) |
| **ICE / SIX / Bloomberg / LSEG-Refinitiv / S&P / FactSet / CSD CA data** | **Commercial** | ❌ | Comprehensive corporate-CA feeds — **reference-only**, pluggable adapter, customer-licensed (§2.2) |

---

## 10. Phased plan & crate/workstream breakdown (parallel-safe, disjoint files)

> **Status as of 2026-08-12** (see the header amendment): **P0 ✅** · **P1 ✅ shipped** ·
> **P2 ⚠ partial** (only `GovvieSource`; no open pull adapters) · **P3 ⚠ partial** (lifecycle +
> apply shipped; MT 564 / `seev.*` parsers absent) · **P4 ❌ NOT shipped** (the applied CA never
> reaches the live pricer — this is gap **G1** in
> [`CORPORATE-ACTION-MONITOR-GAP-ANALYSIS.md`](CORPORATE-ACTION-MONITOR-GAP-ANALYSIS.md) §2, and
> its **CA-P1** is the phase that closes it) · **P5 ❌ NOT shipped**.

- **P0 — this spec + ADRs.** Confirm §11 open items. ADRs: the `celnet-refdata` static→mastered
  evolution (or a new `celnet-refstore`); the `celnet-corpactions` crate boundary; the
  survivorship/effective-dating model; the pluggable vendor-CA adapter interface (guardrail-7
  boundary).
- **P1 — canonical model + mastered store.** `celnet-corpactions` (NEW leaf): `CorporateAction`
  + CAEV/CAMV + the pure schedule/position-effect functions (no server dep). Evolve
  `celnet-refdata` (or `celnet-refstore` NEW): effective-dated, append-only, `celnet-journal`-
  backed golden store + survivorship + lineage; JSON-snapshot admin persistence (identity-store
  chassis). Oracle: the existing price↔yield round-trip gate extended to mastered records.
- **P2 — open connectors (deterministic govvie/rates).** Pull adapters: FiscalData, DMO, H.15,
  ECB, GLEIF, OpenFIGI. Normalize → derive schedules via `CashflowSchedule::from_bond`. This is
  the **complete OSS govvie/rates path** — validated end-to-end before any corporate feed.
- **P3 — CA message adapters + lifecycle.** MT 564/565/566 + seev.031/033/036/037/039 parsers
  (reuse `celnet-fix` framing discipline); the announce→elect→confirm→apply state machine
  (§7.5) driving `RatesPositionStore::book` realise/scale + `InstrumentInventory` (§8).
  Validated against the §10 hand truth table (each CAEV → expected inventory/P&L/DV01 delta).
- **P4 — pricing/hedging wiring.** Schedule extension (call/put/sink/FRN reset) on the bond
  leaf; make-whole call priced off the curve; key-rate ladder re-derive post-event; CA-aware
  analytics fold (double-count guard). Live-verified.
- **P5 — pluggable vendor-CA adapter interface + corporate universe.** Ship the customer-wired
  adapter trait + docs; no bundled commercial data. EDGAR best-effort US corporate-CA extractor
  as the OSS-only stopgap, clearly bounded.

Each phase gated (`just t1` per crate; `just t2` at land). Numerical/lifecycle results validated
against an independent reference (QuantLib oracle / hand truth table), never merely asserted
plausible (guardrail 5).

## 11. Naming (guardrail 8) & open items

**Proposed vendor-neutral names:**
- **`celnet-corpactions`** (NEW leaf) — the CAEV/CAMV `CorporateAction` record + the pure
  schedule/position-effect functions. Purpose-named; no vendor/CSD/product name.
- **`celnet-refstore`** (NEW) *or* evolve **`celnet-refdata`** in place — the effective-dated,
  append-only, mastered golden-source security-master registry. Recommend **evolving
  `celnet-refdata`** (it already *is* "reference data"; the static curated universe becomes the
  OSS seed that bootstraps the mastered store) to avoid a redundant crate — **confirm in P0**.
- Ingestion adapters live as a server module (e.g. `services/refdata_ingest`) behind
  `RefDataSource` / `CorpActionSource` traits — not a crate, since they carry server/IO deps.

**Open items for the next review:**
- **Crate boundary:** evolve `celnet-refdata` vs a new `celnet-refstore` (recommend evolve).
- **Store durability:** `celnet-journal`-backed append log + JSON snapshot vs an embedded
  key-value store — bitemporal query cost at investment-bank universe scale (guardrail 6).
- **Survivorship policy config:** where per-field source-priority rules live (admin GUI, like
  pricing groups?) and their default ordering (official issuer/DMO/Treasury > vendor > derived).
- **Election surface:** does the desk resolve VOLU/CHOS elections in the FI GUI (a CA inbox,
  mirroring the RFQ desk inbox), and how it gates on capability/desk (permissions track).
- **Vendor-CA adapter contract:** the exact trait shape a customer implements, and the
  conformance test suite that proves an adapter feeds the same canonical schema.
- **DTC ISO 20022 migration calendar:** confirm phase dates against the DTCC "Getting Started"
  PDF before any doc cites them (research flag).
- **UK DMO reuse licence:** confirm an explicit open-licence (OGL) grant before redistributing
  DMO-derived data (research flag).
- **FRED vs open-only:** whether to depend on FRED at all given its non-CC0 terms, or restrict
  to public-domain H.15 / ECB-free-reuse sources.

---

### Appendix — full source list

**CA standards / messaging:** [MT 564 handbook](https://www.iso20022.org/15022/uhb/finmt564.htm) ·
[SWIFT MT 564–568 training](https://www.swift.com/myswift/services/training/swift-training-catalogue/browse-swift-training-catalogue/corporate-actions-iso-15022-mt-564-mt-565-mt-566-mt-567-mt-568) ·
[ISO 20022 seev catalogue](https://www.iotafinance.com/en/SWIFT-ISO20022-Business-area-seev-Securities-Events.html) ·
[seev.031 detail](https://www.iotafinance.com/en/SWIFT-ISO20022-Message-seev-031-001-Corporate-Action-Notification.html) ·
[ISO 20022 message archive](https://www.iso20022.org/catalogue-messages/iso-20022-messages-archive) ·
[CAEV code set](https://www.iso20022.org/standardsrepository/public/wqt/Description/mx/dico/codesets/_bHRNZtp-Ed-ak6NoX_4Aeg_-1565392153) ·
[CAMV code set](https://www.iso20022.org/standardsrepository/public/wqt/Description/mx/dico/codesets/_bKsU1dp-Ed-ak6NoX_4Aeg_735779459) ·
[ISO 20022 External Code Sets](https://www.iso20022.org/catalogue-messages/additional-content-messages/external-code-sets) ·
[ISITC CA Market Practice v8.0](https://isitc.org/wp-content/uploads/Corporate_Actions_Market_Practice_v8.0_Dec2022.pdf) ·
[ECB ISO 15022 vs 20022 CA/PV](https://www.ecb.europa.eu/paym/groups/shared/docs/8846a-cmh-tf-2018-01-16-presentation-swift-assetservicing-ca-pv-messages.pdf) ·
[DTCC Getting Started CA ISO 20022](https://www.dtcc.com/-/media/Files/Downloads/issues/Corporate-Actions-Transformation/Getting_Started_CA_ISO_20022.pdf) ·
[DTCC Redemptions UG](https://www.dtcc.com/-/media/Files/Downloads/issues/Corporate-Actions-Transformation/ISO_20022_RedemEntAlloc_UG_FINAL.pdf) ·
[DTCC CA 20022 service](https://dtcclearning.com/products-and-services/dtcc-data-services/ca-20022-service.html) ·
[ECB AMI-SeCo CA standards](https://www.ecb.europa.eu/press/intro/publications/pdf/ecb.amiseco202312_corporateactions.en.pdf).

**Open data:** [FiscalData auctions](https://fiscaldata.treasury.gov/datasets/treasury-securities-auctions-data/) ·
[FiscalData API](https://fiscaldata.treasury.gov/api-documentation/) ·
[TreasuryDirect auction query](https://www.treasurydirect.gov/auctions/auction-query/auction-query-help/) ·
[DMO Gilts in Issue](https://www.dmo.gov.uk/data/gilt-market/gilts-in-issue/) ·
[Fed H.15](https://www.federalreserve.gov/releases/h15/) ·
[ECB euro-area yield curves](https://www.ecb.europa.eu/stats/financial_markets_and_interest_rates/euro_area_yield_curves/html/index.en.html) ·
[ECB data access (SDMX)](https://www.ecb.europa.eu/stats/accessing-our-data/html/index.en.html) ·
[FRED API key](https://fred.stlouisfed.org/docs/api/api_key.html) · [FRED terms](https://fred.stlouisfed.org/docs/api/terms_of_use.html) ·
[GLEIF open data (CC0)](https://www.gleif.org/en/about/open-data) · [GLEIF API](https://www.gleif.org/en/lei-data/gleif-api) ·
[OpenFIGI features/license](https://www.openfigi.com/about/features) ·
[FINRA TRACE](https://www.finra.org/filing-reporting/trace) · [FINRA Rule 7730](https://www.finra.org/rules-guidance/rulebooks/finra-rules/7730) ·
[SEC EDGAR search](https://www.sec.gov/edgar/search/).

**Identifiers / commercial / mastering / analytics:** [ANNA identifiers](https://anna-web.org/identifiers/) ·
[CGS licence fees (CUSIP)](https://www.cusip.com/services/license-fees.html) ·
[ICE Corporate Actions](https://www.ice.com/fixed-income-data-services/data-and-analytics/pricing-and-analytics/reference-data/corporate-actions) ·
[SIX Global Corporate Actions](https://www.six-group.com/en/products-services/financial-information/market-reference-data/corporate-actions-data.html) ·
[LSEG reference data](https://www.lseg.com/en/data-analytics/market-data/data-analytics-pricing/reference-data) ·
[securities master / golden copy](https://devancore.com/glossary/securities-master-data/) ·
[MDM survivorship](https://profisee.com/blog/mdm-survivorship/) ·
[security-master reconciliation](https://www.greshamtech.com/blog/security-master-modernization) ·
[QuantLib license (modified BSD)](https://www.quantlib.org/license.shtml).
