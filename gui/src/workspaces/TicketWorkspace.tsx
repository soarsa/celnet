/**
 * TicketWorkspace — THE differentiator (GUI-DESIGN §4.1). One card that is the
 * analytics surface AND the executable: build a structure (vanilla, a multi-leg
 * strategy, or any exotic family), see a live two-way + the full 14-Greek set +
 * the conventions on the FACE, and hit it without changing screens. The strike
 * solve is inline: a delta-keyed leg (25dC / 25dP / ATM) rides the wire's
 * `StrikeOrDelta.delta` arm, is solved to a level server-side under the request
 * conventions, and the solved strike comes back on the quote's `resolvedStrike`
 * (rendered beside the priced two-way). The last-look window is a visible
 * depleting ring; "Stream this" promotes the exact Instrument into the blotter
 * and "Add to risk" drops it in the scenario grid — the same Instrument object,
 * no re-keying.
 *
 * GW2: the structurable catalogue lives in the {@link PRODUCT_REGISTRY}, not in
 * this shell. The shell owns the market/contract context (pair, tenor, expiry,
 * notional), the per-family input STATE map, and the quote/execute/promote
 * lifecycle; each product family contributes its own `toInstrument` + `InputBlock`
 * + `allowedModels` as a registry entry. Adding a product is a new spec, never a
 * shell edit — which is what lets the catalogue (and the multi-asset future) scale
 * without the former 3606-line monolith.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import { configuredLicense } from "../lib/commands";
import type {
  BrokenDate,
  Instrument,
  Leg,
  MultiDealerQuote,
  PricingModel,
  Quote,
  RatesCurveSet,
  RatesPricingResult,
  Tenor,
} from "../data/contract";
import { pillarTenorLabel } from "../data/contract";
import { Panel } from "../components/Panel";
import { Button } from "../components/Button";
import { DatePicker } from "../components/DatePicker";
import { DealerPanel } from "../components/DealerPanel";
import { DataGrid } from "../components/DataGrid";
import { TwoWayQuote } from "../components/TwoWayQuote";
import { GreeksStrip } from "../components/GreeksStrip";
import { ConventionRow } from "../components/ConventionChip";
import {
  PRODUCT_REGISTRY,
  isRatesSpec,
  specById,
  StructureGallery,
  PayoffChart,
  NetStructureStrip,
  type NetStructureLeg,
  type ProductBuildCtx,
} from "../products";
import { crossAssetInputsFor, crossAssetSpec } from "../products/crossAsset";
import type { OisInputs } from "../products/ois";
import { PayoffDiagram, type PayoffLeg } from "../viz/PayoffDiagram";
import { forward as forwardRate } from "../data/pricing";
import { impliedVolForInstrument, sampleSurface } from "../data/surface";
import {
  fmtPnlAdaptive,
  fmtPremiumPct,
  fmtVol,
  sideVerb,
} from "../lib/format";
import type { ColumnDef } from "../lib/grid";
import { nowNanos } from "../hooks/useClock";
import { samePair } from "../lib/universe";
import styles from "./TicketWorkspace.module.css";

/** The resting booking-model set — the fixed-income family has no option booking model. */
const DEFAULT_ALLOWED_MODELS: readonly PricingModel[] = ["DEFAULT"];

/** A trader-facing label for a booking model (purpose-named; provenance in docs only). */
function pricingModelLabel(m: PricingModel): string {
  return m === "LOCAL_STOCH_VOL" ? "Local-Stoch-Vol" : "Default";
}

/** True for the variance/volatility swaps (priced as a fair strike, not a premium). */
function isSwap(structure: string): boolean {
  return structure === "VARIANCE_SWAP" || structure === "VOLATILITY_SWAP";
}

/** Which expiry input the trader is using: a standard tenor or an arbitrary date. */
type ExpiryMode = "TENOR" | "DATE";

/**
 * How the RFQ is dealt: against the edge's own maker (the single-dealer quote,
 * byte-identical to the pre-panel flow) or fanned across the edge's LP panel
 * (the ranked multi-dealer `MultiDealerQuote`, booked per-line by `lpId`).
 */
type RfqMode = "SINGLE" | "PANEL";

/**
 * A standard-tenor choice — the trader-facing `Tenor` plus its year fraction on
 * ACT/365 (the pricing horizon; the server resolves the true settlement date). The
 * Phase-1 short-end units (ON/TN/SN) and IMM are first-class alongside W/M/Y.
 */
interface TenorChoice {
  label: string;
  tenor: Tenor;
  years: number;
}

const ONE_BIZ_DAY_YEARS = 1 / 365;
const TN_YEARS = 2 / 365;
const SN_YEARS = 3 / 365;

const TENOR_CHOICES: readonly TenorChoice[] = [
  { label: "ON", tenor: { unit: "OVERNIGHT", count: 1 }, years: ONE_BIZ_DAY_YEARS },
  { label: "TN", tenor: { unit: "TOM_NEXT", count: 1 }, years: TN_YEARS },
  { label: "SN", tenor: { unit: "SPOT_NEXT", count: 1 }, years: SN_YEARS },
  { label: "1W", tenor: { unit: "WEEKS", count: 1 }, years: 7 / 365 },
  { label: "2W", tenor: { unit: "WEEKS", count: 2 }, years: 14 / 365 },
  { label: "1M", tenor: { unit: "MONTHS", count: 1 }, years: 30 / 365 },
  { label: "2M", tenor: { unit: "MONTHS", count: 2 }, years: 60 / 365 },
  { label: "3M", tenor: { unit: "MONTHS", count: 3 }, years: 91 / 365 },
  { label: "6M", tenor: { unit: "MONTHS", count: 6 }, years: 182 / 365 },
  { label: "1Y", tenor: { unit: "YEARS", count: 1 }, years: 1 },
  { label: "IMM1", tenor: { unit: "IMM", count: 1 }, years: 0.25 },
];

/** Today (UTC, calendar-only) as a `BrokenDate` — the trade date for date math. */
function todayUtc(): BrokenDate {
  const d = new Date();
  return { year: d.getUTCFullYear(), month: d.getUTCMonth() + 1, day: d.getUTCDate() };
}
function brokenToUtcMs(d: BrokenDate): number {
  return Date.UTC(d.year, d.month - 1, d.day);
}
function addDays(d: BrokenDate, days: number): BrokenDate {
  const dt = new Date(brokenToUtcMs(d) + days * 86_400_000);
  return { year: dt.getUTCFullYear(), month: dt.getUTCMonth() + 1, day: dt.getUTCDate() };
}
/** Whole calendar days from `from` to `to` (UTC midnights). */
function daysBetween(from: BrokenDate, to: BrokenDate): number {
  return Math.round((brokenToUtcMs(to) - brokenToUtcMs(from)) / 86_400_000);
}
function fmtBrokenDate(d: BrokenDate): string {
  return `${d.year}-${String(d.month).padStart(2, "0")}-${String(d.day).padStart(2, "0")}`;
}

/**
 * The expiry year fraction for an arbitrary date under the conventions' day-count.
 * Calendar-day count / day-count basis — a faithful day-count read of the horizon.
 * It is NOT a business-day/holiday-adjusted figure: the server's celnet-calendar
 * owns the exact good-business-day expiry and spot-lagged delivery; we label that
 * honestly rather than reproduce calendar tables client-side.
 */
function expiryYearsForDate(today: BrokenDate, expiry: BrokenDate, basis: number): number {
  return Math.max(ONE_BIZ_DAY_YEARS, daysBetween(today, expiry) / basis);
}

/** A short trader-facing label for the promote (stream / risk-drill) annotations. */
function structureLabel(structure: string): string {
  switch (structure) {
    case "VANILLA":
      return "25Δ call";
    case "RISK_REVERSAL":
      return "25Δ RR";
    case "STRANGLE":
      return "10Δ strangle";
    case "STRADDLE":
      return "ATM straddle";
    case "SEAGULL":
      return "seagull";
    case "SINGLE_BARRIER":
      return "barrier";
    case "DOUBLE_BARRIER":
      return "dbl barrier";
    case "DIGITAL":
      return "digital";
    case "TOUCH":
      return "touch";
    case "VARIANCE_SWAP":
      return "var swap";
    case "VOLATILITY_SWAP":
      return "vol swap";
    case "ASIAN":
      return "Asian";
    case "FORWARD_START":
      return "fwd-start";
    case "CLIQUET":
      return "cliquet";
    case "QUANTO":
      return "quanto";
    case "TARF":
      return "TARF";
    case "ACCUMULATOR":
      return "accumulator";
    case "LOOKBACK":
      return "lookback";
    case "WINDOW_BARRIER":
      return "window barrier";
    case "AMERICAN":
      return "American";
    case "BASKET":
      return "basket";
    default:
      return specById(structure)?.label ?? structure;
  }
}

/**
 * The legs a multi-leg structure carries, as the {@link NetStructureStrip} needs
 * them — the ratio and side per leg (genuinely available off the built
 * instrument's leg ladder). The premium / Greeks are deliberately LEFT ABSENT
 * (the strip then renders an honest "—" rather than a fabricated net) because a
 * client-side priced-per-leg breakdown is not available here — the marked-surface
 * leg pricing resolves server-side. Vanilla and the legless exotics carry no
 * enumerable option legs, so the strip is not shown for them.
 */
function netStructureLegs(instrument: Instrument): NetStructureLeg[] {
  if (instrument.product.kind !== "strategy") return [];
  return instrument.product.strategy.legs.map((leg: Leg) => ({
    ratio: leg.ratio,
    side: leg.side === "SELL" ? "SELL" : "BUY",
  }));
}

/**
 * The typed leg set the multi-leg {@link PayoffDiagram} can honestly draw:
 * available only when the built instrument is a multi-leg strategy whose EVERY
 * leg carries an ABSOLUTE strike level (a typed K on the ladder). Delta-keyed
 * legs (25dC / 25dP / ATM) resolve to a level server-side under the request
 * conventions, so no honest client-side kink location exists for them — those
 * ladders (and the single-payoff families) keep the compact {@link PayoffChart}
 * shape preview. Per-leg premiums are deliberately LEFT ABSENT (they price
 * server-side off the marked surface), so the diagram draws the intrinsic
 * payoff-at-expiry shape — its own header/legend label the today curve as
 * illustrative, never a priced P&L.
 */
function payoffDiagramLegs(instrument: Instrument): PayoffLeg[] | null {
  if (instrument.product.kind !== "strategy") return null;
  const legs = instrument.product.strategy.legs;
  if (legs.length < 2) return null;
  const out: PayoffLeg[] = [];
  for (const leg of legs) {
    const k = leg.strike;
    if (k.kind !== "strike" || !Number.isFinite(k.strike) || k.strike <= 0) return null;
    out.push({
      kind: leg.optionType === "CALL" ? "call" : "put",
      side: leg.side === "SELL" ? "short" : "long",
      strike: k.strike,
      quantity: leg.ratio,
    });
  }
  return out;
}

/**
 * The primary strike a {@link PayoffChart} draws its kink at. The type-erased
 * registry inputs are not statically known here, so we read a numeric `strike`
 * field if the active family exposes one (most exotic families do, with `0` ⇒
 * "default to ATMF"), or the first ABSOLUTE leg strike of a leg-ladder family
 * (the vanilla/strategy editor — a typed level moves the kink; delta-keyed legs
 * resolve server-side, so they honestly fall through), and fall back to the
 * ATM-forward level otherwise. This is a faithful payoff SHAPE preview (kink
 * location), not a priced P&L — the chart's accessible label says so.
 */
function previewStrike(inputs: unknown, atmForward: number): number {
  if (inputs && typeof inputs === "object") {
    if ("strike" in inputs) {
      const k = (inputs as { strike?: unknown }).strike;
      if (typeof k === "number" && k > 0) return k;
    }
    if ("legs" in inputs) {
      const legs = (inputs as { legs?: unknown }).legs;
      if (Array.isArray(legs)) {
        for (const leg of legs) {
          const k = (leg as { strike?: { kind?: unknown; strike?: unknown } } | null)?.strike;
          if (k && k.kind === "strike" && typeof k.strike === "number" && k.strike > 0) {
            return k.strike;
          }
        }
      }
    }
  }
  return atmForward;
}

/** Props for the shared ticket. */
export interface TicketWorkspaceProps {
  /**
   * The structure the ticket opens seeded to. Absent ⇒ the default FX structure
   * (a 25Δ risk reversal). The Fixed-Income rail entry-point passes the OIS id so
   * the `rates` row opens the SHARED ticket in the fixed-income family (the pricing
   * analogue of the #1 Risk / #2 Market-Data lens entry-points), rather than a
   * separate FI pricing silo. Clamped by the gallery to an available family.
   */
  readonly initialStructure?: string;
}

export function TicketWorkspace({
  initialStructure,
}: TicketWorkspaceProps = {}): React.ReactElement {
  const app = useApp();
  // The primary ticket is the default (FX/cross-asset) `ticket` rail — the one that
  // owns the shell-global grammar: it consumes the one-shot cross-asset ticket target
  // (a universe underlier drill) and the ⏎/⌘⏎ keyboard. A FAMILY entry-point mount
  // (the `rates` rail, seeded to OIS) is a focused surface: it must NOT hijack a
  // cross-asset target away to the FX vanilla, and must not double-bind the global
  // keyboard (both ticket panes are persistently mounted at once).
  const isPrimaryTicket = initialStructure === undefined;
  const [structure, setStructure] = useState<string>(initialStructure ?? "RISK_REVERSAL");
  const [expiryMode, setExpiryMode] = useState<ExpiryMode>("TENOR");
  // Standard-tenor selection (index into TENOR_CHOICES); default 1M.
  const [tenorIdx, setTenorIdx] = useState(5);
  const [brokenDate, setBrokenDate] = useState<BrokenDate | null>(null);
  const [notionalMm, setNotionalMm] = useState(10);
  const [quote, setQuote] = useState<Quote | null>(null);
  // The multi-dealer RFQ state: the dealing mode and the live ranked panel (the
  // panel and the single-dealer quote are mutually exclusive priced states).
  const [rfqMode, setRfqMode] = useState<RfqMode>("SINGLE");
  const [dealerPanel, setDealerPanel] = useState<MultiDealerQuote | null>(null);
  // The fixed-income (rates) priced result — the OIS PV / par rate / PV01 / DV01 /
  // key-rate ladder from `priceRates`. Mutually exclusive with the option quote/panel
  // (a given active family produces exactly one of these).
  const [ratesResult, setRatesResult] = useState<RatesPricingResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [fill, setFill] = useState<string | null>(null);
  // The per-family input state, keyed by structure id and seeded from each spec's
  // declared defaults (the registry owns the shape; the shell owns the live value).
  // The active family's inputs are `inputsByStructure[structure]`; an edit replaces
  // just that family's slot. This single map retires the former ~16 per-family
  // useState hooks + the BuildExtras memo (one registry seam, not a monolith).
  const [inputsByStructure, setInputsByStructure] = useState<Record<string, unknown>>(() =>
    Object.fromEntries(PRODUCT_REGISTRY.map((s) => [s.id, s.defaults])),
  );
  // The selected booking/pricing model (`Instrument.pricing_model`). DEFAULT (the
  // per-product closed-form engine) unless the trader picks Local-Stoch-Vol for a
  // supported product; the window barrier locks it to LOCAL_STOCH_VOL inside its
  // own `toInstrument`.
  const [pricingModel, setPricingModel] = useState<PricingModel>("DEFAULT");

  // Drop any priced state (option quote, multi-dealer panel AND the rates result) —
  // every contract edit (structure/expiry/inputs/model) invalidates all equally.
  const clearPriced = useCallback(() => {
    setQuote(null);
    setDealerPanel(null);
    setRatesResult(null);
  }, []);

  // Universe → ticket pre-target: a non-FX underlier selection arms a one-shot
  // target; on arrival (mount or while open) we re-point the ticket at the
  // cross-asset vanilla spec seeded with that EXACT `Underlying` + settlement
  // mechanics — through the spec's own input/wire seam (`crossAssetInputsFor` is
  // the inverse of `crossAssetUnderlying`; no duplicated wire-building) — then
  // consume the target. FX selections never arm it, so FX flows are untouched.
  useEffect(() => {
    // Only the primary ticket consumes the cross-asset target — a family entry-point
    // (the OIS-seeded `rates` ticket) leaves it for the FX ticket, so a metal/crypto
    // universe drill re-points the FX ticket (not the FI one) and the target is
    // consumed exactly once across the two persistently-mounted ticket panes.
    if (!isPrimaryTicket) return;
    const target = app.ticketTarget;
    if (!target) return;
    const seeded = crossAssetInputsFor(target.underlying, target.settlementStyle);
    if (seeded) {
      setStructure(crossAssetSpec.id);
      setInputsByStructure((m) => ({ ...m, [crossAssetSpec.id]: seeded }));
      clearPriced();
    }
    app.clearTicketTarget();
  }, [app, app.ticketTarget, clearPriced, isPrimaryTicket]);

  const today = useMemo(() => todayUtc(), []);
  // Selectable broken-date window: from spot (~2 calendar days) out to ~3 years.
  const dateMin = useMemo(() => addDays(today, 3), [today]);
  const dateMax = useMemo(() => addDays(today, 365 * 3), [today]);
  const dayCountBasis = app.conventions.dayCount === "ACT_360" ? 360 : 365;

  const tenorChoice = TENOR_CHOICES[tenorIdx] ?? TENOR_CHOICES[5]!;

  // Resolve the active expiry to a (Tenor, year-fraction) pair. In DATE mode a
  // chosen calendar date becomes a BROKEN_DATE tenor carrying the explicit date,
  // with `expiryYears` derived under the conventions' day-count — and prices
  // through the SAME real transport (the contract carries broken_date end-to-end).
  const resolved = useMemo((): { tenor: Tenor; years: number } => {
    if (expiryMode === "DATE" && brokenDate) {
      return {
        tenor: { unit: "BROKEN_DATE", count: 0, brokenDate },
        years: expiryYearsForDate(today, brokenDate, dayCountBasis),
      };
    }
    return { tenor: tenorChoice.tenor, years: tenorChoice.years };
  }, [expiryMode, brokenDate, today, dayCountBasis, tenorChoice]);

  const tenorYears = resolved.years;

  // The market/contract context every spec reads to build its wire instrument and
  // render its input block. The forward uses the active pair's real market
  // (spot/rDom/rFor) at the selected horizon; strikes that default to ATMF read it.
  const atmForward = forwardRate(app.pairCtx.market, tenorYears);
  const spot = app.pairCtx.market.spot;
  const pipDecimals = app.pairCtx.pipDecimals;
  const ctx = useMemo<ProductBuildCtx>(
    () => ({
      pair: app.pairCtx.pair,
      // The active underlier's class (FX for a plain pair) — already resolved by
      // AppContext from the scope crumb; the class-aware gallery/Greeks/risk read it.
      assetClass: app.underlier.assetClass,
      // The active non-FX underlier identity (its `Underlying` arm + settlement
      // style): the asset-class-agnostic arms (perpetual / listed-future-option)
      // read it to carry the cross-asset wire keys; the FX-native exotics ignore it.
      // FX rests at the fx arm + LINEAR, so the overlay resolver returns undefined
      // and every FX/metal instrument stays byte-identical.
      underlier: { underlying: app.underlier.underlying, settlementStyle: app.underlier.settlementStyle },
      tenor: resolved.tenor,
      tenorYears,
      notionalMm,
      pricingModel,
      atmForward,
      spot,
      pipDecimals,
      today,
    }),
    [
      app.pairCtx.pair,
      app.underlier.assetClass,
      app.underlier.underlying,
      app.underlier.settlementStyle,
      resolved.tenor,
      tenorYears,
      notionalMm,
      pricingModel,
      atmForward,
      spot,
      pipDecimals,
      today,
    ],
  );

  // The active product family and its current inputs. The registry is a discriminated
  // union: the FX/cross-asset OPTIONS family (`optionSpec`) builds a wire `Instrument`
  // priced via `requestQuote`; the fixed-income RATES family (`ratesSpec`) builds an
  // `OisInstrument` priced via `priceRates`. Every family-specific read below narrows
  // to one arm — the option pricing flow is byte-identical, the rates flow is additive.
  const spec = specById(structure)!;
  const inputs = inputsByStructure[structure];
  const ratesSpec = isRatesSpec(spec) ? spec : null;
  const optionSpec = isRatesSpec(spec) ? null : spec;
  const isRates = ratesSpec !== null;
  // `effectiveCtx` clamps the trader's booking-model selection into the option
  // family's allowed set (the rates family has no booking model — DEFAULT), so
  // switching to a product that does not support LSV silently falls back to DEFAULT
  // (never sends a model the server would reject with `UnsupportedModel`).
  const allowedModels = optionSpec?.allowedModels ?? DEFAULT_ALLOWED_MODELS;
  const effectiveModel: PricingModel = allowedModels.includes(pricingModel)
    ? pricingModel
    : allowedModels[0]!;
  const effectiveCtx: ProductBuildCtx = { ...ctx, pricingModel: effectiveModel };

  // The family's structure law (e.g. the strategy leg templates): a violating
  // ladder still BUILDS (the editor renders the honest messages inline), but a
  // quote request is gated — booking a structure that belies its declared
  // template is never offered.
  const structureViolations = spec.validate?.(inputs as never, effectiveCtx) ?? [];
  const structureLawful = structureViolations.length === 0;

  // Capability gating (slice 5): this ticket is an FX-options surface, so each
  // affordance is gated against `fx_options`. Anonymous sessions run the
  // permissive/legacy path (`can` ⇒ true); a signed-in user is narrowed to their
  // effective set. Controls are DISABLED with an explanatory tooltip, never
  // hidden, and the handlers defensively no-op (the server still enforces).
  const canPrice = app.auth.can("price", "fx_options");
  const canExecute = app.auth.can("execute", "fx_options");
  const canStream = app.auth.can("stream", "fx_options");
  const priceDeniedTitle = capabilityDenialTitle("price", "fx_options");
  const executeDeniedTitle = capabilityDenialTitle("execute", "fx_options");
  const streamDeniedTitle = capabilityDenialTitle("stream", "fx_options");

  // Fixed-income (rates) gating (fe-fi-migration #3): the OIS family is gated on
  // `fixed_income` exactly as the standalone rates surface + the FI rail were —
  // discoverable only when the class is viewable AND licensed (`view` +
  // `configuredLicense`, the #1/#2 lens-gate order), priced only with `price`.
  // license×entitlement decides whether the OIS card appears in the gallery; the
  // `price` capability disables the price button with the honest denial tooltip
  // (never hidden — the server still enforces).
  const licensed = useMemo(() => configuredLicense(), []);
  const fiVisible = app.auth.can("view", "fixed_income") && licensed("fixed_income");
  const canPriceFi = app.auth.can("price", "fixed_income");
  const priceFiDeniedTitle = capabilityDenialTitle("price", "fixed_income");
  // The gallery catalogue: the FX/cross-asset families always, plus the fixed-income
  // families only when the class is viewable + licensed (so a non-FI session never
  // sees an OIS card it cannot reach — the nav layer already hides the FI rail).
  const galleryCatalogue = useMemo(
    () => (fiVisible ? PRODUCT_REGISTRY : PRODUCT_REGISTRY.filter((s) => !isRatesSpec(s))),
    [fiVisible],
  );

  // A trader-facing expiry label that is honest for every mode: a declared
  // no-expiry family (the perpetual) reads "PERP" (it has no expiry date to
  // label); a broken date reads as its calendar date, never coerced into a
  // tenor band.
  const expiryLabel = isRates
    ? "OIS"
    : optionSpec?.noExpiry
      ? "PERP"
      : expiryMode === "DATE" && brokenDate
        ? fmtBrokenDate(brokenDate)
        : tenorChoice.label;

  // In DATE mode the trader must pick a date before there is a horizon to
  // price; a no-expiry family (and the rates family, whose tenor is an input, not
  // a shell expiry) has no horizon to pick, so it is always ready.
  const expiryReady =
    isRates || optionSpec?.noExpiry !== undefined || expiryMode === "TENOR" || brokenDate !== null;

  // Offline (the in-app mock) the LSV engine is NOT available — it is a server-side
  // model (CLAUDE.md: no faked LSV numbers). Detect offline via the documented
  // transport label ("mock/replay"; the live transport's label starts with "live").
  // When LSV is the effective model offline, pricing is gated to the live server.
  const isOffline = !app.transport.label.startsWith("live");
  const lsvUnavailableOffline = isOffline && effectiveModel === "LOCAL_STOCH_VOL";

  // The option family's wire instrument (null for the rates family, which prices
  // an `OisInstrument` via `priceRates` instead — see `requestQuote`). Every
  // option-only consumer below is guarded on it, so the FX flow is unchanged.
  const instrument = optionSpec
    ? optionSpec.toInstrument(inputs as never, effectiveCtx)
    : null;

  // Total-variance interpolation in time of the marked surface's ATM term
  // structure at the (broken) expiry — σ²(t)·t linear in t, the standard
  // arbitrage-consistent time interpolation. DISPLAY-ONLY (the structure's own
  // face vol is the |vega|-weighted smile read below); shown so the trader sees
  // the surface vol the broken date lands on. HONEST GAP: event-clock kinks
  // (central-bank / NFP jump vol) are NOT modelled — this is a smooth-clock read.
  const interpolatedAtmVol = useMemo((): number | null => {
    const surface = app.surface;
    if (!surface || surface.smiles.length === 0) return null;
    if (!samePair(surface.pair, app.pairCtx.pair)) return null;
    const smiles = [...surface.smiles].sort((a, b) => a.tenorYears - b.tenorYears);
    const lo = smiles[0]!;
    const hi = smiles[smiles.length - 1]!;
    const t = tenorYears;
    // ATM is the |Δ|=0.5 node; sampleSurface reads the calibrated points.
    const atmAt = (tt: number): number => sampleSurface(surface, tt, 0.5);
    if (t <= lo.tenorYears) return atmAt(lo.tenorYears);
    if (t >= hi.tenorYears) return atmAt(hi.tenorYears);
    let a = lo;
    let b = hi;
    for (let i = 0; i < smiles.length - 1; i += 1) {
      if (smiles[i]!.tenorYears <= t && smiles[i + 1]!.tenorYears >= t) {
        a = smiles[i]!;
        b = smiles[i + 1]!;
        break;
      }
    }
    const vA = atmAt(a.tenorYears);
    const vB = atmAt(b.tenorYears);
    const wA = vA * vA * a.tenorYears; // total variance at the lower pillar
    const wB = vB * vB * b.tenorYears; // total variance at the upper pillar
    const span = b.tenorYears - a.tenorYears;
    const frac = span <= 0 ? 0 : (t - a.tenorYears) / span;
    const totalVar = wA + (wB - wA) * frac;
    return t > 0 ? Math.sqrt(Math.max(0, totalVar / t)) : vA;
  }, [app.surface, app.pairCtx.pair, tenorYears]);

  // The quote-face vol: the REAL smile vol the structure trades on at its
  // strike(s)/delta(s), read off the marked surface and |vega|-weighted across
  // legs — not a flat ATM. Falls back to the pair's ATM only when the surface
  // has not been marked yet. Uses the ticket's own tenor (expiryYears) so the
  // face reflects the selected expiry's smile, not the surface's default tenor.
  const faceVol =
    instrument && app.surface
      ? impliedVolForInstrument(app.surface, instrument, app.pairCtx.market)
      : app.pairCtx.market.vol;

  const requestQuote = useCallback(async () => {
    // Fixed-income (rates) path: build the OIS + price it via `priceRates` against
    // the family's calibrated curve (PV / par / PV01 / DV01 / key-rate ladder). The
    // SAME transport seam the standalone rates surface used — one contract, two
    // transports (offline in-app bootstrap ≡ the live `price_rates` mirror).
    if (ratesSpec) {
      // Capability guard (the button is disabled and the server enforces).
      if (!canPriceFi) return;
      if (!structureLawful) return;
      setBusy(true);
      setFill(null);
      const ois = ratesSpec.toOisInstrument(inputs as never, effectiveCtx);
      try {
        const priced = await app.transport.priceRates(ratesSpec.curve, ois);
        setRatesResult(priced);
        setQuote(null);
        setDealerPanel(null);
      } catch (err) {
        // A pricing failure (server refusal, transport deadline, or an offline
        // validation throw) is a real, surfaced error — never a fabricated price.
        setRatesResult(null);
        setFill(`Pricing failed — ${err instanceof Error ? err.message : String(err)}`);
      } finally {
        setBusy(false);
      }
      return;
    }
    // Capability guard (belt-and-suspenders; the button is disabled and the
    // server enforces): never dial a price the signed-in user may not request.
    if (!canPrice) return;
    // The LSV engine is server-side: do not request a price offline for an
    // LSV-priced instrument (the mock would have to fake it). Surface the honest
    // gate instead of dialling a price.
    if (lsvUnavailableOffline) {
      setFill("Local-Stoch-Vol pricing is server-side — run against the live server.");
      return;
    }
    // The structure law gates pricing: the violations are already rendered
    // inline by the family's input block (and the button is disabled), so a
    // race-through here simply never dials.
    if (!structureLawful) return;
    // The rates family returned above; from here the active family is the option
    // family, which builds a wire `Instrument` (never reached for a rates spec).
    if (!optionSpec) return;
    setBusy(true);
    setFill(null);
    const inst = optionSpec.toInstrument(inputs as never, effectiveCtx);
    try {
      if (rfqMode === "PANEL") {
        // Fan the RFQ across the edge's LP panel; the reply is the ranked lines.
        const p = await app.transport.requestMultiDealerQuote(
          inst,
          app.conventions,
          `tkt-${Date.now()}`,
        );
        setDealerPanel(p);
        setQuote(null);
      } else {
        const q = await app.transport.requestQuote(
          inst,
          app.conventions,
          `tkt-${Date.now()}`,
        );
        setQuote(q);
        setDealerPanel(null);
      }
    } catch (err) {
      // A pricing failure (server refusal, transport deadline, drop) is a real
      // trading outcome: render it on the fill line and RE-ARM the ticket —
      // never a silent swallow, never a "Pricing…" button stuck forever (the
      // pre-fix behaviour: an unhandled rejection here leaked `busy=true`).
      setFill(`Pricing failed — ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      setBusy(false);
    }
  }, [
    app,
    spec,
    ratesSpec,
    optionSpec,
    inputs,
    effectiveCtx,
    lsvUnavailableOffline,
    rfqMode,
    structureLawful,
    canPrice,
    canPriceFi,
  ]);

  const accept = useCallback(
    async (side: "BUY" | "SELL") => {
      // Capability guard (the accept buttons are disabled; the server enforces).
      if (!canExecute) return;
      if (!quote) return;
      if (quote.validUntilNanos <= nowNanos()) {
        setFill("Quote expired — re-request");
        setQuote(null);
        return;
      }
      try {
        const exec = await app.transport.acceptQuote(quote.quoteId, side, quote.idempotencyKey);
        setFill(
          `Filled ${sideVerb(side)} @ ${exec.tradedPremium.toFixed(3)} · exec #${exec.executionId}`,
        );
        setQuote(null);
      } catch (err) {
        // A server refusal (expired last-look, already booked, …) is a real
        // trading outcome: render it on the fill line, never a silent swallow.
        setFill(`Refused — ${err instanceof Error ? err.message : String(err)}`);
      }
    },
    [app, quote, canExecute],
  );

  /**
   * Book one dealer line off the ranked panel: `acceptQuote` carrying the row's
   * `lpId` trades exactly that pinned line. The expiry gate reads the ROW's own
   * last-look deadline (each line carries its own window), mirroring `accept`.
   */
  const bookDealerLine = useCallback(
    async (quoteId: bigint, lpId: string, side: "BUY" | "SELL") => {
      // Capability guard (the dealer-row accept buttons are disabled too).
      if (!canExecute) return;
      if (!dealerPanel) return;
      const row = dealerPanel.dealers.find((d) => d.lpId === lpId);
      if (!row) return;
      if (row.validUntilNanos <= nowNanos()) {
        setFill("Dealer line expired — re-request the panel");
        return;
      }
      try {
        const exec = await app.transport.acceptQuote(
          quoteId,
          side,
          dealerPanel.idempotencyKey,
          lpId,
        );
        setFill(
          `Filled ${sideVerb(side)} ${lpId} @ ${exec.tradedPremium.toFixed(3)} · exec #${exec.executionId}`,
        );
        setDealerPanel(null);
      } catch (err) {
        // The server refused the line (expired, already booked, unknown row):
        // surface the refusal beside the panel — never a silent swallow. The
        // panel stays mounted so the trader can re-request or pick another line.
        setFill(`Refused ${lpId} — ${err instanceof Error ? err.message : String(err)}`);
      }
    },
    [app, dealerPanel, canExecute],
  );

  // ⏎ requests, ⌘⏎ accepts the offered side (keyboard-first, §4.1). Bound only by
  // the primary ticket — the shell mounts both ticket panes at once, so a family
  // entry-point mount must not double-fire the window keyboard grammar.
  useEffect(() => {
    if (!isPrimaryTicket) return;
    const onKey = (e: KeyboardEvent) => {
      if (app.paletteOpen) return;
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        void accept("BUY");
      } else if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
        const tag = (e.target as HTMLElement)?.tagName;
        if (tag === "INPUT" || tag === "BUTTON") return;
        if (!expiryReady || !structureLawful) return;
        e.preventDefault();
        void requestQuote();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [accept, requestQuote, app.paletteOpen, expiryReady, structureLawful, isPrimaryTicket]);

  /** Replace the active family's inputs and clear any stale quote/panel against them. */
  const updateInputs = useCallback(
    (next: unknown) => {
      setInputsByStructure((m) => ({ ...m, [structure]: next }));
      clearPriced();
    },
    [structure, clearPriced],
  );

  // Set the OIS fixed rate to the par (zero-PV breakeven) rate just priced (the
  // rates family's analogue of the FX inline strike solve). Editing the inputs
  // clears the now-stale price, so the trader re-prices at par.
  const setRatesToPar = useCallback(() => {
    if (!ratesResult) return;
    const cur = inputsByStructure[structure] as OisInputs;
    updateInputs({ ...cur, fixedRatePct: Number((ratesResult.parRate * 100).toFixed(4)) });
  }, [ratesResult, inputsByStructure, structure, updateInputs]);

  const strikePreview = previewStrike(inputs, atmForward);
  const netLegs = instrument ? netStructureLegs(instrument) : [];
  const diagramLegs = instrument ? payoffDiagramLegs(instrument) : null;

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.ticket} noPadding>
        <div className={styles.head}>
          <span className={`num ${styles.pair}`}>
            {isRates && ratesSpec
              ? `${ratesSpec.curve.currency}-SOFR`
              : `${app.pairCtx.pair.base}/${app.pairCtx.pair.quote}`}
          </span>
          <span className={styles.dot}>·</span>
          <span className={styles.headStructure}>{spec.label}</span>
          <div className={styles.headRight}>
            {/* The FX notional field is option-only — the rates family owns its own
              * notional (in the curve currency) in its InputBlock. */}
            {optionSpec && instrument && (
              <label className={styles.notional}>
                <span>notional</span>
                <input
                  className="num"
                  type="number"
                  min={1}
                  value={notionalMm}
                  onChange={(e) => setNotionalMm(Math.max(1, Number(e.target.value)))}
                />
                <span>mm {instrument.quantity.baseCcy ? app.pairCtx.pair.base : app.pairCtx.pair.quote}</span>
              </label>
            )}
          </div>
        </div>

        <StructureGallery
          value={structure}
          specs={galleryCatalogue}
          assetClass={app.underlier.assetClass}
          onSelect={(id) => {
            setStructure(id);
            clearPriced();
          }}
        />

        {!isRates && (allowedModels.length > 1 || structure === "WINDOW_BARRIER") ? (
          <div className={styles.modelBlock}>
            <div className={styles.modelRow}>
              <span className={styles.modelLabel}>Booking model</span>
              <div className={styles.modeToggle} role="tablist" aria-label="booking model">
                {allowedModels.map((m) => (
                  <button
                    key={m}
                    role="tab"
                    aria-selected={effectiveModel === m}
                    className={`${styles.modeTab} ${effectiveModel === m ? styles.modeActive : ""}`}
                    disabled={allowedModels.length === 1}
                    onClick={() => {
                      setPricingModel(m);
                      clearPriced();
                    }}
                  >
                    {pricingModelLabel(m)}
                  </button>
                ))}
              </div>
            </div>
            {effectiveModel === "LOCAL_STOCH_VOL" && (
              <p className={styles.modelNote}>
                {structure === "WINDOW_BARRIER"
                  ? "The window barrier has no closed form — it is priced only by the server's local-stochastic-volatility engine (ADI-PDE, or Monte-Carlo when MC pairs > 0)."
                  : "Priced by the server's local-stochastic-volatility engine (particle-calibrated to the live surface) instead of the closed-form default."}
                {lsvUnavailableOffline
                  ? " Offline preview: this model is server-side only — run against the live server to price it."
                  : ""}
              </p>
            )}
          </div>
        ) : null}

        {!isRates && (
          <div className={styles.modelBlock}>
            <div className={styles.modelRow}>
              <span className={styles.modelLabel}>RFQ mode</span>
              <div className={styles.modeToggle} role="tablist" aria-label="rfq mode">
                <button
                  role="tab"
                  aria-selected={rfqMode === "SINGLE"}
                  className={`${styles.modeTab} ${rfqMode === "SINGLE" ? styles.modeActive : ""}`}
                  onClick={() => {
                    setRfqMode("SINGLE");
                    clearPriced();
                  }}
                >
                  Single-dealer
                </button>
                <button
                  role="tab"
                  aria-selected={rfqMode === "PANEL"}
                  className={`${styles.modeTab} ${rfqMode === "PANEL" ? styles.modeActive : ""}`}
                  onClick={() => {
                    setRfqMode("PANEL");
                    clearPriced();
                  }}
                >
                  LP panel
                </button>
              </div>
            </div>
            {rfqMode === "PANEL" && (
              <p className={styles.modelNote}>
                Fans the RFQ across the edge's LP panel and ranks the lines (best bid /
                best offer); book a row to trade exactly that dealer's price. In-repo
                dealers are the native maker plus deterministic synthetic demo LPs
                (SYNTH-LP-k) — live bank LP connectivity is environment-provisioned.
              </p>
            )}
          </div>
        )}

        {!isRates && optionSpec && (
        <div className={styles.expiryBlock}>
          {optionSpec.noExpiry ? (
            // The declared no-expiry family (the perpetual): the expiry controls
            // are not applicable — show the honest reason instead of offering a
            // tenor the contract cannot carry (the booked instrument is
            // `expiryYears = 0` exactly, with no tenor).
            <>
              <div className={styles.expiryModeRow}>
                <span className={styles.expiryModeLabel}>Expiry</span>
                <span className="num" aria-label="no expiry">
                  — none
                </span>
              </div>
              <p className={styles.resolveEmpty}>{optionSpec.noExpiry.reason}</p>
            </>
          ) : (
            <>
              <div className={styles.expiryModeRow}>
                <span className={styles.expiryModeLabel}>Expiry</span>
                <div className={styles.modeToggle} role="tablist" aria-label="expiry mode">
                  <button
                    role="tab"
                    aria-selected={expiryMode === "TENOR"}
                    className={`${styles.modeTab} ${expiryMode === "TENOR" ? styles.modeActive : ""}`}
                    onClick={() => {
                      setExpiryMode("TENOR");
                      clearPriced();
                    }}
                  >
                    Tenor
                  </button>
                  <button
                    role="tab"
                    aria-selected={expiryMode === "DATE"}
                    className={`${styles.modeTab} ${expiryMode === "DATE" ? styles.modeActive : ""}`}
                    onClick={() => {
                      setExpiryMode("DATE");
                      clearPriced();
                    }}
                  >
                    Broken date
                  </button>
                </div>
              </div>

              {expiryMode === "TENOR" ? (
                <div className={styles.tenorRow}>
                  {TENOR_CHOICES.map((c, i) => (
                    <button
                      key={c.label}
                      className={`${styles.tenorPill} ${i === tenorIdx ? styles.tenorActive : ""}`}
                      onClick={() => {
                        setTenorIdx(i);
                        clearPriced();
                      }}
                    >
                      {c.label}
                    </button>
                  ))}
                </div>
              ) : (
                <div className={styles.dateRow}>
                  <DatePicker
                    value={brokenDate}
                    min={dateMin}
                    max={dateMax}
                    onChange={(d) => {
                      setBrokenDate(d);
                      clearPriced();
                    }}
                  />
                  <div className={styles.dateResolve}>
                    {brokenDate ? (
                      <>
                        <div className={styles.resolveRow}>
                          <span className={styles.resolveLabel}>Expiry</span>
                          <span className={`num ${styles.resolveVal}`}>
                            {fmtBrokenDate(brokenDate)}
                          </span>
                        </div>
                        <div className={styles.resolveRow}>
                          <span className={styles.resolveLabel}>Horizon</span>
                          <span className={`num ${styles.resolveVal}`}>
                            {daysBetween(today, brokenDate)}d · {tenorYears.toFixed(3)}y
                          </span>
                        </div>
                        <div className={styles.resolveRow}>
                          <span className={styles.resolveLabel}>Interp vol</span>
                          <span className={`num ${styles.resolveVal}`}>
                            {interpolatedAtmVol !== null ? fmtVol(interpolatedAtmVol) : "—"}
                          </span>
                        </div>
                        <p className={styles.resolveNote}>
                          ATM vol total-variance interpolated in time. Exact good-business-day
                          expiry &amp; spot-lagged delivery resolve server-side (celnet-calendar).
                        </p>
                        <p className={styles.eventNote}>
                          Event-aware pricing (central-bank / NFP jump vol): not yet — smooth-clock
                          read only.
                        </p>
                      </>
                    ) : (
                      <p className={styles.resolveEmpty}>
                        Pick a date to price an arbitrary broken-date expiry through the live
                        pricing path.
                      </p>
                    )}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
        )}

        <spec.InputBlock value={inputs as never} onChange={updateInputs} ctx={effectiveCtx} />

        {!isRates && instrument && (
          <div className={styles.payoffPreview}>
            {diagramLegs !== null ? (
              <PayoffDiagram legs={diagramLegs} spot={spot} height={240} />
            ) : (
              <PayoffChart structureId={structure} strike={strikePreview} spot={spot} />
            )}
          </div>
        )}

        {netLegs.length > 0 && (
          <NetStructureStrip
            legs={netLegs}
            quoteCcy={app.pairCtx.pair.quote}
            baseCcy={app.pairCtx.pair.base}
          />
        )}

        {ratesSpec &&
          (ratesResult ? (
            <RatesResult result={ratesResult} curve={ratesSpec.curve} />
          ) : (
            <p className={styles.ratesEmpty}>
              Build an OIS and price it to see the PV, par (fair fixed) rate, PV01, DV01
              and the key-rate DV01 ladder.
            </p>
          ))}

        {!isRates &&
          (dealerPanel ? (
          <div className={styles.dealerPanelBlock}>
            <DealerPanel
              panel={dealerPanel}
              windowSeconds={8}
              onBook={(quoteId, lpId, side) => void bookDealerLine(quoteId, lpId, side)}
              bookDisabled={!canExecute}
              bookDisabledTitle={executeDeniedTitle}
            />
          </div>
        ) : quote && isSwap(structure) ? (
          <SwapResult structure={structure} quote={quote} />
        ) : quote ? (
          <div className={styles.quoted}>
            <TwoWayQuote
              price={quote.price}
              conventions={app.conventions}
              validUntilNanos={quote.validUntilNanos}
              windowSeconds={8}
              size="display"
              onHitBid={() => accept("SELL")}
              onLiftOffer={() => accept("BUY")}
            />
            <span className={styles.unit}>
              {structure === "ASIAN" ? `% ${app.pairCtx.pair.base} prem (avg-rate)` : `% ${app.pairCtx.pair.base} prem`}
            </span>
            {instrument &&
              (instrument.product.kind === "vanilla" || instrument.product.kind === "strategy") &&
              quote.resolvedStrike > 0 && (
                // The inline strike solve, made visible: a delta-keyed leg (25dC /
                // ATM) was solved to this level server-side (the first leg's K is
                // the headline; absolute strikes echo back unchanged).
                <span className={`num ${styles.solveChip}`} aria-label="resolved strike">
                  solved K {quote.resolvedStrike.toFixed(pipDecimals)}
                </span>
              )}
            {quote.priceStdError !== undefined && (
              <span className={`num ${styles.stdError}`} aria-label="price std error">
                Monte-Carlo · std error ±{(quote.priceStdError * 100).toFixed(4)} (
                {quote.greeks.price !== 0
                  ? `${((quote.priceStdError / Math.abs(quote.greeks.price)) * 100).toFixed(2)}% of PV`
                  : "—"}
                )
              </span>
            )}
          </div>
        ) : (
          <div className={styles.market}>
            <div className={styles.priceCol}>
              <span className={styles.priceLabel}>BID</span>
              <span className={`num ${styles.placeholder}`}>— · —</span>
            </div>
            <div className={styles.priceCol}>
              <span className={styles.priceLabel}>MID</span>
              <span className={`num ${styles.placeholder}`}>— · —</span>
            </div>
            <div className={styles.priceCol}>
              <span className={styles.priceLabel}>OFFER</span>
              <span className={`num ${styles.placeholder}`}>— · —</span>
              <span className={styles.unit}>% {app.pairCtx.pair.base} prem</span>
            </div>
          </div>
          ))}

        {!isRates && quote && !isSwap(structure) && (
          <div className={styles.greeksRow}>
            <GreeksStrip greeks={quote.greeks} assetClass={app.underlier.assetClass} />
          </div>
        )}

        {!isRates && (
          <div className={styles.convRow}>
            {!quote && <ConventionRow conventions={app.conventions} />}
            {quote && (
              <span className={`num ${styles.volFace}`}>
                vol {fmtVol(faceVol)}
              </span>
            )}
          </div>
        )}

        {fill && <div className={styles.fill}>{fill}</div>}

        <div className={styles.actions}>
          {isRates ? (
            // The fixed-income request button: price the OIS via `priceRates`. Gated
            // on `price·fixed_income` (disabled + honest tooltip, never hidden — the
            // server still enforces), exactly as the standalone rates surface was.
            <Button
              variant="primary"
              size="lg"
              onClick={requestQuote}
              kbd="⏎"
              disabled={busy || !structureLawful || !canPriceFi}
              title={canPriceFi ? undefined : priceFiDeniedTitle}
            >
              {!canPriceFi
                ? "Not permitted"
                : busy
                  ? "Pricing…"
                  : !structureLawful
                    ? "Fix inputs"
                    : ratesResult
                      ? "Re-price"
                      : "Price OIS"}
            </Button>
          ) : (
            <Button
              variant="primary"
              size="lg"
              onClick={requestQuote}
              kbd="⏎"
              disabled={
                busy || !expiryReady || lsvUnavailableOffline || !structureLawful || !canPrice
              }
              title={canPrice ? undefined : priceDeniedTitle}
            >
              {!canPrice
                ? "Not permitted"
                : busy
                  ? "Pricing…"
                  : !expiryReady
                    ? "Pick a date"
                    : !structureLawful
                      ? "Fix structure"
                      : lsvUnavailableOffline
                        ? "LSV — live server only"
                        : quote || dealerPanel
                          ? "Re-request"
                          : rfqMode === "PANEL"
                            ? "Request panel"
                            : "Request quote"}
            </Button>
          )}
          {isRates && ratesResult && (
            <Button
              variant="ghost"
              onClick={setRatesToPar}
              title="set the fixed rate to the par (breakeven) rate"
            >
              Set to par
            </Button>
          )}
          {quote && (
            <>
              <Button
                variant="bid"
                size="lg"
                onClick={() => accept("SELL")}
                disabled={!canExecute}
                title={canExecute ? undefined : executeDeniedTitle}
              >
                Sell {fmtPremiumPct(quote.price.bid)}
              </Button>
              <Button
                variant="offer"
                size="lg"
                onClick={() => accept("BUY")}
                kbd="⌘⏎"
                disabled={!canExecute}
                title={canExecute ? undefined : executeDeniedTitle}
              >
                Buy {fmtPremiumPct(quote.price.offer)}
              </Button>
            </>
          )}
          {optionSpec && instrument && (
            <span className={styles.promote}>
              <Button
                variant="ghost"
                onClick={() => {
                  if (!canStream) return;
                  app.stream.subscribe(instrument, app.conventions, structureLabel(structure));
                  app.setWorkspace("stream");
                }}
                disabled={!canStream}
                title={canStream ? undefined : streamDeniedTitle}
              >
                Stream this ≋
              </Button>
              <Button
                variant="ghost"
                onClick={() =>
                  app.drillToRisk(
                    instrument,
                    `${app.pairCtx.pair.base}/${app.pairCtx.pair.quote} ${expiryLabel} ${structureLabel(structure)}`,
                  )
                }
              >
                Add to risk ⊞
              </Button>
            </span>
          )}
        </div>
      </Panel>

      <p className={styles.caption}>
        {isRates
          ? "One ticket, every asset. The fixed-income OIS is priced through the SAME card as FX and cross-asset — the same workflow, one canonical contract (the live `price_rates` seam), no separate rates silo."
          : "One card = analytics + executable. Conventions on the face, the strike solve inline (25dC / 25dP / ATM legs price to a server-solved K, echoed on the quote), last-look visible. Promote the exact structure to the blotter or the risk grid — same Instrument, no re-keying."}
      </p>
    </div>
  );
}

/**
 * The variance/volatility-swap result panel: the priced FAIR STRIKE (not a
 * premium two-way). For a variance swap the headline is the fair variance strike
 * `K_var` with its `√K_var` shown in vol terms; for a volatility swap it is the
 * fair vol strike `K_vol`. The server (and the standalone build) echo the fair
 * strike in the quote's `resolvedStrike` (and `greeks.price`).
 */
function SwapResult(props: { structure: string; quote: Quote }): React.ReactElement {
  const { structure, quote } = props;
  const fair = quote.resolvedStrike;
  if (structure === "VARIANCE_SWAP") {
    const fairVol = fair > 0 ? Math.sqrt(fair) : 0;
    return (
      <div className={styles.swapResult}>
        <span className={styles.swapHead}>Fair variance strike</span>
        <span className={`num ${styles.swapStrike}`}>K_var {fair.toFixed(6)}</span>
        <span className={`num ${styles.swapSub}`}>√K_var = {fmtVol(fairVol)}</span>
      </div>
    );
  }
  return (
    <div className={styles.swapResult}>
      <span className={styles.swapHead}>Fair volatility strike</span>
      <span className={`num ${styles.swapStrike}`}>K_vol {fmtVol(fair)}</span>
      <span className={`num ${styles.swapSub}`}>convexity-adjusted</span>
    </div>
  );
}

/** One row of the key-rate DV01 ladder (a curve pillar's bucketed DV01). */
interface LadderRow {
  readonly tenorLabel: string;
  readonly dv01: number;
  /** This bucket's share of the total DV01 (percent); `0` when DV01 is ~0. */
  readonly sharePct: number;
}

const RATES_LADDER_COLUMNS: readonly ColumnDef<LadderRow>[] = [
  { key: "pillar", header: "Pillar", width: 96, align: "left", accessor: (r) => r.tenorLabel },
  {
    key: "dv01",
    header: "Key-rate DV01",
    unit: "/bp",
    width: 160,
    accessor: (r) => fmtPnlAdaptive(r.dv01),
  },
  { key: "share", header: "% of DV01", width: 120, accessor: (r) => `${r.sharePct.toFixed(1)}%` },
];

/** Format a decimal rate as a percentage with bp precision (0.0405 → "4.0500%"). */
function fmtRatePct(rate: number): string {
  return `${(rate * 100).toFixed(4)}%`;
}

/**
 * The fixed-income (OIS) priced result — the PV / par (fair fixed) rate / PV01 /
 * DV01 headline metrics + the key-rate DV01 ladder, over the SAME `priceRates`
 * result the standalone rates surface rendered (capability preserved by the
 * fold, not a reimplementation). Each ladder bucket maps to a curve pillar and
 * sums (to first order) to the parallel DV01. All measures are in the curve
 * currency and already carry the instrument direction sign.
 */
function RatesResult({
  result,
  curve,
}: {
  result: RatesPricingResult;
  curve: RatesCurveSet;
}): React.ReactElement {
  const ladder: LadderRow[] = curve.pillars.map((p, i) => {
    const dv01 = result.keyRateLadder[i] ?? 0;
    return {
      tenorLabel: pillarTenorLabel(p.tenor),
      dv01,
      sharePct: result.dv01 !== 0 ? (dv01 / result.dv01) * 100 : 0,
    };
  });
  const ladderGroups = [
    { key: "", label: "", rows: ladder.map((r) => ({ key: r.tenorLabel, datum: r })) },
  ];
  return (
    <div className={styles.ratesResult}>
      <dl className={styles.ratesMetrics}>
        <RatesMetric label="PV" value={fmtPnlAdaptive(result.pv)} unit={curve.currency} emphatic />
        <RatesMetric label="Par rate" value={fmtRatePct(result.parRate)} />
        <RatesMetric label="PV01" value={fmtPnlAdaptive(result.pv01)} unit={`${curve.currency}/bp`} />
        <RatesMetric label="DV01" value={fmtPnlAdaptive(result.dv01)} unit={`${curve.currency}/bp`} />
      </dl>
      <div className={styles.ratesLadder}>
        <h3 className={styles.ratesLadderTitle}>Key-rate DV01 ladder</h3>
        <DataGrid label="key-rate DV01 ladder" columns={RATES_LADDER_COLUMNS} groups={ladderGroups} />
      </div>
    </div>
  );
}

/** One headline OIS measure: a labelled term/value pair in the rates result strip. */
function RatesMetric({
  label,
  value,
  unit,
  emphatic,
}: {
  label: string;
  value: string;
  unit?: string;
  emphatic?: boolean;
}): React.ReactElement {
  return (
    <div className={`${styles.ratesMetric} ${emphatic ? styles.ratesMetricEmphatic : ""}`}>
      <dt className={styles.ratesMetricLabel}>{label}</dt>
      <dd className={styles.ratesMetricValue}>
        {value}
        {unit && <span className={styles.ratesMetricUnit}>{unit}</span>}
      </dd>
    </div>
  );
}
