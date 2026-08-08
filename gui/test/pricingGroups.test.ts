/**
 * FI Pricing Groups (server commit 07fc99f) — the GUI half of the single wire
 * contract, gated here without a server:
 *
 *   1. wsCodec round-trip — the feature/pipeline/group encoders + decoders produce
 *      and read the EXACT snake_case, numeric-enum JSON the server codec
 *      (`crates/celnet-server/src/ws/codec.rs`) decodes/encodes: the numeric
 *      FeatureKind / AxeSide / EspOrRfq enums, the OPTIONAL `reference`
 *      (emit-only-when-present / decode-absent→null), a TIERING feature carrying a
 *      full TieringConfig, and both null and object pipelines.
 *
 *   2. preview math — the indicative two-way waterfall each feature drives, and the
 *      pure add/remove/reorder pipeline ops.
 */

import { describe, expect, it } from "vitest";

import type { FeatureSpec, PricingGroup, TieringConfig } from "../src/data/contract";
import {
  featurePipelineFromWire,
  featurePipelineToWire,
  featureSpecFromWire,
  featureSpecToWire,
  pricingGroupDescFromWire,
  pricingGroupSpecToWire,
  pricingModeToWire,
} from "../src/data/wsCodec";
import {
  applyFeature,
  defaultFeatureSpec,
  defaultPipeline,
  insertFeatureAt,
  moveFeature,
  previewPipeline,
  removeFeatureAt,
  SAMPLE_RAW,
  updateFeatureAt,
} from "../src/lib/pricingGroups";

/** A JSON round-trip that simulates the server echo (encode → wire bytes → decode). */
function roundtrip<T>(value: unknown): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

const fullTiering: TieringConfig = {
  unit: "PRICE_POINTS",
  strategies: [
    {
      kind: "FLAT_MARKUP",
      halfSpread: 0.2,
      kappa: 0,
      sMax: 0,
      smoothingWeight: 0,
      expectedSpread: 0,
      maxDivergence: 0,
      coreSpread: 0,
      maxOutputSpread: 0,
      spreadScaleFactor: 0,
    },
  ],
  guardrails: { hMin: 0, hMax: 1, sMax: 0.5, spreadFloor: 0.01 },
  stalePolicy: "SUPPRESS",
};

/** A group exercising every feature kind + both the null and number `reference`. */
function kitchenSinkGroup(): PricingGroup {
  const midWithRef: FeatureSpec = { ...defaultFeatureSpec("MID_SHIFT"), shift: 0.03, reference: 101.25 };
  const midNoRef: FeatureSpec = { ...defaultFeatureSpec("MID_SHIFT"), shift: -0.02, reference: null };
  const tiering: FeatureSpec = { ...defaultFeatureSpec("TIERING"), tiering: fullTiering };
  const axe: FeatureSpec = { ...defaultFeatureSpec("AXE"), axeSide: "SELL", magnitude: 0.04 };
  const position: FeatureSpec = { ...defaultFeatureSpec("POSITION"), kappa: 0.7, sMax: 0.06 };
  const panic: FeatureSpec = { ...defaultFeatureSpec("PANIC_SKEW"), skew: 0.05, triggered: true };
  return {
    id: "tier1-eu",
    name: "TIER1-EU",
    description: "kitchen sink",
    memberConnectionIds: ["LP-SIM-01", "LP-SIM-02"],
    memberUserIds: ["u-1"],
    memberDesks: ["desk-a"],
    espPipeline: {
      features: [midWithRef, tiering, axe],
      guardrails: { hMin: 0, hMax: 2, sMax: 0.5, spreadFloor: 0.01 },
    },
    rfqPipeline: { features: [midNoRef, position, panic], guardrails: null },
    sharePipeline: false,
    enabled: true,
    // Curve-anchored + book-skew mode so the optional weight is exercised on the round-trip.
    pricingSourceMode: 3,
    bookSkewWeight: 0.35,
  };
}

describe("pricing-group wire codec", () => {
  it("round-trips a full pricing group with all five feature kinds", () => {
    const g = kitchenSinkGroup();
    const decoded = pricingGroupDescFromWire(roundtrip(pricingGroupSpecToWire(g)));
    expect(decoded).toEqual(g);
  });

  it("emits reference ONLY when non-null (absent ⇒ decode null)", () => {
    const noRef = defaultFeatureSpec("MID_SHIFT"); // reference: null
    const wireNoRef = featureSpecToWire(noRef);
    expect("reference" in wireNoRef).toBe(false);
    expect(featureSpecFromWire(roundtrip(wireNoRef))).toEqual(noRef);

    const withRef: FeatureSpec = { ...noRef, reference: 100.5 };
    const wireWithRef = featureSpecToWire(withRef);
    expect(wireWithRef["reference"]).toBe(100.5);
    expect(featureSpecFromWire(roundtrip(wireWithRef))).toEqual(withRef);
  });

  it("maps the numeric FeatureKind enum (MID_SHIFT=0 … PANIC_SKEW=4)", () => {
    expect(featureSpecToWire(defaultFeatureSpec("MID_SHIFT"))["kind"]).toBe(0);
    expect(featureSpecToWire(defaultFeatureSpec("TIERING"))["kind"]).toBe(1);
    expect(featureSpecToWire(defaultFeatureSpec("AXE"))["kind"]).toBe(2);
    expect(featureSpecToWire(defaultFeatureSpec("POSITION"))["kind"]).toBe(3);
    expect(featureSpecToWire(defaultFeatureSpec("PANIC_SKEW"))["kind"]).toBe(4);
  });

  it("maps the numeric AxeSide + EspOrRfq enums", () => {
    expect(featureSpecToWire({ ...defaultFeatureSpec("AXE"), axeSide: "BUY" })["axe_side"]).toBe(0);
    expect(featureSpecToWire({ ...defaultFeatureSpec("AXE"), axeSide: "SELL" })["axe_side"]).toBe(1);
    expect(pricingModeToWire("ESP")).toBe(0);
    expect(pricingModeToWire("RFQ")).toBe(1);
  });

  it("carries a TIERING feature's full TieringConfig through the pipeline codec", () => {
    const pipeline = { features: [{ ...defaultFeatureSpec("TIERING"), tiering: fullTiering }], guardrails: null };
    const decoded = featurePipelineFromWire(roundtrip(featurePipelineToWire(pipeline)));
    expect(decoded).toEqual(pipeline);
  });

  it("decodes an absent pipeline block as null", () => {
    const g = { ...kitchenSinkGroup(), espPipeline: null, rfqPipeline: null };
    const decoded = pricingGroupDescFromWire(roundtrip(pricingGroupSpecToWire(g)));
    expect(decoded.espPipeline).toBeNull();
    expect(decoded.rfqPipeline).toBeNull();
  });
});

describe("pricing-source policy codec", () => {
  it("always emits the integer pricing_source_mode for every mode", () => {
    for (const m of [0, 1, 2, 3] as const) {
      const wire = pricingGroupSpecToWire({ ...kitchenSinkGroup(), pricingSourceMode: m, bookSkewWeight: null });
      expect(wire["pricing_source_mode"]).toBe(m);
    }
  });

  it("emits book_skew_weight ONLY for mode 3 with a set weight", () => {
    // Non-skew mode with a weight present ⇒ still omitted (server ignores it there).
    const nonSkew = pricingGroupSpecToWire({ ...kitchenSinkGroup(), pricingSourceMode: 1, bookSkewWeight: 0.4 });
    expect("book_skew_weight" in nonSkew).toBe(false);

    // Skew mode with a null weight ⇒ omitted so the server applies its 0.5 default.
    const skewDefault = pricingGroupSpecToWire({ ...kitchenSinkGroup(), pricingSourceMode: 3, bookSkewWeight: null });
    expect("book_skew_weight" in skewDefault).toBe(false);

    // Skew mode with a weight ⇒ emitted.
    const skewSet = pricingGroupSpecToWire({ ...kitchenSinkGroup(), pricingSourceMode: 3, bookSkewWeight: 0.25 });
    expect(skewSet["book_skew_weight"]).toBe(0.25);
  });

  it("decodes an absent mode as 0 and an absent weight as null", () => {
    const decoded = pricingGroupDescFromWire({
      id: "g",
      name: "G",
      description: "",
      member_connection_ids: [],
      member_user_ids: [],
      member_desks: [],
      esp_pipeline: null,
      rfq_pipeline: null,
      share_pipeline: false,
      enabled: true,
    });
    expect(decoded.pricingSourceMode).toBe(0);
    expect(decoded.bookSkewWeight).toBeNull();
  });

  it("clamps an out-of-range wire mode to 0", () => {
    const decoded = pricingGroupDescFromWire({
      id: "g",
      name: "G",
      description: "",
      member_connection_ids: [],
      member_user_ids: [],
      member_desks: [],
      esp_pipeline: null,
      rfq_pipeline: null,
      share_pipeline: false,
      enabled: true,
      pricing_source_mode: 9,
    });
    expect(decoded.pricingSourceMode).toBe(0);
  });
});

describe("preview math — per feature", () => {
  it("MID_SHIFT shifts mid, keeping the half-spread", () => {
    const out = applyFeature(SAMPLE_RAW, { ...defaultFeatureSpec("MID_SHIFT"), shift: 0.1, reference: null });
    expect(out.bid).toBeCloseTo(99.6, 10);
    expect(out.offer).toBeCloseTo(99.7, 10);
  });

  it("MID_SHIFT reference OVERRIDES mid", () => {
    const out = applyFeature(SAMPLE_RAW, { ...defaultFeatureSpec("MID_SHIFT"), shift: 999, reference: 100 });
    expect(out.bid).toBeCloseTo(99.95, 10);
    expect(out.offer).toBeCloseTo(100.05, 10);
  });

  it("TIERING widens the half-spread by the strategy half-spreads (clamped by its guardrails)", () => {
    const out = applyFeature(SAMPLE_RAW, { ...defaultFeatureSpec("TIERING"), tiering: fullTiering });
    // raw half 0.05 + FLAT_MARKUP 0.2 = 0.25, within [0,1].
    expect(out.bid).toBeCloseTo(99.3, 10);
    expect(out.offer).toBeCloseTo(99.8, 10);
  });

  it("AXE leans mid toward BUY (+) and SELL (−)", () => {
    const buy = applyFeature(SAMPLE_RAW, { ...defaultFeatureSpec("AXE"), axeSide: "BUY", magnitude: 0.03 });
    expect(buy.bid).toBeCloseTo(99.53, 10);
    expect(buy.offer).toBeCloseTo(99.63, 10);
    const sell = applyFeature(SAMPLE_RAW, { ...defaultFeatureSpec("AXE"), axeSide: "SELL", magnitude: 0.03 });
    expect(sell.bid).toBeCloseTo(99.47, 10);
    expect(sell.offer).toBeCloseTo(99.57, 10);
  });

  it("POSITION clamps the inventory skew at ±sMax", () => {
    const longSkew = applyFeature(SAMPLE_RAW, { ...defaultFeatureSpec("POSITION"), kappa: 0.5, sMax: 0.05 });
    // clamp(0.5·1, ±0.05) = 0.05, mid − 0.05 = 99.50.
    expect(longSkew.bid).toBeCloseTo(99.45, 10);
    expect(longSkew.offer).toBeCloseTo(99.55, 10);
    const shortSkew = applyFeature(SAMPLE_RAW, { ...defaultFeatureSpec("POSITION"), kappa: -0.5, sMax: 0.05 });
    // clamp(−0.5, ±0.05) = −0.05, mid − (−0.05) = 99.60.
    expect(shortSkew.bid).toBeCloseTo(99.55, 10);
    expect(shortSkew.offer).toBeCloseTo(99.65, 10);
  });

  it("PANIC_SKEW applies the overlay ONLY when triggered", () => {
    const on = applyFeature(SAMPLE_RAW, { ...defaultFeatureSpec("PANIC_SKEW"), skew: 0.02, triggered: true });
    expect(on.bid).toBeCloseTo(99.52, 10);
    const off = applyFeature(SAMPLE_RAW, { ...defaultFeatureSpec("PANIC_SKEW"), skew: 0.02, triggered: false });
    expect(off).toEqual(SAMPLE_RAW);
  });
});

describe("preview waterfall", () => {
  it("has length features.length + 1 with index 0 = raw", () => {
    const features = [
      { ...defaultFeatureSpec("MID_SHIFT"), shift: 0.1, reference: null },
      { ...defaultFeatureSpec("AXE"), axeSide: "BUY" as const, magnitude: 0.02 },
    ];
    const steps = previewPipeline(SAMPLE_RAW, features, null);
    expect(steps).toHaveLength(3);
    expect(steps[0]).toEqual(SAMPLE_RAW);
  });

  it("clamps the FINAL two-way to the pipeline guardrails", () => {
    const widen: FeatureSpec = {
      ...defaultFeatureSpec("TIERING"),
      tiering: { ...fullTiering, guardrails: null, strategies: [{ ...fullTiering.strategies[0]!, halfSpread: 0.5 }] },
    };
    const steps = previewPipeline(SAMPLE_RAW, [widen], { hMin: 0, hMax: 0.1, sMax: 0.5, spreadFloor: 0.01 });
    const out = steps[steps.length - 1]!;
    // inner widen → half 0.55, then pipeline guardrail hMax 0.1 → half 0.1, spread 0.2.
    expect(out.offer - out.bid).toBeCloseTo(0.2, 10);
  });
});

describe("pure pipeline ops", () => {
  const base = defaultPipeline().features;
  const a = defaultFeatureSpec("MID_SHIFT");
  const b = defaultFeatureSpec("AXE");
  const c = defaultFeatureSpec("POSITION");

  it("insertFeatureAt inserts at the index without mutating the input", () => {
    const one = insertFeatureAt(base, a, 0);
    const two = insertFeatureAt(one, b, 1);
    expect(two.map((f) => f.kind)).toEqual(["MID_SHIFT", "AXE"]);
    expect(base).toHaveLength(0); // original untouched
  });

  it("removeFeatureAt removes only the indexed feature", () => {
    const list = [a, b, c];
    expect(removeFeatureAt(list, 1).map((f) => f.kind)).toEqual(["MID_SHIFT", "POSITION"]);
    expect(list).toHaveLength(3);
  });

  it("moveFeature reorders immutably", () => {
    const list = [a, b, c];
    expect(moveFeature(list, 0, 2).map((f) => f.kind)).toEqual(["AXE", "POSITION", "MID_SHIFT"]);
    expect(moveFeature(list, 2, 0).map((f) => f.kind)).toEqual(["POSITION", "MID_SHIFT", "AXE"]);
    expect(list.map((f) => f.kind)).toEqual(["MID_SHIFT", "AXE", "POSITION"]);
  });

  it("updateFeatureAt patches only the indexed feature", () => {
    const list = [a, b];
    const next = updateFeatureAt(list, 1, { magnitude: 0.09 });
    expect(next[1]!.magnitude).toBe(0.09);
    expect(next[0]).toBe(a);
    expect(list[1]!.magnitude).toBe(b.magnitude);
  });
});
