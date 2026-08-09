/**
 * Curve-definition contract (server commit 38bcff9a): the wire codec round-trip
 * (interpolation enum + nested pillars), and the CRUD transport surface against the
 * offline MockTransport — the server-parity primary-per-currency invariant and the
 * coded errors (`already_exists` / `not_found` / `failed_precondition` /
 * `invalid_argument`).
 */

import { describe, expect, it } from "vitest";

import type { CurveDefinition } from "../src/data/contract";
import {
  curveInterpolationCode,
  curveInterpolationFromCode,
  CURVE_INTERPOLATIONS,
} from "../src/data/contract";
import {
  createCurveDefinitionRequestToWire,
  curveDefinitionCreatedFromWire,
  curveDefinitionFromWire,
  curveDefinitionToWire,
  updateCurveDefinitionRequestToWire,
} from "../src/data/wsCodec";
import { DEFAULT_USD_SOFR_CURVE } from "../src/data/ratesPricing";
import { MockTransport } from "../src/data/mockSource";

/** A complete curve definition over the default USD-SOFR pillar ladder. */
function def(overrides: Partial<CurveDefinition> = {}): CurveDefinition {
  return {
    curveId: "usd-sofr-test",
    displayName: "USD SOFR test",
    indexLabel: "USD-SOFR",
    dayCount: "ACT/360",
    calendar: "USD",
    interpolation: "log-linear-df",
    pillars: structuredClone(DEFAULT_USD_SOFR_CURVE),
    primary: false,
    ...overrides,
  };
}

describe("CurveInterpolation — wire int code ↔ ergonomic union", () => {
  it("codes the two schemes as 0 (default) and 1", () => {
    expect(curveInterpolationCode("log-linear-df")).toBe(0);
    expect(curveInterpolationCode("monotone-convex-forward")).toBe(1);
  });

  it("decodes each code, unknown ⇒ the default", () => {
    expect(curveInterpolationFromCode(0)).toBe("log-linear-df");
    expect(curveInterpolationFromCode(1)).toBe("monotone-convex-forward");
    expect(curveInterpolationFromCode(99)).toBe("log-linear-df");
  });

  it("lists both schemes in wire-code order (index === code)", () => {
    CURVE_INTERPOLATIONS.forEach((scheme, i) => {
      expect(curveInterpolationCode(scheme)).toBe(i);
    });
  });
});

describe("CurveDefinition codec — round-trips interpolation + pillars", () => {
  for (const scheme of CURVE_INTERPOLATIONS) {
    it(`round-trips a ${scheme} definition byte-for-byte`, () => {
      const original = def({ interpolation: scheme, primary: scheme === "log-linear-df" });
      const wire = curveDefinitionToWire(original);
      // The interpolation rides as its int code; pillars nest as a curve set.
      expect(wire.interpolation).toBe(curveInterpolationCode(scheme));
      expect(typeof wire.pillars).toBe("object");
      const back = curveDefinitionFromWire(wire);
      expect(back).toEqual(original);
    });
  }

  it("wraps the create/update request bodies with the definition envelope", () => {
    const d = def();
    const create = createCurveDefinitionRequestToWire(d);
    expect(curveDefinitionFromWire(create.definition as Record<string, unknown>)).toEqual(d);

    const update = updateCurveDefinitionRequestToWire("usd-sofr", d);
    expect(update.curve_id).toBe("usd-sofr");
    expect(
      curveDefinitionFromWire(update.definition as Record<string, unknown>),
    ).toEqual(d);
  });

  it("unwraps a created reply frame", () => {
    const d = def({ curveId: "usd-sofr", primary: true });
    const frame = { curve_definition_created: curveDefinitionToWire(d) };
    expect(curveDefinitionCreatedFromWire(frame)).toEqual(d);
  });
});

describe("MockTransport curve-definition CRUD — server-parity", () => {
  it("seeds usd-sofr (primary) + usd-sofr-street (non-primary)", async () => {
    const t = new MockTransport();
    const list = await t.listCurveDefinitions();
    expect(list.map((d) => d.curveId).sort()).toEqual(["usd-sofr", "usd-sofr-street"]);
    expect(list.find((d) => d.curveId === "usd-sofr")!.primary).toBe(true);
    expect(list.find((d) => d.curveId === "usd-sofr-street")!.primary).toBe(false);
  });

  it("creates a curve and echoes the server-resolved record", async () => {
    const t = new MockTransport();
    const created = await t.createCurveDefinition(def({ curveId: "usd-sofr-b" }));
    expect(created.curveId).toBe("usd-sofr-b");
    // A second USD curve stays non-primary — usd-sofr already holds the currency default.
    expect(created.primary).toBe(false);
    const list = await t.listCurveDefinitions();
    expect(list.map((d) => d.curveId)).toContain("usd-sofr-b");
  });

  it("maintains one primary per currency — a create asking primary demotes the incumbent", async () => {
    const t = new MockTransport();
    const created = await t.createCurveDefinition(
      def({ curveId: "usd-sofr-b", primary: true }),
    );
    expect(created.primary).toBe(true);
    const list = await t.listCurveDefinitions();
    const primaries = list.filter((d) => d.pillars.currency === "USD" && d.primary);
    expect(primaries.map((d) => d.curveId)).toEqual(["usd-sofr-b"]);
    expect(list.find((d) => d.curveId === "usd-sofr")!.primary).toBe(false);
  });

  it("rejects a duplicate id as already_exists", async () => {
    const t = new MockTransport();
    await expect(
      t.createCurveDefinition(def({ curveId: "usd-sofr" })),
    ).rejects.toThrow(/already_exists/);
  });

  it("rejects malformed pillars as invalid_argument", async () => {
    const t = new MockTransport();
    const empty = def({ curveId: "usd-empty" });
    await expect(
      t.createCurveDefinition({
        ...empty,
        pillars: { ...empty.pillars, pillars: [] },
      }),
    ).rejects.toThrow(/invalid_argument/);
  });

  it("updates an existing curve; the immutable slug is authoritative", async () => {
    const t = new MockTransport();
    const updated = await t.updateCurveDefinition("usd-sofr-street", {
      ...def({ curveId: "ignored-slug" }),
      displayName: "USD SOFR (renamed)",
    });
    // The request curve_id wins — the record keeps its original id.
    expect(updated.curveId).toBe("usd-sofr-street");
    expect(updated.displayName).toBe("USD SOFR (renamed)");
  });

  it("rejects an update to an unknown id as not_found", async () => {
    const t = new MockTransport();
    await expect(
      t.updateCurveDefinition("nope", def({ curveId: "nope" })),
    ).rejects.toThrow(/not_found/);
  });

  it("deletes a non-primary curve", async () => {
    const t = new MockTransport();
    await t.deleteCurveDefinition("usd-sofr-street");
    const list = await t.listCurveDefinitions();
    expect(list.map((d) => d.curveId)).toEqual(["usd-sofr"]);
  });

  it("refuses to delete the primary while siblings remain (failed_precondition)", async () => {
    const t = new MockTransport();
    await expect(t.deleteCurveDefinition("usd-sofr")).rejects.toThrow(
      /failed_precondition/,
    );
  });

  it("rejects deleting an unknown id as not_found", async () => {
    const t = new MockTransport();
    await expect(t.deleteCurveDefinition("nope")).rejects.toThrow(/not_found/);
  });
});
