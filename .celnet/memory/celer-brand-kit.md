---
name: celer-brand-kit
description: "Celer Trader brand kit (colors, font, logo) extracted from the UAT webtrader — for branding Celnet as a Celer product."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 4e9a6b38-74d9-4a36-9109-4d49116555b3
---

Parent company full name is **Celer Technologies** — use this in branding (the lockup caption is "a Celer Technologies product"; the toolbar shows a "Celnet / Celer Technologies" wordmark). The brand mark (pinwheel) appears in ONE place only (the left rail); the toolbar uses a mark-less wordmark to avoid duplicating the logo. No macOS traffic-light dots in the toolbar.

Celer brand, extracted 2026-05-31 from https://soars-uat-lon-webtrader-fx.celer-tech.com/ (login screen + `client-theme/build/client.css`). Celnet is to be branded as a **Celer product**, aligning to this identity while modernising per the macOS-inspired Aurora directive ([[ga-push-directives]]).

- **Brand coral**: `#ff7357` (≈ `#ff7257`) — the signature Celer colour; the pinwheel logo mark. oklch ≈ `oklch(0.72 0.17 35)`.
- **Interactive accent** (sign-in button / card border): indigo/periwinkle ≈ `#6b6bf5`, oklch ≈ `oklch(0.62 0.19 280)`.
- **Base navy** `#1B1F2A` (`rgb(27,31,42)`); **raised surface** `#282C3E` (`rgb(40,44,62)`).
- **Secondary text** `#979CB7` (periwinkle-grey), uppercase/capitalised labels, letter-spacing ~1px.
- **Font**: **Anaheim** (Google Fonts, weights 400..800) — `https://fonts.googleapis.com/css2?family=Anaheim:wght@400..800`.
- **Logo**: geometric **pinwheel "C"** — 4 coral quadrant petals (square mark) above a white "celer" wordmark. The coral mark path (viewBox `0 0 501 500`, fill `#ff7357`):
  `M500.12.7H262.61V238.21C393.78,238.21,500.12,131.88,500.12.7M262.49,262V499.55H500C500,368.38,393.67,262,262.49,262M238.7,499.55V262H1.19c0,131.17,106.34,237.51,237.51,237.51M1.19,238.19H238.7V.67C107.53.67,1.19,107,1.19,238.19`
- **Signature detail**: a small mono **build-hash · UTC-timestamp** footer (bottom-right of the login).

Celnet's GUI uses the "Celnet Aurora" OKLCH/materials design system (`gui/src/design/tokens.css`). Rebrand = marry Celer identity (coral, navy, Anaheim, pinwheel, build-stamp) with Aurora's perceptual colour + macOS materials. See [[session-state-2026-05-31]].
