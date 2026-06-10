/**
 * TicketWorkspace — THE differentiator (GUI-DESIGN §4.1). One card that is the
 * analytics surface AND the executable: build a structure (vanilla, a multi-leg
 * strategy, or any exotic family), see a live two-way + the full 14-Greek set +
 * the conventions on the FACE, and hit it without changing screens. Solve
 * (zero-cost) is inline; the last-look window is a visible depleting ring; "Stream
 * this" promotes the exact Instrument into the blotter and "Add to risk" drops it
 * in the scenario grid — the same Instrument object, no re-keying.
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
import type {
  BrokenDate,
  Instrument,
  Leg,
  MultiDealerQuote,
  PricingModel,
  Quote,
  Tenor,
} from "../data/contract";
import { Panel } from "../components/Panel";
import { Button } from "../components/Button";
import { DatePicker } from "../components/DatePicker";
import { DealerPanel } from "../components/DealerPanel";
import { TwoWayQuote } from "../components/TwoWayQuote";
import { GreeksStrip } from "../components/GreeksStrip";
import { ConventionRow } from "../components/ConventionChip";
import {
  PRODUCT_REGISTRY,
  specById,
  StructureGallery,
  PayoffChart,
  NetStructureStrip,
  type NetStructureLeg,
  type ProductBuildCtx,
} from "../products";
import { crossAssetInputsFor, crossAssetSpec } from "../products/crossAsset";
import { forward as forwardRate } from "../data/pricing";
import { impliedVolForInstrument, sampleSurface } from "../data/surface";
import {
  fmtPremiumPct,
  fmtVol,
  sideVerb,
} from "../lib/format";
import { nowNanos } from "../hooks/useClock";
import { samePair } from "../lib/universe";
import styles from "./TicketWorkspace.module.css";

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
 * The primary strike a {@link PayoffChart} draws its kink at. The type-erased
 * registry inputs are not statically known here, so we read a numeric `strike`
 * field if the active family exposes one (the vanilla/strategy and most exotic
 * families do, with `0` ⇒ "default to ATMF"), and fall back to the ATM-forward
 * level otherwise. This is a faithful payoff SHAPE preview (kink location), not a
 * priced P&L — the chart's accessible label says so.
 */
function previewStrike(inputs: unknown, atmForward: number): number {
  if (inputs && typeof inputs === "object" && "strike" in inputs) {
    const k = (inputs as { strike?: unknown }).strike;
    if (typeof k === "number" && k > 0) return k;
  }
  return atmForward;
}

export function TicketWorkspace(): React.ReactElement {
  const app = useApp();
  const [structure, setStructure] = useState<string>("RISK_REVERSAL");
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

  // Drop any priced state (single-dealer quote AND multi-dealer panel) — every
  // contract edit (structure/expiry/inputs/model) invalidates both equally.
  const clearPriced = useCallback(() => {
    setQuote(null);
    setDealerPanel(null);
  }, []);

  // Universe → ticket pre-target: a non-FX underlier selection arms a one-shot
  // target; on arrival (mount or while open) we re-point the ticket at the
  // cross-asset vanilla spec seeded with that EXACT `Underlying` + settlement
  // mechanics — through the spec's own input/wire seam (`crossAssetInputsFor` is
  // the inverse of `crossAssetUnderlying`; no duplicated wire-building) — then
  // consume the target. FX selections never arm it, so FX flows are untouched.
  useEffect(() => {
    const target = app.ticketTarget;
    if (!target) return;
    const seeded = crossAssetInputsFor(target.underlying, target.settlementStyle);
    if (seeded) {
      setStructure(crossAssetSpec.id);
      setInputsByStructure((m) => ({ ...m, [crossAssetSpec.id]: seeded }));
      clearPriced();
    }
    app.clearTicketTarget();
  }, [app, app.ticketTarget, clearPriced]);

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

  // A trader-facing expiry label that is honest for BOTH modes: a broken date
  // reads as its calendar date, never coerced into a tenor band.
  const expiryLabel =
    expiryMode === "DATE" && brokenDate ? fmtBrokenDate(brokenDate) : tenorChoice.label;

  // In DATE mode the trader must pick a date before there is a horizon to price.
  const expiryReady = expiryMode === "TENOR" || brokenDate !== null;

  // The market/contract context every spec reads to build its wire instrument and
  // render its input block. The forward uses the active pair's real market
  // (spot/rDom/rFor) at the selected horizon; strikes that default to ATMF read it.
  const atmForward = forwardRate(app.pairCtx.market, tenorYears);
  const spot = app.pairCtx.market.spot;
  const pipDecimals = app.pairCtx.pipDecimals;
  const ctx = useMemo<ProductBuildCtx>(
    () => ({
      pair: app.pairCtx.pair,
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

  // The active product family and its current inputs. `effectiveCtx` clamps the
  // trader's booking-model selection into the family's allowed set, so switching
  // to a product that does not support LSV silently falls back to DEFAULT (never
  // sends a model the server would reject with `UnsupportedModel`).
  const spec = specById(structure)!;
  const inputs = inputsByStructure[structure];
  const allowedModels = spec.allowedModels;
  const effectiveModel: PricingModel = allowedModels.includes(pricingModel)
    ? pricingModel
    : allowedModels[0]!;
  const effectiveCtx: ProductBuildCtx = { ...ctx, pricingModel: effectiveModel };

  // Offline (the in-app mock) the LSV engine is NOT available — it is a server-side
  // model (CLAUDE.md: no faked LSV numbers). Detect offline via the documented
  // transport label ("mock/replay"; the live transport's label starts with "live").
  // When LSV is the effective model offline, pricing is gated to the live server.
  const isOffline = !app.transport.label.startsWith("live");
  const lsvUnavailableOffline = isOffline && effectiveModel === "LOCAL_STOCH_VOL";

  const instrument = spec.toInstrument(inputs as never, effectiveCtx);

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
  const faceVol = app.surface
    ? impliedVolForInstrument(app.surface, instrument, app.pairCtx.market)
    : app.pairCtx.market.vol;

  const requestQuote = useCallback(async () => {
    // The LSV engine is server-side: do not request a price offline for an
    // LSV-priced instrument (the mock would have to fake it). Surface the honest
    // gate instead of dialling a price.
    if (lsvUnavailableOffline) {
      setFill("Local-Stoch-Vol pricing is server-side — run against the live server.");
      return;
    }
    setBusy(true);
    setFill(null);
    const inst = spec.toInstrument(inputs as never, effectiveCtx);
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
    setBusy(false);
  }, [app, spec, inputs, effectiveCtx, lsvUnavailableOffline, rfqMode]);

  const accept = useCallback(
    async (side: "BUY" | "SELL") => {
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
    [app, quote],
  );

  /**
   * Book one dealer line off the ranked panel: `acceptQuote` carrying the row's
   * `lpId` trades exactly that pinned line. The expiry gate reads the ROW's own
   * last-look deadline (each line carries its own window), mirroring `accept`.
   */
  const bookDealerLine = useCallback(
    async (quoteId: bigint, lpId: string, side: "BUY" | "SELL") => {
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
    [app, dealerPanel],
  );

  // ⏎ requests, ⌘⏎ accepts the offered side (keyboard-first, §4.1).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (app.paletteOpen) return;
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        void accept("BUY");
      } else if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
        const tag = (e.target as HTMLElement)?.tagName;
        if (tag === "INPUT" || tag === "BUTTON") return;
        if (!expiryReady) return;
        e.preventDefault();
        void requestQuote();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [accept, requestQuote, app.paletteOpen, expiryReady]);

  /** Replace the active family's inputs and clear any stale quote/panel against them. */
  const updateInputs = useCallback(
    (next: unknown) => {
      setInputsByStructure((m) => ({ ...m, [structure]: next }));
      clearPriced();
    },
    [structure, clearPriced],
  );

  const strikePreview = previewStrike(inputs, atmForward);
  const netLegs = netStructureLegs(instrument);

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.ticket} noPadding>
        <div className={styles.head}>
          <span className={`num ${styles.pair}`}>
            {app.pairCtx.pair.base}/{app.pairCtx.pair.quote}
          </span>
          <span className={styles.dot}>·</span>
          <span className={styles.headStructure}>{spec.label}</span>
          <div className={styles.headRight}>
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
          </div>
        </div>

        <StructureGallery
          value={structure}
          onSelect={(id) => {
            setStructure(id);
            clearPriced();
          }}
        />

        {allowedModels.length > 1 || structure === "WINDOW_BARRIER" ? (
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

        <div className={styles.expiryBlock}>
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
        </div>

        <spec.InputBlock value={inputs as never} onChange={updateInputs} ctx={effectiveCtx} />

        <div className={styles.payoffPreview}>
          <PayoffChart structureId={structure} strike={strikePreview} spot={spot} />
        </div>

        {netLegs.length > 0 && (
          <NetStructureStrip
            legs={netLegs}
            quoteCcy={app.pairCtx.pair.quote}
            baseCcy={app.pairCtx.pair.base}
          />
        )}

        {dealerPanel ? (
          <div className={styles.dealerPanelBlock}>
            <DealerPanel
              panel={dealerPanel}
              windowSeconds={8}
              onBook={(quoteId, lpId, side) => void bookDealerLine(quoteId, lpId, side)}
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
        )}

        {quote && !isSwap(structure) && (
          <div className={styles.greeksRow}>
            <GreeksStrip greeks={quote.greeks} />
          </div>
        )}

        <div className={styles.convRow}>
          {!quote && <ConventionRow conventions={app.conventions} />}
          {quote && (
            <span className={`num ${styles.volFace}`}>
              vol {fmtVol(faceVol)}
            </span>
          )}
        </div>

        {fill && <div className={styles.fill}>{fill}</div>}

        <div className={styles.actions}>
          <Button
            variant="primary"
            size="lg"
            onClick={requestQuote}
            kbd="⏎"
            disabled={busy || !expiryReady || lsvUnavailableOffline}
          >
            {busy
              ? "Pricing…"
              : !expiryReady
                ? "Pick a date"
                : lsvUnavailableOffline
                  ? "LSV — live server only"
                  : quote || dealerPanel
                    ? "Re-request"
                    : rfqMode === "PANEL"
                      ? "Request panel"
                      : "Request quote"}
          </Button>
          {quote && (
            <>
              <Button variant="bid" size="lg" onClick={() => accept("SELL")}>
                Sell {fmtPremiumPct(quote.price.bid)}
              </Button>
              <Button variant="offer" size="lg" onClick={() => accept("BUY")} kbd="⌘⏎">
                Buy {fmtPremiumPct(quote.price.offer)}
              </Button>
            </>
          )}
          <span className={styles.promote}>
            <Button
              variant="ghost"
              onClick={() => {
                app.stream.subscribe(instrument, app.conventions, structureLabel(structure));
                app.setWorkspace("stream");
              }}
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
        </div>
      </Panel>

      <p className={styles.caption}>
        One card = analytics + executable. Conventions on the face, Solve inline,
        last-look visible. Promote the exact structure to the blotter or the risk
        grid — same Instrument, no re-keying.
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
