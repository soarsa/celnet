/**
 * TicketWorkspace — THE differentiator (GUI-DESIGN §4.1). One card that is the
 * analytics surface AND the executable: build a structure (vanilla or multi-leg
 * strategy), see a live two-way + the full 14-Greek set + the conventions on the
 * FACE, and hit it without changing screens. Solve (zero-cost) is inline; the
 * last-look window is a visible depleting ring; "Stream this" promotes the exact
 * Instrument into the blotter and "Add to risk" drops it in the scenario grid —
 * the same Instrument object, no re-keying.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import type {
  AccumulatorMonitoring,
  AsianMethod,
  AveragingStyle,
  BrokenDate,
  Instrument,
  Leg,
  LookbackMonitoring,
  LookbackStyle,
  MarketContext,
  OptionType,
  QuantoPayoff,
  Quote,
  StrategyKind,
  TarfRedemption,
  Tenor,
} from "../data/contract";
import { Panel } from "../components/Panel";
import { Button } from "../components/Button";
import { DatePicker } from "../components/DatePicker";
import { TwoWayQuote } from "../components/TwoWayQuote";
import { GreeksStrip } from "../components/GreeksStrip";
import { ConventionRow } from "../components/ConventionChip";
import {
  accumulatorInstrument,
  asianInstrument,
  cliquetInstrument,
  equalFixingSchedule,
  forwardStartInstrument,
  isPlainCliquet,
  lookbackInstrument,
  quantoInstrument,
  strategyInstrument,
  tarfInstrument,
  vanillaInstrument,
  varianceSwapInstrument,
  volatilitySwapInstrument,
  tenorYearsToTenor,
  type AccumulatorTerms,
  type AsianTerms,
  type CliquetTerms,
  type ForwardStartTerms,
  type LookbackTerms,
  type QuantoTerms,
  type TarfTerms,
} from "../data/seed";
import { forward as forwardRate, strikeFromDelta } from "../data/pricing";
import { impliedVolForInstrument, sampleSurface } from "../data/surface";
import {
  fmtPremiumPct,
  fmtRate,
  fmtVol,
  sideVerb,
} from "../lib/format";
import { nowNanos } from "../hooks/useClock";
import { samePair } from "../lib/universe";
import styles from "./TicketWorkspace.module.css";

/**
 * The product the ticket builds: a vanilla, one of the multi-leg strategies, or
 * one of the volatility products newly on the contract (variance / volatility
 * swap, arithmetic-average-rate Asian). Purpose-named, vendor/method-neutral.
 */
type Structure =
  | "VANILLA"
  | StrategyKind
  | "VARIANCE_SWAP"
  | "VOLATILITY_SWAP"
  | "ASIAN"
  | "FORWARD_START"
  | "CLIQUET"
  | "QUANTO"
  | "TARF"
  | "ACCUMULATOR"
  | "LOOKBACK";

/** True for the products that carry no enumerable option legs. */
function isLegless(s: Structure): boolean {
  return (
    s === "VARIANCE_SWAP" ||
    s === "VOLATILITY_SWAP" ||
    s === "ASIAN" ||
    s === "FORWARD_START" ||
    s === "CLIQUET" ||
    s === "QUANTO" ||
    s === "TARF" ||
    s === "ACCUMULATOR" ||
    s === "LOOKBACK"
  );
}

/** True for the variance/volatility swaps (priced as a fair strike, not a premium). */
function isSwap(s: Structure): boolean {
  return s === "VARIANCE_SWAP" || s === "VOLATILITY_SWAP";
}

/** Which expiry input the trader is using: a standard tenor or an arbitrary date. */
type ExpiryMode = "TENOR" | "DATE";

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

const STRUCTURES: { id: Structure; label: string }[] = [
  { id: "VANILLA", label: "Vanilla" },
  { id: "RISK_REVERSAL", label: "Risk Reversal" },
  { id: "STRANGLE", label: "Strangle" },
  { id: "STRADDLE", label: "Straddle" },
  { id: "SEAGULL", label: "Seagull" },
  { id: "VARIANCE_SWAP", label: "Variance Swap" },
  { id: "VOLATILITY_SWAP", label: "Volatility Swap" },
  { id: "ASIAN", label: "Asian" },
  { id: "FORWARD_START", label: "Forward Start" },
  { id: "CLIQUET", label: "Cliquet" },
  { id: "QUANTO", label: "Quanto" },
  { id: "TARF", label: "TARF" },
  { id: "ACCUMULATOR", label: "Accumulator" },
  { id: "LOOKBACK", label: "Lookback" },
];

/** The Asian-specific ticket inputs (option type / strike / schedule / method). */
interface AsianInputs {
  optionType: OptionType;
  /** Strike as an absolute level; defaults to ATM-forward when first shown. */
  strike: number;
  averaging: AveragingStyle;
  observations: number;
  method: AsianMethod;
}

/** The default Asian inputs at first render (a fresh, discrete, ATM-ish call). */
const DEFAULT_ASIAN: AsianInputs = {
  optionType: "CALL",
  strike: 0,
  averaging: "DISCRETE",
  observations: 12,
  method: "CURRAN",
};

/** Build the Asian `AsianTerms` from the ticket inputs (fresh — no seasoning). */
function asianTerms(a: AsianInputs): AsianTerms {
  return {
    optionType: a.optionType,
    strike: a.strike,
    averaging: a.averaging,
    observations: a.observations,
    method: a.method,
    elapsedAvg: 0,
    elapsedWeight: 0,
  };
}

/** The forward-start ticket inputs (option type / reset-moneyness / reset date). */
interface ForwardStartInputs {
  optionType: OptionType;
  /** Strike-reset multiple `m` (`m = 1` is the ATM-forward reset). */
  moneyness: number;
  /** Reset (strike-fixing) date `t₁` in years (clamped to `[0, expiry]` at build). */
  reset: number;
}

const DEFAULT_FORWARD_START: ForwardStartInputs = {
  optionType: "CALL",
  moneyness: 1,
  reset: 0.25,
};

/** Build `ForwardStartTerms` from the inputs, clamping reset into `[0, expiry]`. */
function forwardStartTerms(f: ForwardStartInputs, expiryYears: number): ForwardStartTerms {
  return {
    optionType: f.optionType,
    moneyness: f.moneyness,
    reset: Math.min(Math.max(f.reset, 0), expiryYears),
  };
}

/**
 * The cliquet ticket inputs. The local cap/floor are presence-tracked via
 * `useCap`/`useFloor` toggles; any clamp on switches the pricer to Monte-Carlo
 * (the build surfaces the standard error). `mcPairs`/`mcSeed` tune the clamped MC
 * and are ignored for a plain (unclamped) ratchet.
 */
interface CliquetInputs {
  optionType: OptionType;
  moneyness: number;
  periods: number;
  useLocalCap: boolean;
  localCap: number;
  useLocalFloor: boolean;
  localFloor: number;
  mcPairs: number;
  mcSeed: bigint;
}

const DEFAULT_CLIQUET: CliquetInputs = {
  optionType: "CALL",
  moneyness: 1,
  periods: 4,
  useLocalCap: false,
  localCap: 0.05,
  useLocalFloor: false,
  localFloor: 0,
  mcPairs: 0,
  mcSeed: 0xc11_c0e7n,
};

/** Build `CliquetTerms` from the inputs (clamps omitted unless their toggle is on). */
function cliquetTerms(c: CliquetInputs): CliquetTerms {
  const terms: CliquetTerms = {
    optionType: c.optionType,
    moneyness: c.moneyness,
    periods: Math.max(1, Math.trunc(c.periods)),
    mcPairs: Math.max(0, Math.trunc(c.mcPairs)),
    mcSeed: c.mcSeed,
  };
  if (c.useLocalCap) terms.localCap = c.localCap;
  if (c.useLocalFloor) terms.localFloor = c.localFloor;
  return terms;
}

/** The quanto ticket inputs (payoff kind / option type / strike / σ_Z / ρ). */
interface QuantoInputs {
  payoff: QuantoPayoff;
  optionType: OptionType;
  /** Strike as an absolute level; defaults to ATM-forward when first shown. */
  strike: number;
  conversionVol: number;
  correlation: number;
}

const DEFAULT_QUANTO: QuantoInputs = {
  payoff: "VANILLA",
  optionType: "CALL",
  strike: 0,
  conversionVol: 0.1,
  correlation: 0.3,
};

/** Build `QuantoTerms` from the inputs (strike falls back to ATM-forward). */
function quantoTerms(q: QuantoInputs, atmForward: number): QuantoTerms {
  return {
    payoff: q.payoff,
    optionType: q.optionType,
    strike: q.strike > 0 ? q.strike : atmForward,
    conversionVol: Math.max(0, q.conversionVol),
    correlation: Math.min(1, Math.max(-1, q.correlation)),
  };
}

/**
 * The TARF ticket inputs (favourable side / strike / target / leverage / gap-risk
 * redemption / fixing count / MC). A TARF is always Monte-Carlo, so the build
 * always surfaces a standard error. `strike` 0 ⇒ default to the ATM-forward level.
 */
interface TarfInputs {
  optionType: OptionType;
  strike: number;
  target: number;
  leverage: number;
  redemption: TarfRedemption;
  fixings: number;
  mcPairs: number;
  mcSeed: bigint;
}

const DEFAULT_TARF: TarfInputs = {
  optionType: "PUT",
  strike: 0,
  target: 0.1,
  leverage: 2,
  redemption: "FULL_GAIN",
  fixings: 12,
  mcPairs: 0,
  mcSeed: 0x7a_2fn,
};

/** Build `TarfTerms` from the inputs (equally-spaced schedule; strike ATMF-defaulted). */
function tarfTerms(t: TarfInputs, atmForward: number, expiryYears: number): TarfTerms {
  const fixings = Math.max(1, Math.trunc(t.fixings));
  return {
    optionType: t.optionType,
    strike: t.strike > 0 ? t.strike : atmForward,
    target: Math.max(0, t.target),
    leverage: Math.max(0, t.leverage),
    redemption: t.redemption,
    schedule: equalFixingSchedule(fixings, expiryYears, 1),
    mcPairs: Math.max(0, Math.trunc(t.mcPairs)),
    mcSeed: t.mcSeed,
  };
}

/**
 * The accumulator ticket inputs (pivot / knock-out barrier / leverage / monitoring
 * / fixing count / MC). Always Monte-Carlo. `pivot` 0 ⇒ default to the ATM-forward
 * level; the barrier is held strictly above the pivot at build.
 */
interface AccumulatorInputs {
  pivot: number;
  barrierOffset: number;
  leverage: number;
  monitoring: AccumulatorMonitoring;
  fixings: number;
  mcPairs: number;
  mcSeed: bigint;
}

const DEFAULT_ACCUMULATOR: AccumulatorInputs = {
  pivot: 0,
  barrierOffset: 0.05,
  leverage: 2,
  monitoring: "DISCRETE",
  fixings: 12,
  mcPairs: 0,
  mcSeed: 0xacc_1en,
};

/**
 * Build `AccumulatorTerms` (pivot ATMF-defaulted; barrier = pivot·(1+offset), kept
 * strictly above the pivot; equally-spaced schedule).
 */
function accumulatorTerms(
  a: AccumulatorInputs,
  atmForward: number,
  expiryYears: number,
): AccumulatorTerms {
  const pivot = a.pivot > 0 ? a.pivot : atmForward;
  const offset = Math.max(1e-4, a.barrierOffset);
  const fixings = Math.max(1, Math.trunc(a.fixings));
  return {
    pivot,
    barrier: pivot * (1 + offset),
    leverage: Math.max(0, a.leverage),
    monitoring: a.monitoring,
    schedule: equalFixingSchedule(fixings, expiryYears, 1),
    mcPairs: Math.max(0, Math.trunc(a.mcPairs)),
    mcSeed: a.mcSeed,
  };
}

/**
 * The lookback ticket inputs (style floating|fixed / option / monitoring
 * continuous|discrete / strike / observations / MC). CONTINUOUS prices by exact
 * closed form (no std-error); DISCRETE prices by Monte-Carlo (surfaces a stderr).
 * `strike` 0 ⇒ default to the ATM-forward level (FIXED family only).
 */
interface LookbackInputs {
  style: LookbackStyle;
  optionType: OptionType;
  monitoring: LookbackMonitoring;
  strike: number;
  observations: number;
  mcPairs: number;
  mcSeed: bigint;
}

const DEFAULT_LOOKBACK: LookbackInputs = {
  style: "FLOATING",
  optionType: "CALL",
  monitoring: "CONTINUOUS",
  strike: 0,
  observations: 52,
  mcPairs: 0,
  mcSeed: 0x100_b_acen,
};

/** Build `LookbackTerms` from the inputs (strike ATMF-defaulted for the FIXED family). */
function lookbackTerms(l: LookbackInputs, atmForward: number): LookbackTerms {
  return {
    style: l.style,
    optionType: l.optionType,
    monitoring: l.monitoring,
    strike: l.strike > 0 ? l.strike : atmForward,
    observations: Math.max(0, Math.trunc(l.observations)),
    mcPairs: Math.max(0, Math.trunc(l.mcPairs)),
    mcSeed: l.mcSeed,
  };
}

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

/** The non-structural build inputs the volatility products need. */
interface BuildExtras {
  /** Variance/volatility swap strike in vol terms (0 ⇒ price the fair strike). */
  swapStrikeVol: number;
  /** Asian inputs (option type / strike / schedule / method). */
  asian: AsianInputs;
  /** Forward-start inputs (option type / reset-moneyness / reset date). */
  forwardStart: ForwardStartInputs;
  /** Cliquet inputs (option type / moneyness / periods / clamps / MC). */
  cliquet: CliquetInputs;
  /** Quanto terms (payoff / option type / strike / σ_Z / ρ), strike ATMF-defaulted. */
  quanto: QuantoTerms;
  /** TARF terms (favourable side / strike / target / leverage / redemption / MC). */
  tarf: TarfTerms;
  /** Accumulator terms (pivot / barrier / leverage / monitoring / MC). */
  accumulator: AccumulatorTerms;
  /** Lookback terms (style / option / monitoring / strike / observations / MC). */
  lookback: LookbackTerms;
}

function buildInstrument(
  structure: Structure,
  pair: { base: string; quote: string },
  tenor: Tenor,
  tenorYears: number,
  notionalMm: number,
  extras: BuildExtras,
): Instrument {
  let base: Instrument;
  switch (structure) {
    case "VANILLA":
      base = vanillaInstrument(pair, tenorYears, "CALL", 0.25, notionalMm);
      break;
    case "VARIANCE_SWAP":
      base = varianceSwapInstrument(pair, tenorYears, notionalMm, extras.swapStrikeVol);
      break;
    case "VOLATILITY_SWAP":
      base = volatilitySwapInstrument(pair, tenorYears, notionalMm, extras.swapStrikeVol);
      break;
    case "ASIAN":
      base = asianInstrument(pair, tenorYears, notionalMm, asianTerms(extras.asian));
      break;
    case "FORWARD_START":
      base = forwardStartInstrument(
        pair,
        tenorYears,
        notionalMm,
        forwardStartTerms(extras.forwardStart, tenorYears),
      );
      break;
    case "CLIQUET":
      base = cliquetInstrument(pair, tenorYears, notionalMm, cliquetTerms(extras.cliquet));
      break;
    case "QUANTO":
      base = quantoInstrument(pair, tenorYears, notionalMm, extras.quanto);
      break;
    case "TARF":
      base = tarfInstrument(pair, tenorYears, notionalMm, extras.tarf);
      break;
    case "ACCUMULATOR":
      base = accumulatorInstrument(pair, tenorYears, notionalMm, extras.accumulator);
      break;
    case "LOOKBACK":
      base = lookbackInstrument(pair, tenorYears, notionalMm, extras.lookback);
      break;
    default:
      base = strategyInstrument(pair, tenorYears, structure, notionalMm);
      break;
  }
  // Stamp the trader-facing tenor (ON/TN/SN/IMM/BROKEN_DATE) onto the instrument;
  // `expiryYears` stays authoritative for pricing (see celnet.proto Instrument).
  return { ...base, tenor };
}

export function TicketWorkspace(): React.ReactElement {
  const app = useApp();
  const [structure, setStructure] = useState<Structure>("RISK_REVERSAL");
  const [expiryMode, setExpiryMode] = useState<ExpiryMode>("TENOR");
  // Standard-tenor selection (index into TENOR_CHOICES); default 1M.
  const [tenorIdx, setTenorIdx] = useState(5);
  const [brokenDate, setBrokenDate] = useState<BrokenDate | null>(null);
  const [notionalMm, setNotionalMm] = useState(10);
  const [quote, setQuote] = useState<Quote | null>(null);
  const [busy, setBusy] = useState(false);
  const [fill, setFill] = useState<string | null>(null);
  // Variance/volatility-swap strike in VOL terms; 0 ⇒ request the fair strike off
  // the priced reply (the server echoes K_var/K_vol in `resolved_strike`).
  const [swapStrikeVol, setSwapStrikeVol] = useState(0);
  // Asian inputs (option type / strike / schedule / method). Strike 0 ⇒ default to
  // the ATM-forward level on first build so the ticket prices a sensible default.
  const [asianInputs, setAsianInputs] = useState<AsianInputs>(DEFAULT_ASIAN);
  // Wave-2 product inputs: forward-start (reset), cliquet (schedule/clamps/MC),
  // quanto (payoff/σ_Z/ρ; strike ATMF-defaulted like the Asian).
  const [forwardStartInputs, setForwardStartInputs] =
    useState<ForwardStartInputs>(DEFAULT_FORWARD_START);
  const [cliquetInputs, setCliquetInputs] = useState<CliquetInputs>(DEFAULT_CLIQUET);
  const [quantoInputs, setQuantoInputs] = useState<QuantoInputs>(DEFAULT_QUANTO);
  // Wave-3 product inputs: TARF + accumulator (geared fixings, always MC), lookback
  // (continuous closed-form OR discrete MC). Strike/pivot ATMF-defaulted like the
  // Asian; the equally-spaced schedule is derived from the fixing count at build.
  const [tarfInputs, setTarfInputs] = useState<TarfInputs>(DEFAULT_TARF);
  const [accumulatorInputs, setAccumulatorInputs] =
    useState<AccumulatorInputs>(DEFAULT_ACCUMULATOR);
  const [lookbackInputs, setLookbackInputs] = useState<LookbackInputs>(DEFAULT_LOOKBACK);

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

  // The Asian strike defaults to the ATM-forward level when the trader has not
  // typed one (strike 0), so the ticket prices a sensible at-the-money average by
  // default; an explicit strike overrides it. The forward uses the active pair's
  // real market (spot/rDom/rFor) at the selected horizon.
  const atmForward = forwardRate(app.pairCtx.market, tenorYears);
  const extras = useMemo<BuildExtras>(
    () => ({
      swapStrikeVol,
      asian: {
        ...asianInputs,
        strike: asianInputs.strike > 0 ? asianInputs.strike : atmForward,
      },
      forwardStart: forwardStartInputs,
      cliquet: cliquetInputs,
      quanto: quantoTerms(quantoInputs, atmForward),
      tarf: tarfTerms(tarfInputs, atmForward, tenorYears),
      accumulator: accumulatorTerms(accumulatorInputs, atmForward, tenorYears),
      lookback: lookbackTerms(lookbackInputs, atmForward),
    }),
    [
      swapStrikeVol,
      asianInputs,
      forwardStartInputs,
      cliquetInputs,
      quantoInputs,
      tarfInputs,
      accumulatorInputs,
      lookbackInputs,
      atmForward,
      tenorYears,
    ],
  );

  const instrument = buildInstrument(
    structure,
    app.pairCtx.pair,
    resolved.tenor,
    tenorYears,
    notionalMm,
    extras,
  );

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
    setBusy(true);
    setFill(null);
    const inst = buildInstrument(
      structure,
      app.pairCtx.pair,
      resolved.tenor,
      tenorYears,
      notionalMm,
      extras,
    );
    const q = await app.transport.requestQuote(
      inst,
      app.conventions,
      `tkt-${Date.now()}`,
    );
    setQuote(q);
    setBusy(false);
  }, [app, structure, resolved.tenor, tenorYears, notionalMm, extras]);

  const accept = useCallback(
    async (side: "BUY" | "SELL") => {
      if (!quote) return;
      if (quote.validUntilNanos <= nowNanos()) {
        setFill("Quote expired — re-request");
        setQuote(null);
        return;
      }
      const exec = await app.transport.acceptQuote(quote.quoteId, side, quote.idempotencyKey);
      setFill(`Filled ${sideVerb(side)} @ ${exec.tradedPremium.toFixed(3)} · exec #${exec.executionId}`);
      setQuote(null);
    },
    [app, quote],
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

  // P0-10: strikes (and any delta→strike resolution) are computed off the
  // ACTIVE pair's real market (spot/vol/rDom/rFor) — never the old hardcoded
  // rDom 0.04 / rFor 0.02 constants — so the displayed legs reprice with the
  // pair the trader is actually looking at.
  const legs = describeLegs(instrument, app.pairCtx.market, tenorYears);

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.ticket} noPadding>
        <div className={styles.head}>
          <span className={`num ${styles.pair}`}>
            {app.pairCtx.pair.base}/{app.pairCtx.pair.quote}
          </span>
          <span className={styles.dot}>·</span>
          <select
            className={styles.structureSelect}
            value={structure}
            onChange={(e) => {
              setStructure(e.target.value as Structure);
              setQuote(null);
            }}
            aria-label="structure"
          >
            {STRUCTURES.map((s) => (
              <option key={s.id} value={s.id}>
                {s.label}
              </option>
            ))}
          </select>
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
                  setQuote(null);
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
                  setQuote(null);
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
                    setQuote(null);
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
                  setQuote(null);
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

        {isLegless(structure) ? (
          <ProductInputs
            structure={structure}
            swapStrikeVol={swapStrikeVol}
            onSwapStrikeVol={(v) => {
              setSwapStrikeVol(v);
              setQuote(null);
            }}
            asian={asianInputs}
            atmForward={atmForward}
            pipDecimals={app.pairCtx.pipDecimals}
            onAsian={(next) => {
              setAsianInputs(next);
              setQuote(null);
            }}
            forwardStart={forwardStartInputs}
            onForwardStart={(next) => {
              setForwardStartInputs(next);
              setQuote(null);
            }}
            cliquet={cliquetInputs}
            onCliquet={(next) => {
              setCliquetInputs(next);
              setQuote(null);
            }}
            quanto={quantoInputs}
            onQuanto={(next) => {
              setQuantoInputs(next);
              setQuote(null);
            }}
            tarf={tarfInputs}
            onTarf={(next) => {
              setTarfInputs(next);
              setQuote(null);
            }}
            accumulator={accumulatorInputs}
            onAccumulator={(next) => {
              setAccumulatorInputs(next);
              setQuote(null);
            }}
            lookback={lookbackInputs}
            onLookback={(next) => {
              setLookbackInputs(next);
              setQuote(null);
            }}
          />
        ) : (
          <div className={styles.legs}>
            {legs.map((leg, i) => (
              <div className={styles.leg} key={i}>
                <span className={styles.legNo}>LEG {i + 1}</span>
                <span className={`${styles.legSide} ${leg.side === "BUY" ? styles.buy : styles.sell}`}>
                  {leg.side}
                </span>
                <span className={styles.legType}>{leg.type}</span>
                <span className={`num ${styles.legDelta}`}>{leg.deltaLabel}</span>
                <span className={styles.legArrow}>▸</span>
                <span className={`num ${styles.legStrike}`}>K {fmtRate(leg.strike, app.pairCtx.pipDecimals)}</span>
              </div>
            ))}
            {structure !== "VANILLA" && (
              <button
                className={styles.solveChip}
                onClick={requestQuote}
                disabled={!expiryReady}
                title="Solve zero-cost strike inline"
              >
                Solve: zero-cost
              </button>
            )}
          </div>
        )}

        {quote && isSwap(structure) ? (
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
            disabled={busy || !expiryReady}
          >
            {busy
              ? "Pricing…"
              : !expiryReady
                ? "Pick a date"
                : quote
                  ? "Re-request"
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

interface LegView {
  side: "BUY" | "SELL";
  type: string;
  deltaLabel: string;
  strike: number;
}

function describeLegs(
  instrument: Instrument,
  market: MarketContext,
  t: number,
): LegView[] {
  const toView = (leg: Leg): LegView => {
    const strike =
      leg.strike.kind === "strike"
        ? leg.strike.strike
        : strikeFromDelta(leg.strike.delta, market, t);
    const deltaLabel =
      leg.strike.kind === "delta"
        ? `${Math.round(Math.abs(leg.strike.delta) * 100)}Δ`
        : "abs";
    return {
      side: leg.side === "SELL" ? "SELL" : "BUY",
      type: leg.optionType === "CALL" ? "Call" : "Put",
      deltaLabel,
      strike,
    };
  };
  switch (instrument.product.kind) {
    case "vanilla": {
      const v = instrument.product.vanilla;
      return [toView({ optionType: v.optionType, strike: v.strike, side: "BUY", ratio: 1 })];
    }
    case "strategy":
      return instrument.product.strategy.legs.map(toView);
    case "varianceSwap":
    case "volatilitySwap":
    case "asianOption":
    case "forwardStart":
    case "cliquet":
    case "quanto":
    case "tarf":
    case "accumulator":
    case "lookback":
      // Vol-strip / average-rate / reset / converted / path-dependent products
      // carry no enumerable option legs; the ticket renders their own input block
      // instead of a leg ladder.
      return [];
  }
}

function structureLabel(s: Structure): string {
  switch (s) {
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
  }
}

/** A keyboard-reachable Call/Put toggle reused across the product input blocks. */
function OptionToggle(props: {
  value: OptionType;
  onChange: (next: OptionType) => void;
}): React.ReactElement {
  return (
    <div className={styles.toggleGroup} role="tablist" aria-label="option type">
      {(["CALL", "PUT"] as OptionType[]).map((ot) => (
        <button
          key={ot}
          role="tab"
          aria-selected={props.value === ot}
          className={`${styles.modeTab} ${props.value === ot ? styles.modeActive : ""}`}
          onClick={() => props.onChange(ot)}
        >
          {ot === "CALL" ? "Call" : "Put"}
        </button>
      ))}
    </div>
  );
}

/**
 * The product-specific input block for the legless products. A
 * variance/volatility swap takes a single strike in VOL terms (0 ⇒ price the
 * fair strike); an Asian takes its option type, strike, averaging schedule
 * (discrete fixings count or continuous) and the analytic method; a forward-start
 * takes its option type, reset-moneyness and reset date; a cliquet takes its
 * direction, per-period moneyness, period count and optional local cap/floor (any
 * clamp switches pricing to Monte-Carlo, surfacing a standard error); a quanto
 * takes its payoff kind, option type, strike, conversion vol and correlation.
 * Every control is keyboard-reachable and resets the live quote on change (the
 * parent clears it) so a stale price is never shown against changed terms.
 */
function ProductInputs(props: {
  structure: Structure;
  swapStrikeVol: number;
  onSwapStrikeVol: (v: number) => void;
  asian: AsianInputs;
  atmForward: number;
  pipDecimals: number;
  onAsian: (next: AsianInputs) => void;
  forwardStart: ForwardStartInputs;
  onForwardStart: (next: ForwardStartInputs) => void;
  cliquet: CliquetInputs;
  onCliquet: (next: CliquetInputs) => void;
  quanto: QuantoInputs;
  onQuanto: (next: QuantoInputs) => void;
  tarf: TarfInputs;
  onTarf: (next: TarfInputs) => void;
  accumulator: AccumulatorInputs;
  onAccumulator: (next: AccumulatorInputs) => void;
  lookback: LookbackInputs;
  onLookback: (next: LookbackInputs) => void;
}): React.ReactElement {
  const {
    structure,
    swapStrikeVol,
    onSwapStrikeVol,
    asian,
    atmForward,
    pipDecimals,
    onAsian,
    forwardStart,
    onForwardStart,
    cliquet,
    onCliquet,
    quanto,
    onQuanto,
    tarf,
    onTarf,
    accumulator,
    onAccumulator,
    lookback,
    onLookback,
  } = props;

  if (structure === "VARIANCE_SWAP" || structure === "VOLATILITY_SWAP") {
    const isVar = structure === "VARIANCE_SWAP";
    return (
      <div className={styles.product}>
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Strike vol</span>
          <label className={styles.productField}>
            <input
              className="num"
              type="number"
              min={0}
              step={0.005}
              value={swapStrikeVol}
              aria-label="strike vol"
              onChange={(ev) => onSwapStrikeVol(Math.max(0, Number(ev.target.value)))}
            />
            <span>vol{swapStrikeVol > 0 ? ` = ${fmtVol(swapStrikeVol)}` : ""}</span>
          </label>
        </div>
        <p className={styles.productNote}>
          {isVar
            ? "Fair variance strike K_var = strike_vol². Leave 0 to price the fair strike (log-contract static replication, server-side)."
            : "Fair volatility strike K_vol (convexity-adjusted). Leave 0 to price the fair strike server-side."}
        </p>
      </div>
    );
  }

  if (structure === "FORWARD_START") {
    return (
      <div className={styles.product}>
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Option</span>
          <OptionToggle
            value={forwardStart.optionType}
            onChange={(ot) => onForwardStart({ ...forwardStart, optionType: ot })}
          />
          <label className={styles.productField}>
            <span>Reset m</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.01}
              value={forwardStart.moneyness}
              aria-label="moneyness"
              onChange={(ev) =>
                onForwardStart({ ...forwardStart, moneyness: Math.max(0, Number(ev.target.value)) })
              }
            />
            <span>×S(t₁)</span>
          </label>
          <label className={styles.productField}>
            <span>Reset t₁</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.05}
              value={forwardStart.reset}
              aria-label="reset"
              onChange={(ev) =>
                onForwardStart({ ...forwardStart, reset: Math.max(0, Number(ev.target.value)) })
              }
            />
            <span>y</span>
          </label>
        </div>
        <p className={styles.productNote}>
          Forward-start vanilla: the strike fixes at t₁ to m·S(t₁). Priced by the FX dual-carry
          closed form V = e^(−r_f·t₁)·S₀·u(m, T−t₁); at t₁→0 it is a plain vanilla struck at m·S₀.
        </p>
      </div>
    );
  }

  if (structure === "CLIQUET") {
    const plain = isPlainCliquet(cliquetTerms(cliquet));
    return (
      <div className={styles.product}>
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Option</span>
          <OptionToggle
            value={cliquet.optionType}
            onChange={(ot) => onCliquet({ ...cliquet, optionType: ot })}
          />
          <label className={styles.productField}>
            <span>Per-period m</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.01}
              value={cliquet.moneyness}
              aria-label="moneyness"
              onChange={(ev) =>
                onCliquet({ ...cliquet, moneyness: Math.max(0, Number(ev.target.value)) })
              }
            />
          </label>
          <label className={styles.productField}>
            <span>Periods</span>
            <input
              className="num"
              type="number"
              min={1}
              step={1}
              value={cliquet.periods}
              aria-label="periods"
              onChange={(ev) =>
                onCliquet({ ...cliquet, periods: Math.max(1, Math.trunc(Number(ev.target.value))) })
              }
            />
          </label>
        </div>
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Local clamp</span>
          <label className={styles.productField}>
            <input
              type="checkbox"
              checked={cliquet.useLocalCap}
              aria-label="use local cap"
              onChange={(ev) => onCliquet({ ...cliquet, useLocalCap: ev.target.checked })}
            />
            <span>Cap</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.005}
              value={cliquet.localCap}
              disabled={!cliquet.useLocalCap}
              aria-label="local cap"
              onChange={(ev) =>
                onCliquet({ ...cliquet, localCap: Math.max(0, Number(ev.target.value)) })
              }
            />
          </label>
          <label className={styles.productField}>
            <input
              type="checkbox"
              checked={cliquet.useLocalFloor}
              aria-label="use local floor"
              onChange={(ev) => onCliquet({ ...cliquet, useLocalFloor: ev.target.checked })}
            />
            <span>Floor</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.005}
              value={cliquet.localFloor}
              disabled={!cliquet.useLocalFloor}
              aria-label="local floor"
              onChange={(ev) =>
                onCliquet({ ...cliquet, localFloor: Math.max(0, Number(ev.target.value)) })
              }
            />
          </label>
        </div>
        {!plain && (
          <div className={styles.productRow}>
            <span className={styles.productLabel}>MC pairs</span>
            <label className={styles.productField}>
              <input
                className="num"
                type="number"
                min={0}
                step={1000}
                value={cliquet.mcPairs}
                aria-label="mc pairs"
                onChange={(ev) =>
                  onCliquet({ ...cliquet, mcPairs: Math.max(0, Math.trunc(Number(ev.target.value))) })
                }
              />
              <span>{cliquet.mcPairs > 0 ? "pairs" : "default"}</span>
            </label>
          </div>
        )}
        <p className={styles.productNote}>
          {plain
            ? "Plain ratchet: priced in closed form as the exact sum of forward-start legs (no Monte-Carlo error)."
            : "Clamped cliquet: a local cap/floor has no closed form, so it is priced by antithetic Monte-Carlo and reports a standard error alongside the price."}
        </p>
      </div>
    );
  }

  if (structure === "QUANTO") {
    return (
      <div className={styles.product}>
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Payoff</span>
          <div className={styles.toggleGroup} role="tablist" aria-label="payoff">
            {(["VANILLA", "DIGITAL"] as QuantoPayoff[]).map((p) => (
              <button
                key={p}
                role="tab"
                aria-selected={quanto.payoff === p}
                className={`${styles.modeTab} ${quanto.payoff === p ? styles.modeActive : ""}`}
                onClick={() => onQuanto({ ...quanto, payoff: p })}
              >
                {p === "VANILLA" ? "Vanilla" : "Digital"}
              </button>
            ))}
          </div>
          <OptionToggle
            value={quanto.optionType}
            onChange={(ot) => onQuanto({ ...quanto, optionType: ot })}
          />
        </div>
        <div className={styles.productRow}>
          <label className={styles.productField}>
            <span>Strike</span>
            <input
              className="num"
              type="number"
              min={0}
              step={Math.pow(10, -pipDecimals)}
              value={quanto.strike}
              aria-label="strike"
              placeholder={atmForward.toFixed(pipDecimals)}
              onChange={(ev) => onQuanto({ ...quanto, strike: Math.max(0, Number(ev.target.value)) })}
            />
            <span>{quanto.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
          </label>
          <label className={styles.productField}>
            <span>Conv vol σ_Z</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.005}
              value={quanto.conversionVol}
              aria-label="conversion vol"
              onChange={(ev) =>
                onQuanto({ ...quanto, conversionVol: Math.max(0, Number(ev.target.value)) })
              }
            />
            <span>{fmtVol(quanto.conversionVol)}</span>
          </label>
          <label className={styles.productField}>
            <span>Corr ρ</span>
            <input
              className="num"
              type="number"
              min={-1}
              max={1}
              step={0.05}
              value={quanto.correlation}
              aria-label="correlation"
              onChange={(ev) =>
                onQuanto({
                  ...quanto,
                  correlation: Math.min(1, Math.max(-1, Number(ev.target.value))),
                })
              }
            />
          </label>
        </div>
        <p className={styles.productNote}>
          Quanto {quanto.payoff === "VANILLA" ? "vanilla" : "cash-or-nothing digital"}: the payoff is
          settlement-currency converted with the quanto-drift adjustment −ρ·σ_S·σ_Z; at ρ=0 it
          collapses to the plain {quanto.payoff === "VANILLA" ? "vanilla" : "digital"}.
        </p>
      </div>
    );
  }

  if (structure === "TARF") {
    return (
      <div className={styles.product}>
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Gain side</span>
          <OptionToggle
            value={tarf.optionType}
            onChange={(ot) => onTarf({ ...tarf, optionType: ot })}
          />
          <label className={styles.productField}>
            <span>Strike</span>
            <input
              className="num"
              type="number"
              min={0}
              step={Math.pow(10, -pipDecimals)}
              value={tarf.strike}
              aria-label="strike"
              placeholder={atmForward.toFixed(pipDecimals)}
              onChange={(ev) => onTarf({ ...tarf, strike: Math.max(0, Number(ev.target.value)) })}
            />
            <span>{tarf.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
          </label>
        </div>
        <div className={styles.productRow}>
          <label className={styles.productField}>
            <span>Target</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.01}
              value={tarf.target}
              aria-label="target"
              onChange={(ev) => onTarf({ ...tarf, target: Math.max(0, Number(ev.target.value)) })}
            />
          </label>
          <label className={styles.productField}>
            <span>Leverage</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.5}
              value={tarf.leverage}
              aria-label="leverage"
              onChange={(ev) => onTarf({ ...tarf, leverage: Math.max(0, Number(ev.target.value)) })}
            />
            <span>×</span>
          </label>
          <label className={styles.productField}>
            <span>Fixings</span>
            <input
              className="num"
              type="number"
              min={1}
              step={1}
              value={tarf.fixings}
              aria-label="fixings"
              onChange={(ev) =>
                onTarf({ ...tarf, fixings: Math.max(1, Math.trunc(Number(ev.target.value))) })
              }
            />
          </label>
        </div>
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Redemption</span>
          <div className={styles.toggleGroup} role="tablist" aria-label="redemption">
            {(["FULL_GAIN", "CAPPED_GAIN"] as TarfRedemption[]).map((r) => (
              <button
                key={r}
                role="tab"
                aria-selected={tarf.redemption === r}
                className={`${styles.modeTab} ${tarf.redemption === r ? styles.modeActive : ""}`}
                onClick={() => onTarf({ ...tarf, redemption: r })}
              >
                {r === "FULL_GAIN" ? "Full gain" : "Capped gain"}
              </button>
            ))}
          </div>
          <label className={styles.productField}>
            <span>MC pairs</span>
            <input
              className="num"
              type="number"
              min={0}
              step={1000}
              value={tarf.mcPairs}
              aria-label="mc pairs"
              onChange={(ev) =>
                onTarf({ ...tarf, mcPairs: Math.max(0, Math.trunc(Number(ev.target.value))) })
              }
            />
            <span>{tarf.mcPairs > 0 ? "pairs" : "default"}</span>
          </label>
        </div>
        <p className={styles.productNote}>
          Target-Redemption Forward: a strip of geared fixings accruing client gains until the
          cumulative target redeems the structure (full-gain carries the gap risk, capped-gain does
          not). Priced by antithetic Monte-Carlo (bank present value) — it reports a standard error.
        </p>
      </div>
    );
  }

  if (structure === "ACCUMULATOR") {
    return (
      <div className={styles.product}>
        <div className={styles.productRow}>
          <label className={styles.productField}>
            <span>Pivot</span>
            <input
              className="num"
              type="number"
              min={0}
              step={Math.pow(10, -pipDecimals)}
              value={accumulator.pivot}
              aria-label="pivot"
              placeholder={atmForward.toFixed(pipDecimals)}
              onChange={(ev) =>
                onAccumulator({ ...accumulator, pivot: Math.max(0, Number(ev.target.value)) })
              }
            />
            <span>{accumulator.pivot > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
          </label>
          <label className={styles.productField}>
            <span>KO +</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.005}
              value={accumulator.barrierOffset}
              aria-label="barrier offset"
              onChange={(ev) =>
                onAccumulator({
                  ...accumulator,
                  barrierOffset: Math.max(1e-4, Number(ev.target.value)),
                })
              }
            />
            <span>·pivot</span>
          </label>
          <label className={styles.productField}>
            <span>Leverage</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.5}
              value={accumulator.leverage}
              aria-label="leverage"
              onChange={(ev) =>
                onAccumulator({ ...accumulator, leverage: Math.max(0, Number(ev.target.value)) })
              }
            />
            <span>×</span>
          </label>
        </div>
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Knock-out</span>
          <div className={styles.toggleGroup} role="tablist" aria-label="monitoring">
            {(["DISCRETE", "CONTINUOUS"] as AccumulatorMonitoring[]).map((mon) => (
              <button
                key={mon}
                role="tab"
                aria-selected={accumulator.monitoring === mon}
                className={`${styles.modeTab} ${accumulator.monitoring === mon ? styles.modeActive : ""}`}
                onClick={() => onAccumulator({ ...accumulator, monitoring: mon })}
              >
                {mon === "DISCRETE" ? "At fixings" : "Continuous"}
              </button>
            ))}
          </div>
          <label className={styles.productField}>
            <span>Fixings</span>
            <input
              className="num"
              type="number"
              min={1}
              step={1}
              value={accumulator.fixings}
              aria-label="fixings"
              onChange={(ev) =>
                onAccumulator({
                  ...accumulator,
                  fixings: Math.max(1, Math.trunc(Number(ev.target.value))),
                })
              }
            />
          </label>
          <label className={styles.productField}>
            <span>MC pairs</span>
            <input
              className="num"
              type="number"
              min={0}
              step={1000}
              value={accumulator.mcPairs}
              aria-label="mc pairs"
              onChange={(ev) =>
                onAccumulator({
                  ...accumulator,
                  mcPairs: Math.max(0, Math.trunc(Number(ev.target.value))),
                })
              }
            />
            <span>{accumulator.mcPairs > 0 ? "pairs" : "default"}</span>
          </label>
        </div>
        <p className={styles.productNote}>
          Accumulator: periodic accrual at the pivot with an up-and-out knock-out barrier and geared
          downside. Priced by antithetic Monte-Carlo (client present value) — it reports a standard
          error; continuous monitoring knocks out more often than at-fixings.
        </p>
      </div>
    );
  }

  if (structure === "LOOKBACK") {
    const isFixed = lookback.style === "FIXED";
    const isDiscrete = lookback.monitoring === "DISCRETE";
    return (
      <div className={styles.product}>
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Strike</span>
          <div className={styles.toggleGroup} role="tablist" aria-label="style">
            {(["FLOATING", "FIXED"] as LookbackStyle[]).map((st) => (
              <button
                key={st}
                role="tab"
                aria-selected={lookback.style === st}
                className={`${styles.modeTab} ${lookback.style === st ? styles.modeActive : ""}`}
                onClick={() => onLookback({ ...lookback, style: st })}
              >
                {st === "FLOATING" ? "Floating" : "Fixed"}
              </button>
            ))}
          </div>
          <OptionToggle
            value={lookback.optionType}
            onChange={(ot) => onLookback({ ...lookback, optionType: ot })}
          />
        </div>
        <div className={styles.productRow}>
          <span className={styles.productLabel}>Monitoring</span>
          <div className={styles.toggleGroup} role="tablist" aria-label="monitoring">
            {(["CONTINUOUS", "DISCRETE"] as LookbackMonitoring[]).map((mon) => (
              <button
                key={mon}
                role="tab"
                aria-selected={lookback.monitoring === mon}
                className={`${styles.modeTab} ${lookback.monitoring === mon ? styles.modeActive : ""}`}
                onClick={() => onLookback({ ...lookback, monitoring: mon })}
              >
                {mon === "CONTINUOUS" ? "Continuous" : "Discrete"}
              </button>
            ))}
          </div>
          {isFixed && (
            <label className={styles.productField}>
              <span>Strike K</span>
              <input
                className="num"
                type="number"
                min={0}
                step={Math.pow(10, -pipDecimals)}
                value={lookback.strike}
                aria-label="strike"
                placeholder={atmForward.toFixed(pipDecimals)}
                onChange={(ev) =>
                  onLookback({ ...lookback, strike: Math.max(0, Number(ev.target.value)) })
                }
              />
              <span>{lookback.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
            </label>
          )}
        </div>
        {isDiscrete && (
          <div className={styles.productRow}>
            <label className={styles.productField}>
              <span>Observations</span>
              <input
                className="num"
                type="number"
                min={0}
                step={1}
                value={lookback.observations}
                aria-label="observations"
                onChange={(ev) =>
                  onLookback({
                    ...lookback,
                    observations: Math.max(0, Math.trunc(Number(ev.target.value))),
                  })
                }
              />
              <span>{lookback.observations > 0 ? "" : "default"}</span>
            </label>
            <label className={styles.productField}>
              <span>MC pairs</span>
              <input
                className="num"
                type="number"
                min={0}
                step={1000}
                value={lookback.mcPairs}
                aria-label="mc pairs"
                onChange={(ev) =>
                  onLookback({
                    ...lookback,
                    mcPairs: Math.max(0, Math.trunc(Number(ev.target.value))),
                  })
                }
              />
              <span>{lookback.mcPairs > 0 ? "pairs" : "default"}</span>
            </label>
          </div>
        )}
        <p className={styles.productNote}>
          {isDiscrete
            ? "Discrete-monitored lookback: the extremum is sampled on a finite observation grid, so it is priced by antithetic Monte-Carlo and reports a standard error."
            : "Continuous-monitored lookback: priced by the exact closed form on the running path extremum (no Monte-Carlo error)."}
        </p>
      </div>
    );
  }

  // Asian.
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Option</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="option type">
          {(["CALL", "PUT"] as OptionType[]).map((ot) => (
            <button
              key={ot}
              role="tab"
              aria-selected={asian.optionType === ot}
              className={`${styles.modeTab} ${asian.optionType === ot ? styles.modeActive : ""}`}
              onClick={() => onAsian({ ...asian, optionType: ot })}
            >
              {ot === "CALL" ? "Call" : "Put"}
            </button>
          ))}
        </div>
        <label className={styles.productField}>
          <span>Strike</span>
          <input
            className="num"
            type="number"
            min={0}
            step={Math.pow(10, -pipDecimals)}
            value={asian.strike}
            aria-label="strike"
            placeholder={atmForward.toFixed(pipDecimals)}
            onChange={(ev) => onAsian({ ...asian, strike: Math.max(0, Number(ev.target.value)) })}
          />
          <span>{asian.strike > 0 ? "" : `ATMF ${fmtRate(atmForward, pipDecimals)}`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Averaging</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="averaging">
          {(["DISCRETE", "CONTINUOUS"] as AveragingStyle[]).map((av) => (
            <button
              key={av}
              role="tab"
              aria-selected={asian.averaging === av}
              className={`${styles.modeTab} ${asian.averaging === av ? styles.modeActive : ""}`}
              onClick={() => onAsian({ ...asian, averaging: av })}
            >
              {av === "DISCRETE" ? "Discrete" : "Continuous"}
            </button>
          ))}
        </div>
        {asian.averaging === "DISCRETE" && (
          <label className={styles.productField}>
            <span>Fixings</span>
            <input
              className="num"
              type="number"
              min={1}
              step={1}
              value={asian.observations}
              aria-label="observations"
              onChange={(ev) =>
                onAsian({ ...asian, observations: Math.max(1, Math.trunc(Number(ev.target.value))) })
              }
            />
          </label>
        )}
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Method</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="method">
          {(["CURRAN", "TURNBULL_WAKEMAN"] as AsianMethod[]).map((mm) => (
            <button
              key={mm}
              role="tab"
              aria-selected={asian.method === mm}
              className={`${styles.modeTab} ${asian.method === mm ? styles.modeActive : ""}`}
              onClick={() => onAsian({ ...asian, method: mm })}
            >
              {mm === "CURRAN" ? "Curran" : "Turnbull-Wakeman"}
            </button>
          ))}
        </div>
      </div>
      <p className={styles.productNote}>
        Arithmetic-average-rate Asian. The selected analytic method is priced server-side; the
        standalone build prices a two-moment closed form (exact in the single-fixing / zero-vol
        limits).
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
function SwapResult(props: { structure: Structure; quote: Quote }): React.ReactElement {
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

// Re-export to satisfy tree-shaking honesty of the tenor helper used elsewhere.
void tenorYearsToTenor;
