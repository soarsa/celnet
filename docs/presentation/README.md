# Presentation decks

Self-contained HTML decks for showing Celnet to an audience. No build step, no external
assets — open the file in a browser, or print it to PDF.

| Deck | What it covers | Slides |
|---|---|---|
| [celnet-fixed-income-overview.html](celnet-fixed-income-overview.html) | Wireframe walkthrough of the eleven fixed-income screens plus how the platform fits together architecturally | 19 |

## Running one

```bash
open docs/presentation/celnet-fixed-income-overview.html
```

- **Navigate:** `→` / `space` / `PageDown` forward, `←` / `PageUp` back, `Home` / `End`.
  The on-screen counter (bottom right) also has arrows.
- **Deep-link a slide:** append `#7` to the URL. The deck follows hash changes.
- **Export to PDF:** File ▸ Print ▸ Save as PDF, landscape. Each slide prints as one
  1280×720 page.

## What is in the FI deck

1–5 the premise, the crate stack, the trade lifecycle, and the navigation model.
6–16 one plate per screen, walking the Fixed Income rail top to bottom: Streaming ·
Agg Book · Curves · Quoting · Pricing · Risk · Book · Hedging · Risk Transfer ·
Administration & reference data · Analytics.
17–19 five-client parity, the non-functional requirements, and where to read next.

## House rules for these decks

- **The product's own design language, mirrored — not a deck theme.** Every colour, face
  and radius in the deck comes from [`gui/src/design/tokens.css`](../../gui/src/design/tokens.css):
  the dark surface stack (inset 0.185 / base 0.215 / raised 0.285, OKLCH hue ~264), CelNet
  coral for identity and the annotation numbers, CelNet indigo for selection and primary
  actions, bid-green / offer-red / warn-amber for semantics — never the brand hue for a
  quantitative one — Anaheim for the wordmark, Space Grotesk for headings and labels, and
  JetBrains Mono for **every** numeric, which is the app's own rule. It is synced by hand:
  when the app's tokens move, move these. The one external fetch is the Google Fonts link
  the app itself uses; the fallback stacks are real, so the deck degrades to system faces
  offline rather than breaking.
- **Still a wireframe.** Hairlines, hatched placeholders and flat panes, so the audience
  reads structure and flow. It wears the product's clothes; it is not a screenshot of it.
- **Every plate mirrors a screen that exists.** Rail rows, tab strips and column sets are
  taken from `gui/src/lib/commands.ts` and `gui/src/workspaces/*`; the numbers are
  illustrative, the structure is not. When the GUI's information architecture changes, the
  affected plate changes with it.
- **Fixed 1280×720 canvas**, scaled to the viewport by a few lines of inline script, so a
  slide looks identical on a laptop, a projector and in the printed PDF.
- **Annotation numbers are inline badges** attached to the label they annotate, never
  absolutely positioned over the plate — a pinned marker drifts the moment the layout moves
  and lands on top of a value the audience is trying to read.
