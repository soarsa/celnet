# Vendor FIX Adapter Inventory

> Source of truth for the 122 vendor folders under `~/Downloads/Vendors/` that
> the Celnet FIX platform may need to integrate. Each row captures what we
> have on disk today — not what is built. See `VENDOR_ADAPTERS_PLAN.md` for the
> sequenced delivery plan that consumes this inventory.

**Last walked:** 2026-06-08 (122 vendor folders + 2 loose PDFs).
**Currently implemented in `backend-rust/src/adapter/`:** 3 of ~150 logical adapters
(`bloomberg_fxgo_order`, `bloomberg_fxgo_price`, `rabofx_esp_order`).

---

## Coverage summary

| Tier | Count | Definition |
|------|------:|------------|
| T1   | 21 | Must-have: top bank LPs + dominant FX ECNs/aggregators. Drives revenue, drives sales conversations. |
| T2   | 24 | Important: secondary banks, regional dealers, established aggregators. Won on case-by-case basis. |
| T3   | 50 | Long-tail: smaller venues, niche platforms, specialist LPs. Build on customer demand. |
| T4   | 27 | Skip / triage: empty folders, vendor libraries (not FIX), REST-only APIs, non-FX scope, duplicates, agreements-only. |

Implementation effort scales with **products per vendor**, not vendor count.
A Tier-1 dealer like Citi means 4–6 adapter modules (ESP, RFS, STP, COLO,
Forwards, MiFID variants); a Tier-3 venue is usually 1–2.

**Logical adapter count target (T1+T2+T3 implemented):** ~150 modules. See plan
doc for sequencing.

---

## Tier 1 — Must-have (21 vendors)

Bank LPs and ECNs/aggregators that anchor every institutional FX deployment.

| vendor | classification | products | fix_versions | conn types | XML | cert doc | key specs | notes |
|--------|----------------|----------|--------------|------------|-----|----------|-----------|-------|
| Citi | Bank LP | Spot ESP, RFS, STP, COLO, Forwards, Batch | 4.4–7.0 | ORDER, PRICE, RFQ, STP | no | yes | Citi FxFIX COLO Spot v7.0.16, RFS v4.9, STP v1.8 | 6+ distinct integration guides, MiFID variants |
| JPM | Bank LP | Spot, FX Algo, Commodities | 4.4 | ORDER | no | no | JPMC FX Algo v1-83, FX v1-3-41, Commod v1.0.0 | Multiple algo spec versions |
| Goldmans | Bank LP | Spot, RFQ, Algo | 5.0, 5.5, 7.0–7.26 | ORDER, RFQ | no | no | GS FX FIX Spec v7.26 | Major version progression |
| UBS | Bank LP | B2B, FX Execution, Algo, Liquidity, MD | 1.27–2.6 | ORDER, PRICE | no | yes | UBS FX Algo API v2.4, B2B RoE v1.35, Conformance Guide | Many RoE + algo versions, cert docs |
| BAML | Bank LP | InstinctFX Spot, Options | 9.7.6–9.7.7 | ORDER, PRICE | no | no | InstinctFX FIX Spec v9.7.6.15, BAMLX v1.0 | MiFID II variant |
| BOFA | Bank LP | FXtransact, InstinctFX, DropCopy | 4.3, 9.4 | ORDER, PRICE | no | yes | FXO v1.25, FXtransact Cert v1.0, InstinctFX v9.4.3 | Cert plan present |
| BNP Paribas | Bank LP | eFX RFQ, Streaming Prices | 1.1–1.3 | PRICE, RFQ | no | no | eFX FIX RFQ v1.2.7, Streaming v1.3.8 | Both RFQ and streaming |
| Deutsche Bank | Bank LP | AutobahnFX Spot/Fwds/NDF/Swaps/Algo/RAPID | 1.7, 2.3, 3.2–3.5 | ORDER, PRICE, RFQ, STP | no | no | Autobahn (Single-Leg) 3.5.5, Autobahn RAPID, Autobahn Algo 2.3 | Multiple product lines (RAPID/Algo/Standard/FORT) |
| Credit Suisse | Bank LP | RFS, SER Spot, Client | 1.8, 2.0 | ORDER, PRICE, RFQ, STP | no | no | CS_FIX_RFS_2.0.6a, CS_FIX_SER-SPOT, CS_Fix_Client_Specification | FX Outrights focus |
| MorganStanley | Bank LP | Spot, Laddered Pricing, Algo, MD | (unspecified) | ORDER, PRICE | no | yes | FX FIX Core API, Implementation Details, Laddered Pricing | Algo conformance, MiFID II variant |
| Barclays | Bank LP | BARX Spot, Forwards | 4.6 | ORDER, PRICE | no | no | BARX FX ROE 4.6.1, FIX Orders ROE v3.2 | MiFID II addon |
| HSBC | Bank LP | RFSQ, FXFO, Market Data | 4.2, 5.0+ | PRICE, RFQ | no | no | HSBC FX RFSQ v8.6.8, MDS v3, FXFO v8.46 | 3 distinct APIs (RFSQ, MDS, FXFO) |
| EBS | ECN/Venue | Spot, NDF, Forwards, Swaps, Maker, Direct, Brokertec | 2.0, 4.4 | ORDER, PRICE, RFQ | no | no | EBSBrokertec Gateway v4.4, Ai FIX Messages 2.0.3, EBS Maker 2.3 | Multiple platforms (Brokertec/Direct/Ai), MiFID variants |
| HOTSPOT | ECN/Venue | Spot, Forwards, NDF, Swaps, BookFeed, Maker | 4.0+ | ORDER, PRICE | no | yes | FIXProxy Spec, Forwards-Swaps OE, BookFeed Spec | Multi-product, cert docs |
| FXAll | Aggregator | Spot, Algo, STP, TCPI | 4.4, 4.5 | ORDER, STP | no | no | FXall FX Trading v4.5, TR Liquidity Provider, TCPI Guide | Multiple protocols (TCPI + FIX), Refinitiv-owned |
| Currenex | Aggregator | Spot ESP, RFQ Maker/Taker, Orders, STP | 4.2, 4.4 | ORDER, PRICE, RFQ, STP | no | yes | Currenex Full Spec, RFQ Maker 3.0, Order Spec | Conformance test plans + maker conformance xlsx |
| 360T | Aggregator | RFQ Maker, Order MM, STP, SuperSonicTex, Algo | 1.8–1.20 | RFQ, ORDER, STP | no | yes | RFQ MM v1.20.2, Order MM v6.0, SEP API Acceptance | Many MM versions, MiFID II |
| FXSpotStream | Aggregator | Spot, Algo, Drop Copy | 1.4, 1.5 | ORDER, PRICE | no | no | FSS FIX ROE v1.4.4, v1.5.2, Algo Pre-Post | Proprietary ROE |
| FastMatch | ECN/Venue | Spot, Voice, Stream | 3.4, 3.5, 4.0 | ORDER, PRICE | no | no | FASTMATCH Client 4.0.1, 3.5.3, Stream 2.3 | Multiple client versions |
| LMAX | ECN/Venue | Spot, Forward, Swap, RFS, OE | 4.2, 4.4 | ORDER, PRICE | no | yes | LMAX Exchange FIX 4.2 v2.8.6, MTF Drop Copy v2.3 | Conformance template |
| Bloomberg | MD Vendor + OMS | FXGO Order, FXGO Price, Algo, RFS, Batch | 4.1–5.0 | ORDER, PRICE, RFQ | yes | yes | FIXbook Sell Side 5.0.21, FXGO FIXbook Sellside, blpapi SDK | **Partially implemented** (FXGO Order + Price); algo/batch outstanding |

---

## Tier 2 — Important (24 vendors)

Secondary banks, regional dealers, established aggregators, vendor-library deps.

| vendor | classification | products | fix_versions | conn types | XML | cert doc | notes |
|--------|----------------|----------|--------------|------------|-----|----------|-------|
| RBS | Bank LP | NatWest FX Spot, Prime Brokerage, MD | 5.0 | ORDER, PRICE | no | yes | NatWest Markets RoE v5.0.28, SSL guidance |
| Societe Generale | Bank LP | SGCIB Spot | 4.4 | PRICE, ORDER | no | no | SGCIB FIX 4.4 v1.3.g |
| StandardChart | Bank LP | S2BX FX, SCALE | 4.4, 4.7 | ORDER, PRICE | no | yes | S2BX RoE v4.7, SCALEFX UAT |
| SEB | Bank LP | Spot FX, MD, OMS | 4.3 | PRICE, ORDER | no | yes | SEB FIX 4.3 v3.40, OMS45-FOTA Dev Guide |
| ANZ | Bank LP | RFS Spot | (unspecified) | PRICE, RFQ | no | no | DAPI RFS v1.5.6–v2.5 |
| Commerzbank | Bank LP | Spot, Forwards, Swaps (Commander) | 1.10–1.15 | ORDER, PRICE, RFQ, STP | no | no | Commander RoE V1.15g, V1.13b |
| MUFG | Bank LP | Spot, Forwards, Laddered, RFQ | 2.0 | ORDER, PRICE, RFQ | no | no | MUFG-eFX-API-2.0 (REST-flavoured) |
| Rabobank | Bank LP | RFS, RFQ, Spot, Limit Orders | (HTML) | RFQ, ORDER | no | no | **Implemented**: ESP Order. RFS outstanding. |
| Nomura | Bank LP | Spot, RFQ, Streaming, Broker | (unspecified) | ORDER, PRICE, RFQ | no | yes | RFQ RoE v1.7, Streaming RoE v1.5 |
| UOB | Bank LP | FX Spot integration | 1.0 | ORDER, PRICE | no | no | UOB FX API Integration Guide v1.0 |
| Integral | Aggregator | Spot, Swaps, RFS, MD, OE, ESP, Maker, Taker | 4.0, 4.10, 5.0, 5.6 | ORDER, RFQ, PRICE | yes | yes | Maker RFS v2.1, Taker v6.7, ESP Guide; 93 files |
| bidFx | Aggregator | Spot, MD (multi-bank) | 4.4 | PRICE, ORDER | no | no | BidFX 4.4 ROE, TradingScreen FIX API |
| FXCONNECT | Aggregator | Spot, RFS, RFQ (StateStreet) | 4.0+ | RFQ, PRICE | no | no | FXC Universal Pricing API RFSMV (MTF updates) |
| FXCM PB | Aggregator | Spot, STP (FXCMPro) | 4.4 | STP | no | no | FXCMPRO FIX STP Publisher v1.0 |
| ICAP | ECN/Venue | Forwards, NDF, Swaps (TP ICAP) | 5.0 | ORDER | no | no | TP ICAP NDF FIX 5.0 v3.0.6 |
| Talos | Crypto Venue | Spot Crypto | 9.21 | PRICE, ORDER | no | no | Talos FIX API 9.21 |
| B2C2 | Crypto Venue | Spot, Derivatives Crypto | 2.5 | ORDER, PRICE | no | no | B2C2 FIX API v2.5.4 |
| PrimeXM | Aggregator | Spot, CFD, Algo, Trading | 4.4 | ORDER, PRICE | no | yes | PrimeXM FIX 4.4 Trading API v1.5.7, Saxo Cert List |
| oneZero | Aggregator | Spot, CFD, Forward, Algo, Maker, Taker | 4.4 | ORDER, PRICE | no | yes | oneZero FIX 4.4 v1.9.0, External FIX 4.4 v1.1.2 |
| smartTrade | Aggregator | Maker, LiquidityFX | (unspecified) | ORDER, PRICE | no | no | smartTrade Maker RoE v1.0.2, LiquidityFX v3.2.3.0 |
| Cobalt | Aggregator | FX Spot | 1.8 | PRICE, STP | no | no | FIXRulesOfEngagement v1.8.18 |
| Tradition | Aggregator/MD | Market Data Feed (TCDF) | (unspecified) | PRICE | no | no | tcdf_specification, tcdf_feedlib |
| Reuters-Refinitiv | MD Vendor | Trade Notification, Pricing | 5.0 | PRICE | no | no | TN FIX Gateway 5.0 SP1, RET-AD 3.4 Pricing |
| IEX | ECN/Venue (equities) | Equities Order Entry | 4.2 | ORDER | no | yes | IEX FIX Spec, Cert Tests | Equities — only if scope expands beyond FX |

---

## Tier 3 — Long-tail (50 vendors)

Smaller venues, regional platforms, specialist LPs. Build on customer demand.

| vendor | classification | products | conn types | cert doc | notes |
|--------|----------------|----------|------------|----------|-------|
| 11B (24Exchange Maker) | ECN/Venue | Spot FX 4.4 Taker | PRICE | no | 24Exchange ROE v1.05–1.11 |
| 24X (24Exchange Celnet) | ECN/Venue | Spot ESP, RFQ | ORDER, PRICE, RFQ | yes | 24X-Celnet Cert ESP-RFQ-TAKER |
| BAMLX | (covered under BAML) | — | — | — | folded into BAML T1 |
| BGC | Broker | FX Spot, Voice RFQ | PRICE, RFQ | no | BGC & GFI FIX Spec v1.2.1, voice-to-trade |
| bidFx | (covered T2) | — | — | — | — |
| Bierbaum | OMS/EMS | STP, Client OE | ORDER, STP | no | BierbaumPro Client v2.0.5, STP v1.3 |
| Broadway | OMS/EMS | FIX Protocol | ORDER | no | Broadway FIX Client Protocol 2.0 R08 |
| CCM Alpha Fund | Hedge Fund | FIX Protocol | ORDER | no | CCM_FIX_Spec_v2.3 |
| CFH Clearing | Clearing | Clearing, Settlement | ORDER | no | CFH Clearing FIX API v2.5 |
| Citadel | Hedge Fund | FX Order Execution | ORDER | no | Citadel FIX Order Spec FX v1.12 |
| CMC markets | Retail Broker | Spot FX, CFDs | ORDER, PRICE | no | FIX API User Guide 6.1.10 |
| Datatec | Vendor | Spot Trading (SMF) | ORDER, PRICE | no | 20160713 FIX Implementation |
| Edgewater - Vision | MD Vendor | EdgeFX, NDF, Top of Book | PRICE | yes | EdgeFX v1-10, NDF API, ToB API |
| fidessa | OMS/EMS | Buy-side Order Flow | ORDER | no | FO 17.2.9 + FO 20.2 RoE |
| First Derivatives (ADS, R5) | Aggregator | Liquidity, Price, PoP Drop Copy | ORDER, PRICE, RFQ | yes | FIX-Liquidity-Provider 4.2, Taker API 4.0 R3 |
| FlexTrade | OMS/EMS | Order Mgmt, Block Trading, Algo | ORDER | yes | Flextrade API 4.4, 4.2, ISAM FX |
| Fluent | ECN/Venue | Spot | ORDER, PRICE | no | FluentStream FIXSpecs v3.9.1.2 |
| Fortex | ECN/Venue | Spot | ORDER | no | FortexFIX_Direct V1.6 |
| FxOpen | Crypto/FX | Spot, Crypto | ORDER | no | **Has QuickFIX XML** (FIX44 ext.1.59.xml) |
| GainGTX | ECN/Venue | Spot, OE | ORDER | no | GainGTX FIX RoE, GTX FIX LP |
| GKFX | (PrimeXM rebrand) | Spot | ORDER | no | PrimeXM FIX 4.4 v1.5.3 |
| Isprime | Bank LP | Spot | ORDER | no | ISPrime FX FIX Client Protocol v1.0 |
| Jeffries Mahi | Prime Broker | Spot, Pricing | ORDER, PRICE | no | MFX Compass Client Pricing Orders |
| Jump | Liquidity Provider | Counterparty trading | ORDER | yes | JLQD FIX Spec v2.7.2, Counterparty v2.2 |
| Kondor | OMS/EMS | Spot, Fwds, FXO, Swaps | ORDER | yes (XML scenarios) | 16 XML scenario files (4 products × 4 scenarios) |
| Lucera / Lumefx | Aggregator | Spot, Forward, RFQ, MD, OE | ORDER, PRICE, RFQ | yes | Lumefx Client 1.5, 2.1, 2.6 |
| LucidMarkets | ECN/Venue | Spot, Forward | ORDER, PRICE | no | Lucid Markets FX FIX Specification |
| mahifx | Aggregator | Spot, MD | ORDER, PRICE | no | mfxcompass-integration-guide |
| Mako | OMS/EMS | Spot, Forward, OE, Trade Capture | ORDER, PRICE | yes | Mako FX Trader Spec, LiquidityPool Trader |
| MarketFactory | Aggregator | Spot, MD, OE | ORDER, PRICE | no | mffix.pdf, apidoc.pdf, mmapidoc.pdf |
| Markit | MD Vendor | Market Data, Ref Data | PRICE | no | MarkitSERV FX FIX v1.15 |
| MexTrader | Prime Broker | Spot, Algo (REST) | ORDER, PRICE | no | MexTrader API v1.2 — non-FIX (REST) |
| NewChangeFX | Aggregator | Spot, Forward, STP | ORDER, PRICE, STP | yes | FiX Spec Feb 2019, API Conformance Test |
| ParFX | ECN/Venue | MD, OE | PRICE, ORDER | no | ParFX MD 2.0.0, OE 2.0.6 |
| PriceMarkets | ECN/Venue | MD Protocol | PRICE | no | priceMARKETSFIXProtocol |
| QuantitiveBrokers | Bank LP | Spot, OTC, Forwards | ORDER, PRICE | no | QB FIX Specifications |
| RCMX | ECN/Venue (futures) | Futures Algos | ORDER | no | RCMFuturesAlgosFIXManual |
| ReactiveMarkets | ECN/Venue | Maker, MD | PRICE, ORDER | no | Maker API v1-4-8 |
| Solid FX | Aggregator | Spot, LP Interface | PRICE, ORDER | no | Solid FX FIX v2.4.1, LP Interface v1.8 |
| Spectra | ECN/Venue | Spot, SPAX | ORDER, PRICE | yes (XML) | **FIX44_SPAX_TRADING.xml** present |
| Spotex | ECN/Venue | Spot, Last Look | ORDER, PRICE | no | Spotex Spec + LastLook Spec |
| StateStreet | Prime Broker | OE Custody | ORDER | yes | State Street FIX Order Spec v1.9.0 |
| Synoption | Crypto Venue | OTC FX, Crypto Options Taker | ORDER, RFQ | no | Synoption Taker RoE 1.3 |
| Tera / TerraExchange | Crypto Venue (dupe) | MD Crypto | PRICE | no | TeraExchange MD FIX v1.6 — folders are duplicates |
| Tradair | Aggregator | Taker, RFS | ORDER, RFQ | no | TradAir Taker v1.3.5, RFS v1.0.3 |
| Tradertools | OMS/EMS | OE FIX Message Spec | ORDER | no | TraderTools FIX Spec May 2017 |
| UKFX_Datasoft | OMS/EMS | ESP, RFQ Taker | RFQ, PRICE | yes | Celnet Cert ESP-RFQ-TAKER (Apr 2021) |
| Vidarr | Prime Broker | Capital, Order | ORDER | no | Vidarr Capital FIX Spec v1.3.0 |
| Virtu | ECN/Venue | MD, Spot (VFX) | PRICE, ORDER | no | VFX 2.1.1/2.1.2/2.1.3 |
| Wolverine | ECN/Venue | Spot Liquidity, Trade Data | ORDER, PRICE | no | WFX Spot Liquidity v2.0, Trade Data v3.2 |
| XTX | ECN/Venue | EFX Spot, RFQ, Algo, Fwds, TermTrading | ORDER, RFQ | no | xtx EFX 1.3, 1.6-fwds, algo 1.2 (rich spec set) |
| Netdania | Aggregator | Spot, MD (charting) | ORDER, PRICE | no | NetDania FIX API v1.3 |

---

## Tier 4 — Skip / triage (27)

Empty folders, non-FIX, vendor libraries, duplicates, agreements-only.

| vendor | reason to defer/skip |
|--------|----------------------|
| para | Empty folder |
| TradeWeb | Empty folder |
| Cloudera | Big-data platform; not a FIX vendor (Hadoop product cert doc only) |
| beeks | Network/connectivity infra; not a FIX adapter |
| Sungard | Java JARs + XLS spreadsheets — vendor library, not a FIX integration |
| vela | MAMA library + NDA docs — vendor library |
| interactiveData | Signed agreements only, no technical spec |
| Traiana | Link/reference file only |
| RIPE | Login text file only |
| sivlerLakeCBS | Typo folder; minimal content (one XML text file) |
| TerraExchange | Duplicate of Tera (same file) |
| Cambridge FX | REST/JSON only (Postman collection), not FIX |
| Bitstamp | REST only ("Bitstamp spec url.docx") |
| MexTrader | REST API (docx) — promote to T3 if customer demand |
| MUFG | Listed as 2.0 but spec is REST-flavoured — keep T2 only if FIX confirmed |
| MT5 | MetaTrader 5; spec missing ("request for API document") |
| NationalStockExchange | NNF protocol, not FIX |
| Murex | Download incomplete (.crdownload) — re-acquire spec then T3 |
| FOW | Equity data dump (CSV/ZIP); not a FIX vendor |
| ITarle | Equities/futures-focused — out of FX scope |
| OTL | Multi-exchange (Eurex/ICE/LME); not FX |
| CME | Futures exchange; outside FX scope unless expansion |
| RTNS | Refinitiv Trade Notification adapter — install/config docs only, no FIX spec to implement |
| Cloudera | (already listed) |
| BAMLX | Folded into BAML T1 |
| 11B | Folded — same vendor family as 24X |
| FXEcoSystems | FXGO FIXbook docs — folded into Bloomberg T1 |
| GKFX | PrimeXM rebrand — folded into PrimeXM T2 |
| (Default FIX mapper PDF, loose) | Reference doc, not vendor-specific — use during framework work |
| (TraderTools loose PDF) | Duplicate of Tradertools folder |

---

## Cross-cutting observations

1. **PDFs dominate.** Of 122 vendor folders, only ~4 have any QuickFIX XML on
   disk (Bloomberg, FxOpen, Integral, Spectra, Kondor scenario files). Every
   other adapter starts from a PDF spec — see Plan §3.1 on extraction tooling.

2. **"Cert plan" / "UAT" / "conformance" docs exist for ~17 vendors.** These
   are the easiest to drive an automated conformance harness against because
   the vendor has already enumerated the test cases. Highlights: Citi, BOFA,
   UBS, HOTSPOT, Currenex, 360T, LMAX, Integral, Edgewater, FlexTrade, Mako,
   NewChangeFX, Nomura, PrimeXM, StandardChart, StateStreet, UKFX_Datasoft,
   Jump, MorganStanley.

3. **Versioning is wild.** Citi has v4.9 RFS / v7.0.16 COLO / v1.8 STP all in
   one folder; Goldmans has v5.0–v7.26; UBS has v1.27–v2.6 across multiple
   product lines. Picking which version to embed is a per-vendor decision the
   plan defers to spec-extraction time.

4. **Bank LPs are multi-product, not multi-version.** A single bank typically
   needs 4–6 adapter modules (Spot ESP, RFS/RFQ, Forwards, NDF, Algo, STP,
   sometimes MiFID variants). Tier-1 vendor count (21) ≠ Tier-1 adapter count
   (~70).

5. **Folded duplicates.** 11B↔24X (24Exchange family), BAMLX↔BAML, GKFX↔PrimeXM,
   FXEcoSystems↔Bloomberg FXGO, Tera↔TerraExchange, RTNS↔Reuters-Refinitiv.
   Reflected above.
