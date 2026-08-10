/**
 * The in-app Help & guided-tutorials CONTENT REGISTRY — the single source of truth
 * for every pricing-feature / tiering-strategy / concept explanation the trader
 * sees. Authored from `docs/FI-TIERING-RESEARCH.md` (§5 strategies, §9 how-to +
 * worked oracles) and `docs/FI-PRICING-GROUPS-DESIGN.md` (the feature pipeline),
 * so the GUI no longer deep-links OUT to the docs — the registry IS the in-app
 * corpus. Every worked example uses the docs' real numbers (Flat ±25bp → 99.30 /
 * 99.80; the SCALE_SMOOTH divergence oracle; the inventory-skew two-way).
 *
 * Pure data + lookup/search — no React, no transport — so the help panels, the
 * Help center search, and the vitest suite all share ONE registry. Tutorials are
 * referenced by {@link HelpEntry.tourId} and defined in `lib/tours.ts`.
 */

import type { FeatureKind, TieringStrategyKind } from "../data/contract";
import type { TourId } from "./tours";

/** A category groups the index and colours the entry badge. */
export type HelpCategory = "feature" | "strategy" | "concept";

/**
 * A WORKED numeric example: a scenario line, an ordered set of labelled
 * input/output rows (rendered as a compact table), and an optional takeaway.
 */
export interface WorkedExample {
  /** The one-line setup ("LP composite 99.50 / 99.60, mid 99.55"). */
  scenario: string;
  /** Ordered labelled rows — inputs then the computed outputs. */
  rows: readonly { label: string; value: string }[];
  /** An optional closing takeaway sentence. */
  takeaway?: string;
}

/** One help topic: a feature, a tiering strategy, or a cross-cutting concept. */
export interface HelpEntry {
  /** Stable registry key ("feature.tiering", "concept.bid-offer-tiering"). */
  id: string;
  category: HelpCategory;
  /** Display title. */
  title: string;
  /** One-line statement of what it is for. */
  purpose: string;
  /** Short prose: how the mechanism actually works. */
  howItWorks: string;
  /** A worked numeric example using the docs' real figures. */
  example: WorkedExample;
  /** Ordered configuration steps. */
  howToConfigure: readonly string[];
  /** When to reach for it. */
  whenToUse: string;
  /** The failure modes / caveats. */
  risks: string;
  /** Extra search keywords beyond the title/purpose text. */
  keywords: readonly string[];
  /** The guided tour that walks this topic, if any. */
  tourId?: TourId;
}

// --- id maps (so the editor / cards resolve their entry) ---------------------

/** Pricing-feature kind → its help entry id. */
export const FEATURE_HELP_ID: Record<FeatureKind, string> = {
  MID_SHIFT: "feature.mid-shift",
  TIERING: "feature.tiering",
  AXE: "feature.axe",
  POSITION: "feature.position",
  PANIC_SKEW: "feature.panic-skew",
};

/** Tiering-strategy kind → its help entry id. */
export const STRATEGY_HELP_ID: Record<TieringStrategyKind, string> = {
  FLAT_MARKUP: "strategy.flat-markup",
  INVENTORY_SKEW: "strategy.inventory-skew",
  SCALED_SMOOTHED_SPREAD: "strategy.scaled-smoothed-spread",
};

// --- the registry ------------------------------------------------------------

const ENTRY_LIST: readonly HelpEntry[] = [
  // ============================ PRICING FEATURES ============================
  {
    id: "feature.mid-shift",
    category: "feature",
    title: "MID SHIFT",
    purpose: "Construct the desk's price by shifting the mid, or pin it to a reference.",
    howItWorks:
      "MID SHIFT is the trader's price-construction step, applied right after RAW and before TIERING. It either adds a signed offset to the consolidated mid (a manual bias) or replaces the mid outright with an absolute reference price. The half-spread is untouched — only the centre of the two-way moves.",
    example: {
      scenario: "RAW composite 99.50 / 99.60 (mid 99.55, half-spread 0.05).",
      rows: [
        { label: "Shift", value: "+0.20 price points" },
        { label: "New mid", value: "99.55 + 0.20 = 99.75" },
        { label: "Outbound two-way", value: "99.70 / 99.80 (half-spread kept)" },
        { label: "Reference override", value: "set 99.60 ⇒ mid pinned to 99.60 → 99.55 / 99.65" },
      ],
      takeaway: "Only the mid moves; the spread comes from the later TIERING feature.",
    },
    howToConfigure: [
      "Drag MID SHIFT onto the pipeline (it slots after RAW).",
      "Enter a signed Shift in the chosen unit for a manual bias.",
      "Or tick “Override mid with a reference price” and enter an absolute price to pin the mid.",
      "Watch the live preview: the two-way after MID SHIFT recentres by exactly your shift.",
    ],
    whenToUse:
      "When the desk wants to lean the whole book off the raw mid, or price off an explicit reference rather than the live composite.",
    risks:
      "A large manual shift moves the book away from the market — combine with guardrails and keep the reference fresh; a stale reference override streams a stale price.",
    keywords: ["mid", "shift", "bias", "reference", "construction", "offset", "price"],
    tourId: "build-pricing-group",
  },
  {
    id: "feature.tiering",
    category: "feature",
    title: "TIERING",
    purpose: "Apply margin — widen (and optionally lean) the two-way around mid before publish.",
    howItWorks:
      "TIERING is the margin step: it takes the mid and produces the outbound bid/offer by adding a half-spread H around it, using the shared tiering strategies (Flat markup, Inventory skew, Scaled Smoothed Spread) and clamping the result inside the book's guardrails. It is margin only — directional emergency overlays are the separate PANIC/SKEW feature.",
    example: {
      scenario: "LP composite 99.50 / 99.60 (mid 99.55, market spread 0.10).",
      rows: [
        { label: "Strategy", value: "Flat markup, H = 25 price bps (= 0.25)" },
        { label: "Bid = mid − H", value: "99.55 − 0.25 = 99.30" },
        { label: "Offer = mid + H", value: "99.55 + 0.25 = 99.80" },
        { label: "You stream", value: "99.30 / 99.80 (our spread 0.50)" },
      ],
      takeaway: "The 0.10 LP spread becomes your 0.50 quoted spread — the 0.25 half-spread is your margin.",
    },
    howToConfigure: [
      "Add a TIERING feature (or review a session's applied tiering on Fixed Income → Pricing → Tiering tab).",
      "Pick the spread unit (Price bps for a fixed price offset; Yield bps for a duration-consistent curve spread).",
      "Add a strategy — Flat markup for a constant margin; Scaled Smoothed Spread instead when you want spread-volatility damping.",
      "Set the half-spread H (or the SCALE_SMOOTH params), then set the guardrails (h_min / h_max / spread_floor).",
      "Confirm the sample preview: 99.50 / 99.60 → your outbound two-way.",
    ],
    whenToUse:
      "Always — every outbound price needs a margin. Flat for a predictable spread; Scaled Smoothed Spread when the raw market spread is jumpy.",
    risks:
      "Too tight a margin leaks to informed/large flow; too wide loses the trade. Flat markup ignores inventory and volatility — layer POSITION / a vol strategy if that matters.",
    keywords: ["tiering", "margin", "markup", "spread", "widen", "half-spread", "bid", "offer"],
    tourId: "bid-offer-tiering",
  },
  {
    id: "feature.axe",
    category: "feature",
    title: "AXE",
    purpose: "Skew the mid toward the side the desk wants to trade, to attract that flow.",
    howItWorks:
      "AXE leans the mid by a fixed magnitude toward the desk's axe. Leaning to BUY lifts the mid to make the desk's bid more competitive (attracting sellers); leaning to SELL drops the mid (attracting buyers). The half-spread is unchanged, so the whole two-way shifts.",
    example: {
      scenario: "Mid 99.55, half-spread 0.05 ⇒ 99.50 / 99.60.",
      rows: [
        { label: "Axe side", value: "BUY (want to buy — attract sellers)" },
        { label: "Magnitude", value: "0.02" },
        { label: "New mid", value: "99.55 + 0.02 = 99.57" },
        { label: "Outbound two-way", value: "99.52 / 99.62 (spread kept)" },
      ],
      takeaway: "A BUY axe lifts both sides so your offer is keener to sellers hitting it.",
    },
    howToConfigure: [
      "Add an AXE feature to the pipeline.",
      "Choose the side the desk wants to trade (Buy or Sell).",
      "Set the magnitude (how far to lean the mid).",
      "Confirm the preview mid moves in the axe direction.",
    ],
    whenToUse:
      "When the desk has a directional interest and wants to bias flow toward filling it, without changing the spread.",
    risks:
      "An axe left on after the interest is gone skews the whole book off-market. Keep magnitudes modest and clear the axe once filled.",
    keywords: ["axe", "skew", "lean", "buy", "sell", "attract", "flow", "direction"],
    tourId: "build-pricing-group",
  },
  {
    id: "feature.position",
    category: "feature",
    title: "POSITION",
    purpose: "Lean the price against signed inventory (clamped) to mean-revert the book's risk.",
    howItWorks:
      "POSITION is the inventory-skew feature: it computes a skew s = clamp(κ · q, ±sMax) from the signed net position q and shifts the whole two-way to shed that risk. Long inventory (q > 0) skews the book down — cheaper offer, lower bid — so fills reduce the position; short inventory skews up. The spread is unchanged, so the book never crosses.",
    example: {
      scenario: "Mid 99.55, long position q = +1, half-spread 0.05.",
      rows: [
        { label: "κ (per unit)", value: "0.5" },
        { label: "sMax (clamp)", value: "0.05" },
        { label: "Skew = clamp(0.5·1, ±0.05)", value: "0.05 (clamp binds)" },
        { label: "Outbound two-way", value: "99.45 / 99.55 (shifted down 0.05)" },
      ],
      takeaway: "Being long pushes the whole book down so sellers to you are less welcome and buyers keener — you shed the long.",
    },
    howToConfigure: [
      "Add a POSITION feature (needs a live inventory feed — with no position the skew is 0).",
      "Set κ (skew per unit inventory) small first.",
      "Set sMax (the maximum lean) as a hard clamp.",
      "Raise κ until inventory mean-reverts without self-adverse fills.",
    ],
    whenToUse:
      "When you hold risk you want to bleed off through your own two-way rather than hedging externally.",
    risks:
      "Mis-set κ over-skews and produces self-adverse fills. Inventory is not the sole spread driver for bonds — don't over-weight it.",
    keywords: ["position", "inventory", "skew", "kappa", "lean", "risk", "mean-revert", "smax"],
    tourId: "build-pricing-group",
  },
  {
    id: "feature.panic-skew",
    category: "feature",
    title: "PANIC / SKEW",
    purpose: "An emergency directional overlay applied on top of the tier while triggered.",
    howItWorks:
      "PANIC/SKEW is a separate risk overlay — NOT part of the margin. While the panic flag is triggered it shifts the mid by a fixed emergency skew; untriggered it is a no-op. It sits last in the pipeline so it moves the already-tiered price. Its applied_skew is recorded separately in the execution provenance.",
    example: {
      scenario: "Tiered two-way 99.30 / 99.80 (mid 99.55).",
      rows: [
        { label: "Emergency skew", value: "+0.02" },
        { label: "Triggered", value: "yes ⇒ mid 99.57 → 99.32 / 99.82" },
        { label: "Not triggered", value: "no-op ⇒ 99.30 / 99.80 unchanged" },
      ],
      takeaway: "It is an on/off emergency lean layered over the margin, not a spread change.",
    },
    howToConfigure: [
      "Add a PANIC/SKEW feature at the end of the pipeline.",
      "Set the emergency skew magnitude and direction (signed).",
      "Toggle Triggered when the desk activates the overlay.",
      "Confirm the preview only moves while Triggered is on.",
    ],
    whenToUse:
      "In a stressed / event window where the desk wants an immediate directional protection over the normal margin.",
    risks:
      "Leaving the trigger on after the event keeps the book skewed. It double-shifts if you also have an AXE running in the same direction.",
    keywords: ["panic", "skew", "emergency", "overlay", "risk", "trigger", "stress"],
    tourId: "build-pricing-group",
  },

  // ============================ TIERING STRATEGIES ==========================
  {
    id: "strategy.flat-markup",
    category: "strategy",
    title: "Flat markup",
    purpose: "The always-on baseline: a constant symmetric half-spread H around mid, no skew.",
    howItWorks:
      "h = H (constant), s = 0. A fixed margin the same for every tick, independent of inventory or the observed market spread. Set H in the config's spread unit; the engine converts it to a price offset (Price bps: 1 bp = 0.01 price points).",
    example: {
      scenario: "Mid 99.55, H = 25 price bps.",
      rows: [
        { label: "25 price bps", value: "= 0.25 price points" },
        { label: "Bid = mid − H", value: "99.30" },
        { label: "Offer = mid + H", value: "99.80" },
        { label: "Result", value: "99.30 / 99.80 — a fixed 0.50 spread" },
      ],
      takeaway: "Predictable and simple; the reference worked example ±25bp → 99.30 / 99.80.",
    },
    howToConfigure: [
      "Add a Flat markup strategy.",
      "Set the half-spread H in the config's spread unit.",
      "Do NOT also add Scaled Smoothed Spread — that strategy SETS the spread and would double-count.",
      "Optionally add Inventory skew alongside to lean the two-way.",
    ],
    whenToUse:
      "As the default margin when you want a fixed, predictable spread and don't need it to react to inventory or the market spread.",
    risks:
      "Ignores inventory and volatility, and leaks to informed / large flow. It is a floor, not a smart quote.",
    keywords: ["flat", "markup", "baseline", "constant", "half-spread", "margin", "H"],
    tourId: "configure-tiering-feature",
  },
  {
    id: "strategy.inventory-skew",
    category: "strategy",
    title: "Inventory skew",
    purpose: "A base half-spread plus a skew linear in signed inventory, clamped.",
    howItWorks:
      "s = clamp(κ · q, ±sMax), h = H, with the two-way bid = mid − h − s, offer = mid + h − s. A long book (q > 0, κ > 0) skews the whole two-way DOWN — cheaper offer, lower bid — to shed risk; a short book skews up. The spread 2h is unchanged, so the book never crosses.",
    example: {
      scenario: "Mid 99.55, H = 0.25, long book q = +1.",
      rows: [
        { label: "κ", value: "0.10 per unit ⇒ s = 0.10" },
        { label: "Bid = mid − h − s", value: "99.55 − 0.25 − 0.10 = 99.20" },
        { label: "Offer = mid + h − s", value: "99.55 + 0.25 − 0.10 = 99.70" },
        { label: "Result", value: "99.20 / 99.70 — spread 0.50 kept, book shifted down 0.10" },
      ],
      takeaway: "Skew moves the centre without touching the spread, so long inventory mean-reverts through your fills.",
    },
    howToConfigure: [
      "Add an Inventory skew strategy (layer it on top of Flat markup or Scaled Smoothed Spread).",
      "Set the base half-spread H.",
      "Set κ (skew per unit inventory) small, then raise it.",
      "Set the strategy sMax to cap the lean.",
    ],
    whenToUse:
      "When you hold risk and want your own two-way to bleed it off. Requires a live position feed (no position ⇒ skew 0 ⇒ equals Flat markup).",
    risks:
      "Mis-set κ over-skews and produces self-adverse fills. For corporate bonds inventory is not the single dominant spread driver — don't over-rely on it.",
    keywords: ["inventory", "skew", "kappa", "smax", "lean", "long", "short", "position", "mean-revert"],
    tourId: "configure-tiering-feature",
  },
  {
    id: "strategy.scaled-smoothed-spread",
    category: "strategy",
    title: "Scaled Smoothed Spread (SCALE_SMOOTH)",
    purpose: "A spread SOURCE that damps spread volatility by smoothing the observed spread.",
    howItWorks:
      "Per instrument: smooth the observed raw spread with a fading-memory EWMA (Sₙ = w·Rₙ + (1−w)·Sₙ₋₁), measure its divergence Dₙ = |Sₙ − e| from an expected level e, and set an ABSOLUTE output spread Oₙ = min(m, c·(1 + Pₙ)) where Pₙ = 0 inside the dead-band d, else f·Dₙ/e. The two-way is symmetric: bid = M − Oₙ/2, offer = M + Oₙ/2. Use it INSTEAD OF Flat markup (both set the spread).",
    example: {
      scenario: "c = .0002, e = .00008, d = .00004, m = .0008, f = 1.2 (the §9 oracle).",
      rows: [
        { label: "Dₙ ≤ d", value: "Pₙ = 0 ⇒ Oₙ = c = .0002" },
        { label: "Dₙ = e (.00008)", value: "Pₙ = 1.2 ⇒ Oₙ = .00044" },
        { label: "Dₙ = .001", value: "Pₙ = f·Dₙ/e = 15 ⇒ Oₙ = min(m, .0002·16) = .0008 (capped)" },
      ],
      takeaway: "Big spread spikes decay geometrically and the output is capped at m — a smooth, bounded quoted spread.",
    },
    howToConfigure: [
      "Add a Scaled Smoothed Spread strategy (NOT alongside Flat markup).",
      "Set w (smoothing weight, 0 < w ≤ 1 — lower = smoother), and e (expected spread).",
      "Set d (dead-band), c (core spread), m (max output / indicative-fallback), f (scale gain).",
      "Set guardrails so they don't clamp it: spread_floor ≤ c and h_max ≥ m/2.",
      "Inventory skew may still layer a lean on top.",
    ],
    whenToUse:
      "When the raw market spread is jumpy and you want a damped, bounded quoted spread instead of a fixed margin.",
    risks:
      "A locked/crossed composite has no meaningful observed spread — the line then publishes at m and is marked indicative. Never pair it with Flat markup (double-counts the half-spread).",
    keywords: ["scaled", "smoothed", "spread", "scale_smooth", "ewma", "damp", "divergence", "volatility", "output"],
    tourId: "configure-tiering-feature",
  },

  // ================================ CONCEPTS ================================
  {
    id: "concept.bid-offer-tiering",
    category: "concept",
    title: "Bid / offer tiering",
    purpose: "Constructing the price you stream to clients by widening around the mid.",
    howItWorks:
      "We consolidate LP feeds into a composite best bid/offer, then construct our OWN two-way by widening around the mid with a half-spread h and optionally skewing by s: bid = mid − h − s, offer = mid + h − s. The half-spread balances fill frequency vs profit; the skew manages inventory. Guardrails (min/max spread, max skew, anti-cross) clamp the result so the book can never lock or cross.",
    example: {
      scenario: "LP composite bid 99.50 / offer 99.60 (mid 99.55, market spread 0.10).",
      rows: [
        { label: "Tier", value: "flat ±25 bp (h = 0.25, s = 0)" },
        { label: "Your bid = mid − h", value: "99.55 − 0.25 = 99.30" },
        { label: "Your offer = mid + h", value: "99.55 + 0.25 = 99.80" },
        { label: "You stream", value: "99.30 / 99.80 — our spread 0.50" },
      ],
      takeaway: "The desk's margin is the difference between your 0.50 quoted spread and the 0.10 LP spread.",
    },
    howToConfigure: [
      "Open Fixed Income → Pricing → Tiering tab (or a pricing group's TIERING feature).",
      "Enable tiering and pick a spread unit.",
      "Add a Flat markup strategy and set the half-spread (e.g. 25 price bps).",
      "Set guardrails and apply — the sample preview shows 99.50 / 99.60 → 99.30 / 99.80.",
    ],
    whenToUse:
      "Every outbound stream/quote: it is how the desk turns raw liquidity into a margined, risk-aware two-way.",
    risks:
      "Too tight leaks margin to informed flow; too wide loses trades. Always keep the anti-cross / spread-floor guardrails on.",
    keywords: ["bid", "offer", "tiering", "widen", "mid", "half-spread", "skew", "spread", "guardrail", "composite"],
    tourId: "bid-offer-tiering",
  },
  {
    id: "concept.pricing-groups",
    category: "concept",
    title: "Pricing groups",
    purpose: "Bind sets of clients / FIX sessions to their own pricing pipelines.",
    howItWorks:
      "A pricing group is a named grouping that maps many FIX sessions / users / desks to ONE ordered feature pipeline (RAW → the trader's features → OUTBOUND). Different groups get different margins off the SAME raw liquidity. Each group carries a separate pipeline per mode (ESP vs RFS/RFQ), and every member resolves to exactly one group for deterministic pricing.",
    example: {
      scenario: "Same RAW composite 99.50 / 99.60 (mid 99.55), two client groups.",
      rows: [
        { label: "GROUP-A", value: "Flat +25 bp ⇒ 99.30 / 99.80" },
        { label: "GROUP-B", value: "Flat +40 bp ⇒ 99.15 / 99.95" },
        { label: "Membership", value: "each client's FIX session resolves to one group" },
      ],
      takeaway: "One raw price, per-client margins — the standard dealer client-tiering model.",
    },
    howToConfigure: [
      "Open Fixed Income → Pricing → Pricing Groups tab and create a group.",
      "Enable a custom pipeline and drag features (MID SHIFT · TIERING · AXE · POSITION · PANIC/SKEW) onto the canvas.",
      "Configure each feature and watch the live preview waterfall.",
      "Assign FIX connections / users / desks as members, then save.",
    ],
    whenToUse:
      "When different clients or relationships should receive different prices off the same liquidity.",
    risks:
      "A member in two enabled groups breaks deterministic pricing (validated against). Keep the desk-default fallback tier sane.",
    keywords: ["pricing", "group", "pipeline", "client", "tier", "membership", "feature", "drag"],
    tourId: "build-pricing-group",
  },
  {
    id: "concept.esp-vs-rfs",
    category: "concept",
    title: "ESP vs RFS / RFQ",
    purpose: "The two outbound pricing modes a group prices independently.",
    howItWorks:
      "ESP (executable streaming prices) is the continuous RFS / aggregated-book stream a client subscribes to. RFS/RFQ is request-for-quote pricing plus quote/order execution. A pricing group configures a SEPARATE feature pipeline for each mode — or shares one — so streamed and quoted prices can differ by design.",
    example: {
      scenario: "One group, two modes off the same RAW.",
      rows: [
        { label: "ESP pipeline", value: "RAW → TIERING (+20 bp) → stream" },
        { label: "RFQ pipeline", value: "RAW → TIERING (+35 bp) → AXE → quote" },
        { label: "Share toggle", value: "on ⇒ RFQ mirrors ESP; off ⇒ edit each separately" },
      ],
      takeaway: "Streaming is often keener than a quoted, last-look RFQ price — the modes are configured apart.",
    },
    howToConfigure: [
      "In a pricing group, use the ESP / RFQ toggle on the canvas.",
      "Turn the share switch OFF to edit the RFQ pipeline independently of ESP.",
      "Compose each mode's pipeline from the palette.",
    ],
    whenToUse:
      "Whenever streamed and quoted flow warrant different margins or features (e.g. an AXE only on RFQ).",
    risks:
      "Forgetting a mode falls back to the book-default pricing — verify both pipelines before relying on them.",
    keywords: ["esp", "rfs", "rfq", "streaming", "quote", "mode", "pipeline", "share"],
    tourId: "build-pricing-group",
  },
  {
    id: "concept.pricing-provenance",
    category: "concept",
    title: "Pricing provenance",
    purpose: "Every execution records the raw price, the markup applied, the group and strategy.",
    howItWorks:
      "When the per-client two-way is computed the engine stamps a provenance waterfall — the two-way after each pipeline stage — onto the Quote, and copies it to the Execution / Deal. Analytics can then reconstruct exactly what a client executed against: RAW → constructed → tiered → outbound, plus the applied margin and skew per feature.",
    example: {
      scenario: "A client lifts an offer priced through RAW → MID SHIFT → TIERING.",
      rows: [
        { label: "raw_mid", value: "99.55 (consolidated LP composite)" },
        { label: "tiered two-way", value: "99.30 / 99.80" },
        { label: "applied_margin", value: "0.25 (the TIERING markup)" },
        { label: "realized markout", value: "executed price vs raw_mid, per group / strategy / client" },
      ],
      takeaway: "Provenance turns each fill into an auditable, per-feature attribution of the price.",
    },
    howToConfigure: [
      "Provenance is captured automatically where tiering is applied — no configuration.",
      "Surface it on the deal blotter / executions-analytics view.",
      "Use realized markout (executed vs raw_mid) to calibrate tiers against real flow.",
    ],
    whenToUse:
      "Post-trade: to explain a fill, audit the markup, or calibrate margins from realized markouts.",
    risks:
      "Provenance is only as good as the raw_mid captured at quote time — a stale raw distorts the markout attribution.",
    keywords: ["provenance", "execution", "markout", "raw", "margin", "audit", "analytics", "waterfall", "attribution"],
    tourId: "build-pricing-group",
  },

  // ============================== CURVES ====================================
  {
    id: "concept.curve-definitions",
    category: "concept",
    title: "Curve definitions",
    purpose: "Name, persist, and manage the discount curves the desk prices and risks off.",
    howItWorks:
      "A curve definition is a reference-data record: an immutable slug id, a display name, the projected/discount index, its day-count and holiday calendar, an interpolation scheme, and the calibrating par-OIS pillar set (currency + reference date + ≥1 pillar). The engine bootstraps a discount curve from the pillars; the metadata says how. Exactly ONE curve per currency is the PRIMARY (default) — the server maintains that invariant, promoting/demoting as you create, edit, or delete.",
    example: {
      scenario: "A USD desk runs two curves off SOFR.",
      rows: [
        { label: "usd-sofr", value: "USD-SOFR · ACT/360 · USD cal · log-linear DF · PRIMARY" },
        { label: "usd-sofr-street", value: "a street/mark variant · monotone-convex forward · non-primary" },
        { label: "Slug", value: "immutable — renaming the display name never re-keys the record" },
        { label: "Pillars", value: "each carries a tenor (years / months / date) and a par rate" },
      ],
      takeaway: "One currency, many named curves; the primary is the default the pricer reaches for.",
    },
    howToConfigure: [
      "Open Fixed Income → Curves → Dashboard to see every persisted curve.",
      "Click New curve, then fill display name, currency, index, day-count, calendar and pick an interpolation scheme.",
      "A slug id is generated from the display name on create; it is immutable thereafter.",
      "Select a curve and open the Pillars tab to edit its calibrating par-OIS ladder, then Save.",
      "Editing / creating / deleting needs the Refdata·Fixed-Income capability; viewers see the dashboard read-only.",
    ],
    whenToUse:
      "Whenever the desk needs more than one discount curve — a primary plus street/mark or scenario variants — each named and persisted rather than hand-built per request.",
    risks:
      "The slug is immutable — pick it deliberately. You cannot delete a currency's primary while siblings remain (make another primary first); malformed pillars (non-increasing maturities, an unbootstrappable set) are rejected as invalid.",
    keywords: ["curve", "definition", "discount", "reference data", "slug", "primary", "pillars", "index", "day count", "calendar", "manage"],
  },
  {
    id: "concept.curve-interpolation",
    category: "concept",
    title: "Curve interpolation",
    purpose: "How the bootstrap connects the pillars between quotes — the two supported schemes.",
    howItWorks:
      "Interpolation is the rule the bootstrap uses BETWEEN calibrating pillars. LOG-LINEAR ON THE DISCOUNT FACTOR (the shipping default) is linear in ln DF, so instantaneous forwards are piecewise-constant — arbitrage-free in DF space, simple and robust. MONOTONE-CONVEX ON THE FORWARD (Hagan–West) interpolates the instantaneous forward with a shape-preserving monotone-convex spline — smoother forwards, no spurious oscillation. It is a per-curve choice on the definition, carried on the wire as an int code (0 = log-linear-df, 1 = monotone-convex-forward).",
    example: {
      scenario: "Same pillar quotes, two schemes.",
      rows: [
        { label: "Log-linear DF (0)", value: "ln DF linear between pillars ⇒ step forwards" },
        { label: "Monotone-convex fwd (1)", value: "shape-preserving spline on f(t) ⇒ smooth forwards" },
        { label: "Both", value: "reprice the calibrating pillars exactly; they differ only between them" },
      ],
      takeaway: "Pick log-linear for robustness and speed; monotone-convex when smooth forwards matter.",
    },
    howToConfigure: [
      "Open a curve's definition editor (New curve, or edit an existing one).",
      "Under Interpolation, choose Log-linear (DF) or Monotone convex (forward).",
      "Save — the choice is persisted on the definition and used by the engine's bootstrap.",
    ],
    whenToUse:
      "Log-linear DF as the default for OIS discounting; monotone-convex forward when a smooth forward curve is needed (e.g. forward-rate-sensitive analytics) and you accept the extra shaping.",
    risks:
      "Monotone-convex is more sensitive to noisy / crossed pillar quotes than log-linear. Both are only as good as the pillar set — a sparse ladder leaves large gaps the scheme must span.",
    keywords: ["interpolation", "log-linear", "discount factor", "monotone convex", "forward", "hagan", "west", "scheme", "bootstrap", "smooth"],
  },

  // ============================== HEDGING ===================================
  {
    id: "concept.hedge-execution-mode",
    category: "concept",
    title: "Hedge execution mode",
    purpose: "Decide whether a hedge policy is a dry-run or trades live — and how it externalises.",
    howItWorks:
      "Each auto-hedge policy is armed at one of four execution modes (it replaces the old advisory-only on/off). ADVISORY is a dry-run: the engine computes intents and stamps shadow provenance but trades NOTHING. LP PANEL routes every external leg to the standing LP panel only. COMPOSITE crosses the live consolidated Agg-Book composite mid, charging the composite half-spread. LP PANEL → COMPOSITE (the default) tries the LP panel first and falls back to the composite mid. Whenever a mode crosses the composite, the composite spread (bp) sets the half-spread charged around the consolidated mid, so a live composite hedge books a real offsetting leg at mid ± spread and stamps a real price, signed slippage, and lpWon = COMPOSITE.",
    example: {
      scenario: "Breach hedge of a long book, composite mid 100.25, composite spread 0.5 bp.",
      rows: [
        { label: "Advisory", value: "intent emitted, nothing traded — no price on the hedge row" },
        { label: "LP panel", value: "fan the external leg to the standing panel; lpWon = the touched LP" },
        { label: "Composite", value: "cross the mid: hedge px 100.25 ± (100.25 · 0.5/10000); lpWon = COMPOSITE" },
        { label: "LP panel → Composite", value: "try the panel, else the composite mid (the default)" },
      ],
      takeaway: "Advisory is the safe shadow; the three live modes differ only in where the external leg fills.",
    },
    howToConfigure: [
      "Open Hedging → Monitor → Engine controls.",
      "Pick an execution mode: Advisory (dry-run), LP panel, Composite, or LP panel → Composite.",
      "For a composite-touching mode, set the Composite spread (bp) — the half-spread around the consolidated mid (default 0.5).",
      "Keep the kill switch for a hard, firm-wide halt (all risk warehouses) independent of the mode.",
    ],
    whenToUse:
      "Run Advisory while you calibrate a new policy, then arm LP panel → Composite for production so the desk gets fills from the panel with a composite backstop.",
    risks:
      "A live mode trades real risk — verify the policy in Advisory first. Too tight a composite spread over-crosses the book at the mid; too wide leaves residual. The kill switch overrides every mode.",
    keywords: ["execution", "mode", "advisory", "lp panel", "composite", "spread", "live", "dry-run", "hedge", "kill switch"],
  },
  {
    id: "concept.hedge-policy-scope",
    category: "concept",
    title: "Hedge policy scope & Clear risk",
    purpose: "Bind a hedge exit policy to the Firm, a single Book, or a Bucket (whole portfolio).",
    howItWorks:
      "A hedge exit policy can be authored at three scopes. FIRM is the single default policy evaluated for every book with no override. BOOK is a per-book override whose conditions read that one book's own net risk. BUCKET binds to a risk-book subtree ROOT and its conditions read the WHOLE portfolio's rolled-up aggregate (net notional / DV01 / …) — so you can write “if PORTFOLIO notional > n → submit market order / clear risk”. A Book or Bucket with an EMPTY policy falls back to the Firm policy (saving an empty override removes it). Among the exit actions, CLEAR RISK is a parameter-free leaf that flattens the book's entire net to zero via the live composite — the “panic flat” terminal.",
    example: {
      scenario: "A rates portfolio rolled up under the subtree-root book fi-emea.",
      rows: [
        { label: "Firm", value: "default: Breach → Split (net then hedge)" },
        { label: "Book fi-rates-emea", value: "override: Counterparty = CITADEL → Submit market order" },
        { label: "Bucket fi-emea", value: "if PORTFOLIO net notional > 5bn → Clear risk (flatten to zero)" },
        { label: "Empty override", value: "removes the scope's policy ⇒ it falls back to Firm" },
      ],
      takeaway: "Firm sets the baseline; Book/Bucket refine it; Bucket reads the whole portfolio aggregate.",
    },
    howToConfigure: [
      "Open Hedging → Exit Policy and pick the Policy scope: Firm, Book, or Bucket.",
      "For Book/Bucket, choose the risk book (a Bucket is a subtree-root portfolio) from the picker.",
      "Author the rules table (IF <conditions> THEN <exit action>) exactly as for the Firm policy, then Save.",
      "Add Clear risk as the exit action to flatten the book's whole net to zero (no parameters).",
      "Use “Remove override” on a Book/Bucket to delete its policy and fall back to Firm.",
    ],
    whenToUse:
      "Use Firm for the house default, a Book override for a desk with distinct handling, and a Bucket policy when a decision must read the whole portfolio's aggregate rather than one book. Reach for Clear risk when a breach must be flattened outright.",
    risks:
      "A Bucket condition reads the ROLLED-UP aggregate, not a single book — size thresholds accordingly. Clear risk flattens the ENTIRE net in one action; use it only where a full flatten is intended. An empty Book/Bucket policy silently falls back to Firm.",
    keywords: ["scope", "firm", "book", "bucket", "portfolio", "aggregate", "subtree", "clear risk", "flatten", "override", "policy"],
  },
  {
    id: "concept.hedge-rule-from-pricing-group",
    category: "concept",
    title: "Create a hedging rule from a pricing group",
    purpose: "Seed a hedge exit-policy rule pre-scoped to a pricing group, then finish it in the Hedging builder.",
    howItWorks:
      "From a pricing group's editor (the “Create hedging rule” button) or its roster row (right-click / the context-menu key) you hand off a NEW draft rule to Hedging → Exit Policy. Only ONE fact carries across: the group's DESK membership. A pricing group has no currency, product or counterparty and no book/bucket scope, and a hedge rule is a flat AND of conditions (it cannot OR several desks), so the seed is honest about the gap. Exactly one member desk seeds a `desk = <desk>` condition; zero or many desks seed a NO-condition draft (identical to the manual “+ Create hedge rule”) and the hint tells you what to add. The scope stays FIRM and the exit action defaults to Warehouse — you complete the conditions, pick the real exit action, and Save.",
    example: {
      scenario: "Three pricing groups handed off to the Hedging builder.",
      rows: [
        { label: "One desk (g10-rates)", value: "seeds Desk = g10-rates at Firm scope" },
        { label: "Many desks", value: "no condition seeded — add one desk condition (one rule per desk)" },
        { label: "No desks (user/session)", value: "no condition seeded — the graph can't test membership; add conditions" },
        { label: "Always", value: "exit action defaults to Warehouse — pick the real action + Save" },
      ],
      takeaway: "Desk membership is the only bridge into the hedge vocabulary; the builder is where you finish the rule.",
    },
    howToConfigure: [
      "Open Fixed Income → Pricing → Pricing Groups.",
      "Either open a group and click “Create hedging rule”, or right-click the group in the roster (or press the context-menu key) and choose “Create hedging rule”.",
      "You land in Hedging → Exit Policy with a new draft: a single-desk group is pre-scoped Desk = <desk>; otherwise add the conditions the hint asks for.",
      "Pick the exit action (Warehouse is the safe default), review the preview, and Save — nothing auto-saves.",
      "Seeding needs the Hedge · Fixed-Income capability — you can seed from a group you can only view.",
    ],
    whenToUse:
      "When a pricing group corresponds to a desk whose warehoused flow you want an exit policy for, and you want the desk condition and the jump to the builder in one click instead of hand-composing it.",
    risks:
      "A group's desk is its ONLY testable overlap — the seed never invents a currency/product/counterparty condition a group can't substantiate. A multi-desk or user/session group seeds no condition on purpose; author one rule per desk. The rule is Firm-scoped — narrow it to a Book/Bucket in the builder if you need per-book handling.",
    keywords: ["hedging", "rule", "pricing group", "seed", "desk", "exit policy", "create", "right-click", "context menu", "firm", "warehouse"],
  },
];

/** The registry keyed by id (built once from the ordered list). */
export const HELP_ENTRIES: Readonly<Record<string, HelpEntry>> = Object.freeze(
  Object.fromEntries(ENTRY_LIST.map((e) => [e.id, e])),
);

/** All entries in authored (index) order. */
export const HELP_INDEX: readonly HelpEntry[] = ENTRY_LIST;

/** Look up one entry by id (undefined when unknown). */
export function getHelp(id: string): HelpEntry | undefined {
  return HELP_ENTRIES[id];
}

/** The entries of one category, in index order. */
export function helpByCategory(category: HelpCategory): HelpEntry[] {
  return ENTRY_LIST.filter((e) => e.category === category);
}

/** The searchable haystack for an entry (lower-cased title + purpose + prose + keywords). */
function haystack(e: HelpEntry): string {
  return [e.title, e.purpose, e.howItWorks, e.whenToUse, ...e.keywords].join(" ").toLowerCase();
}

/**
 * Search the registry: tokenise the query on whitespace and rank each entry by how
 * many query terms it contains (title/keyword hits weighted higher than prose).
 * An empty query returns the full index in order. Case-insensitive substring match
 * so "bid offer tiering" surfaces the bid/offer-tiering concept first.
 */
export function searchHelp(query: string): HelpEntry[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return [...ENTRY_LIST];
  const scored = ENTRY_LIST.map((e) => {
    const hay = haystack(e);
    const title = e.title.toLowerCase();
    const kw = e.keywords.map((k) => k.toLowerCase());
    let score = 0;
    for (const t of terms) {
      if (!hay.includes(t)) continue;
      score += 1; // base: term present anywhere
      if (title.includes(t)) score += 3; // title hit weighs more
      if (kw.some((k) => k === t)) score += 2; // exact keyword hit
    }
    return { e, score };
  }).filter((s) => s.score > 0);
  scored.sort((a, b) => b.score - a.score || a.e.title.localeCompare(b.e.title));
  return scored.map((s) => s.e);
}
