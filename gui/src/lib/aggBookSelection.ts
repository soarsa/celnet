/**
 * aggBookSelection — the pure logic behind the Aggregated Book's per-user
 * "view only what I want" security selection.
 *
 * Two concerns, both React-free and directly testable:
 *
 *   1. The PICKER options — the FI reference-data universe (bond definitions
 *      from `transport.listInstruments()`) turned into grouped, searchable,
 *      stably-ordered options keyed by the canonical `instrumentId`. The picker
 *      lists the FULL refdata universe, not merely what is currently streaming.
 *
 *   2. The FILTER — given the trader's chosen `instrumentId`s, decide which live
 *      composite lines to render. A composite line may be keyed by the canonical
 *      id OR by a market identifier (ISIN/CUSIP), so a chosen instrument expands
 *      to ALL of its identifiers and a line matches on any of them.
 *
 * The safety invariant (a cleared selection is never an accidentally-blank book):
 * an EMPTY selection means "show all" — {@link filterInstrumentsBySelection}
 * returns every line untouched.
 */

import type { AggregatedInstrument, InstrumentDef } from "../data/contract";

/** One selectable security in the picker, derived from a bond definition. */
export interface SecurityOption {
  /** The canonical reference-data `instrumentId` (the value stored in the pref). */
  instrumentId: string;
  /** The primary display label (the instrument name). */
  label: string;
  /** A secondary line (issuer · coupon), shown under the label. */
  sublabel: string;
  /** The group heading this option is bucketed under (the issuer). */
  group: string;
  /** ISIN, when the bond carries one (shown as a muted identifier). */
  isin: string;
  /** CUSIP, when the bond carries one. */
  cusip: string;
  /** Pre-lowercased haystack for substring search (name · issuer · isin · cusip · ccy). */
  searchText: string;
}

/** A group of options under a shared heading (issuer), in display order. */
export interface SecurityGroup {
  /** The group heading (the bond issuer, e.g. "US Treasury"). */
  group: string;
  /** The options in this group, in display order. */
  options: SecurityOption[];
}

/** The external identifier value for a scheme on a definition, or "". */
function externalId(def: InstrumentDef, scheme: "isin" | "cusip"): string {
  return def.externalIds.find((e) => e.scheme === scheme)?.value ?? "";
}

/** A trimmed coupon descriptor for the option sublabel (e.g. "4.25%" / "Zero"). */
function couponBrief(def: Extract<InstrumentDef, { family: "bond" }>): string {
  switch (def.bond.couponType) {
    case "zero":
      return "Zero";
    case "frn":
      return "FRN";
    case "fixed":
      return `${Number.parseFloat(def.bond.couponRate.toFixed(3))}%`;
  }
}

/**
 * Build the picker options from the instrument registry: BOND-family definitions
 * only (they carry the issuer/coupon terms the picker shows and are the securities
 * an aggregated book prices), sorted by instrument name within each issuer group.
 * A definition with a blank `instrumentId` is skipped — it cannot be selected.
 */
export function securityOptions(defs: readonly InstrumentDef[]): SecurityOption[] {
  const out: SecurityOption[] = [];
  for (const def of defs) {
    if (def.family !== "bond") continue;
    if (!def.instrumentId) continue;
    const issuer = def.bond.issuer || "Other";
    const isin = externalId(def, "isin");
    const cusip = externalId(def, "cusip");
    const coupon = couponBrief(def);
    out.push({
      instrumentId: def.instrumentId,
      label: def.name || def.instrumentId,
      sublabel: [issuer, coupon].filter(Boolean).join(" · "),
      group: issuer,
      isin,
      cusip,
      searchText: [def.name, issuer, isin, cusip, def.currency, def.instrumentId]
        .join(" ")
        .toLowerCase(),
    });
  }
  return out;
}

/**
 * Group the picker options by issuer, groups alphabetically and options by label
 * within a group, so the panel renders a stable, scannable hierarchy.
 */
export function groupSecurityOptions(
  options: readonly SecurityOption[],
): SecurityGroup[] {
  const byGroup = new Map<string, SecurityOption[]>();
  for (const opt of options) {
    const bucket = byGroup.get(opt.group);
    if (bucket) bucket.push(opt);
    else byGroup.set(opt.group, [opt]);
  }
  return [...byGroup.entries()]
    .map(([group, opts]) => ({
      group,
      options: [...opts].sort((a, b) => a.label.localeCompare(b.label)),
    }))
    .sort((a, b) => a.group.localeCompare(b.group));
}

/** Filter grouped options by a lowercased query (substring over `searchText`). */
export function filterSecurityGroups(
  groups: readonly SecurityGroup[],
  query: string,
): SecurityGroup[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...groups];
  return groups
    .map((g) => ({
      group: g.group,
      options: g.options.filter((o) => o.searchText.includes(q)),
    }))
    .filter((g) => g.options.length > 0);
}

/**
 * Expand a chosen set of canonical `instrumentId`s into the full set of matchable
 * identifiers — the id itself PLUS the ISIN/CUSIP the definition carries — so a
 * composite line resolves whether the server keyed it by internal id or by a
 * market identifier. Ids with no matching definition still contribute themselves.
 */
export function selectedIdentifierSet(
  defs: readonly InstrumentDef[],
  selectedIds: readonly string[],
): Set<string> {
  const chosen = new Set(selectedIds);
  const identifiers = new Set<string>(chosen);
  for (const def of defs) {
    if (!chosen.has(def.instrumentId)) continue;
    for (const ext of def.externalIds) {
      if ((ext.scheme === "isin" || ext.scheme === "cusip") && ext.value) {
        identifiers.add(ext.value);
      }
    }
  }
  return identifiers;
}

/** Does a composite line match the expanded identifier set (id / ISIN / CUSIP)? */
export function instrumentMatchesSelection(
  inst: Pick<AggregatedInstrument, "instrumentId" | "isin" | "cusip">,
  identifiers: ReadonlySet<string>,
): boolean {
  return (
    identifiers.has(inst.instrumentId) ||
    (inst.isin.length > 0 && identifiers.has(inst.isin)) ||
    (inst.cusip.length > 0 && identifiers.has(inst.cusip))
  );
}

/**
 * Apply the trader's selection to the live composite lines. An EMPTY selection
 * returns every line unchanged (show all — the never-blank-by-accident rule).
 * Otherwise only the lines whose id/ISIN/CUSIP is in the chosen set survive, in
 * their original order.
 */
export function filterInstrumentsBySelection(
  instruments: readonly AggregatedInstrument[],
  defs: readonly InstrumentDef[],
  selectedIds: readonly string[],
): AggregatedInstrument[] {
  if (selectedIds.length === 0) return [...instruments];
  const identifiers = selectedIdentifierSet(defs, selectedIds);
  return instruments.filter((inst) => instrumentMatchesSelection(inst, identifiers));
}
