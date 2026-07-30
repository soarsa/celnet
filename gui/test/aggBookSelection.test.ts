/**
 * aggBookSelection — the pure logic behind the Aggregated Book "view only what I
 * want" security selection: the picker options (grouped/searchable) and the
 * display filter (empty ⇒ show all; id/ISIN/CUSIP matching).
 */

import { describe, expect, it } from "vitest";

import type { AggregatedInstrument, InstrumentDef } from "../src/data/contract";
import {
  filterInstrumentsBySelection,
  filterSecurityGroups,
  groupSecurityOptions,
  instrumentMatchesSelection,
  securityOptions,
  selectedIdentifierSet,
} from "../src/lib/aggBookSelection";

/** A bond instrument definition fixture keyed by its CUSIP, carrying ISIN+CUSIP. */
function bondDef(
  id: string,
  name: string,
  issuer: string,
  isin: string,
  cusip: string,
): InstrumentDef {
  return {
    instrumentId: id,
    name,
    description: "",
    currency: "USD",
    externalIds: [
      { scheme: "isin", value: isin },
      { scheme: "cusip", value: cusip },
    ],
    family: "bond",
    bond: {
      issuer,
      couponRate: 4.25,
      couponType: "fixed",
      couponFrequency: "semi_annual",
      dayCount: "act_act",
      maturityDate: { year: 2028, month: 6, day: 30 },
      redemption: 100,
      calendars: ["united_states"],
    },
  };
}

/** A non-bond (money-market deposit) definition — excluded from picker options. */
const DEPOSIT_DEF: InstrumentDef = {
  instrumentId: "usd-sofr-on",
  name: "USD SOFR O/N deposit",
  description: "",
  currency: "USD",
  externalIds: [],
  family: "deposit",
  deposit: {
    index: "SOFR",
    tenor: "1D",
    dayCount: "act_360",
    businessDayConvention: "following",
    calendars: ["united_states"],
    spotLagDays: 0,
  },
};

const DEFS: InstrumentDef[] = [
  bondDef("91282CJL6", "US Treasury 4.25% 2028", "US Treasury", "US91282CJL63", "91282CJL6"),
  bondDef("UKT1H26", "UK Gilt 1.5% 2026", "UK DMO", "GB00BMGR2791", "UKT1H26X"),
  DEPOSIT_DEF,
];

/** A composite line fixture — identity + a (here-irrelevant) price shell. */
function inst(
  instrumentId: string,
  isin: string,
  cusip: string,
): AggregatedInstrument {
  return {
    instrumentId,
    displayName: instrumentId,
    isin,
    cusip,
    bestBid: 99,
    bestOffer: 99.1,
    bidSize: 1_000_000,
    offerSize: 1_000_000,
    confidence: 0.9,
    contributions: [],
  };
}

describe("securityOptions", () => {
  it("keeps only bond-family definitions with a non-blank id", () => {
    const opts = securityOptions(DEFS);
    expect(opts.map((o) => o.instrumentId).sort()).toEqual(["91282CJL6", "UKT1H26"]);
    // The deposit (non-bond) is excluded.
    expect(opts.some((o) => o.instrumentId === "usd-sofr-on")).toBe(false);
  });

  it("surfaces issuer group, coupon sublabel, and identifiers for search", () => {
    const opt = securityOptions(DEFS).find((o) => o.instrumentId === "91282CJL6")!;
    expect(opt.group).toBe("US Treasury");
    expect(opt.sublabel).toContain("US Treasury");
    expect(opt.sublabel).toContain("4.25%");
    expect(opt.isin).toBe("US91282CJL63");
    expect(opt.searchText).toContain("us91282cjl63");
    expect(opt.searchText).toContain("us treasury");
  });
});

describe("groupSecurityOptions + filterSecurityGroups", () => {
  it("buckets options by issuer, groups + options sorted", () => {
    const groups = groupSecurityOptions(securityOptions(DEFS));
    expect(groups.map((g) => g.group)).toEqual(["UK DMO", "US Treasury"]);
  });

  it("substring search filters across name/issuer/isin and drops empty groups", () => {
    const groups = groupSecurityOptions(securityOptions(DEFS));
    const hit = filterSecurityGroups(groups, "gilt");
    expect(hit).toHaveLength(1);
    expect(hit[0].group).toBe("UK DMO");
    // An empty query returns every group unchanged.
    expect(filterSecurityGroups(groups, "  ")).toHaveLength(2);
    // A no-match query yields no groups.
    expect(filterSecurityGroups(groups, "zzz")).toHaveLength(0);
  });
});

describe("selectedIdentifierSet + instrumentMatchesSelection", () => {
  it("expands a chosen id to its ISIN + CUSIP identifiers", () => {
    const set = selectedIdentifierSet(DEFS, ["91282CJL6"]);
    expect(set.has("91282CJL6")).toBe(true);
    expect(set.has("US91282CJL63")).toBe(true); // ISIN
    expect(set.has("UKT1H26")).toBe(false);
  });

  it("matches a composite line keyed by ISIN even when chosen by internal id", () => {
    const set = selectedIdentifierSet(DEFS, ["91282CJL6"]);
    // Line the server keyed by ISIN, not the canonical id.
    expect(instrumentMatchesSelection(inst("US91282CJL63", "US91282CJL63", ""), set)).toBe(true);
    expect(instrumentMatchesSelection(inst("UKT1H26", "GB00BMGR2791", "UKT1H26X"), set)).toBe(false);
  });
});

describe("filterInstrumentsBySelection", () => {
  const lines = [
    inst("91282CJL6", "US91282CJL63", "91282CJL6"),
    inst("UKT1H26", "GB00BMGR2791", "UKT1H26X"),
  ];

  it("EMPTY selection shows ALL lines (never an accidentally-blank book)", () => {
    expect(filterInstrumentsBySelection(lines, DEFS, [])).toHaveLength(2);
  });

  it("a non-empty selection keeps only the chosen securities, in order", () => {
    const shown = filterInstrumentsBySelection(lines, DEFS, ["UKT1H26"]);
    expect(shown.map((i) => i.instrumentId)).toEqual(["UKT1H26"]);
  });

  it("a selection that matches nothing currently quoting yields an empty grid", () => {
    expect(filterInstrumentsBySelection(lines, DEFS, ["not-streaming"])).toHaveLength(0);
  });
});
