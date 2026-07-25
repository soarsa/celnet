/**
 * FI-TIERING phase 3 — the per-book outbound-tiering config: wire codec round-trip
 * + client-side validation.
 *
 * The codec assertions pin the BYTE-FOR-FIELD contract with the server codec
 * (`crates/celnet-server/src/ws/codec.rs` tiering_*_{to,from}_json): the exact
 * snake_case field names and the NUMERIC proto3 enum ints. The validation
 * assertions mirror the `celnet-tiering` guardrail invariants. We drive the FULL
 * spec→wire→desc path (through the real request encoder + desc decoder) so a
 * field-name or enum-int drift fails here rather than silently on the wire.
 */

import { describe, expect, it } from "vitest";

import type { AggregatedBookSpec, TieringConfig } from "../src/data/contract";
import {
  aggregatedBookDescFromWire,
  createAggregatedBookRequestToWire,
  tieringConfigFromWire,
  tieringConfigToWire,
  type WireObject,
} from "../src/data/wsCodec";
import {
  defaultTieringConfig,
  defaultTieringStrategy,
  hasTieringErrors,
  TIERING_STRATEGY_KINDS,
  TIERING_STRATEGY_META,
  validateTiering,
} from "../src/lib/tiering";

/** A fully-populated config exercising all three strategy kinds + guardrails. */
const richConfig: TieringConfig = {
  unit: "YIELD_BPS",
  strategies: [
    { ...defaultTieringStrategy("FLAT_MARKUP"), halfSpread: 25 },
    { ...defaultTieringStrategy("INVENTORY_SKEW"), halfSpread: 30, kappa: 0.75, sMax: 0.4 },
    {
      ...defaultTieringStrategy("SCALED_SMOOTHED_SPREAD"),
      smoothingWeight: 0.3,
      expectedSpread: 0.00008,
      maxDivergence: 0.00004,
      coreSpread: 0.0002,
      maxOutputSpread: 0.0008,
      spreadScaleFactor: 1.2,
    },
  ],
  guardrails: { hMin: 0.05, hMax: 1.25, sMax: 0.5, spreadFloor: 0.02 },
  stalePolicy: "WIDEN_TO_MAX",
};

function specWith(tiering: TieringConfig | null): AggregatedBookSpec {
  return {
    id: "us-treasuries",
    name: "US Treasuries",
    memberConnectionIds: ["LP-SIM-01", "LP-SIM-02"],
    scopeMode: "ALL_MEMBERS_QUOTE",
    instrumentIds: [],
    params: {
      stalenessTauMs: 2000,
      maxQuoteAgeMs: 5000,
      divergenceGating: true,
      minContributors: 2,
      depthLevels: 1,
    },
    enabled: true,
    tiering,
  };
}

describe("tiering wire codec (byte-for-field with the server)", () => {
  it("emits the exact snake_case field names + numeric enum ints", () => {
    const wire = tieringConfigToWire(richConfig);
    // Enum ints: YIELD_BPS=1, WIDEN_TO_MAX=1, FLAT_MARKUP=0, INVENTORY_SKEW=1.
    expect(wire.unit).toBe(1);
    expect(wire.stale_policy).toBe(1);
    const strategies = wire.strategies as WireObject[];
    // Every strategy emits ALL ten fields (matching the server's full field walk); a
    // kind's ignored fields are proto3 zero.
    expect(strategies[0]).toEqual({
      kind: 0,
      half_spread: 25,
      kappa: 0,
      s_max: 0,
      smoothing_weight: 0,
      expected_spread: 0,
      max_divergence: 0,
      core_spread: 0,
      max_output_spread: 0,
      spread_scale_factor: 0,
    });
    expect(strategies[1]).toEqual({
      kind: 1,
      half_spread: 30,
      kappa: 0.75,
      s_max: 0.4,
      smoothing_weight: 0,
      expected_spread: 0,
      max_divergence: 0,
      core_spread: 0,
      max_output_spread: 0,
      spread_scale_factor: 0,
    });
    // SCALED_SMOOTHED_SPREAD: enum int 2 + the six snake_case params.
    expect(strategies[2]).toEqual({
      kind: 2,
      half_spread: 0,
      kappa: 0,
      s_max: 0,
      smoothing_weight: 0.3,
      expected_spread: 0.00008,
      max_divergence: 0.00004,
      core_spread: 0.0002,
      max_output_spread: 0.0008,
      spread_scale_factor: 1.2,
    });
    expect(wire.guardrails).toEqual({
      h_min: 0.05,
      h_max: 1.25,
      s_max: 0.5,
      spread_floor: 0.02,
    });
  });

  it("round-trips a rich config exactly (encode → decode)", () => {
    expect(tieringConfigFromWire(tieringConfigToWire(richConfig))).toEqual(richConfig);
  });

  it("round-trips the default (flat 25 price bps) config exactly", () => {
    const cfg = defaultTieringConfig();
    expect(tieringConfigFromWire(tieringConfigToWire(cfg))).toEqual(cfg);
    // The worked-example baseline: PRICE_BPS(0), one FLAT_MARKUP(0) @ 25.
    const wire = tieringConfigToWire(cfg);
    expect(wire.unit).toBe(0);
    expect((wire.strategies as WireObject[])[0]).toMatchObject({ kind: 0, half_spread: 25 });
  });

  it("decodes an absent/empty tiering block as disabled (null)", () => {
    expect(aggregatedBookDescFromWire({ id: "x", name: "X" }).tiering).toBeNull();
  });

  it("drives the full spec→wire→desc path with tiering enabled", () => {
    const req = createAggregatedBookRequestToWire(specWith(richConfig));
    const specWire = req.spec as WireObject;
    // The request encoder carries the tiering block under the nested spec.
    expect(specWire.tiering).toBeTruthy();
    // Simulate the server echo (spec + minted id) and decode as a desc.
    const desc = aggregatedBookDescFromWire({ ...specWire, id: "us-treasuries" });
    expect(desc.tiering).toEqual(richConfig);
  });

  it("drives the full spec→wire→desc path with tiering disabled", () => {
    const req = createAggregatedBookRequestToWire(specWith(null));
    const specWire = req.spec as WireObject;
    expect(specWire.tiering).toBeNull();
    const desc = aggregatedBookDescFromWire({ ...specWire, id: "us-treasuries" });
    expect(desc.tiering).toBeNull();
  });

  it("renders guardrails as null on the wire when absent", () => {
    const noGuards: TieringConfig = { ...richConfig, guardrails: null };
    expect(tieringConfigToWire(noGuards).guardrails).toBeNull();
    expect(tieringConfigFromWire(tieringConfigToWire(noGuards)).guardrails).toBeNull();
  });
});

describe("tiering validation (mirrors the server guardrail invariants)", () => {
  it("accepts the default config", () => {
    expect(hasTieringErrors(validateTiering(defaultTieringConfig()))).toBe(false);
  });

  it("accepts a rich two-strategy config", () => {
    expect(hasTieringErrors(validateTiering(richConfig))).toBe(false);
  });

  it("rejects h_max < h_min", () => {
    const bad: TieringConfig = {
      ...defaultTieringConfig(),
      guardrails: { hMin: 0.5, hMax: 0.1, sMax: 0.5, spreadFloor: 0.01 },
    };
    const errors = validateTiering(bad);
    expect(errors.guardrails.hMax).toBeDefined();
    expect(hasTieringErrors(errors)).toBe(true);
  });

  it("rejects spread_floor <= 0", () => {
    const bad: TieringConfig = {
      ...defaultTieringConfig(),
      guardrails: { hMin: 0, hMax: 1, sMax: 0.5, spreadFloor: 0 },
    };
    expect(validateTiering(bad).guardrails.spreadFloor).toBeDefined();
  });

  it("rejects a non-finite guardrail", () => {
    const bad: TieringConfig = {
      ...defaultTieringConfig(),
      guardrails: { hMin: 0, hMax: Number.NaN, sMax: 0.5, spreadFloor: 0.01 },
    };
    expect(validateTiering(bad).guardrails.hMax).toBeDefined();
  });

  it("rejects a negative guardrail skew cap", () => {
    const bad: TieringConfig = {
      ...defaultTieringConfig(),
      guardrails: { hMin: 0, hMax: 1, sMax: -0.1, spreadFloor: 0.01 },
    };
    expect(validateTiering(bad).guardrails.sMax).toBeDefined();
  });

  it("rejects an empty strategy list", () => {
    const bad: TieringConfig = { ...defaultTieringConfig(), strategies: [] };
    const errors = validateTiering(bad);
    expect(errors.form).toBeDefined();
    expect(hasTieringErrors(errors)).toBe(true);
  });

  it("rejects a negative half-spread on a strategy", () => {
    const bad: TieringConfig = {
      ...defaultTieringConfig(),
      strategies: [{ kind: "FLAT_MARKUP", halfSpread: -1, kappa: 0, sMax: 0 }],
    };
    expect(validateTiering(bad).strategies[0]?.halfSpread).toBeDefined();
  });

  it("rejects a negative strategy sMax on an inventory-skew strategy", () => {
    const bad: TieringConfig = {
      ...defaultTieringConfig(),
      strategies: [{ kind: "INVENTORY_SKEW", halfSpread: 25, kappa: 0.5, sMax: -1 }],
    };
    expect(validateTiering(bad).strategies[0]?.sMax).toBeDefined();
  });

  it("allows a negative kappa (skew may lean either way)", () => {
    const cfg: TieringConfig = {
      ...defaultTieringConfig(),
      strategies: [{ ...defaultTieringStrategy("INVENTORY_SKEW"), kappa: -0.5 }],
    };
    expect(hasTieringErrors(validateTiering(cfg))).toBe(false);
  });

  it("accepts a valid Scaled-Smoothed-Spread strategy", () => {
    const cfg: TieringConfig = {
      ...defaultTieringConfig(),
      strategies: [defaultTieringStrategy("SCALED_SMOOTHED_SPREAD")],
    };
    expect(hasTieringErrors(validateTiering(cfg))).toBe(false);
  });

  it("rejects Scaled-Smoothed smoothing weight outside (0, 1]", () => {
    const cfg: TieringConfig = {
      ...defaultTieringConfig(),
      strategies: [{ ...defaultTieringStrategy("SCALED_SMOOTHED_SPREAD"), smoothingWeight: 1.5 }],
    };
    expect(validateTiering(cfg).strategies[0]?.smoothingWeight).toBeDefined();
  });

  it("rejects Scaled-Smoothed expected spread e <= 0", () => {
    const cfg: TieringConfig = {
      ...defaultTieringConfig(),
      strategies: [{ ...defaultTieringStrategy("SCALED_SMOOTHED_SPREAD"), expectedSpread: 0 }],
    };
    expect(validateTiering(cfg).strategies[0]?.expectedSpread).toBeDefined();
  });

  it("rejects Scaled-Smoothed max output spread m < core spread c", () => {
    const cfg: TieringConfig = {
      ...defaultTieringConfig(),
      strategies: [
        { ...defaultTieringStrategy("SCALED_SMOOTHED_SPREAD"), coreSpread: 0.5, maxOutputSpread: 0.2 },
      ],
    };
    expect(validateTiering(cfg).strategies[0]?.maxOutputSpread).toBeDefined();
  });

  it("ignores half-spread for a Scaled-Smoothed strategy (spread source, not additive)", () => {
    const cfg: TieringConfig = {
      ...defaultTieringConfig(),
      strategies: [{ ...defaultTieringStrategy("SCALED_SMOOTHED_SPREAD"), halfSpread: -999 }],
    };
    // A negative half-spread is irrelevant to SCALE_SMOOTH ⇒ no error.
    expect(hasTieringErrors(validateTiering(cfg))).toBe(false);
  });
});

describe("per-strategy documentation links", () => {
  it("provides a title, purpose, and docs link for every strategy kind", () => {
    for (const kind of TIERING_STRATEGY_KINDS) {
      const meta = TIERING_STRATEGY_META[kind];
      expect(meta.title.length).toBeGreaterThan(0);
      expect(meta.purpose.length).toBeGreaterThan(0);
      expect(meta.docHref).toMatch(/^https:\/\/.*FI-TIERING-RESEARCH\.md#/);
    }
  });
});
