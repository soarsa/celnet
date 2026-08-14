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
    id: "concept.risk-models",
    category: "concept",
    title: "Risk models \u2014 back-to-back vs internalise",
    purpose: "Choosing how a risk portfolio manages the risk it takes on.",
    howItWorks:
      "Every risk portfolio runs one of three postures. BACK-TO-BACK hedges every fill straight out on the street and warehouses nothing \u2014 you capture the spread between the client price and the street price, and carry no directional exposure between the two. INTERNALISE TO A DV01 BUDGET holds client flow against a DV01 budget so opposing client flow can net off internally, and sheds only the overflow above the band edge; you keep more spread (no street leg on the netted portion) at the cost of carrying risk. CUSTOM is the escape hatch: the exit-policy graph authored for that scope governs unchanged, and it is what an unbound portfolio means. The model is a control over the existing exit primitives, not a second engine \u2014 each derives an ordinary hedge graph from the same vocabulary you could author by hand.",
    example: {
      scenario:
        "Desk EMEA is bound to Internalise with a 25,000 DV01 budget; the toxic book FI-MAREX is bound to Back-to-back.",
      rows: [
        { label: "FI-RATES (no binding of its own)", value: "inherits desk EMEA \u2192 Internalise, budget 25,000" },
        { label: "FI-MAREX (bound at book scope)", value: "Back-to-back \u2014 its own binding beats the desk" },
        { label: "A fill into FI-RATES", value: "warehoused while book DV01 \u2264 25,000; only the overflow is shed" },
        { label: "A fill into FI-MAREX", value: "hedged out immediately, in full" },
      ],
      takeaway:
        "Precedence is most-specific-wins \u2014 instrument beats book beats desk \u2014 so one toxic book can run back-to-back while the rest of the estate warehouses.",
    },
    howToConfigure: [
      "Open Risk \u2192 Portfolios and select the portfolio.",
      "In the Risk model panel, choose Custom, Back-to-back, or Internalise to a DV01 budget.",
      "For Internalise, set the DV01 warehouse budget (or leave it blank to inherit the configured threshold).",
      "Read the resolution line beneath the picker \u2014 it states which scope actually supplied the posture.",
      "The model saves immediately; it lives on the firm hedge config, not the portfolio definition, so it does not wait for Save changes.",
    ],
    whenToUse:
      "Back-to-back for toxic or illiquid flow where you do not want carry. Internalise where two-way client flow is genuine and netting it internally earns more than it risks.",
    risks:
      "Internalising carries directional risk between the client trade and the shed \u2014 that is the point, but it must be sized. A portfolio with no binding is CUSTOM, which means its authored graph governs; it does NOT mean 'no hedging'.",
    keywords: ["risk model", "back to back", "back-to-back", "internalise", "internalize", "warehouse", "posture", "scope", "precedence", "hedging model"],
  },
  {
    id: "concept.dv01-hedge",
    category: "concept",
    title: "DV01 and how the hedge is sized",
    purpose: "Neutralising interest-rate risk on a bond position by shorting a benchmark.",
    howItWorks:
      "DV01 is the P&L change for a 1 basis-point move in yield. For a bond position it is approximately notional \u00d7 modified duration \u00d7 0.0001 \u2014 though the engine computes it analytically from the bond's cashflows rather than from that approximation, and validates the result against a finite-difference reprice. To immunise, the desk shorts a liquid benchmark carrying the SAME DV01: the hedge size is the ratio of the two DV01s, units = position DV01 \u00f7 the vehicle's DV01 per unit. The vehicle is chosen from the firm's registry by product, currency and maturity bucket. If the registry has no DV01 per unit for it, the engine REFUSES to size the trade and falls back to selling the same security back \u2014 it will not trade a guessed size.",
    example: {
      scenario:
        "The desk buys $50,000,000 face of a 10-year corporate bond with a spread duration of 7.0 years, and hedges with the on-the-run 10-year Treasury (DV01 $800 per $1,000,000).",
      rows: [
        { label: "Position DV01", value: "$50,000,000 \u00d7 7.0 \u00d7 0.0001 = $35,000 per bp" },
        { label: "Hedge vehicle DV01 per $1mm", value: "$800" },
        { label: "Treasury notional to short", value: "$35,000 \u00f7 $800 = $43.75mm" },
        { label: "Resulting book", value: "Long $50mm corporates, short $43.75mm Treasuries \u2192 net rate DV01 \u2248 0" },
      ],
      takeaway:
        "The hedge ratio is a DV01 ratio, not a notional ratio \u2014 $43.75mm of Treasuries offsets $50mm of corporates because their durations differ.",
    },
    howToConfigure: [
      "Open Hedging \u2192 the vehicle registry and confirm a row exists for the product / currency / maturity you trade.",
      "Give each vehicle its DV01 per unit \u2014 this is the denominator of the hedge ratio, and without it the engine will not size a trade.",
      "On the portfolio, choose a risk model: Back-to-back hedges the whole DV01, Internalise hedges only the overflow above the budget.",
      "Check the hedge trace after a fill: it shows the DV01 used, the vehicle chosen, and the resulting size.",
    ],
    whenToUse:
      "Whenever the desk holds a rate-sensitive position it does not intend to take a directional view on.",
    risks:
      "A DV01 hedge neutralises RATE risk only. Credit spread risk (CS01) survives it untouched \u2014 a corporate bond hedged to DV01-neutral is not a flat position. Duration also drifts as yields and time move, so the ratio needs re-striking, and a whole-contract futures hedge leaves a rounding residual.",
    keywords: ["dv01", "duration", "hedge ratio", "immunise", "immunize", "treasury", "benchmark", "basis point", "pv01", "neutral"],
  },
  {
    id: "concept.dv01-budget",
    category: "concept",
    title: "DV01 budget vs DV01 limit",
    purpose: "Where you start hedging versus where you stop trading.",
    howItWorks:
      "These are two different controls and they are not interchangeable. The DV01 BUDGET is soft: it is the level a portfolio warehouses up to before it begins shedding risk to the street, and crossing it triggers hedging. The DV01 LIMIT is hard: it is a pre-trade cap, and crossing it blocks the trade. The budget should sit below the limit \u2014 the gap between them is the working room where the desk manages inventory. Leaving the budget BLANK means inherit the scope's configured warehouse threshold; it does not mean a budget of zero. A zero budget would mean 'warehouse nothing', the opposite of what an empty field implies, so the engine treats non-positive as inherit and never invents a cap.",
    example: {
      scenario: "A rates portfolio with a 25,000 DV01 budget and a 40,000 DV01 limit.",
      rows: [
        { label: "Book DV01 = 18,000", value: "inside the budget \u2014 flow is warehoused, nothing is shed" },
        { label: "Book DV01 = 30,000", value: "over budget \u2014 the 5,000 overflow is hedged out to the street" },
        { label: "A fill taking DV01 to 42,000", value: "over the hard limit \u2014 blocked pre-trade" },
        { label: "Budget left blank", value: "inherits the scope's configured warehouse threshold" },
      ],
      takeaway: "Budget is where you start working out of a position; limit is where you stop taking it on.",
    },
    howToConfigure: [
      "Set the DV01 budget in the portfolio's Risk model panel (visible only for the Internalise model).",
      "Set the DV01 limit in the portfolio's Pre-trade limits box.",
      "Keep budget below limit \u2014 if they are equal, the desk has no room to shed before trading is blocked.",
    ],
    whenToUse:
      "Any portfolio running the Internalise model. Back-to-back warehouses nothing, so the budget does not apply to it.",
    risks:
      "The pre-trade DV01 LIMIT is not yet enforced at the booking gate \u2014 net and gross notional caps are. Treat the DV01 limit as documentation of intent until that seam lands, and rely on the budget for live risk control.",
    keywords: ["dv01 budget", "dv01 limit", "warehouse", "threshold", "inherit", "blank", "pre-trade", "cap", "overflow", "band"],
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
    id: "feature.lp-panel",
    category: "feature",
    title: "LP PANEL — INBOUND LIQUIDITY",
    purpose:
      "See which liquidity providers are actually feeding the platform, and how much of each composite they really set.",
    howItWorks:
      "Administration → LP Panel folds one row per provider across every ENABLED aggregated book that lists it as a member. A provider is identified by its FIX connection id — the same string is the aggregation venue id and the lp_name each inbound quote carries — so one row ties a connection, its book membership and its venue together. Rate is quote-updates per second, measured by differencing the server's lifetime counter across polls. Fresh vs Excluded is the consolidation's own verdict: a quote aged past the book's max quote age, or gated as a divergent outlier, sets no price. Top of book counts the instruments where the provider is on the best bid / best offer, and Weight is its mean share of the composite mid. Open Quotes on a row for the per-instrument detail behind those numbers.",
    example: {
      scenario: "A venue that looks healthy on Connections but is contributing nothing.",
      rows: [
        { label: "Connections says", value: "◉ running, bound on 127.0.0.1:56003" },
        { label: "LP Panel says", value: "stale · 8 instruments · 0 fresh · 8 excluded" },
        { label: "Quotes drill-down", value: "every line verdict = stale, weight 0%" },
        { label: "Diagnosis", value: "the feed is connected but its ticks are older than the book's max quote age" },
      ],
      takeaway:
        "Bound is not the same as contributing — only this screen distinguishes a live venue from a silent or excluded one.",
    },
    howToConfigure: [
      "Open Administration → LP Panel (requires the Manage Liquidity capability).",
      "Read the summary strip first: if the inbound kill-switch banner is showing, ingest is off firm-wide and every row below is expected to go stale.",
      "Sort by Rate to find silent providers, or by Weight to see who actually drives the composite.",
      "Check the Books column — 'none' means the provider is in no enabled book, so its pushes are dropped at ingest.",
      "A blank provider name with 'no connection' means a book lists a member id that matches no managed FIX connection; fix the id in Aggregation or define the connection in Connections.",
      "Click Quotes on a row to inspect its live prices, ages, weights and per-instrument verdicts.",
    ],
    whenToUse:
      "Whenever composite pricing looks wrong, thin, or unexpectedly wide — and after any change to a book's membership, to confirm the intended venues are genuinely contributing.",
    risks:
      "Rate needs two polls before it reads anything, so a just-opened panel shows a dash rather than zero. The counter is lifetime since the edge started, so an edge restart resets it. A provider excluded everywhere reports 0% weight — that is a real exclusion, not a missing measurement.",
    keywords: [
      "lp",
      "lp panel",
      "inbound",
      "liquidity",
      "provider",
      "venue",
      "feed",
      "stale",
      "top of book",
      "composite",
      "aggregated book",
      "contribution",
      "weight",
      "simulator",
      "lp-sim",
    ],
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
      "Open Hedging Rules → Execution mode.",
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
      "Open Hedging Rules → Exit Policy and pick the Policy scope: Firm, Book, or Bucket.",
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
    id: "concept.hedge-field-availability",
    category: "concept",
    title: "Why some risk-state fields are unavailable",
    purpose: "Understand the greyed-out condition fields, and why a rule on one is refused.",
    howItWorks:
      "A hedge condition is only meaningful if something actually MEASURES the field it tests. Most of the vocabulary is computed on every fill — net DV01, net notional, utilisation, overflow, breached, inventory sign, internal offset available, hedge cost, and the identity fields (instrument, currency, product, book, counterparty). A few are not, because the data they would need does not exist anywhere in the platform: DESK (a booked fill carries no desk — the desk belongs to the FIX/RFQ session that priced the quote, not to the resulting position), NET VEGA and NET GAMMA (only linear-rates cells are evaluated, and they carry no volatility or convexity risk), COUNTERPARTY TOXICITY (no post-fill mark trajectory is retained, so markout cannot be derived) and INVENTORY AGE (a stored position carries no acquisition timestamp). A condition on one of those would compare against a permanent zero, so it could never fire — it would sit in your policy looking live and silently do nothing. Rather than let that happen, the field is shown greyed-out with its reason, the builder refuses to select it, and the server refuses to save a policy containing it. The fields are shown rather than hidden on purpose: an older policy may still reference one, and you need to see why it stopped being selectable.",
    example: {
      scenario: "A trader tries to write “counterparty toxicity > 0.6 → submit market order”.",
      rows: [
        { label: "In the palette", value: "the chip is dashed and greyed, and states the reason in place of its hint" },
        { label: "In the rule editor", value: "the dropdown entry reads “Counterparty toxicity — unavailable” and cannot be picked" },
        { label: "If a policy still holds one", value: "the field stays selected and shown, with a warning that Save will be refused" },
        { label: "The workable form", value: "name the client directly: Counterparty contains “TOXIC” → submit market order" },
      ],
      takeaway: "An unavailable field is a missing measurement, not a missing feature — express the intent with a field that is measured.",
    },
    howToConfigure: [
      "Read the greyed chip's reason in the risk-state field palette before reaching for it.",
      "Replace an intent that needs toxicity or inventory age with one the platform measures — counterparty identity, utilisation, overflow, or hedge cost.",
      "Scope by BOOK rather than DESK: the book is carried on every fill, the desk is not.",
      "If an existing policy shows the Save-will-be-refused warning, rebind that condition to an available field and Save again.",
    ],
    whenToUse:
      "Whenever a field you expected to use is greyed out, or a policy that looks correct is refused on save. The reason shown is the same one the server gives.",
    risks:
      "These fields are unavailable because nothing measures them — not because they are switched off. Do not read a greyed field as “enable it somewhere”; the intent has to be re-expressed against a measured field.",
    keywords: [
      "unavailable",
      "greyed",
      "disabled",
      "field",
      "toxicity",
      "counterparty toxicity",
      "inventory age",
      "desk",
      "net vega",
      "net gamma",
      "refused",
      "validation",
      "condition",
    ],
  },
  {
    id: "concept.hedge-rule-from-pricing-group",
    category: "concept",
    title: "Create a hedging rule from a pricing group",
    purpose: "Seed a hedge exit-policy rule pre-scoped to a pricing group, then finish it in the Hedging builder.",
    howItWorks:
      "From a pricing group's editor (the “Create hedging rule” button) or its roster row (right-click / the context-menu key) you hand off a NEW draft rule to Hedging Rules → Exit Policy. Only ONE fact carries across: the group's DESK membership. A pricing group has no currency, product or counterparty and no book/bucket scope, and a hedge rule is a flat AND of conditions (it cannot OR several desks), so the seed is honest about the gap. Exactly one member desk seeds a `desk = <desk>` condition; zero or many desks seed a NO-condition draft (identical to the manual “+ Create hedge rule”) and the hint tells you what to add. The scope stays FIRM and the exit action defaults to Warehouse — you complete the conditions, pick the real exit action, and Save.",
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
      "You land in Hedging Rules → Exit Policy with a new draft: a single-desk group is pre-scoped Desk = <desk>; otherwise add the conditions the hint asks for.",
      "Pick the exit action (Warehouse is the safe default), review the preview, and Save — nothing auto-saves.",
      "Seeding needs the Hedge · Fixed-Income capability — you can seed from a group you can only view.",
    ],
    whenToUse:
      "When a pricing group corresponds to a desk whose warehoused flow you want an exit policy for, and you want the desk condition and the jump to the builder in one click instead of hand-composing it.",
    risks:
      "A group's desk is its ONLY testable overlap — the seed never invents a currency/product/counterparty condition a group can't substantiate. A multi-desk or user/session group seeds no condition on purpose; author one rule per desk. The rule is Firm-scoped — narrow it to a Book/Bucket in the builder if you need per-book handling.",
    keywords: ["hedging", "rule", "pricing group", "seed", "desk", "exit policy", "create", "right-click", "context menu", "firm", "warehouse"],
  },
  {
    id: "concept.hedge-rule-wizard",
    category: "concept",
    title: "Exit-policy rule wizard",
    purpose: "Turn a plain-English hedge intent into a complete, valid exit policy — with flow-driven suggested thresholds.",
    howItWorks:
      "The rule wizard (Hedging Rules → Exit Policy → “🪄 Rule wizard”, next to “+ Create hedge rule”) runs in two woven modes. ANALYSE MY FLOW reads the data the GUI already holds — the selected book's rolled-up risk from the risk-dashboard store and its recent client-blotter deals — and surfaces the book's Net DV01 (a $/bp RISK) and Net notional (a $ FACE amount) kept honestly DISTINCT, the position/deal counts, and peak utilisation. From that it suggests a rounded warehouse cap (≈ 1.25× the current |exposure|) and a flatten trigger at 0.8× the cap; with no flow it says so rather than inventing a number. SCENARIO PRESETS then generate the rules: each card is a real intent with a live preview of the exact rules. The generated rules are the SAME first-match-wins graphs the manual builder emits, so they pass the validity/conflict checks; Insert REPLACES the current scope's draft and hands to the normal list + Save path — nothing auto-saves.",
    example: {
      scenario: "DEFAULT_BOOK: net notional ≈ $250m over 6 positions; you want to warehouse then flatten.",
      rows: [
        { label: "Analyse", value: "net DV01 $5k/bp · net notional $250m · peak util 80%" },
        { label: "Suggested cap", value: "$250m × 1.25 → $500m (rounded)" },
        { label: "Flatten trigger", value: "0.8 × $500m = $200m" },
        { label: "Scenario “warehouse then flatten”", value: "Net notional > $200m → Submit market order (flatten); Otherwise → Warehouse" },
      ],
      takeaway: "The wizard writes the two rules; you review them in the validity panel and press Save policy.",
    },
    howToConfigure: [
      "Open Hedging Rules → Exit Policy and pick a scope (Firm, Book or Bucket).",
      "Click “🪄 Rule wizard” next to “+ Create hedge rule”.",
      "Step 1 — pick a book to Analyse; apply a Net DV01 or Net notional suggestion (or skip).",
      "Step 2 — choose a scenario card and set its metric + threshold(s) or counterparty; watch the live rule preview.",
      "Step 3 — review the generated rules (valid + conflict-free), press Insert rules, then Save policy on the Exit Policy screen.",
    ],
    whenToUse:
      "When you want a correct starter exit policy fast — especially to size a “warehouse then flatten” threshold off the book's real exposure rather than guessing, or to stand up a back-to-back / tiered / pure-internalise policy in a few clicks.",
    risks:
      "Insert REPLACES the current scope's draft rules — review before Save. Suggested thresholds are heuristics off current exposure, not a risk mandate; tune them. Net DV01 ($/bp) and Net notional ($) are different metrics — pick the one your appetite is expressed in. A live execution mode still governs whether the policy trades (see Hedge execution mode).",
    keywords: ["wizard", "exit policy", "rule", "scenario", "analyse", "flow", "suggest", "threshold", "net notional", "net dv01", "warehouse", "flatten", "back-to-back", "tiered", "internalise", "guided"],
    tourId: "hedge-rule-wizard",
  },
  {
    id: "concept.hedge-vehicle",
    category: "concept",
    title: "Hedge vehicle — what a rule trades",
    purpose: "Choose what an exit action hedges WITH, not just what kind of exit it is.",
    howItWorks:
      "Every size-bearing exit leaf (Submit market order, RFQ out, Split, Clear risk, Cross internal) now says WITH WHAT as well as WHAT. SELF sells the same security back: leg and fill are the identical instrument, so their DV01 ratio is identically 1 under any duration measure — exact, with no registry, no duration model and no residual. That is the right answer for a swap or a govvie, and it is the default, so every rule authored before this existed keeps behaving exactly as it did. It is NOT how a corporate bond is hedged. A corp is hedged with a benchmark at matching maturity, in practice a Treasury future, and the moment the hedge instrument differs from the position that ratio stops being 1 — the size then needs a DV01 per unit. BENCHMARK resolves the instrument from the hedge-vehicle registry by instrument + maturity bucket. INSTRUMENT and FUTURE name one explicitly; a FUTURE is sized in whole contracts, so it always rounds. A named vehicle must be a registry row, because that row is where the DV01 per unit comes from — the server rejects one it cannot price. Warehouse, Skew and Escalate place no order, so they carry no vehicle at all.",
    example: {
      scenario: "A 9y USD corporate is 138% of its DV01 budget; the shed target is 24,840 DV01.",
      rows: [
        { label: "Self", value: "sell back the same bond — ratio 1, exact, no residual" },
        { label: "Benchmark", value: "the 7–10y USD registry row resolves to TY-DEC26" },
        { label: "DV01 per unit", value: "78 per contract (from that registry row)" },
        { label: "Exact units", value: "24,840 / 78 = 318.46 contracts" },
        { label: "Traded", value: "318 contracts (whole lots) ⇒ 24,804 DV01 hedged" },
        { label: "Residual", value: "+36 DV01 — rounded down, still on the book" },
      ],
      takeaway:
        "Self is exact by construction; any other vehicle needs the registry's DV01 per unit and leaves a rounding residual you should read the sign of.",
    },
    howToConfigure: [
      "Open Hedging Rules → Exit Policy and edit (or create) a rule.",
      "Pick a size-bearing exit action — the “Hedge with” picker appears beneath it.",
      "Leave it on Same security (self) to keep today's exact behaviour.",
      "Choose Benchmark to resolve by maturity bucket, or Named instrument / Named future to pin one.",
      "For a named vehicle, pick from the list — it offers exactly the registered hedge instruments.",
      "If the list is empty, add the vehicle first under Hedging Rules → Vehicles.",
    ],
    whenToUse:
      "Reach for Benchmark or Future the moment a book holds risk that is not economically hedgeable with itself — corporates, illiquid credit, anything where selling the line back would move the market or is simply not possible.",
    risks:
      "A non-self vehicle is only as good as its DV01 per unit: a stale or wrong registry row mis-sizes every hedge that resolves through it. A future rounds to whole contracts, so there is always a residual — read its sign. And a size computed off the coarse bond exposure proxy is approximate, which the suggestion surface flags explicitly.",
    keywords: [
      "vehicle", "hedge with", "benchmark", "future", "contract", "self", "same security", "corporate bond",
      "treasury future", "dv01 per unit", "maturity bucket", "ratio", "exit action",
    ],
  },
  {
    id: "concept.hedge-vehicle-registry",
    category: "concept",
    title: "Hedge-vehicle registry",
    purpose: "Define what each class of risk is hedged with — and the DV01 per unit that sizes it.",
    howItWorks:
      "The registry is the firm's roster of hedge vehicles. Each row matches on three ANDed axes — instrument, product, currency, each optional (empty ⇒ matches anything) — plus a half-open maturity bucket [min, max) in years, and names the instrument a match hedges WITH. It carries that instrument's DV01 per unit and what one unit IS (a “contract” for a future, “1mm face” for cash). The maturity bucket is what makes a BENCHMARK vehicle resolvable: a 9y corporate falls into the 7–10y row and picks up its future. The DV01 per unit is the divisor that turns a target DV01 into tradeable units, so it is the reason a named vehicle must be registered at all — an unregistered instrument has no size. The roster round-trips through the same engine-config save as the LP panels; there is no separate CRUD step.",
    example: {
      scenario: "Four rows covering USD corporates, GBP gilts and USD swaps.",
      rows: [
        { label: "USD BOND 7–10y", value: "→ TY-DEC26, future, 78 DV01/contract" },
        { label: "USD BOND 2–5y", value: "→ FV-DEC26, future, 42 DV01/contract" },
        { label: "GBP BOND 3–7y", value: "→ G-MAR27, future, 64 DV01/contract" },
        { label: "USD OIS any", value: "→ USD-SOFR-OIS-5Y, cash, 480 DV01 per 1mm face" },
        { label: "Unset bucket", value: "min = max = 0 ⇒ any maturity" },
      ],
      takeaway: "One row per (asset class × maturity bucket); the DV01 per unit is the number that does the work.",
    },
    howToConfigure: [
      "Open Hedging Rules → Vehicles.",
      "Press “+ Add hedge vehicle” and give it a unique id.",
      "Narrow the match as far as you need — leave instrument / product / currency empty for “any”.",
      "Set the maturity bucket [min, max) in years; leave both at 0 for any maturity.",
      "Pick the hedge instrument from the search box — it lists the real reference-data universe, and a “live” tag marks the ones an LP is currently quoting (those are the ones that can actually fill).",
      "Prefer a rolling product (“ZF — front month”) over a specific delivery month: it re-points itself at each quarterly roll, so the row keeps working. A pinned month like ZFU26 stops trading and strands every hedge routed at it.",
      "Picking a future fills in the DV01 per unit, the whole-contract flag and the unit label from its published terms — check the figure rather than retyping it.",
      "For a cash bond nothing is pre-filled: its DV01 depends on the live curve, not on any static term, so enter it yourself. It must be greater than zero, or nothing can be sized off the row.",
      "An instrument reference data does not carry (a EUR or GBP future today) can still be typed in and committed — it is flagged as unknown until it is added.",
      "Save; the row is committed with the rest of the engine config.",
    ],
    whenToUse:
      "Before authoring any rule that hedges with a Benchmark, Named instrument or Named future — those vehicles cannot resolve or size without a row here.",
    risks:
      "A future's pre-filled DV01 is its STANDARDIZED figure at the contract's own 6% notional yield — a futures contract has no constant basis-point value, so it understates the live figure whenever yields sit below 6%. Override it if the desk sizes off the live curve. Overlapping maturity buckets make which row wins ambiguous; keep them disjoint. A row pinned to a delivery month rather than a product goes stale at the quarterly roll. Deleting a row that a live rule names leaves that rule unpriceable and the server will reject it on the next save.",
    keywords: [
      "registry", "vehicle", "roster", "dv01 per unit", "maturity bucket", "benchmark", "future", "contract",
      "unit label", "hedge instrument", "product", "currency", "match",
      "picker", "dropdown", "front month", "roll", "auto-roll", "live liquidity", "reference data",
    ],
  },
  {
    id: "concept.hedge-exit-mode",
    category: "concept",
    title: "Exit mode — auto vs suggest",
    purpose: "Decide whether a resolved hedge fires by itself, or waits for you as a standing row.",
    howItWorks:
      "AUTO is the existing behaviour: a breach resolves the exit policy and trades. SUGGEST changes only the last step — the engine still measures the book, still resolves the policy and still SIZES the hedge in its vehicle, then trades nothing and publishes a STANDING suggestion onto Fixed Income → Book → Hedge flows with a “Hedge now” button beside it. It is deliberately NOT a confirmation dialog: nothing pops up, nothing interrupts, and the row is not lost if you ignore it. That is the point — a modal is dismissed reflexively and takes the decision with it, whereas a standing row waits until someone acts on it. Bindings use the same desk / book / instrument scoping and the same most-specific-wins resolution (instrument > book > desk) as the warehouse thresholds and the LP panels. An unbound scope is Auto.",
    example: {
      scenario: "The credit book is bound to SUGGEST; the rates book is left on AUTO.",
      rows: [
        { label: "Rates book breaches", value: "policy resolves → hedge TRADES immediately" },
        { label: "Credit book breaches", value: "policy resolves, hedge is sized — nothing trades" },
        { label: "What you see", value: "a standing row on Fixed Income → Book → Hedge flows: “Sell 318 contracts of TY-DEC26”" },
        { label: "Hedge now", value: "fires the sized hedge — no confirmation step" },
        { label: "Dismiss", value: "drops the row and trades nothing" },
        { label: "Ignore it", value: "the row stays; nothing is lost" },
      ],
      takeaway: "Suggest buys a human check WITHOUT a popup — the deliberation happens on the row, not in a dialog.",
    },
    howToConfigure: [
      "Open Hedging Rules → Exit mode.",
      "Press “+ Bind a scope” and pick desk, book or instrument, then its id.",
      "Choose Auto (fire on breach) or Suggest (raise a standing row).",
      "Save; the binding is committed with the rest of the engine config.",
      "Watch the standing rows under Fixed Income → Book → Hedge flows → Suggestions.",
    ],
    whenToUse:
      "Use Suggest where a hedge deserves a human eye before it goes out — a benchmark-future hedge on an illiquid corporate, an unusually large clip, or a book you are still calibrating. Keep Auto where the flow is routine and latency matters.",
    risks:
      "A suggestion does NOT hedge: risk keeps running until someone acts on the row, so a Suggest-scoped book needs someone watching it. The sized numbers are a snapshot at raise time — the mid and the exposure move afterwards. And Suggest is orthogonal to the execution mode: Advisory still trades nothing even when you press Hedge now.",
    keywords: [
      "exit mode", "auto", "suggest", "standing", "suggestion", "no popup", "no dialog", "hedge now",
      "dismiss", "scope", "desk", "book", "instrument", "most-specific",
    ],
  },
  {
    id: "concept.hedge-suggestion",
    category: "concept",
    title: "How to read a hedge suggestion",
    purpose: "Understand the instruction, the sizing arithmetic, and the residual before you press Hedge now.",
    howItWorks:
      "A suggestion row leads with the INSTRUCTION — “Sell 318 contracts of TY-DEC26” — because that is the thing you would do. Beneath it sits the risk context (RAG band, book · instrument, utilisation, when it was raised) and the engine's rationale. If the hedge uses a vehicle other than the position itself, the row also shows the sizing: the TARGET DV01, the DV01 per unit, the EXACT units before rounding, the units that will actually TRADE, and the RESIDUAL. Read the residual's sign, because it is easy to read backwards: it is target minus hedged, so “rounded down, 36 DV01 still on the book” means you remain under-hedged by 36, while “rounded up, over-hedged by 31 DV01” means the whole-lot rounding took off more than the target. When the size was computed off the duration-blind exposure proxy — the coarse measure that treats every bond as duration 1 — the row carries a prominent APPROXIMATE warning. That size is not exact and must not be traded as though it were.",
    example: {
      scenario: "A standing suggestion on the credit book, raised at a breach of 138% of budget.",
      rows: [
        { label: "Headline", value: "Sell 318 contracts of TY-DEC26" },
        { label: "Context", value: "BREACH · fi-credit-emea · XS2034-ACME-4H · 138% of budget" },
        { label: "Target / per unit", value: "24,840 DV01 target; 78 DV01 per contract" },
        { label: "Exact vs traded", value: "318.46 exact → 318 contracts traded" },
        { label: "Residual", value: "rounded down, 36 DV01 still on the book" },
        { label: "Warning", value: "shown when the size came off the duration-blind exposure proxy" },
      ],
      takeaway: "The headline is what to do; the residual and the warning are what it will NOT do.",
    },
    howToConfigure: [
      "Open Fixed Income → Book → Hedge flows; standing suggestions sit at the top under “Suggestions”.",
      "Read the headline, then check the residual's wording — under-hedged or over-hedged.",
      "If the approximate-size warning is present, verify the size against your own duration before acting.",
      "Press “Hedge now” to fire it, or “Dismiss” to drop it. Neither asks for confirmation.",
      "A rejected hedge restores the row and shows the reason inline — the suggestion is never lost.",
    ],
    whenToUse:
      "Whenever a Suggest-scoped book breaches. Treat the board as a to-do list: a row that is still there is risk that is still unhedged.",
    risks:
      "The numbers are a snapshot at raise time — an old row may be sized off a stale mid and exposure. A whole-lot vehicle always leaves a residual, so a book hedged this way is never exactly flat. And an approximate (proxy-sized) hedge can be materially wrong on a long-duration bond, where the proxy understates the true DV01 several-fold.",
    keywords: [
      "suggestion", "standing", "headline", "residual", "rounded", "over-hedged", "under-hedged",
      "approximate", "proxy", "duration", "hedge now", "dismiss", "hedge flows", "read",
    ],
  },
  {
    id: "concept.decision-audit",
    category: "concept",
    title: "The decision audit — why a rule fired, or did not",
    purpose:
      "Read the recorded evidence behind every acceptance, routing and hedge decision, including the ones that deliberately did nothing.",
    howItWorks:
      "Three trader-configurable graphs gate your flow: acceptance decides whether an inbound lift is taken, risk routing decides which book it lands in, and the hedge exit policy decides whether the resulting risk is warehoused or shed. Each of them used to leave a trace only when it ACTED — a booked deal, a fired hedge — which meant the most common question on the desk (\u201cwhy did nothing happen?\u201d) had no answer. The server now writes a journal row for EVERY evaluation. A row carries the exact node path walked through your own graph, the stated reason, the policy that was in force, and the risk state at that instant (net risk against the cap, the utilisation, the RAG band). Select a row and the inspector replays the walk step by step and charts the utilisation history of that book \u00b7 instrument cell against the amber and red guides, so you see the portfolio state that produced the decision. A node your graph no longer contains is flagged as such rather than re-labelled \u2014 the audit never guesses.",
    example: {
      scenario:
        "A rates book sits at 97% of its DV01 cap and no hedge fires. The audit table is filtered to Engine = Hedge policy, Verdict = Did nothing.",
      rows: [
        { label: "Verdict", value: "WAREHOUSE (the graph resolved a warehouse hold)" },
        { label: "Reason", value: "red \u00b7 WAREHOUSE \u2014 the policy graph resolved a warehouse hold at node 1" },
        { label: "Path walked", value: "node 0 (breached = false?) \u2192 node 1 (Action: WAREHOUSE)" },
        { label: "Risk at decision", value: "-97,000 of 100,000 (97%, red)" },
        { label: "Policy in force", value: "book:rates-usd" },
      ],
      takeaway:
        "The rule ran and chose to hold: node 0's condition sent the walk down the warehouse branch. The fix is that branch's condition, not the threshold.",
    },
    howToConfigure: [
      "Open Risk \u2192 Audit. The table lists recorded decisions newest first; every column header sorts.",
      "Narrow with the Engine and Verdict chips \u2014 \u201cHedge policy\u201d + \u201cDid nothing\u201d is the view for \u201cwhy did my hedge not fire\u201d.",
      "Type in Search to match a reason, scope, counterparty or hedge id; type a book to restrict the whole query server-side.",
      "Click a row to see the walked path and the utilisation-against-band history in the inspector.",
      "Read the Suggested rules rail beneath: each card states what the evidence shows, cites the rows it came from, and links to the rule editor that fixes it.",
      "Press \u201cShow the N cited rows\u201d on a card to filter the table to exactly that evidence.",
    ],
    whenToUse:
      "Whenever the platform did something you did not expect \u2014 or, more often, did NOT do something you did expect. It is also the fastest way to confirm a rule change actually took effect: make the change, wait for the next fill, and read the new rows.",
    risks:
      "The journal is a BOUNDED, in-memory window, not a durable compliance archive. When the ring has rolled, the surface prints a prominent warning naming how many decisions have been lost \u2014 do not read a filtered-empty table as \u201cthe rule never ran\u201d without checking that banner. The walked path is resolved against the graph as it stands NOW, so a rule edited since the decision shows its nodes as stale rather than as the rule that actually ran. Suggestions are counted patterns, not predictions: they tell you what the recorded history contains, and they never fire a rule for you.",
    keywords: [
      "audit", "decision", "journal", "why", "did not fire", "no action", "warehouse",
      "reason", "path", "walked", "provenance", "evidence", "suggestion", "advice",
      "acceptance", "routing", "hedge", "band", "utilisation", "utilization", "trace",
    ],
  },
  {
    id: "concept.street-execution",
    category: "concept",
    title: "Street-side execution — what we sent, and what came back",
    purpose:
      "Read every outbound order the desk sent to the street: which LP, which product, requested against filled, the price and slippage, and the outcome with its reason.",
    howItWorks:
      "The LP league table grades each provider’s BEHAVIOUR — how fast it ticks, how often it wins. It carries no order economics, so it could not answer what the desk actually asks: what left the building. The server now records one row per OUTBOUND street order — every attempt, not only the fills — with the side, the requested and filled quantity, the reference and fill price, the signed slippage, the terminal outcome and its reason, the ranked panel of LPs that showed a firm price, and the hedge decision that produced it. Two consequences matter. First, an order that reached NO named LP is recorded as a COMPOSITE BACKSTOP: the street showed no firm price and the shed was filled against the internal composite mid instead. Before this, such an order was recorded nowhere, so a desk whose every hedge missed the street looked identical to a desk that never hedged. Second, the LPs we were ranked against but dealt away from are now retained, which is what makes MISSED and MEAN COVER real numbers on the league table rather than structural zeros. Group the breakdown by LP, product family, instrument, tenor bucket or hour; the aggregation is folded server-side over the whole matching window, never just the page you can see.",
    example: {
      scenario:
        "A rates book breaches its DV01 cap. The hedge fires for 4,000 DV01 of USSW10 and three LPs show a firm bid.",
      rows: [
        { label: "Order", value: "SELL 4,000 USSW10 (ois, 10.0y) · ref px 0.030000" },
        { label: "Panel", value: "3 LPs firm — best 0.030050, cover 0.030070" },
        { label: "Outcome", value: "Filled @ 0.030050 · slippage +0.50 bp" },
        { label: "League table effect", value: "winner +1 deal won; the other two +1 missed each; the cover carries the real distance" },
        { label: "Parent hedge", value: "HDG-1 — walk back to the breach that caused it" },
      ],
      takeaway:
        "If the same order had shown COMPOSITE BACKSTOP with a 0-LP panel, the reading is the opposite: the hedge fired and the street offered nothing, so no LP is credited or debited.",
    },
    howToConfigure: [
      "Open Analytics → Street Liquidity. The league table is the top half; the execution panels sit beneath it.",
      "Pick a Group by axis — Liquidity provider, Product family, Instrument, Tenor bucket or Hour (UTC).",
      "Narrow with the LP, Product family and Outcome controls. A blank control does not constrain.",
      "Sort any breakdown column; an absent metric always sorts last, in either direction.",
      "Read the blotter beneath for the individual orders, newest first. Hover the Panel count to see each competing LP and its price.",
      "Use the Parent hedge column to walk from a street order back to the hedge decision that produced it (and, from there, to the Risk → Audit row for WHY it fired).",
    ],
    whenToUse:
      "Whenever the league table shows an LP at zero and you need to know whether that is a real absence of business or an absence of capture; whenever you want to know which LPs are actually filling your hedges and at what cost; and whenever a hedge fired but the risk did not fall as expected.",
    risks:
      "A ROW IS ONE ORDER THAT REALLY WENT OUT. A hedge is routed: a FIX NewOrderSingle is sent to the named provider and the row records that provider\u2019s own execution report, so \u201cFilled on <LP>\u201d means that LP was sent an order and traded with you. One hedge can therefore produce SEVERAL rows \u2014 a refusal on the best-priced provider is followed by an order to the cover, and both are shown, because both really happened. READ THE REASON: a venue code (FOK_UNFILLABLE, IOC_DEPTH_EXHAUSTED, NOT_MARKETABLE, NOT_A_WHOLE_LOT\u2026) is the counterparty\u2019s answer, whereas \u201cno_order_endpoint\u201d is YOUR configuration \u2014 that provider is quoting but has no order route set on its FIX connection, so nothing could be sent to it and nobody refused you. A composite backstop credits NO provider: it is the absence of a street price you could deal on, and when it follows a full set of refusals its reason reads \u201cstreet_declined\u201d rather than \u201cno_firm_lp_price\u201d. The log is a BOUNDED, in-memory rolling window of recent activity, not a durable execution archive \u2014 an empty blotter after a quiet period may mean the orders have aged out. A metric shown as \u201c\u2014\u201d is GENUINELY ABSENT and never a zero: a group that never traded has no mean slippage, and order type, time-in-force and response latency are absent exactly on the rows where no order was sent (a composite backstop, a provider with no order route). Finally, the panel records the LPs that showed a firm executable price at the moment the order was worked; an LP that was connected but not quoting that instrument on that side is correctly absent from it, not a miss.",
    keywords: [
      "street", "execution", "order", "blotter", "outbound", "lp", "liquidity provider",
      "fill", "partial", "reject", "last look", "expired", "slippage", "cover", "panel",
      "composite", "backstop", "breakdown", "tenor", "family", "win rate", "fill ratio",
      "hedge", "parent", "missed", "competition", "venue", "latency", "time in force",
    ],
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
