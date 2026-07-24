/**
 * Instrument reference-data registry — codec round-trip + MockTransport CRUD.
 *
 * Exercises the REAL `src/data/wsCodec.ts` and the REAL offline `MockTransport`
 * (no server, no mocks-as-real). For every instrument family the wire encoder
 * produces snake_case JSON with the single family sub-object keyed by its wire
 * token (`ois`, `stir_future`, `vanilla_irs`, …); the decoder reconstructs the
 * camelCase GUI shape losslessly, including external ids and bond broken dates
 * (the optional ones omitted on the wire when absent).
 */
import { describe, expect, it } from "vitest";

import type { InstrumentDef } from "../src/data/contract";
import { MockTransport } from "../src/data/mockSource";
import {
  createInstrumentRequestToWire,
  deleteInstrumentResponseFromWire,
  instrumentDefFromWire,
  instrumentDefToWire,
  instrumentsResponseFromWire,
  type WireObject,
} from "../src/data/wsCodec";

const OIS: InstrumentDef = {
  instrumentId: "usd-sofr-ois-5y",
  name: "USD SOFR OIS 5Y",
  description: "USD overnight-indexed swap vs SOFR, 5Y",
  currency: "USD",
  externalIds: [{ scheme: "ticker", value: "USOSFR5" }],
  family: "ois",
  ois: {
    tenor: "5Y",
    index: "SOFR",
    fixedFrequency: "annual",
    fixedDayCount: "act_360",
    floatDayCount: "act_360",
    businessDayConvention: "modified_following",
    calendars: ["united_states"],
    spotLagDays: 2,
  },
};

const STIR: InstrumentDef = {
  instrumentId: "sr3-h26",
  name: "SOFR 3M Future H26",
  description: "3M SOFR future, Mar-2026",
  currency: "USD",
  externalIds: [{ scheme: "ticker", value: "SR3H26" }],
  family: "stir_future",
  stirFuture: {
    contractCode: "SR3",
    referenceStart: "2026-03-18",
    referenceEnd: "2026-06-17",
    dayCount: "act_360",
    calendars: ["united_states"],
    convexityVol: 0.005,
    contractSize: 2_500_000,
  },
};

const BOND_FULL: InstrumentDef = {
  instrumentId: "us-treasury-4-25-2035",
  name: "US Treasury 4.25% 2035",
  description: "US Treasury note",
  currency: "USD",
  externalIds: [
    { scheme: "isin", value: "US91282CHK24" },
    { scheme: "cusip", value: "91282CHK2" },
  ],
  family: "bond",
  bond: {
    issuer: "US Treasury",
    couponRate: 4.25,
    couponType: "fixed",
    couponFrequency: "semi_annual",
    dayCount: "act_act",
    issueDate: { year: 2025, month: 2, day: 15 },
    datedDate: { year: 2025, month: 2, day: 15 },
    firstCouponDate: { year: 2025, month: 8, day: 15 },
    maturityDate: { year: 2035, month: 2, day: 15 },
    redemption: 100,
    calendars: ["united_states"],
  },
};

const BOND_ZERO: InstrumentDef = {
  instrumentId: "zcb-2030",
  name: "Zero 2030",
  description: "zero-coupon",
  currency: "USD",
  externalIds: [],
  family: "bond",
  bond: {
    issuer: "ACME",
    couponRate: 0,
    couponType: "zero",
    couponFrequency: "",
    dayCount: "act_act",
    maturityDate: { year: 2030, month: 6, day: 1 },
    redemption: 100,
    calendars: ["target2"],
  },
};

describe("wsCodec instrumentDef round-trip", () => {
  for (const def of [OIS, STIR, BOND_FULL, BOND_ZERO]) {
    it(`is lossless for the ${def.family} family`, () => {
      expect(instrumentDefFromWire(instrumentDefToWire(def))).toEqual(def);
    });
  }

  it("encodes snake_case keys + the family sub-object keyed by its wire token", () => {
    const wire = instrumentDefToWire(STIR);
    expect(wire.instrument_id).toBe("sr3-h26");
    expect(wire.external_ids).toEqual([{ scheme: "ticker", value: "SR3H26" }]);
    expect(wire.deposit).toBeUndefined();
    const stir = wire.stir_future as WireObject;
    expect(stir).toBeDefined();
    expect(stir.contract_code).toBe("SR3");
    expect(stir.day_count).toBe("act_360");
    expect(stir.contract_size).toBe(2_500_000);
  });

  it("emits bond broken dates and omits the absent optional ones", () => {
    const full = instrumentDefToWire(BOND_FULL).bond as WireObject;
    expect(full.maturity_date).toEqual({ year: 2035, month: 2, day: 15 });
    expect(full.issue_date).toEqual({ year: 2025, month: 2, day: 15 });
    expect(full.first_coupon_date).toEqual({ year: 2025, month: 8, day: 15 });

    const zero = instrumentDefToWire(BOND_ZERO).bond as WireObject;
    expect(zero.maturity_date).toEqual({ year: 2030, month: 6, day: 1 });
    expect("issue_date" in zero).toBe(false);
    expect("dated_date" in zero).toBe(false);
    expect("first_coupon_date" in zero).toBe(false);
    expect(zero.coupon_type).toBe("zero");
    expect(zero.coupon_frequency).toBe("");
  });

  it("decodes a list response into camelCase defs", () => {
    const wire = { instruments: [instrumentDefToWire(OIS), instrumentDefToWire(BOND_FULL)] };
    expect(instrumentsResponseFromWire(wire)).toEqual([OIS, BOND_FULL]);
  });

  it("reads a delete response boolean", () => {
    expect(deleteInstrumentResponseFromWire({ removed: true })).toBe(true);
    expect(deleteInstrumentResponseFromWire({ removed: false })).toBe(false);
    expect(deleteInstrumentResponseFromWire({})).toBe(false);
  });

  it("wraps a create request under `instrument`", () => {
    const req = createInstrumentRequestToWire(OIS);
    expect((req.instrument as WireObject).instrument_id).toBe("usd-sofr-ois-5y");
  });
});

describe("MockTransport instrument CRUD", () => {
  it("seeds an OIS and the curated bond universe, returning immutable copies", async () => {
    const t = new MockTransport();
    const list = await t.listInstruments();
    // The registry seeds exactly two families: an OIS plus the curated Treasury
    // bond universe backing the offline Aggregated Book (id/ISIN/CUSIP-matched to
    // its composite lines). Assert the family SET (not an exact multiset) so the
    // curated bond list can grow without churning this immutability test.
    expect([...new Set(list.map((d) => d.family))].sort()).toEqual(["bond", "ois"]);
    expect(list.filter((d) => d.family === "ois")).toHaveLength(1);
    expect(list.filter((d) => d.family === "bond").length).toBeGreaterThan(1);
    // Mutating a returned copy must not affect the store.
    list[0]!.name = "tampered";
    const again = await t.listInstruments();
    expect(again.some((d) => d.name === "tampered")).toBe(false);
  });

  it("mints an id from the name on create when blank", async () => {
    const t = new MockTransport();
    const created = await t.createInstrument({ ...OIS, instrumentId: "", name: "EUR ESTR OIS 2Y" });
    expect(created.instrumentId).toBe("eur-estr-ois-2y");
    const got = await t.getInstrument("eur-estr-ois-2y");
    expect(got?.name).toBe("EUR ESTR OIS 2Y");
  });

  it("rejects a blank name and a duplicate name", async () => {
    const t = new MockTransport();
    await expect(t.createInstrument({ ...OIS, instrumentId: "", name: "   " })).rejects.toThrow();
    await expect(
      t.createInstrument({ ...OIS, instrumentId: "", name: "USD SOFR OIS 5Y" }),
    ).rejects.toThrow();
  });

  it("updates and deletes a definition", async () => {
    const t = new MockTransport();
    const updated = await t.updateInstrument({
      ...OIS,
      instrumentId: "usd-sofr-ois-5y",
      description: "edited",
    });
    expect(updated.description).toBe("edited");
    expect(await t.deleteInstrument("usd-sofr-ois-5y")).toBe(true);
    expect(await t.deleteInstrument("usd-sofr-ois-5y")).toBe(false);
    expect(await t.getInstrument("usd-sofr-ois-5y")).toBeNull();
  });
});
