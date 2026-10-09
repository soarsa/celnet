# Celnet — Trader GUI Design

> Design corpus for the Celnet FX-options trader front end. This is a **design doc only**
> — no application code is written by this lane. It is the spec the GUI build wave will
> implement (the `frontend-design` skill applies there). It builds **on top of** the single
> current Celnet wire contract documented in `docs/API-CLIENTS.md` (`celnet-proto` →
> `celnet-client` typed SDK; gRPC primary, WebSocket JSON-mirror designed). One contract,
> two transports — the GUI is just another consumer of that contract, never a fork of it
> (CLAUDE.md rule 9: no versioned APIs).
>
> North star: **out-intuit and out-function SynOption Optimus** (and every incumbent) with
> an Apple-grade interface — calm, deep, fast, and correct. We earn the word *beautiful* by
> restraint and rigor, not decoration.

---

## 0. Research basis — what the incumbents do, and where they hurt

This section is the competitive ground truth the design reacts to. Sources are listed in §11;
findings are deliberately about **workflow and UX**, not feature checklists (those live in
`docs/CAPABILITIES-VS-COMPETITION.md`).

### 0.1 SynOption Optimus / Titan (the target to beat)

- **What it is.** MAS-licensed FX-options analytics + multi-bank trading venue. Coverage
  ~75 FX pairs (deliverable + NDF). Sells itself on "comprehensive analytics," customizable
  **spot / multi-currency / single-currency grids**, "realized vs implied" grids, an
  "intuitive pricing screen for trade execution," Greeks comparison, and dynamic spot/vol
  stress scenarios. **Titan** is the client-distribution / workflow layer; clients can
  **publish their own vol surfaces** via UI or FIX. Lifecycle module books deals, exercises,
  expiries, fixings, barriers.
- **Workflow.** Discrete **RFQ** to multiple banks → two-way → accept; analytics-first
  ("trade idea generation") framing; portfolio + lifecycle management bolted alongside.
- **Where it's dated / where we win (the concrete UX gaps).**
  1. **Grid-maximalism.** "Customizable grids" everywhere → dense spreadsheet walls,
     every cell the same visual weight, no typographic hierarchy. The trader hunts for the
     one number that changed. *We win with hierarchy + motion-as-signal (only the changed
     number animates).*
  2. **Analytics ≠ decision.** Optimus is pitched as an analytics workbench; the path from
     "I have an idea" to "I have a live two-way I can hit" crosses several screens. *We win
     with a single ticket that is simultaneously the analytics surface and the executable.*
  3. **RFQ-centric, weak live RFS feel.** Discrete request/response cadence; streaming is
     not the spine. *We win by making RFS the resting state — a living blotter of two-ways
     you can click — with RFQ as a deliberate escalation.*
  4. **Convention opacity.** Industry-wide trap: a price without its premium/delta/cut
     conventions on the face. *We win by putting resolved conventions on every quote face
     (the contract already carries them — `docs/API-CLIENTS.md` §3).*
  5. **Web-app generic chrome.** Looks like a configurable BI tool, not a crafted
     instrument. *We win with an Apple-grade design system: materials, depth, restraint.*

### 0.2 Bloomberg OVML / BVOL / MARS

- **OVML** is the FX-options pricer launched from the Terminal/IB chat — type `OVML` + a
  contract to price a strategy and paste a link into chat. **MARS** (Multi-Asset Risk
  System) is the booking/risk/scenario layer; **BVOL** is the surface. You can view the
  whole surface from many participants, build synthetic surfaces for illiquid pairs, and see
  the surface flex as you nudge **ATM / RR / BF**.
- **Where it's dated / where we win.** Terminal-amber, function-code muscle memory
  (`OVML<GO>`), four-pane monochrome density, seat-priced and closed-model. Powerful but
  *learned*, not *intuitive*; the surface and the ticket and the risk live in different
  functions. *We win with one cohesive spatial model, no function codes, modern type and
  color, and an open typed SDK underneath (no terminal lock-in).*

### 0.3 Digital Vega Medusa (Evo)

- Award-winning multi-dealer FX-options venue: **multi-dealer RFQ + streaming**, up to
  **5 LPs in competition**, up to **4 simultaneous requests**, 60+ pairs, buy-side
  aggregation, LP selection by recent performance, allocation/splitting, post-trade.
- **Strength to match:** competition-in-one-view (best bid/best offer across LPs), bounded
  tradable window on RFS (~minutes). **Where we win:** Medusa *aggregates* LP pricing — it
  outsources the quant. Celnet *is* the engine, so our GUI can show **why** (smile point
  provenance, arb report, full Greeks, scenario) live next to the tradable price. We also
  out-design the visual layer.

### 0.4 Cross-cutting trading-UI failure modes (industry literature)

- Clutter & data-overload: firms "cram every bit of data" → inboxes nobody opens, heatmaps
  that confuse. Cluttered order tickets amplify execution friction.
- Lack of unified analytics; time-consuming navigation between tools.
- Trading software UX lags consumer/mobile by a decade.
- *Design consequence (adopted below):* progressive disclosure, one canonical ticket,
  motion only as signal, keyboard-first, P99-bounded render budget so the UI never stutters
  under a fast tape.

---

## 1. Design principles (the constitution)

1. **Calm under fire.** A fast market must not produce a flickering screen. Default state is
   quiet; change is the only thing that draws the eye, and it does so with a single
   purposeful flash that decays. (HIG: *motion with purpose*.)
2. **One ticket to rule them.** The pricing ticket is the analytics surface **and** the
   executable. No "price here, trade there." Idea → live two-way → hit, without a context
   switch.
3. **RFS is the resting state.** The blotter streams two-ways continuously; RFQ is a
   deliberate escalation, not the default cadence. This is the structural inversion of
   Optimus.
4. **Conventions on the face, always.** Every quote shows resolved premium/delta/cut/ATM
   conventions (the contract carries them). Transparency is a feature, not a tooltip.
5. **Provenance one keystroke away.** For any smile point or price, the trader can reveal
   *why* — calibration inputs, arb report, Greeks decomposition — without leaving the
   surface.
6. **Keyboard-first, mouse-optional.** A trader's hands stay on the keyboard. Command
   palette (⌘K), tab/enter ticket flow, hotkey blotter actions. Pointer is for the surface
   and the grid.
7. **Depth, not chrome.** Hierarchy via materials, elevation, and type scale — not borders
   and gridlines everywhere. (HIG: materials/vibrancy convey structure without distraction.)
8. **Correctness is visible.** Stale data, arb violations, expired last-look windows, and
   degraded streams are *shown*, never silently wrong. (Maps to the contract's
   sequence/resync/`valid_until` semantics in `docs/API-CLIENTS.md` §4.)
9. **Light & dark are equals.** Dark mode is the trading-floor default (low ambient light,
   long sessions), but light mode is fully designed, not an afterthought. Semantic tokens,
   not hard-coded colors.
10. **Render budget is a latency budget.** The UI inherits the platform's P99 discipline:
    a frame budget (≤16.6 ms at 60 Hz, target 120 Hz on ProMotion) with conflation so a
    burst tape never blows it (mirrors `docs/SCALE-OUT.md` §5 delta-conflation on the wire).

---

## 2. Information architecture & navigation

A **single-window, multi-pane workspace** — not a sea of MDI child windows (the Optimus/
Bloomberg failure). One persistent left rail of *workspaces*; the main canvas hosts the
active workspace; a right **Inspector** is contextual (provenance, conventions, Greeks);
a slim **status ribbon** carries connection/stream health and the global clock.

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│  ●●●            Celnet · EURUSD ▾            ⌘K  Search / command…        ◷ 14:32:07 │  ← title + command bar
├────┬─────────────────────────────────────────────────────────────────┬────────────┤
│ R  │                                                                   │            │
│ a  │                                                                   │ Inspector  │
│ i  │                      ACTIVE WORKSPACE CANVAS                       │ (context)  │
│ l  │                                                                   │            │
│    │                                                                   │            │
├────┴─────────────────────────────────────────────────────────────────┴────────────┤
│  ◉ Stream healthy · seq 184312 · 3 LPs · last-look 4.8s        P99 render 6.2ms     │  ← status ribbon
└──────────────────────────────────────────────────────────────────────────────────┘
```

Left rail workspaces (each a hero screen in §4):

| Icon | Workspace | Primary contract surface used |
|---|---|---|
| ⌁  | **Ticket** (RFQ) | `QuoteService.RequestQuote/Accept/Reject`, `PricingService.Price` |
| ≋  | **Stream** (RFS blotter) | `StreamService.StreamSession` (multiplex subscribe/snapshot/update/resync + click-to-trade `Execute`) |
| ◷  | **Surface** (vol marking + viz) | `SurfaceService.GetSmile / MarkSurface / Scenario` |
| ⊞  | **Risk** (scenario / what-if grid) | `SurfaceService.Scenario` (+ Greeks from quotes/positions) |
| Σ  | **Positions / P&L** | *designed-but-not-on-wire* (`GetPosition`/`AttributePnl`, API-v2) |

> Honesty note (CLAUDE.md rule 5/10, no overclaim): the **Positions/P&L** workspace is
> designed here but depends on the API-v2 `GetPosition`/`AttributePnl` services that
> `docs/API-CLIENTS.md` §7 lists as *not yet on the wire*. It ships when that contract does;
> the GUI build wave must gate it behind that capability, not stub fake P&L.

**Navigation grammar.** ⌘1..⌘5 jump workspaces; ⌘K opens the command palette (fuzzy: pairs,
tenors, strategies, actions — "EURUSD 1M 25d RR", "mark surface", "shock vega +1"); the
pair switcher (⌘P) is a fuzzy CCY-pair picker that re-targets the active workspace. No
function codes, no `<GO>`.

---

## 3. Design system ("Celnet Aurora")

Apple-grade, vendor-neutral, named for purpose. Tokens below are the contract the build wave
implements as CSS custom properties (web) or a SwiftUI theme (native). All color is **OKLCH**
for perceptual uniformity (so semantic up/down greens/reds are equally vivid in light & dark
and survive contrast tuning).

### 3.1 Typography

- **Type family.** UI text: a clean humanist/neo-grotesque (system `-apple-system`/SF Pro on
  Apple; **Inter** as the cross-platform fallback). **Numerics: a tabular, slashed-zero
  monospace** for every price/Greek/vol so digits never reflow as they tick (SF Mono / **IBM
  Plex Mono** fallback, `font-variant-numeric: tabular-nums`). This single decision kills the
  number-jitter that makes incumbent blotters feel cheap.
- **Type scale** (1.20 minor-third, base 13px — dense-pro density, not consumer 16px):

  | Token | px / line | Use |
  |---|---|---|
  | `type-display` | 28 / 32 | the live two-way mid on the ticket |
  | `type-title` | 20 / 26 | workspace + section headers |
  | `type-headline`| 16 / 22 | panel headers, active strike |
  | `type-body` | 13 / 18 | default UI text |
  | `type-callout` | 12 / 16 | grid cells, secondary numerics |
  | `type-caption` | 11 / 14 | labels, conventions chips, units |
  | `type-micro` | 10 / 12 | axis ticks, status ribbon |

  Weights: 400 body, 500 emphasis, 600 headers. Tracking tightened −0.01em at display sizes
  (HIG: bolder, tighter at scale).

### 3.2 Color & semantic tokens

Tokens are **semantic**, never raw hex in components. Two appearances (Dark default, Light);
a third **Increased-Contrast** variant satisfies accessibility (HIG 2025 adjusted system
hues for exactly this).

| Semantic token | Dark (OKLCH) | Light (OKLCH) | Meaning |
|---|---|---|---|
| `bg-base` | 0.16 0.012 250 | 0.99 0.003 250 | window background |
| `bg-raised` | 0.20 0.015 250 | 0.975 0.004 250 | panels (sit on a material) |
| `bg-overlay` | 0.24 0.02 250 | 1.00 0 0 | popovers, ticket card |
| `text-primary` | 0.97 0.01 250 | 0.20 0.02 250 | numerics, headers |
| `text-secondary`| 0.72 0.015 250 | 0.45 0.02 250 | labels |
| `text-tertiary` | 0.55 0.015 250 | 0.60 0.015 250 | units, ticks |
| `accent` | 0.72 0.15 250 | 0.55 0.17 255 | Celnet blue — selection, focus |
| `bid` (buy/up) | 0.78 0.16 155 | 0.55 0.15 155 | bid side, up-tick |
| `offer` (sell/dn)| 0.70 0.17 25 | 0.55 0.19 27 | offer side, down-tick |
| `warn` | 0.82 0.16 75 | 0.62 0.16 70 | last-look expiring, stale |
| `danger` | 0.65 0.21 25 | 0.52 0.22 27 | arb violation, error |
| `flash-up` | 0.85 0.18 155 @α | — | tick-up flash (decays) |
| `flash-dn` | 0.78 0.19 25 @α | — | tick-down flash (decays) |

Rules: bid/offer hues are **distinct in hue, not just brightness** (red/green colorblind
safety: bid = teal-green ~155°, offer = warm-red ~25°, plus an optional ▲/▼ glyph and a
position cue). Surface heatmaps use a perceptually-uniform diverging ramp (cool→warm through
a light neutral), never rainbow (rainbow ramps mislead magnitude — established viz practice).

### 3.3 Spacing, radius, elevation

- **Spacing scale** (4px base): 2, 4, 8, 12, 16, 24, 32, 48. Grid gutters 8; panel padding
  16; ticket card padding 24.
- **Radius:** `r-sm` 6 (chips/inputs), `r-md` 10 (panels), `r-lg` 16 (ticket/overlay cards),
  `r-full` (pills). Continuous (squircle-style) corners on Apple, rounded fallback on web.
- **Materials / elevation (HIG materials + Liquid-Glass-informed, restrained).** Four levels:
  - `mat-window` — opaque base.
  - `mat-panel` — `bg-raised`, hairline top highlight, soft 1-level shadow.
  - `mat-float` — popovers/ticket: translucent **regular material** (backdrop blur ~20px +
    saturation), vibrant text pulled forward (HIG vibrancy), 2-level shadow.
  - `mat-hud` — the command palette / quote-confirm: **thick material**, strongest blur,
    used sparingly for the one thing that must own focus.
  > Translucency is a hierarchy tool, **not** applied behind dense numeric grids (where it
  > would hurt legibility). Liquid Glass is used at the *control/overlay* layer that floats
  > above content — exactly its intended role — never as wallpaper behind a blotter.

### 3.4 Motion (purpose only)

- **Tick flash:** on a price change, the cell background flashes `flash-up/flash-dn` at α≈0.5
  and decays to 0 over **450 ms** ease-out. The *digits* never move (tabular nums); only the
  wash decays. This is the single most important motion in the app.
- **Quote arrival:** RFQ two-way card scales 0.98→1.0 + fades in over 180 ms (spring,
  low stiffness). Last-look countdown is a thin ring that depletes against the `valid_until`
  deadline; turns `warn` under 1.5 s.
- **Workspace switch:** 200 ms cross-fade + 8px parallax of the entering canvas (depth cue).
- **Surface manipulation:** direct, 1:1, inertia-free while dragging; eased settle on release.
- **Respect `prefers-reduced-motion`:** flashes become a 1-frame static tint; transitions
  become instant. Motion is never load-bearing for meaning (color/glyph carry it too).

### 3.5 Core components

`PriceTile` (tabular numeric, flash-on-change, side-tinted), `TwoWayQuote` (bid | mid |
offer with last-look ring + convention chips), `ConventionChip` (e.g. `pa` / `25Δ` / `NY
1500` / `spot-prem`), `Sparkline` (canvas, last-N mid), `GreeksStrip` (Δ Γ ν Θ + vanna/volga
on expand), `StrategyLegRow`, `TenorPill`, `ShockCell` (risk grid, diverging tint),
`StatusBadge` (stream health), `ArbBanner` (danger, with the failing constraint named),
`CommandPalette` (⌘K, fuzzy), `Inspector` (contextual provenance). Every component has Dark/
Light/Increased-Contrast and a reduced-motion variant.

---

## 4. Hero screens (workflows → screens)

### 4.1 Ticket (RFQ) — *the* differentiator

The ticket is one card that is analytics + executable. The trader builds a structure
(vanilla or multi-leg strategy — risk reversal / strangle / straddle / seagull as one
request, exactly as the contract's `Strategy(repeated Leg)` models), sees a **live two-way +
full Greeks + the conventions on the face**, and hits it — without changing screens.
`Solve` (zero-cost strike/premium) is inline. Last-look window is a visible depleting ring.

```
┌─ TICKET ───────────────────────────────────────────────────────────────────────┐
│  EURUSD  ·  Risk Reversal  ▾                              [ Vanilla ▾ ] + Add leg│
│                                                                                   │
│   LEG 1   Buy   1M (12 Jun)   Call   25Δ ▸ K 1.0925   10mm EUR                     │
│   LEG 2   Sell  1M (12 Jun)   Put    25Δ ▸ K 1.0612   10mm EUR     [Solve: zero-cost]│
│  ─────────────────────────────────────────────────────────────────────────────  │
│                                                                                   │
│        BID                       MID                        OFFER                  │
│      0.142 %                   0.155 %                     0.168 %     EUR prem    │  ← display numerics, tick-flash
│      ◷ last-look  ▓▓▓▓▓▓▓░░  3.9s                                                  │  ← depleting ring
│                                                                                   │
│   Δ +0.018   Γ 0.21   ν 0.084   Θ −0.003     vanna ▸  volga ▸     [⌄ full Greeks] │
│                                                                                   │
│   conv:  [pa] [25Δ fwd] [NY 1500 cut] [spot prem]      vol: 7.42 / 7.55 / 7.69    │  ← conventions on the FACE
│                                                                                   │
│   [ Request quote  ⏎ ]      [ Stream this ≋ ]      [ Add to risk ⊞ ]              │
└───────────────────────────────────────────────────────────────────────────────┘
```

UX wins over Optimus: idea→tradable in one card; conventions visible (not buried); Solve
inline; "Stream this" promotes the exact structure into the RFS blotter; "Add to risk" drops
it into the scenario grid — all the same `Instrument` object, no re-keying. Keyboard: Tab
through legs, ⏎ requests, ⌘⏎ accepts the live side.

### 4.2 Stream (RFS blotter) — the resting state

A living table of streaming two-ways for subscribed structures. Every row is one
subscription multiplexed over a **single** `StreamService.StreamSession` bidirectional
channel (each row owns its `SubscriptionId`; snapshot+delta+resync under the hood —
`docs/API-CLIENTS.md` §4). **Click a side to trade**: click-to-trade is *implemented* — the
row carries the maker's short-lived `TradableToken`s (one to SELL at the bid, one to BUY at
the offer), and a click sends an `Execute` presenting that token, booking the streamed
premium with no separate RFQ round-trip (the SDK surfaces an `Executed` or a typed
`StreamReject` for last-look expiry/forged/already-consumed). Only changed numbers flash;
sequence/health per row.

```
┌─ STREAM  ≋  ───────────────────────────────────────────────────────────────────┐
│  Pair    Structure        Tenor  │  Bid       Mid       Offer   │  Δ      ν   │ ◉ │
│ ─────────────────────────────────┼──────────────────────────────┼─────────────┼───│
│  EURUSD  ATM straddle      1M     │  7.42      7.55      7.69 ▲  │ 0.00  0.112 │ ◉ │  ← only ▲ cell flashed
│  EURUSD  25Δ RR            1M     │  0.142     0.155     0.168   │ +.018 0.084 │ ◉ │
│  GBPUSD  25Δ fly           2M     │  0.231     0.240     0.250 ▼ │ 0.00  0.061 │ ◉ │
│  USDJPY  ATM              ON      │ 11.20     11.45     11.71    │ 0.00  0.009 │ ◐ │  ← ◐ = resyncing
│  AUDUSD  10Δ strangle      3M     │  0.512     0.531     0.551   │ 0.00  0.150 │ ◉ │
│ ─────────────────────────────────┴──────────────────────────────┴─────────────┴───│
│  + Subscribe (⌘K)        Conflated 60Hz · 0 gaps · 3 LPs in competition            │
└───────────────────────────────────────────────────────────────────────────────┘
```

UX wins: streaming is the spine (Optimus inversion); per-row stream-health is honest
(`◉` healthy / `◐` resyncing / `○` stale) mapping to real seq/resync state; competition view
(best-of-LPs) matches Medusa's strength; render conflated to a frame budget so a fast tape
never stutters.

### 4.3 Surface (vol marking + visualization) — the "show me why"

Three linked views of one marked surface (a `MarkSurface` version): a **3D surface**
(tenor × delta/strike × vol, WebGPU), a **smile-per-tenor** 2D overlay, and the **broker
marking panel** (ATM / 25Δ&10Δ RR&BF per tenor — the conventions traders actually mark in).
Selecting a point anywhere cross-highlights everywhere and opens provenance in the Inspector
(calibration inputs + **arb report**). Edits to ATM/RR/BF reprice the surface live (Bloomberg
BVOL parity) with an arb banner if a butterfly/calendar constraint breaks.

```
┌─ SURFACE  ◷  ──────────────────────────────────┬─ MARKING ──────────────────────┐
│   vol                                            │ Tenor  ATM   25RR   25BF  10RR │
│    ▲          ╱╲___                              │ ─────────────────────────────  │
│    │       ╱╲╱     ╲__   ← 3D surface (WebGPU)    │ ON    11.45  −0.10  0.12  −.18 │
│    │     ╱╲╱  ╲   ╱     ╲_  drag-rotate, hover    │ 1W     8.90  −0.15  0.15  −.26 │
│    │   ╱╲╱     ╲╱          ╲                       │ 1M    [7.55] −0.30  0.22  −.55 │ ← editing
│    │  ╱  delta ────────── tenor                   │ 2M     7.40  −0.34  0.24  −.61 │
│    └───────────────────────────────────▶          │ 3M     7.35  −0.38  0.27  −.70 │
│                                                   │ ─────────────────────────────  │
│  ── smile · 1M ──────────────                     │ ◉ arb-free  ·  ✓ butterfly ≥0  │
│   vol ╲__        __╱   ● selected (25Δ put)        │ source: 3-pt broker · 14:30:55 │
│       ╲__ ____ __╱                                │ [ Re-mark ]  [ Publish ]       │
│      10P 25P ATM 25C 10C                          │                                │
└─────────────────────────────────────────────────┴────────────────────────────────┘
```

UX wins: provenance + arb-report inline (Medusa can't — it aggregates LPs; we own the
engine); diverging-ramp heatmap (no rainbow); edit-in-broker-conventions with live arb
guard; 3D + smile + grid are one linked object, not three functions (Bloomberg
`OVML`/`BVOL`/`MARS` split).

### 4.4 Risk (scenario / what-if grid)

A spot×vol (and rate) **shock grid** driven by `SurfaceService.Scenario` (`ShockAxis`
absolute/relative), per selected structure or book. Each cell is a P&L (or Greek) under the
shock, tinted on a diverging ramp; the spot/vol "today" cell is anchored. Scrub a slider to
animate the grid across a shock path; pin scenarios for comparison.

```
┌─ RISK  ⊞  ──────────────────────────────────────────────────────────────────────┐
│  EURUSD 25Δ RR (10mm)        metric: P&L ▾     axes: spot × vol ▾    rel ▾         │
│                                                                                   │
│   vol →    −2%     −1%      ATM      +1%      +2%                                  │
│  spot ↓ ┌───────┬───────┬─────────┬───────┬───────┐                               │
│  −1.5%  │ -82k  │ -41k  │  -12k   │ +28k  │ +66k  │  ← diverging tint, magnitude  │
│  −0.5%  │ -44k  │ -19k  │   -3k   │ +21k  │ +49k  │                               │
│   0.0%  │ -28k  │  -8k  │ ▣  0    │ +14k  │ +38k  │  ← anchored "now"             │
│  +0.5%  │  -9k  │  +6k  │  +11k   │ +25k  │ +51k  │                               │
│  +1.5%  │ +33k  │ +41k  │  +52k   │ +68k  │ +88k  │                               │
│         └───────┴───────┴─────────┴───────┴───────┘                               │
│  ▶ scrub shock path     pinned: [base] [CB-event +1.5σ]      Σ vega ladder ▸      │
└───────────────────────────────────────────────────────────────────────────────┘
```

UX wins: scenario is one click from the ticket/blotter (same `Instrument`); vega/gamma
ladders are a disclosure, not a separate screen; pinned scenarios for fast compare;
diverging perceptual ramp reads magnitude honestly.

> **Positions / P&L (Σ)** mirrors the Risk grid layout at book level with risk-factor
> attribution (delta/gamma/vega/theta/vanna/volga between two marks). Gated on API-v2
> `GetPosition`/`AttributePnl` (§2 honesty note); not drawn as a hero until that contract
> lands, to avoid implying data we don't yet stream.

---

## 5. Out-intuiting SynOption — the concrete scorecard

| # | SynOption Optimus | Celnet | Why the trader prefers Celnet |
|---|---|---|---|
| 1 | Analytics workbench; idea→trade spans screens | One ticket = analytics + executable | Fewer context switches; faster idea→fill |
| 2 | RFQ-centric cadence | RFS blotter is the resting state; RFQ escalates | Live two-ways always on; click to trade |
| 3 | Customizable grids, flat visual weight | Typographic hierarchy + flash-as-signal | Eye finds the changed number instantly |
| 4 | Conventions implicit / buried | Conventions on every quote face | No premium/cut/delta-convention surprises |
| 5 | Closed Orion models, no SDK | Open typed gRPC/WS SDK underneath | Same contract for GUI and programmatic MM |
| 6 | Surface published, but provenance opaque | Smile point provenance + arb report inline | Trader sees *why*, marks with confidence |
| 7 | Generic BI-tool chrome | Apple-grade design system (materials/depth) | Reads as a crafted instrument, calm under load |
| 8 | Function/grid muscle memory | ⌘K command palette, keyboard-first | Zero function-codes; discoverable + fast |
| 9 | Rainbow/flat heatmaps | Perceptual diverging ramps, no rainbow | Magnitude read correctly, colorblind-safe |
| 10| Stream health implicit | Per-row seq/resync/stale honesty badges | Never silently trading on stale data |

---

## 6. Tech stack — recommendation & rationale

**Recommendation: a web stack — React + TypeScript, talking the Celnet contract over
gRPC-Web/WebSocket via the typed SDK, with a WebGPU surface renderer.** A native
macOS/SwiftUI build is specced as an *optional* later "pro desktop" skin over the same
SDK; web is the primary because of cross-platform reach, deployment velocity, and zero-
install distribution to the CelNet estate and counterparties.

### 6.1 The choice, weighed

| Axis | Web (React/TS) — **chosen** | Native macOS/SwiftUI |
|---|---|---|
| Cross-platform | ✅ macOS/Win/Linux, one build | ❌ Apple-only |
| Distribution | ✅ zero-install, instant updates, hot-upgrade-friendly | ❌ notarized installs |
| Streaming | ✅ gRPC-Web/WS over the designed WS mirror | ✅ native gRPC/WS |
| Surface viz perf | ✅ WebGPU (35M-pt-class renderers exist) | ✅ Metal |
| Apple-grade visuals | ✅ achievable (materials via CSS backdrop-filter, OKLCH, SF/Inter) | ✅ native materials/vibrancy free |
| Talent / iteration | ✅ large pool, fast | ⚠️ smaller, slower |
| HIG fidelity | ⚠️ Liquid-Glass effects approximated | ✅ first-class |

Web wins on reach + distribution + streaming, which matter most for a multi-tenant,
hot-upgradable platform serving the CelNet front end and external MMs. SwiftUI's only decisive
edge is free Liquid-Glass fidelity — which we *approximate* well enough in CSS and reserve
the native build for a later "pro desktop" lane over the **same** typed SDK (no contract
fork — CLAUDE.md rule 9).

### 6.2 Concrete web stack (all OSS / permissive — CLAUDE.md rule 7)

- **Framework:** React 19 + TypeScript, **Vite** build. (MIT.)
- **State / streaming:** TanStack Query for request/response (Price/Quote); a thin
  store (**Zustand**, MIT) for streaming RFS state fed by the SDK's snapshot+delta+resync.
- **Transport:** **Connect-ES** (gRPC-Web + Connect protocol, Apache-2.0) against
  `celnet-proto`, *or* the designed WebSocket JSON-mirror — **both serialized from the same
  Rust types** so the GUI cannot drift from the contract (`docs/API-CLIENTS.md` §2). Proto →
  TS types via `buf` (the GUI's types are *generated*, never hand-written, mirroring the
  one-contract rule).
- **Surface renderer:** **WebGPU** (custom WGSL) for the 3D vol surface + heatmaps; **Canvas
  2D** for sparklines and the smile overlays (cheap, sharp). WebGL fallback for browsers
  without WebGPU. No commercial chart lib (rule 7) — bespoke renderer keeps it Apple-grade
  and dependency-light. *Note: WGSL/WebGPU is f32, matching the platform's "f32 on GPU"
  numeric policy (`docs/ARCHITECTURE.md` §); the surface viz is presentation, the f64
  pricing stays server-side.*
- **2D charts (grids/ladders):** hand-rolled Canvas/SVG components on the design system; no
  heavyweight dep.
- **Styling:** CSS custom properties for the §3 tokens + **CSS Modules** (or vanilla-extract,
  MIT) for zero-runtime, type-safe styles. `backdrop-filter` for materials; OKLCH color.
- **Motion:** the platform `Web Animations API` + a thin spring helper; honor
  `prefers-reduced-motion`. No heavy animation runtime on the hot render path.
- **Virtualization:** windowed rendering (e.g. TanStack Virtual, MIT) for IB-scale blotters
  so a 10k-row stream renders only visible rows within the frame budget.
- **Testing:** Vitest + Testing Library; Playwright for the workflow E2E (the Playwright MCP
  is already available in this environment). Lighthouse/a11y audit gates (WCAG AA contrast,
  keyboard nav).

### 6.3 Performance contract for the GUI (inherits platform P99 discipline)

- Frame budget: 8.3 ms target (120 Hz ProMotion), 16.6 ms ceiling (60 Hz). Measured P99 shown
  in the status ribbon (the screenshot's "P99 render 6.2ms" is a real instrument, not decor).
- **Conflation at the edge of the UI:** the SDK applies wire-level delta-conflation
  (`docs/SCALE-OUT.md` §5); the render layer additionally coalesces all updates within a
  frame into one paint (rAF batching) so a burst tape can't cause >1 paint/frame.
- Off-main-thread: proto decode + diff in a Web Worker; WebGPU on its own queue; main thread
  only commits a coalesced view-model per frame.
- Zero-allocation-minded render path (object pools for tile view-models) mirrors the core's
  no-alloc ethos at the UI layer — GC pauses are the front-end equivalent of jitter.

---

## 7. Accessibility & internationalization

- **Contrast:** all text ≥ WCAG AA; the Increased-Contrast appearance hits AAA for numerics.
- **Color-independence:** up/down and bid/offer never rely on color alone (hue + glyph +
  position). Diverging ramps tested for deuteranopia/protanopia.
- **Keyboard:** every action reachable without a pointer; visible focus ring (`accent`);
  ⌘K palette is the universal escape hatch; focus order follows the ticket's logical flow.
- **Reduced motion / transparency:** honor `prefers-reduced-motion` and
  `prefers-reduced-transparency` (materials degrade to solid `bg-raised`).
- **Locale:** number/percent/date formatting via `Intl`; pip/big-figure display conventions
  per pair; 24h clock; the conventions chips localize labels but never the canonical values.

---

## 8. Mapping to the contract (no fork, no overclaim)

| GUI surface | Contract RPC (`docs/API-CLIENTS.md`) | Status |
|---|---|---|
| Ticket pricing | `PricingService.Price` | implemented |
| Ticket RFQ + last-look | `QuoteService.RequestQuote/Accept/Reject` (idempotency, `valid_until`) | implemented |
| Stream blotter | `StreamService.StreamSession` (multiplex subscribe/snapshot/update/resync/seq) | implemented |
| Surface 3D/smile/marking | `SurfaceService.GetSmile / MarkSurface` (+ `ArbReport`) | implemented |
| Risk grid | `SurfaceService.Scenario` (`ShockAxis`) | implemented |
| Click-to-trade on blotter | RFS `Execute` (presents the line's `TradableToken`) → `Executed` / `StreamReject` | implemented |
| Positions / P&L (Σ) | `GetPosition` / `AttributePnl` | **API-v2 (designed)** — workspace gated off until on wire |
| Browser transport | WebSocket JSON-mirror | **designed, not yet wired** — GUI uses gRPC-Web/Connect until the mirror lands |

The GUI consumes the **one** current contract; anything marked API-v2/designed is drawn here
for completeness but must be capability-gated in the build so the front end never implies
data the wire doesn't carry (CLAUDE.md rules 5 & 10).

---

## 9. What the build wave does next

1. Stand up the design-token layer (§3) as CSS custom properties + a Storybook of §3.5
   components in Dark/Light/Increased-Contrast/reduced-motion.
2. Generate TS types from `celnet-proto` via `buf`; wire Connect-ES to a running
   `celnet-server`; build the streaming store over the SDK's snapshot+delta+resync.
3. Build hero screens in order: **Ticket → Stream → Surface → Risk** (each behind real RPCs;
   no mock data — exercise against the real edge, mirroring the SDK's "evolve API by use"
   discipline in `docs/API-CLIENTS.md` §5).
4. WebGPU surface renderer (WGSL) with WebGL fallback + the diverging ramp.
5. Performance harness: frame-budget P99 in the ribbon; conflation + Web-Worker decode;
   Playwright workflow E2E + Lighthouse/a11y gates.
6. (Later lane) Optional SwiftUI "pro desktop" over the same SDK for native Liquid-Glass
   fidelity — no contract fork.

---

## 10. Risks

1. **Liquid-Glass fidelity on web.** `backdrop-filter` materials approximate, not equal,
   native vibrancy; over-use hurts legibility behind dense grids. *Mitigation:* materials only
   at the overlay/control layer (§3.3), never behind blotters; `prefers-reduced-transparency`
   degrades to solid.
2. **WebGPU availability/jitter.** Not universal; GPU scheduling can stutter. *Mitigation:*
   WebGL fallback; surface viz is presentation, never on the trade-decision critical path.
3. **WS-mirror not yet wired.** The browser-friendly transport is designed-only
   (`docs/API-CLIENTS.md` §2). *Mitigation:* ship on gRPC-Web/Connect now; swap to the mirror
   when it lands, with no UI change (types generated from the same contract).
4. **Capability over-draw.** Positions/P&L (`GetPosition`/`AttributePnl`) is still not on the
   wire. *Mitigation:* capability-gate that workspace; never render fake P&L (CLAUDE.md rule 2).
   (Click-to-trade is no longer a gap — the multiplex `StreamSession` + `Execute` shipped.)
5. **Render budget under IB-scale fan-out.** 10k-row streams can blow the frame budget.
   *Mitigation:* virtualization + per-frame conflation + worker decode (§6.3); measured P99
   surfaced in the ribbon so regressions are visible.
6. **Number-jitter / motion fatigue.** Over-flashing exhausts the eye. *Mitigation:* tabular
   nums + decaying single-flash + reduced-motion path; flash only on actual value change.

---

## 11. Sources

Competitor UX (workflow & interface):
- SynOption Optimus — https://synoption.com/optimus.php
- SynOption Primus market-data API — https://synoption.com/primus-market-data-api/
- SynOption Optimus trade-idea workflow — https://synoption.com/using-optimus-for-trade-idea-generation-in-eurjpy/
- SynOption portfolio-management tool launch — https://thefullfx.com/synoption-launches-options-portfolio-management-tool/
- Bloomberg OVML / FX-options pricing webinar — https://www.bloomberg.com/professional/insights/webinar/pricing-fx-options-tips-tricks/
- Bloomberg MARS front-office risk brochure — https://data.bloomberglp.com/professional/sites/10/MARS-Front-Office-Risk-Brochure.pdf
- Bloomberg MARS API/Python (FIF) — https://www.fif.com/index.php?option=com_content&view=article&id=21480
- Digital Vega Medusa Evo — https://www.digitalvega.com/medusa-evo-2
- Digital Vega execution / RFQ — https://www.digitalvega.com/execution
- Medusa quick user guide (slides) — https://slideplayer.com/slide/12376044/

Trading-UI usability literature:
- DevExperts — trading-platform UX/UI no-nos — https://devexperts.com/blog/trading-platform-ux-ui-design-no-nos/
- ION — trading software UX catching up to the smartphone — https://iongroup.com/blog/markets/bridging-the-gap-how-trading-software-ux-ui-can-catch-up-to-the-smartphone/
- RON Design Lab — TradingView UI/UX case — https://rondesignlab.com/cases/tradingview-platform-for-traders
- LuxAlgo — latency standards in trading systems — https://www.luxalgo.com/blog/latency-standards-in-trading-systems/

Apple HIG / design language (2025):
- HIG — Materials — https://developer.apple.com/design/human-interface-guidelines/materials
- HIG — index — https://developer.apple.com/design/human-interface-guidelines
- WWDC25 — Get to know the new design system — https://developer.apple.com/videos/play/wwdc2025/356/
- Liquid Glass analysis — https://medium.com/@ozkanbirak/apples-new-design-language-liquid-glass-analyse-9d61eb8d23fb
- HIG — Dark Mode — https://developer.apple.com/design/human-interface-guidelines/macos/visual-design/dark-mode/

Surface visualization / WebGPU:
- ChartGPU (WebGPU charting, OSS) — https://github.com/ChartGPU/ChartGPU
- SciChart React (large-dataset viz reference) — https://www.scichart.com/react-charts/
- VolSurface (browser 3D vol surface, OSS) — https://github.com/hyobyun/VolSurface
- 3D vol-surface cross-section viz — https://medium.com/@navnoorbawa/enhancing-volatility-surface-analysis-with-3d-cross-sectional-visualization-f94b5dd786ca

Internal cross-references: `docs/API-CLIENTS.md` (the wire contract), `docs/ARCHITECTURE.md`
(numeric/transport policy), `docs/SCALE-OUT.md` (conflation), `docs/CAPABILITIES-VS-COMPETITION.md`,
`docs/COMPETITIVE-ANALYSIS.md`.
