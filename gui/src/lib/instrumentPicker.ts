/**
 * instrumentPicker — turn the reference-data registry into the options a trader PICKS
 * a hedge instrument from, instead of typing a contract code and a DV01 by hand.
 *
 * # Why this exists
 *
 * The hedge-vehicle registry used to make a trader type `ZFU26` and its DV01 per
 * contract from memory. Both facts are already on the wire: the server seeds the listed
 * Treasury futures complex into reference data, and each seeded contract carries its own
 * derived DV01 per contract. A typed contract code can be wrong in three ways a picked
 * one cannot — a contract that does not exist, one that has stopped trading, and one no
 * LP quotes — and a typed DV01 is simply unverifiable.
 *
 * # The two kinds of futures option
 *
 * A futures hedge can be configured two ways, and the difference matters four times a
 * year:
 *
 *  - a **delivery month** (`ZFU26`) — a specific market, which stops trading on a
 *    specific day. Pinned deliberately; never re-pointed.
 *  - a **product** (`ZF`) — the standing intent "hedge in the 5-Year contract", which
 *    the server resolves to whichever contract is currently the front one. This is the
 *    option that keeps working after the quarterly roll.
 *
 * {@link hedgeVehicleOptions} offers both, product first, because a policy pinned to a
 * delivery month is the thing that silently breaks.
 *
 * # Liquidity
 *
 * An instrument that exists is not the same as an instrument that can FILL. When the
 * caller can say which instruments currently have a live composite, options carry
 * {@link InstrumentOption.hasLiveLiquidity} and sort ahead of the rest — the difference
 * between a hedge that executes and one that backstops.
 *
 * React-free and directly testable, in the shape of {@link ./aggBookSelection}.
 */

import type { BrokenDate, InstrumentDef } from "../data/contract";

/** One selectable instrument in a picker. */
export interface InstrumentOption {
  /**
   * The value stored on the configuration — a canonical `instrumentId`, or a futures
   * PRODUCT symbol for a rolling vehicle (see {@link isRollingProduct}).
   */
  value: string;
  /** The primary display label. */
  label: string;
  /** A secondary line (terms / issuer / expiry), shown under the label. */
  sublabel: string;
  /** The group heading this option is bucketed under. */
  group: string;
  /** Pre-lowercased haystack for substring search. */
  searchText: string;
  /** ISO 4217 currency, when the definition carries one. */
  currency: string;
  /** Whether this vehicle trades in whole lots (a listed future). */
  isFuture: boolean;
  /**
   * Whether this option names a PRODUCT that resolves to its front month at use time,
   * rather than a fixed delivery month. A rolling vehicle survives the quarterly roll.
   */
  isRollingProduct: boolean;
  /**
   * The DV01 of one unit, when it is DERIVABLE from the definition's own terms — the
   * value that pre-fills the registry's `dv01PerUnit`. `null` when the definition does
   * not carry one, in which case the trader must supply it and nothing is guessed.
   */
  dv01PerUnit: number | null;
  /** The unit the DV01 is per (`"contract"` / `"1mm face"`). */
  unitLabel: string;
  /** Whether a live composite currently covers this instrument (when known). */
  hasLiveLiquidity: boolean;
}

/** A group of options under a shared heading, in display order. */
export interface InstrumentOptionGroup {
  group: string;
  options: InstrumentOption[];
}

/** Options controlling which instruments a picker offers. */
export interface HedgeVehicleOptionsInput {
  /** The full reference-data registry (`transport.listInstruments()`). */
  defs: readonly InstrumentDef[];
  /**
   * The instrument ids (and ISIN/CUSIP aliases) a live composite currently covers.
   * Omit when unknown — options then carry `hasLiveLiquidity: false` and are NOT
   * demoted, since "unknown" must never read as "illiquid".
   */
  liquidIds?: ReadonlySet<string> | undefined;
}

/** The unit label a vehicle's size is quoted in. */
export function unitLabelFor(isFuture: boolean): string {
  return isFuture ? "contract" : "1mm face";
}

/** `YYYY-MM-DD` for a broken date, for display and search. */
function ymd(d: BrokenDate): string {
  const mm = String(d.month).padStart(2, "0");
  const dd = String(d.day).padStart(2, "0");
  return `${d.year}-${mm}-${dd}`;
}

/** Order two broken dates chronologically. */
function compareDates(a: BrokenDate, b: BrokenDate): number {
  return a.year - b.year || a.month - b.month || a.day - b.day;
}

/** The external identifier value for a scheme on a definition, or "". */
function externalId(def: InstrumentDef, scheme: string): string {
  return def.externalIds.find((e) => e.scheme === scheme)?.value ?? "";
}

/** Every identifier an instrument may be keyed by on a live composite line. */
function identifiersOf(def: InstrumentDef): string[] {
  return [def.instrumentId, ...def.externalIds.map((e) => e.value)].filter(
    (s) => s.length > 0,
  );
}

/** Whether any of the instrument's identifiers appears in the liquid set. */
function isLiquid(def: InstrumentDef, liquidIds?: ReadonlySet<string>): boolean {
  if (!liquidIds) return false;
  return identifiersOf(def).some((id) => liquidIds.has(id));
}

/** The bond-family options (cash securities), grouped by issuer. */
function bondOptions(
  defs: readonly InstrumentDef[],
  liquidIds?: ReadonlySet<string>,
): InstrumentOption[] {
  const out: InstrumentOption[] = [];
  for (const def of defs) {
    if (def.family !== "bond" || !def.instrumentId) continue;
    const issuer = def.bond.issuer || "Other";
    const isin = externalId(def, "isin");
    const cusip = externalId(def, "cusip");
    const coupon =
      def.bond.couponType === "fixed"
        ? `${Number.parseFloat(def.bond.couponRate.toFixed(3))}%`
        : def.bond.couponType === "zero"
          ? "Zero"
          : "FRN";
    out.push({
      value: def.instrumentId,
      label: def.name || def.instrumentId,
      sublabel: `${coupon} · matures ${ymd(def.bond.maturityDate)}`,
      group: `Cash bond · ${issuer}`,
      searchText: [def.name, issuer, isin, cusip, def.currency, def.instrumentId]
        .join(" ")
        .toLowerCase(),
      currency: def.currency,
      isFuture: false,
      isRollingProduct: false,
      // A cash bond's DV01 is a function of its own cashflow schedule AND the live
      // curve — it is not a static term, so it is never pre-filled from reference
      // data. The trader supplies it, and `Dv01Basis` records what it came from.
      dv01PerUnit: null,
      unitLabel: unitLabelFor(false),
      hasLiveLiquidity: isLiquid(def, liquidIds),
    });
  }
  return out;
}

/** The seeded listed Treasury futures, as specific delivery months. */
function futureContractOptions(
  defs: readonly InstrumentDef[],
  liquidIds?: ReadonlySet<string>,
): InstrumentOption[] {
  const out: InstrumentOption[] = [];
  for (const def of defs) {
    if (def.family !== "bond_future" || !def.instrumentId) continue;
    const f = def.bondFuture;
    const dv01 = f.dv01PerContractAtNotionalYield;
    out.push({
      value: def.instrumentId,
      label: def.instrumentId,
      sublabel: `${def.name} · last trades ${ymd(f.lastTradingDate)}`,
      group: "Treasury future · delivery month",
      searchText: [def.instrumentId, def.name, f.contractSymbol, def.currency]
        .join(" ")
        .toLowerCase(),
      currency: def.currency,
      isFuture: true,
      isRollingProduct: false,
      dv01PerUnit: Number.isFinite(dv01) && dv01 > 0 ? dv01 : null,
      unitLabel: unitLabelFor(true),
      hasLiveLiquidity: isLiquid(def, liquidIds),
    });
  }
  return out.sort((a, b) => a.value.localeCompare(b.value));
}

/**
 * The rolling PRODUCT options, derived from the seeded delivery months: one per distinct
 * contract symbol, carrying the FRONT contract's terms as its defaults.
 *
 * "Front" here is the nearest delivery month among the seeded contracts, which is the
 * same ordering the server resolves the roll by — so the DV01 this pre-fills is the one
 * the vehicle will actually be sized against today.
 */
function futureProductOptions(
  defs: readonly InstrumentDef[],
  liquidIds?: ReadonlySet<string>,
): InstrumentOption[] {
  const bySymbol = new Map<string, InstrumentDef[]>();
  for (const def of defs) {
    if (def.family !== "bond_future" || !def.bondFuture.contractSymbol) continue;
    const bucket = bySymbol.get(def.bondFuture.contractSymbol);
    if (bucket) bucket.push(def);
    else bySymbol.set(def.bondFuture.contractSymbol, [def]);
  }
  const out: InstrumentOption[] = [];
  for (const [symbol, contracts] of bySymbol) {
    const front = [...contracts].sort((a, b) =>
      a.family === "bond_future" && b.family === "bond_future"
        ? compareDates(
            a.bondFuture.deliveryMonthStart,
            b.bondFuture.deliveryMonthStart,
          )
        : 0,
    )[0];
    if (!front || front.family !== "bond_future") continue;
    const dv01 = front.bondFuture.dv01PerContractAtNotionalYield;
    // The product's display stem is the contract name minus its month-year suffix.
    const stem = front.name.replace(/\s+\S+\s+\d{4}$/, "") || symbol;
    out.push({
      value: symbol,
      label: `${symbol} — front month`,
      sublabel: `${stem} · auto-rolls (currently ${front.instrumentId})`,
      group: "Treasury future · rolling product",
      searchText: [symbol, stem, front.instrumentId, "front", "roll"]
        .join(" ")
        .toLowerCase(),
      currency: front.currency,
      isFuture: true,
      isRollingProduct: true,
      dv01PerUnit: Number.isFinite(dv01) && dv01 > 0 ? dv01 : null,
      unitLabel: unitLabelFor(true),
      hasLiveLiquidity: isLiquid(front, liquidIds),
    });
  }
  return out.sort((a, b) => a.value.localeCompare(b.value));
}

/**
 * Every instrument a hedge vehicle may be configured as, in the order a trader should
 * see them: rolling futures products first (the option that survives the roll), then
 * specific delivery months, then cash bonds.
 */
export function hedgeVehicleOptions({
  defs,
  liquidIds,
}: HedgeVehicleOptionsInput): InstrumentOption[] {
  return [
    ...futureProductOptions(defs, liquidIds),
    ...futureContractOptions(defs, liquidIds),
    ...bondOptions(defs, liquidIds),
  ];
}

/**
 * Group options under their headings, preserving the group order the options arrived in
 * (so "rolling product" stays above "delivery month") and, within a group, sorting
 * live-liquidity options first — an instrument that can actually fill outranks one that
 * merely exists — then by label.
 */
export function groupInstrumentOptions(
  options: readonly InstrumentOption[],
): InstrumentOptionGroup[] {
  const byGroup = new Map<string, InstrumentOption[]>();
  for (const opt of options) {
    const bucket = byGroup.get(opt.group);
    if (bucket) bucket.push(opt);
    else byGroup.set(opt.group, [opt]);
  }
  return [...byGroup.entries()].map(([group, opts]) => ({
    group,
    options: [...opts].sort(
      (a, b) =>
        Number(b.hasLiveLiquidity) - Number(a.hasLiveLiquidity) ||
        a.label.localeCompare(b.label),
    ),
  }));
}

/** Filter grouped options by a query (substring over `searchText`). */
export function filterInstrumentGroups(
  groups: readonly InstrumentOptionGroup[],
  query: string,
): InstrumentOptionGroup[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...groups];
  return groups
    .map((g) => ({
      group: g.group,
      options: g.options.filter((o) => o.searchText.includes(q)),
    }))
    .filter((g) => g.options.length > 0);
}

/** The option whose `value` is `value`, or `undefined`. */
export function findInstrumentOption(
  options: readonly InstrumentOption[],
  value: string,
): InstrumentOption | undefined {
  return options.find((o) => o.value === value);
}

/**
 * The live-composite identifier set from the aggregated book's instruments — every id an
 * option may be keyed by, so a picker can mark what can actually fill.
 */
export function liquidIdentifierSet(
  instruments: readonly { instrumentId: string; isin: string; cusip: string }[],
): Set<string> {
  const out = new Set<string>();
  for (const i of instruments) {
    if (i.instrumentId) out.add(i.instrumentId);
    if (i.isin) out.add(i.isin);
    if (i.cusip) out.add(i.cusip);
  }
  return out;
}
