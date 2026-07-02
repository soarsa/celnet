/**
 * A small, deterministic overnight-index-swap (OIS) pricer used ONLY by the
 * standalone in-app source so the GUI produces real, internally consistent
 * fixed-income analytics (PV, par rate, PV01, DV01, key-rate ladder) WITHOUT a
 * server. It is the linear-rates analogue of `src/data/pricing.ts`: the
 * authoritative f64 pricing lives server-side in `celnet-rates`
 * (`ois.rs`/`risk.rs`/`bootstrap.rs`/`schedule.rs`); this is a presentation-side
 * stand-in that the transport seam (`src/data/transport`) replaces with the live
 * `price_rates` RPC when wired.
 *
 * It is a GENUINE OIS discounting computation, not a stub — it reproduces the
 * server's USD-SOFR self-discounting math bit-for-bit so an offline price agrees
 * with the live edge to floating-point precision:
 *
 *   - the USD-SOFR fixed-leg schedule is generated from the curve reference date
 *     under the exact server conventions (annual fixed leg, modified-following
 *     roll on the US settlement calendar, ACT/360 accrual, ACT/365F discount
 *     time) — the calendar/day-count layer of `celnet-rates::schedule`;
 *   - the discount curve is sequentially bootstrapped from the par-OIS pillars
 *     (one unknown discount factor per pillar, solved so each OIS reprices to its
 *     quoted par rate) and interpolated log-linear-on-log-DF — the shipping
 *     default of `celnet-rates::curve` (what QuantLib's `InterpolatedDiscountCurve`
 *     with `LogLinear` traits computes);
 *   - PV01 is the analytic annuity risk `N·annuity·1bp`; DV01 re-bootstraps under
 *     a +1bp parallel bump of every pillar; the key-rate ladder bumps each pillar
 *     alone, so each bucket maps to a tradeable hedge instrument and the ladder
 *     sums (to first order) to the parallel DV01.
 *
 * No method/person/vendor names appear in identifiers (CLAUDE.md rule 8); the
 * mathematical provenance is documented here, never in API names.
 */

import type {
  BrokenDate,
  OisInstrument,
  PillarTenor,
  RatesCurveSet,
  RatesPricingResult,
} from "./contract";

/** One basis point, in absolute rate terms. */
const ONE_BP = 1e-4;

/** ISO-4217 code of the only currency the rates arm prices (USD-SOFR P0 arm). */
const SUPPORTED_CURRENCY = "USD";

// ---------------------------------------------------------------------------
// civil-date core — a proleptic-Gregorian day-number arithmetic (no Date/TZ)
// ---------------------------------------------------------------------------
//
// All schedule math is exact integer date arithmetic so it is timezone- and
// locale-independent (a browser `Date` would drift across DST/locale). Dates are
// the {year, month, day} triple the wire carries; `dayNumber` is the signed count
// of days from the Unix epoch (1970-01-01), via the standard civil↔days
// algorithm, so date differences and weekdays are exact.

/** True for a leap year under the proleptic Gregorian calendar. */
function isLeapYear(year: number): boolean {
  return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
}

/** Length of `month` (1..12) in `year`, leap-year aware. */
function monthLength(year: number, month: number): number {
  if (month === 2) return isLeapYear(year) ? 29 : 28;
  return month === 4 || month === 6 || month === 9 || month === 11 ? 30 : 31;
}

/** Days from 1970-01-01 to the civil date `(y, m, d)` (Howard Hinnant's algorithm). */
function dayNumber(date: BrokenDate): number {
  const y = date.month <= 2 ? date.year - 1 : date.year;
  const era = Math.floor((y >= 0 ? y : y - 399) / 400);
  const yoe = y - era * 400; // [0, 399]
  const m = date.month;
  const doy =
    Math.floor((153 * (m > 2 ? m - 3 : m + 9) + 2) / 5) + date.day - 1; // [0, 365]
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy; // [0, 146096]
  return era * 146097 + doe - 719468;
}

/** The civil date `z` days from 1970-01-01 (inverse of {@link dayNumber}). */
function civilFromDayNumber(z: number): BrokenDate {
  const zz = z + 719468;
  const era = Math.floor((zz >= 0 ? zz : zz - 146096) / 146097);
  const doe = zz - era * 146097; // [0, 146096]
  const yoe = Math.floor(
    (doe -
      Math.floor(doe / 1460) +
      Math.floor(doe / 36524) -
      Math.floor(doe / 146096)) /
      365,
  ); // [0, 399]
  const y = yoe + era * 400;
  const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100)); // [0, 365]
  const mp = Math.floor((5 * doy + 2) / 153); // [0, 11]
  const day = doy - Math.floor((153 * mp + 2) / 5) + 1; // [1, 31]
  const month = mp < 10 ? mp + 3 : mp - 9; // [1, 12]
  return { year: month <= 2 ? y + 1 : y, month, day };
}

/** A new date `count` days after `date` (negative shifts backward). */
function addDays(date: BrokenDate, count: number): BrokenDate {
  return civilFromDayNumber(dayNumber(date) + count);
}

/**
 * The weekday of `date` as days-from-Monday (Mon=0 .. Sun=6), matching the
 * `celnet-calendar` `Weekday::number_days_from_monday` ordering the holiday rules
 * key on. 1970-01-01 is a Thursday (Hinnant: day 0 is a Thursday).
 */
function weekdayFromMonday(date: BrokenDate): number {
  // (dayNumber + 3) mod 7 gives Mon=0..Sun=6 (1970-01-01, dayNumber 0, is Thu=3).
  const w = (dayNumber(date) + 3) % 7;
  return w < 0 ? w + 7 : w;
}

/** True on Saturday or Sunday (the only weekend rule the US calendar uses). */
function isWeekend(date: BrokenDate): boolean {
  const w = weekdayFromMonday(date);
  return w === 5 || w === 6; // Sat=5, Sun=6
}

/**
 * Add `months` whole calendar months to `date`, clamping the day to the target
 * month's length (31 Jan + 1M ⇒ 28/29 Feb), mirroring `celnet_calendar::add_months`.
 */
function addMonths(date: BrokenDate, months: number): BrokenDate {
  const total = date.year * 12 + (date.month - 1) + months;
  const year = Math.floor(total / 12);
  const month = total - year * 12 + 1; // 1..12 (total is non-negative for our dates)
  const day = Math.min(date.day, monthLength(year, month));
  return { year, month, day };
}

// ---------------------------------------------------------------------------
// US settlement calendar — the real (Gregorian-computable) federal bank holidays
// ---------------------------------------------------------------------------
//
// A faithful port of `celnet_calendar::is_us_holiday`: the US federal settlement
// holidays are fully Gregorian-computable (no lunar dates), so the offline
// calendar is REAL, not an approximation — every roll lands on exactly the day
// the server's `BusinessCalendar::single(UnitedStates)` does.

/** The `n`-th `weekday` (Mon=0..Sun=6) of `(year, month)` — e.g. the 3rd Monday. */
function nthWeekday(
  year: number,
  month: number,
  weekday: number,
  n: number,
): BrokenDate {
  const first: BrokenDate = { year, month, day: 1 };
  const firstWd = weekdayFromMonday(first);
  const offset = (((weekday - firstWd) % 7) + 7) % 7;
  return addDays(first, offset + 7 * (n - 1));
}

/** The last `weekday` (Mon=0..Sun=6) of `(year, month)` — e.g. the last Monday. */
function lastWeekday(year: number, month: number, weekday: number): BrokenDate {
  const last: BrokenDate = { year, month, day: monthLength(year, month) };
  const lastWd = weekdayFromMonday(last);
  const back = (((lastWd - weekday) % 7) + 7) % 7;
  return addDays(last, -back);
}

/** The US weekend-observance shift for a fixed-date holiday (Sat→Fri, Sun→Mon). */
function usObserved(date: BrokenDate): BrokenDate {
  const w = weekdayFromMonday(date);
  if (w === 5) return addDays(date, -1); // Saturday → observed Friday
  if (w === 6) return addDays(date, 1); // Sunday → observed Monday
  return date;
}

const MONDAY = 0;
const THURSDAY = 3;

/** True when `date` is a US federal settlement holiday (mirrors `is_us_holiday`). */
function isUsHoliday(date: BrokenDate): boolean {
  const y = date.year;
  const z = dayNumber(date);
  const on = (d: BrokenDate): boolean => dayNumber(d) === z;

  if (on(usObserved({ year: y, month: 1, day: 1 }))) return true; // New Year's Day
  if (on(nthWeekday(y, 1, MONDAY, 3))) return true; // MLK Day
  if (on(nthWeekday(y, 2, MONDAY, 3))) return true; // Washington's Birthday
  if (on(lastWeekday(y, 5, MONDAY))) return true; // Memorial Day
  if (y >= 2021 && on(usObserved({ year: y, month: 6, day: 19 }))) return true; // Juneteenth
  if (on(usObserved({ year: y, month: 7, day: 4 }))) return true; // Independence Day
  if (on(nthWeekday(y, 9, MONDAY, 1))) return true; // Labor Day
  if (on(nthWeekday(y, 11, THURSDAY, 4))) return true; // Thanksgiving
  if (on(usObserved({ year: y, month: 12, day: 25 }))) return true; // Christmas
  return false;
}

/** True when `date` is a US settlement business day (neither weekend nor holiday). */
function isBusinessDay(date: BrokenDate): boolean {
  return !isWeekend(date) && !isUsHoliday(date);
}

/** The first business day strictly after `date`. */
function nextBusinessDay(date: BrokenDate): BrokenDate {
  let d = addDays(date, 1);
  while (!isBusinessDay(d)) d = addDays(d, 1);
  return d;
}

/** The first business day strictly before `date`. */
function prevBusinessDay(date: BrokenDate): BrokenDate {
  let d = addDays(date, -1);
  while (!isBusinessDay(d)) d = addDays(d, -1);
  return d;
}

/** Roll `date` forward onto a business day (unchanged if already one). */
function rollFollowing(date: BrokenDate): BrokenDate {
  return isBusinessDay(date) ? date : nextBusinessDay(date);
}

/** Roll `date` backward onto a business day (unchanged if already one). */
function rollPreceding(date: BrokenDate): BrokenDate {
  return isBusinessDay(date) ? date : prevBusinessDay(date);
}

/**
 * Modified-following roll: forward to a business day, but stay in the calendar
 * month — if rolling forward would cross into the next month, roll backward
 * instead (mirrors `RollRule::ModifiedFollowing`).
 */
function rollModifiedFollowing(date: BrokenDate): BrokenDate {
  const fwd = rollFollowing(date);
  return fwd.month !== date.month ? rollPreceding(date) : fwd;
}

// ---------------------------------------------------------------------------
// USD-SOFR OIS schedule — annual fixed leg, ACT/360 accrual, ACT/365F pay-time
// ---------------------------------------------------------------------------

/** One accrual period of the OIS fixed leg, in curve year-fraction coordinates. */
interface FixedPeriod {
  /** Payment time (period end), ACT/365F year-fraction from the schedule start. */
  readonly pay: number;
  /** Year-fraction accrual for the period (ACT/360); strictly positive. */
  readonly accrual: number;
}

/** ACT/360 year fraction between two dates (the USD-SOFR fixed-leg accrual basis). */
function act360(start: BrokenDate, end: BrokenDate): number {
  return (dayNumber(end) - dayNumber(start)) / 360;
}

/** ACT/365F year fraction between two dates (the curve discount-time basis). */
function act365f(start: BrokenDate, end: BrokenDate): number {
  return (dayNumber(end) - dayNumber(start)) / 365;
}

/**
 * Build a spot-starting USD-SOFR OIS fixed-leg schedule of `years` annual periods
 * from the curve reference date (mirrors `celnet_rates::usd_sofr_ois_schedule`).
 *
 * `reference` is the spot (settlement) date and becomes the schedule origin
 * (curve time 0); it is normalised forward to the next US business day if it is
 * not already one. Each annual period end is `12·i` months after the start,
 * rolled modified-following on the US calendar; the fixed-leg accrual is ACT/360
 * and each payment's discount-time coordinate is ACT/365F from the start.
 */
function usdSofrOisSchedule(
  reference: BrokenDate,
  years: number,
): FixedPeriod[] {
  const start = rollFollowing(reference);
  const periods: FixedPeriod[] = [];
  let prev = start;
  for (let i = 1; i <= years; i += 1) {
    const end = rollModifiedFollowing(addMonths(start, 12 * i));
    periods.push({ pay: act365f(start, end), accrual: act360(prev, end) });
    prev = end;
  }
  return periods;
}

/**
 * Spot-starting USD-SOFR schedule whose final period ends at an explicit
 * `maturity` civil date — the generalisation for custom (broken-period) tenors and
 * odd-dated ("broken date") pillars. Annual periods roll forward in 12-month steps
 * from spot; a final stub ends at the modified-following adjusted maturity. For a
 * whole-year N this reproduces `usdSofrOisSchedule(reference, N)` exactly (mirrors
 * the server `usd_ois_schedule_to_maturity`).
 */
function usdSofrOisScheduleToMaturity(
  reference: BrokenDate,
  maturity: BrokenDate,
): FixedPeriod[] {
  const start = rollFollowing(reference);
  const end = rollModifiedFollowing(maturity);
  if (dayNumber(end) <= dayNumber(start)) {
    throw new RatesPricingError(
      "pillar maturity must be after the curve spot date",
    );
  }
  const periods: FixedPeriod[] = [];
  let prev = start;
  for (let i = 1; ; i += 1) {
    const roll = rollModifiedFollowing(addMonths(start, 12 * i));
    if (dayNumber(roll) >= dayNumber(end)) break;
    periods.push({ pay: act365f(start, roll), accrual: act360(prev, roll) });
    prev = roll;
  }
  periods.push({ pay: act365f(start, end), accrual: act360(prev, end) });
  return periods;
}

/** Spot-starting USD-SOFR schedule of a `months`-month tenor (the month arm). */
function usdSofrOisScheduleForMonths(
  reference: BrokenDate,
  months: number,
): FixedPeriod[] {
  const start = rollFollowing(reference);
  return usdSofrOisScheduleToMaturity(reference, addMonths(start, months));
}

/** Resolve a pillar's `PillarTenor` to its spot-starting USD-SOFR schedule. */
function pillarSchedule(
  tenor: PillarTenor,
  reference: BrokenDate,
): FixedPeriod[] {
  switch (tenor.kind) {
    case "years":
      if (tenor.years < 1)
        throw new RatesPricingError("pillar year tenor must be >= 1");
      return usdSofrOisSchedule(reference, tenor.years);
    case "months":
      if (tenor.months < 1)
        throw new RatesPricingError("pillar month tenor must be >= 1");
      return usdSofrOisScheduleForMonths(reference, tenor.months);
    case "date":
      return usdSofrOisScheduleToMaturity(reference, tenor.maturityDate);
  }
}

// ---------------------------------------------------------------------------
// discount curve — log-linear-on-log-DF, flat-forward extrapolation
// ---------------------------------------------------------------------------
//
// The immutable discount-factor snapshot. Pillars are stored in
// log-discount-factor space; the curve interpolates linearly in `ln DF` against
// year-fraction time (the shipping default of `celnet_rates::Curve`), which is
// arbitrage-free in DF space with piecewise-constant instantaneous forwards and
// matches QuantLib's `LogLinear` discount interpolation exactly.

interface CurveNode {
  readonly t: number;
  readonly lnDf: number;
}

/**
 * A bootstrapped discount curve: ascending `(t, ln DF)` pillars with the origin
 * `(0, ln 1 = 0)` first. Opaque to callers — build one with
 * {@link bootstrapCurveFromSet} and sample it with {@link discountFactorAt},
 * {@link zeroRateAt} and {@link instantaneousForwardAt}.
 */
export interface DiscountCurve {
  readonly nodes: readonly CurveNode[];
}

/** Build a curve from ascending `(time, discount factor)` pillars (origin prepended-aware). */
function curveFromDiscountFactors(
  pillars: ReadonlyArray<{ t: number; df: number }>,
): DiscountCurve {
  const nodes = pillars.map((p) => ({ t: p.t, lnDf: Math.log(p.df) }));
  return { nodes };
}

/**
 * `ln DF(t)` under log-linear interpolation with flat-forward extrapolation at
 * both ends: linear in `ln DF` on the segment bracketing `t`.
 */
function lnDiscount(curve: DiscountCurve, t: number): number {
  const nodes = curve.nodes;
  const n = nodes.length;
  // First node with time strictly greater than t, clamped so [hi-1, hi] is a real
  // segment (the standard flat-forward extrapolation below the first / above the last).
  let hi = 1;
  while (hi < n - 1 && nodes[hi]!.t <= t) hi += 1;
  const a = nodes[hi - 1]!;
  const b = nodes[hi]!;
  const slope = (b.lnDf - a.lnDf) / (b.t - a.t);
  return a.lnDf + slope * (t - a.t);
}

/** The discount factor `DF(t)`. For `t <= 0` this is exactly 1 (the reference date). */
function discountFactor(curve: DiscountCurve, t: number): number {
  if (t <= 0) return 1;
  return Math.exp(lnDiscount(curve, t));
}

// ---------------------------------------------------------------------------
// OIS analytics — annuity, par rate, present value
// ---------------------------------------------------------------------------
//
// For a collateralised OIS whose discount and projection curve are the same (the
// USD-SOFR self-discounting case), the compounded-overnight float leg telescopes
// to the discount-factor identity `DF(0) − DF(T)` per unit notional. The fixed
// leg is the annuity `Σ δ_i · DF(pay_i)`; the par (fair fixed) rate is
// `(DF(0) − DF(T)) / annuity`. Each schedule here starts at the curve origin, so
// `DF(0) = 1`.

/** The fixed-leg annuity `A = Σ δ_i · DF(pay_i)` (per unit notional). */
function oisAnnuity(
  curve: DiscountCurve,
  schedule: readonly FixedPeriod[],
): number {
  let annuity = 0;
  for (const p of schedule) annuity += p.accrual * discountFactor(curve, p.pay);
  return annuity;
}

/** The swap maturity — the final period's payment time. */
function maturity(schedule: readonly FixedPeriod[]): number {
  return schedule[schedule.length - 1]!.pay;
}

/** The par (fair fixed) rate `K* = (1 − DF(T)) / annuity`. */
function oisParRate(
  curve: DiscountCurve,
  schedule: readonly FixedPeriod[],
): number {
  const dfT = discountFactor(curve, maturity(schedule));
  return (1 - dfT) / oisAnnuity(curve, schedule);
}

/**
 * Present value of RECEIVING fixed at `fixedRate` on `notional`:
 * `notional · (K·A − (1 − DF(T)))`. Positive when the fixed rate exceeds par;
 * pay-fixed is the negation. The caller applies the direction sign.
 */
function oisReceiveFixedPv(
  curve: DiscountCurve,
  schedule: readonly FixedPeriod[],
  fixedRate: number,
  notional: number,
): number {
  const annuity = oisAnnuity(curve, schedule);
  const dfT = discountFactor(curve, maturity(schedule));
  return notional * (fixedRate * annuity - (1 - dfT));
}

// ---------------------------------------------------------------------------
// sequential bootstrap — one unknown DF per pillar via a monotone root-find
// ---------------------------------------------------------------------------

/** A calibrating OIS quote: its fixed-leg schedule and the quoted par rate. */
interface OisQuote {
  readonly schedule: readonly FixedPeriod[];
  readonly parRate: number;
}

/** Lower / upper continuously-compounded zero-rate brackets for a pillar solve. */
const Z_LO = -0.5;
const Z_HI = 1.0;
/** Abscissa tolerance and iteration cap for the per-pillar root-find. */
const SOLVE_TOL = 1e-13;
const SOLVE_MAX_ITER = 200;

/**
 * Brent's method root-find of a continuous `f` bracketed by `[lo, hi]`
 * (combined bisection / secant / inverse-quadratic interpolation). The pillar
 * residual below is monotone in the zero rate, so the root is unique; Brent gives
 * the same root to `SOLVE_TOL` as the server's `celnet_rates::solver::brent_root`.
 */
function brentRoot(f: (x: number) => number, lo: number, hi: number): number {
  let a = lo;
  let b = hi;
  let fa = f(a);
  let fb = f(b);
  if (fa === 0) return a;
  if (fb === 0) return b;
  if (fa * fb > 0) {
    throw new RatesPricingError(
      "pillar root-solve failed: the par-rate bracket does not contain a root",
    );
  }
  if (Math.abs(fa) < Math.abs(fb)) {
    [a, b] = [b, a];
    [fa, fb] = [fb, fa];
  }
  let c = a;
  let fc = fa;
  let mflag = true;
  let d = c;
  for (let iter = 0; iter < SOLVE_MAX_ITER; iter += 1) {
    if (Math.abs(b - a) <= SOLVE_TOL) return b;
    let s: number;
    if (fa !== fc && fb !== fc) {
      // Inverse-quadratic interpolation.
      s =
        (a * fb * fc) / ((fa - fb) * (fa - fc)) +
        (b * fa * fc) / ((fb - fa) * (fb - fc)) +
        (c * fa * fb) / ((fc - fa) * (fc - fb));
    } else {
      // Secant step.
      s = b - fb * ((b - a) / (fb - fa));
    }
    const lower = (3 * a + b) / 4;
    const between = (s - lower) * (s - b) < 0;
    const delta = Math.abs(SOLVE_TOL);
    if (
      !between ||
      (mflag && Math.abs(s - b) >= Math.abs(b - c) / 2) ||
      (!mflag && Math.abs(s - b) >= Math.abs(c - d) / 2) ||
      (mflag && Math.abs(b - c) < delta) ||
      (!mflag && Math.abs(c - d) < delta)
    ) {
      s = (a + b) / 2; // Bisection fallback.
      mflag = true;
    } else {
      mflag = false;
    }
    const fs = f(s);
    d = c;
    c = b;
    fc = fb;
    if (fa * fs < 0) {
      b = s;
      fb = fs;
    } else {
      a = s;
      fa = fs;
    }
    if (Math.abs(fa) < Math.abs(fb)) {
      [a, b] = [b, a];
      [fa, fb] = [fb, fa];
    }
    if (fs === 0) return s;
  }
  return b;
}

/**
 * Bootstrap a self-discounting curve from spot-starting OIS quotes ordered by
 * increasing maturity. Each quote adds one pillar at its maturity, solved so the
 * OIS reprices to its quoted par rate; intermediate payments are interpolated
 * from the curve built so far (log-linear-DF). Mirrors `bootstrap_ois`.
 */
function bootstrapOis(quotes: readonly OisQuote[]): DiscountCurve {
  const pillars: { t: number; df: number }[] = [{ t: 0, df: 1 }];
  let prevMaturity = 0;
  for (const quote of quotes) {
    const t = maturity(quote.schedule);
    if (t <= prevMaturity) {
      throw new RatesPricingError(
        "quote maturities must be strictly increasing",
      );
    }
    const target = quote.parRate;
    const residual = (z: number): number => {
      const candidate = [...pillars, { t, df: Math.exp(-z * t) }];
      return (
        oisParRate(curveFromDiscountFactors(candidate), quote.schedule) - target
      );
    };
    const z = brentRoot(residual, Z_LO, Z_HI);
    pillars.push({ t, df: Math.exp(-z * t) });
    prevMaturity = t;
  }
  return curveFromDiscountFactors(pillars);
}

// ---------------------------------------------------------------------------
// curve risk — PV01 (analytic), DV01 (parallel bump), key-rate ladder
// ---------------------------------------------------------------------------

/** The receive-fixed risk report of an OIS, before the direction sign is applied. */
interface OisRisk {
  readonly pv: number;
  readonly pv01: number;
  readonly dv01: number;
  readonly keyRate: number[];
}

/** Bump a single quote's par rate by `delta` (immutably). */
function bumpQuote(quote: OisQuote, delta: number): OisQuote {
  return { schedule: quote.schedule, parRate: quote.parRate + delta };
}

/**
 * Compute the receive-fixed PV, PV01, DV01 and key-rate ladder for an OIS priced
 * off a curve bootstrapped from `quotes` (mirrors `celnet_rates::ois_risk`).
 *
 * - PV01 is analytic (`N·annuity·1bp`; the swap PV is linear in the fixed rate).
 * - DV01 re-bootstraps under a +1bp parallel bump of every calibrating quote.
 * - The key-rate ladder bumps each calibrating quote alone; the resulting vector
 *   sums (to first order) to the parallel DV01, the residual being curve
 *   cross-gamma between pillars.
 */
function oisRisk(
  quotes: readonly OisQuote[],
  schedule: readonly FixedPeriod[],
  fixedRate: number,
  notional: number,
): OisRisk {
  const base = bootstrapOis(quotes);
  const pv = oisReceiveFixedPv(base, schedule, fixedRate, notional);
  const pv01 = notional * oisAnnuity(base, schedule) * ONE_BP;

  // Parallel DV01: every calibrating quote bumped +1bp.
  const parallel = quotes.map((q) => bumpQuote(q, ONE_BP));
  const bumpedCurve = bootstrapOis(parallel);
  const dv01 =
    oisReceiveFixedPv(bumpedCurve, schedule, fixedRate, notional) - pv;

  // Key-rate ladder: each calibrating quote bumped +1bp in isolation.
  const keyRate: number[] = [];
  for (let target = 0; target < quotes.length; target += 1) {
    const single = quotes.map((q, j) =>
      j === target ? bumpQuote(q, ONE_BP) : q,
    );
    const curve = bootstrapOis(single);
    keyRate.push(oisReceiveFixedPv(curve, schedule, fixedRate, notional) - pv);
  }

  return { pv, pv01, dv01, keyRate };
}

// ---------------------------------------------------------------------------
// the public offline pricer + the default USD-SOFR market
// ---------------------------------------------------------------------------

/** A typed failure of the offline rates pricer (mirrors the server's `RatesPriceError`). */
export class RatesPricingError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RatesPricingError";
  }
}

/**
 * The default USD-SOFR par-OIS pillar ladder + reference date — the SAME P0
 * static curve the server's `default_usd_sofr_curve_set()` calibrates against
 * (pending a live SOFR feed). A real calibrating market, not a stub: it
 * bootstraps a real self-discounting curve and produces real par rates and risk,
 * so an offline price agrees with the live edge that uses the same curve.
 */
export const DEFAULT_USD_SOFR_CURVE: RatesCurveSet = {
  currency: SUPPORTED_CURRENCY,
  referenceDate: { year: 2026, month: 6, day: 25 },
  pillars: [
    { tenor: { kind: "years", years: 1 }, parRate: 0.0432 },
    { tenor: { kind: "years", years: 2 }, parRate: 0.0418 },
    { tenor: { kind: "years", years: 3 }, parRate: 0.0409 },
    { tenor: { kind: "years", years: 5 }, parRate: 0.0405 },
    { tenor: { kind: "years", years: 7 }, parRate: 0.0408 },
    { tenor: { kind: "years", years: 10 }, parRate: 0.0415 },
    { tenor: { kind: "years", years: 15 }, parRate: 0.0421 },
    { tenor: { kind: "years", years: 20 }, parRate: 0.0424 },
    { tenor: { kind: "years", years: 30 }, parRate: 0.0423 },
  ],
};

/** Rebuild the calibrating OIS quotes from a curve set, validating it fully. */
function buildQuotes(curve: RatesCurveSet): OisQuote[] {
  if (curve.currency.toUpperCase() !== SUPPORTED_CURRENCY) {
    throw new RatesPricingError(
      `unsupported currency \`${curve.currency}\` (only USD is priced in the rates arm)`,
    );
  }
  if (curve.pillars.length === 0) {
    throw new RatesPricingError("curve set carries no OIS pillars");
  }
  const quotes: OisQuote[] = [];
  // Order by final ACT/365F pay-time from spot — strictly increasing iff the
  // resolved maturities are, regardless of which arm located each pillar.
  let prevPay = 0;
  for (const pillar of curve.pillars) {
    const schedule = pillarSchedule(pillar.tenor, curve.referenceDate);
    const lastPay = schedule[schedule.length - 1]!.pay;
    if (lastPay <= prevPay) {
      throw new RatesPricingError(
        "curve pillar maturities must be strictly increasing",
      );
    }
    prevPay = lastPay;
    quotes.push({ schedule, parRate: pillar.parRate });
  }
  return quotes;
}

/** A pillar's maturity in year-fraction (ACT/365F) from the curve spot date. */
export function pillarMaturityYears(
  tenor: PillarTenor,
  reference: BrokenDate,
): number {
  const schedule = pillarSchedule(tenor, reference);
  return schedule[schedule.length - 1]!.pay;
}

/** The receive-fixed sign for an OIS direction: receive = +1, pay = −1. */
function directionSign(instrument: OisInstrument): number {
  return instrument.direction === "RECEIVE_FIXED" ? 1 : -1;
}

/**
 * Price a single OIS against a curve set, returning the direction-signed PV +
 * first-order risk (PV / PV01 / DV01 / key-rate ladder). The par rate is
 * direction-independent. Validates the curve and instrument exactly as the
 * server's `price_rates` does, so an offline rejection matches a live one.
 *
 * @throws {RatesPricingError} on a malformed curve / instrument or a numeric
 * bootstrap failure.
 */
export function priceRatesOffline(
  curve: RatesCurveSet,
  instrument: OisInstrument,
): RatesPricingResult {
  const quotes = buildQuotes(curve);
  if (instrument.tenorYears < 1)
    throw new RatesPricingError("tenor_years must be >= 1");
  if (instrument.notional <= 0)
    throw new RatesPricingError("notional must be > 0");

  const schedule = usdSofrOisSchedule(
    curve.referenceDate,
    instrument.tenorYears,
  );
  const risk = oisRisk(
    quotes,
    schedule,
    instrument.fixedRate,
    instrument.notional,
  );
  const par = oisParRate(bootstrapOis(quotes), schedule);
  const sign = directionSign(instrument);

  return {
    pv: sign * risk.pv,
    parRate: par,
    pv01: sign * risk.pv01,
    dv01: sign * risk.dv01,
    keyRateLadder: risk.keyRate.map((k) => sign * k),
  };
}

// ---------------------------------------------------------------------------
// curve inspection — public sampling over the SAME bootstrapped discount curve
// ---------------------------------------------------------------------------
//
// The Curve workspace (FI-ARCHITECTURE §4.2) inspects the bootstrapped curve
// directly: the discount factor `DF(t)`, the continuously-compounded zero rate
// `z(t) = −ln DF(t)/t`, and the instantaneous forward `f(t) = −d ln DF/dt`. These
// are thin PUBLIC views over the EXISTING private curve math (`bootstrapOis`,
// `lnDiscount`, `discountFactor`) — no re-implementation, so an inspected curve
// is the very curve the offline pricer and the live edge price against.

/** Central-difference step (years) for the instantaneous-forward derivative. */
const FORWARD_DT = 1e-4;

/** Default number of sample points {@link sampleCurve} lays across the span. */
const DEFAULT_CURVE_SAMPLES = 96;

/**
 * Bootstrap the self-discounting discount curve implied by a curve set, validating
 * it through the SAME `buildQuotes` path the offline pricer uses (currency,
 * non-empty, strictly-increasing tenors). Reuses `bootstrapOis` verbatim.
 *
 * @throws {RatesPricingError} on a malformed curve set or a numeric bootstrap
 * failure.
 */
export function bootstrapCurveFromSet(curve: RatesCurveSet): DiscountCurve {
  return bootstrapOis(buildQuotes(curve));
}

/** The discount factor `DF(t)`; exactly `1` at and before the reference date. */
export function discountFactorAt(curve: DiscountCurve, t: number): number {
  return discountFactor(curve, t);
}

/**
 * The continuously-compounded zero rate `z(t) = −ln DF(t)/t`. The `t → 0` limit is
 * the instantaneous short rate `f(0⁺)` (l'Hôpital on the `0/0` form), so the origin
 * is continuous rather than a singularity. Uses `ln DF` directly (no `exp`/`log`
 * round-trip), so it is exactly consistent with {@link discountFactorAt}.
 */
export function zeroRateAt(curve: DiscountCurve, t: number): number {
  if (t <= 0) return instantaneousForwardAt(curve, 0);
  return -lnDiscount(curve, t) / t;
}

/**
 * The instantaneous forward `f(t) = −d ln DF/dt`, by a symmetric central difference
 * on `ln DF`. Because the curve interpolates linearly in `ln DF` (the shipping
 * log-linear-on-log-DF scheme), `ln DF` is piecewise-linear, so the central
 * difference recovers the engine's piecewise-FLAT instantaneous forward exactly in
 * the interior of a segment, and averages the two adjacent segment slopes at a
 * pillar (where the flat forward steps). `lnDiscount` flat-forward-extrapolates
 * below `t = 0`, so `f(0)` is the first segment's forward.
 */
export function instantaneousForwardAt(
  curve: DiscountCurve,
  t: number,
): number {
  const lo = lnDiscount(curve, t - FORWARD_DT);
  const hi = lnDiscount(curve, t + FORWARD_DT);
  return -(hi - lo) / (2 * FORWARD_DT);
}

/** Options for {@link sampleCurve}: the sample count and the upper time bound. */
export interface CurveSampleOptions {
  /** Number of points laid across the span (clamped to `>= 2`); default 96. */
  readonly samples?: number;
  /** Upper time bound in years; default the last pillar tenor (the curve span). */
  readonly maxTenor?: number;
}

/** One sampled point of the bootstrapped curve in the three standard views. */
export interface CurveSamplePoint {
  /** Year-fraction time from the reference date. */
  readonly t: number;
  /** Discount factor `DF(t)`. */
  readonly df: number;
  /** Continuously-compounded zero rate `z(t)`. */
  readonly zero: number;
  /** Instantaneous forward `f(t)`. */
  readonly forward: number;
}

/**
 * Sample the bootstrapped curve at `N` evenly-spaced times across `0 .. maxTenor`
 * (the curve span by default), returning `DF`, the zero rate and the instantaneous
 * forward at each. Bootstraps once, then reads the three public views — allocation
 * is a single `N`-length array.
 *
 * @throws {RatesPricingError} on a malformed curve set (via {@link bootstrapCurveFromSet}).
 */
export function sampleCurve(
  curve: RatesCurveSet,
  opts: CurveSampleOptions = {},
): CurveSamplePoint[] {
  const discount = bootstrapCurveFromSet(curve);
  const lastPillar = curve.pillars[curve.pillars.length - 1]!;
  const span =
    opts.maxTenor ?? pillarMaturityYears(lastPillar.tenor, curve.referenceDate);
  const n = Math.max(2, Math.trunc(opts.samples ?? DEFAULT_CURVE_SAMPLES));
  const out: CurveSamplePoint[] = new Array(n);
  for (let i = 0; i < n; i += 1) {
    const t = (span * i) / (n - 1);
    out[i] = {
      t,
      df: discountFactorAt(discount, t),
      zero: zeroRateAt(discount, t),
      forward: instantaneousForwardAt(discount, t),
    };
  }
  return out;
}
