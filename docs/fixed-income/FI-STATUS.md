# Fixed Income — implementation status & outstanding features

**Branch:** `feature/fixedincome` · **Updated:** 2026-06-25
**Scope tracked:** the locked P0 (USD-only, linear rates + cash, no vol/credit — see
[`OPEN-QUESTIONS.md`](./OPEN-QUESTIONS.md) D3–D12) plus the cross-asset/UI items.

This is the single source of truth for *what is built vs outstanding*. Each outstanding item is a
**gated slice** (compiles + `clippy -D warnings` + tests + rustfmt; numeric items validated against
QuantLib or a closed-form/structural identity per [`FI-VERIFICATION-CONTRACT.md`](./FI-VERIFICATION-CONTRACT.md)).

---

## ✅ Delivered (gated + pushed to `origin`)

The `celnet-rates` crate — the disjoint-leaf numeric core (depends only on the shared seams):

| Slice | Commit | What | Tests |
|---|---|---|---|
| 1 | `089e63d` | `Curve` — immutable Arc-backed discount/forward snapshot; **log-linear-on-log-DF** interpolation (Q10 default); DF / zero / instantaneous-forward / forward-rate accessors. | 12 |
| 2 | `8190eaf` | `solver::brent_root` — derivative-free bracketing root-finder for calibration. | +6 |
| 3 | `2f12098` | OIS pricing identities (self-discounting par/annuity/PV) + **sequential SOFR bootstrap**. | +9 |
| 4 | `00e4668` | **USD-SOFR date/schedule layer** — `usd_sofr_ois_schedule` (modified-following US calendar, ACT/360 accrual, ACT/365F discount-time); reuses `celnet-calendar`. | +6 |
| 5 | `93a9fd3` | OIS **risk** — `pv01` (analytic annuity), `ois_risk` → DV01 (parallel quote bump) + **key-rate ladder** (per-instrument Jacobian). | +4 |

**Net: a working USD-SOFR rates engine** — bootstrap a curve from dated OIS quotes → price any OIS →
PV / PV01 / DV01 / key-rate ladder. `celnet-rates`: **37/37 tests**, clippy `-D` clean.

GUI (separate from the rates core):

| Commit | What |
|---|---|
| `6c978cf` | **Administration rail section** — Connections + Admin grouped into an admin-gated Administration group, out of the main trading rail (closes the prior admin-visibility gap). 721/721 GUI tests. |

---

## ⏳ Outstanding (the integration phase — each a gated slice)

### A. More rates products (`celnet-rates`)
- **FRA** — single forward-fixing off the curve's simple forward.
- **Vanilla IRS conventions** — frequency/day-count variants beyond the annual OIS already built.
- **STIR & bond futures** — convexity (deterministic placeholder per Q11), CF/CTD/implied-repo.
- **Cash-bond analytics** — yield, **G-spread / Z-spread / ASW**, OAS on option-free (= Z).

### B. Curve completeness (`celnet-rates`)
- **Monotone-convex-on-forwards** interpolation (the smooth-view scheme; log-linear-DF is shipped).
- **Turn-of-year / central-bank-meeting** forward jumps.
- **Deterministic STIR convexity** placeholder wired into the short-end build.

### C. Wire contract (`celnet-proto`, additive — single contract, guardrail #9)
- New arms on the one `celnet.proto`: **`CurveSet`**, **`RatesInstrument`** oneof
  (`fra` · `ois` · `vanilla_swap` · `tenor_basis_swap` · `cross_ccy_basis_swap` · `stir_future` ·
  `bond_future`), **`PricingResult { pv, par_rate, pv01, dv01, key_rate_ladder }`** — next free
  field numbers, no renumber.
- **Golden vectors + parity rows** per arm (`tools/check-verification-coverage.mjs` parses the oneof).

### D. Server (`celnet-server`, `celnet-risk-cube`)
- Pricing service consumes `celnet-rates` for the rates arms.
- **Rates risk folds into the existing server-owned `RiskService`** rollup — clients never loop-sum
  (FI-ARCHITECTURE §3); FRTB GIRR cross-checked vs ORE.

### E. FIX fixed-income dialect (`celnet-fix`) — **"FIX API supports FI"**
- NEW `crates/celnet-fix/src/dialect_rates.rs` mirroring `dialect_fx.rs`: decode inbound FI
  **RFQ**, **RFS-stream subscribe**, and **order** messages → route to the rates engine → quote / fill.
- Gated by a loopback FIX initiator (mirror `tests/fix_acceptor.rs`).

### F. Five-client parity (slice 9)
- **GUI** — the D2 **Options | Fixed-Income asset-class tab layer** (above the rail) + the FI
  workspace set (Curve · Ticket · RFQ · IOI · RFS · Blotter), per FI-ARCHITECTURE §4 and the
  [`mockups/`](./mockups/). *Real-GUI implementation pending the proto arms it renders.*
- **Excel** add-in — `CELNET.*` rates functions (curve DF, swap PV, par, PV01/DV01, key-rate).
- **Rust SDK** (`celnet-client`) — typed builders for the rates instrument vocab.
- **FIX** — the dialect in (E).
- **Federation** — rates pricing/risk fans out across shards.

---

## 🚫 Deferred (locked, **not** half-built — see OPEN-QUESTIONS Q3/Q7)
- **Rates vol** (`celnet-rates-vol`: swaptions/caps, normal/shifted-SABR cube, 1F Gaussian) — Q7.
- **Credit** (`celnet-credit`: single-name + index CDS) — Q3 (low appetite).
- Inflation (deflation-floor ILB), full multi-CSA / CTD-collateral, callable/putable OAS.
- Multi-currency generalisation + cross-currency basis (post-USD, hard currencies first — Q1).

---

## UI changes — explicit status
- **Administration tab:** ✅ done (`6c978cf`).
- **FI asset-class tabs + FI workspace set:** ⏳ outstanding (item F above) — designed in
  [`mockups/`](./mockups/) and FI-ARCHITECTURE §4; the real GUI build is sequenced behind the
  `celnet-proto` arms (C) it renders, so the workspaces show live contract data, not placeholders.

---

## Build order for the outstanding phase
**C (proto arms) → D (server consumes `celnet-rates`) → E (FIX dialect) → F (SDK · Excel · GUI ·
federation)**, with A/B product breadth landing into `celnet-rates` in parallel (disjoint leaf).
The proto arms (C) are the keystone every client surface depends on, so they go first.
