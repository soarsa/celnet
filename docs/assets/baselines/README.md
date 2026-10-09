# UI Visual Baselines

Content-addressed reference screenshots for the `ui:matches-mockup` grounding targets.
Every file in this directory is a committed, stable pixel-baseline that automated and
manual visual verification checks can diff against.

## How baselines are used

A baseline becomes the grounding target for a `ui:matches-mockup` claim anchored to the
GUI component or view it depicts. Visual regression tools (e.g. Playwright screenshot
diff, pixelmatch) load the baseline from this directory and compare against a freshly
captured screenshot at the same viewport. A mismatch blocks the lane it is anchored to.

## Directory layout

```
docs/assets/baselines/          <- tracked canonical baselines (this dir, git-committed)
docs/assets/celnet-capabilities/ <- product capability shots (also baselines; see §Capability shots below)
```

## Baseline catalogue

Columns: **file** | **view / component** | **gui state** | **sha256 (full)**

### Phase-0 live-stream QA captures (`qa-p0-*`)

Captured against the live WebSocket backend (3 LPs, 6 subscriptions, seq ~100–10000).
Size: 1200x728.

| File | View | State | SHA-256 |
|---|---|---|---|
| `qa-p0-stream.png` | Stream workspace — live premium blotter | Live WS, Premium column visible, 6 rows, EUR/USD selected, p99 render 13.5ms | `0ddd56378980da6799757e2e4954a26fef696e9bc3d0fb6a62b821ba222ad7de` |
| `qa-p0-book.png` | Book workspace — aggregate book + vega ladder | 6 positions / 4 pairs, Net P&L +8.8m, Net Vega +86m, Aggregate Vega Ladder visible | `77c5b52b789275d4de7622ac7dc5a2ff8b58a3aa8a252731e88e4af5c2ba46ff` |
| `qa-p0-drill-risk.png` | Risk workspace — spot×vol scenario cube (P&L mode) | USD/JPY ON CALL 25MM drilled from Book, Spot×Vol heatmap, Vega Ladder + cross-gamma sidebar | `4f929ab3be6eeac534dcefab2528108d16a885f5d6bebc6a07f8e7222f6113c5` |
| `qa-surface-editable.png` | Surface workspace — 3-D vol surface + marking grid | EUR/USD, WebGPU canvas mesh, 1M row editable, arb-free badge, smile slice at bottom | `389af43ddebc68bd04b6dd7cd7a197a1f4f204c7f0b8887102bff847a2ac0def` |
| `qa-universe-navigator.png` | Universe navigator modal | Pairs dialog open, Majors group, 5 pairs, keyboard hints, Phase 1 registry note | `3079444219ed95932de20e7d5fb2c931954f22bbcce38e6a693f06c88d90da2a` |
| `qa-vol-cube.png` | Vol Cube workspace with Pairs modal | Surface vol-cube grid visible in background, Universe navigator dialog open on top | `efb2258cc14d2a6c1b82bb25d01c3a86bb6ede9d2c1a8449df9a706f2be86cef` |

### Phase-0 pair-menu / palette QA captures (`qa-pairmenu-*`, `qa-palette-open`)

Size: 1200x676.

| File | View | State | SHA-256 |
|---|---|---|---|
| `qa-pairmenu-open.png` | Stream workspace — pair selector dropdown open | Dropdown showing 5 pairs + "Search all pairs & commands" shortcut, EUR/USD highlighted | `f1f118752b9970d7160bd2da36dd1794ff039db8d927d7acd4543841ff4b836b` |
| `qa-pairmenu-fixed.png` | Stream workspace — pair selector dropdown (rebrand) | Same dropdown after Celnet rebrand header; "A CELNET PRODUCT" subtitle visible | `ce595cc7d87516c952a75a1950a7d2376880a5ac0887443ca1455a0ef5b2d424` |
| `qa-palette-open.png` | Command palette open over Stream | Full command palette with WORKSPACE / PAIR categories; Celnet rebrand header | `b84aef2c58c739777cf7f690c7c1d6075c9605c0953cd18f19691fb6b969cd92` |

### Rebrand wave captures (`rebrand-*`)

Captured after the Celnet → "A CELNET PRODUCT" rebrand, before live WebSocket was wired
(mock/replay transport). Size: 1200x676.

| File | View | State | SHA-256 |
|---|---|---|---|
| `rebrand-1-ticket.png` | Stream workspace (mock/replay) | Rebrand header present, "NO LIVE STREAM" on all pair tiles, mock prices | `e50af41533bc3934e2e80ae7241ac735d4402078f63632e379c80a0a14c1cae5` |
| `rebrand-2-book.png` | Book workspace (mock/replay) | 6 positions, Net P&L +8.7m, Vega Ladder; resyncing state | `193054f25a98c28e93506f70a7101bf9b8e30580e353d8ddacc55ea033653934` |
| `rebrand-3-risk.png` | Risk workspace — spot×vol heatmap (mock/replay) | EUR/USD 25ΔRR 10MM, P&L mode, 5×5 heatmap, Vega Ladder 1M tenors | `31351c4bc7f5f7ed2ab070fcaf71da5f27733f5996febcad805581e22f3f2a7d` |
| `rebrand-4-live-stream.png` | Stream workspace — live WS (rebrand) | Live WS wired, trend sparklines active, all pair tiles show PREMIUM | `5f440dfeb1efc94c2ad91b35d27c8e9aba20f6678c19f55fbd7eae8303a0f72f` |
| `rebrand-5-book-live.png` | Book workspace — live WS (rebrand) | Live WS, 6 positions, Net Theta −1476m, full Vega Ladder | `bdda3f527ccb407ef0e3a2d5d5866cd5008aa291af4e437f011a21930eb957d0` |

### Early GUI snapshots (`celnet-gui*`, `fe-ready`)

Pre-rebrand GUI using the generic teal logo. Size: 1200x676 (celnet-gui*), 1200x728
(fe-ready).

| File | View | State | SHA-256 |
|---|---|---|---|
| `celnet-gui.png` | Stream workspace — early GUI, mock/replay | Teal logo, no trend sparklines, resyncing; earliest committed stream screenshot | `3b21850e630cf3bf6799054c350423ca67c0328f58c7ec8607d3e891d391893e` |
| `celnet-gui-live.png` | Stream workspace — early GUI, live WS | Teal logo, live WS wired, trend sparklines active, seq 3611 | `d70691a93ad0d16851e4e31a105f484cfaa95eb0d219a2970a9f2f8e28b48d61` |
| `fe-ready.png` | Stream workspace — full Celnet rebrand, live WS | Final Celnet header, Premium column, 5-column filter bar | `0156005809ea8388d99c5b31ac174cc17aa466f85ac072eefdf90a4bfbec64f0` |

### Ticket workspace (`ticket`)

| File | View | State | SHA-256 |
|---|---|---|---|
| `ticket.png` | Ticket workspace — Risk Reversal structuring | EUR/USD Risk Reversal, 1M, Leg 1 BUY Call 25Δ K 1.0947, Leg 2 SELL Put 25Δ K 1.0632, awaiting quote | `cbbb8d8298dd11cb4fdb8133766bd9abb1c09a19891f9c4574a990ca9da40e0e` |

---


## Capability shots (sibling directory)

These live in `docs/assets/celnet-capabilities/` (not copied here) but are also canonical
baselines for the views they depict.

| File | View | SHA-256 |
|---|---|---|
| `shot-01-stream-blotter.png` | Stream workspace blotter — product-doc quality | `dd44843a0333f121e65fc3106895bee1d0d1e6982866ac91a8679c253b32e1d0` |
| `shot-02-ticket-structuring.png` | Ticket workspace — structuring flow | `0b5963e32b68576b4856e4d6835b5ac890b12de30cacf5b92c3ac39a3a61e5ef` |
| `shot-03-surface-marking.png` | Surface workspace — vol surface marking | `435478beed39ed81fdd6184816690534569f1275d1564ef660e4e8c5d740a1cf` |
| `shot-04-risk-scenario.png` | Risk workspace — scenario grid | `b044b7d0172648726db76926eaa69458252e518e2f2f8ac13c4d166e18466c2a` |
| `shot-05-book-aggregate.png` | Book workspace — aggregate book | `84424a207320b46cb23c1236092c4d56d2c715dee05b661ffedbceb9ac96d1ab` |
| `shot-06-pair-navigator.png` | Universe / pair navigator | `84f0e429cee18fbc011e8ef4763e88e8f4fc2e73d122e81c035c42ab8075caf7` |
| `shot-07-command-palette.png` | Command palette | `ba403cf950c241d7121c19d967350c50f708da209b883e9e358b1ec38dc55326` |
| `shot-08-clicktrade-lastlook.png` | Click-to-trade — last-look confirmation | `3ba7fb72b4b510ee172a02f9b2c5a2e383b1e5e2450f74050826126766587b28` |
| `shot-08b-clicktrade-retry.png` | Click-to-trade — retry flow | `3b7f0f32c9922f7adb76eee312da0f3313e189b79e15514a527973129d2062ff` |
| `shot-09-excel-taskpane.png` | Excel task-pane add-in | `47e765404caf5280ab14a986719e22bf2dafe143b46013d875086f333584986f` |
| `shot-10-excel-grid-branded.png` | Excel grid — branded layout | `83f44ec31d4c29518a26de727ffdf1f0d406a2599347bcc0eb15b51dcd86f0c5` |

---

## Updating a baseline

1. Capture the new screenshot at 1200×676 or 1200×728 (match the slot's existing size).
2. `git mv` the old file to a timestamped archive name if you want to keep history, or
   simply overwrite the existing path.
3. Update the SHA-256 entry in this table.
4. Author or update the `ui:matches-mockup` knowledge claim anchored to the affected view.

## Source history

Root-level screenshots were moved here via `git mv` from the repository root. The move
commits are in the main branch history and preserve full file provenance.
