/**
 * Hedge-VEHICLE + SUGGEST-mode wsCodec tests — the GUI end of the two new slices of the
 * ONE `celnet.wire` hedge contract.
 *
 * The load-bearing property, tested first and hardest: a policy authored BEFORE the
 * vehicle feature must round-trip byte-identically with its original behaviour. The wire
 * has no `vehicle_kind` on those actions, proto3 reads an absent enum as its zero, and the
 * zero is `self` — the same security sold back. If that default ever drifts, every
 * pre-existing rule silently starts hedging with something else.
 */
import { describe, expect, it } from "vitest";

import {
  exitActionFromWire,
  exitActionToWire,
  executeHedgeSuggestionRequestToWire,
  executeHedgeSuggestionResponseFromWire,
  hedgeConfigFromWire,
  hedgeConfigToWire,
  hedgeExitModeBindingFromWire,
  hedgeExitModeBindingToWire,
  hedgeExitModeFromWire,
  hedgeExitModeToWire,
  hedgeIntentFromWire,
  hedgeIntentToWire,
  hedgeProvenanceFromWire,
  hedgeProvenanceToWire,
  hedgeSuggestionFromWire,
  hedgeSuggestionToWire,
  hedgeSuggestionsResponseFromWire,
  hedgeVehicleKindFromWire,
  hedgeVehicleKindToWire,
  hedgeVehiclePlanFromWire,
  hedgeVehiclePlanToWire,
  hedgeVehicleRuleFromWire,
  hedgeVehicleRuleToWire,
  listHedgeSuggestionsRequestToWire,
} from "../src/data/wsCodec";
import type {
  ExitAction,
  HedgeConfig,
  HedgeExitModeBinding,
  HedgeIntent,
  HedgeProvenance,
  HedgeSuggestion,
  HedgeVehiclePlan,
  HedgeVehicleRule,
} from "../src/data/contract";

const plan: HedgeVehiclePlan = {
  hedgeInstrumentId: "TY-DEC26",
  unitLabel: "contract",
  wholeUnits: true,
  dv01Basis: "exposure-proxy",
  durationCorrect: false,
  targetDv01: 24_840,
  dv01PerUnit: 78,
  exactUnits: 24_840 / 78,
  units: 318,
  hedgedDv01: 24_804,
  residualDv01: 36,
  summary: "Sell 318 contracts of TY-DEC26",
};

const action: ExitAction = {
  kind: "submit_market_order",
  instrument: "",
  size: { kind: "overflow", fixed: 0 },
  skewBp: null,
  toEdge: false,
  style: "immediate",
  lps: [],
  internalFirst: false,
  reason: "",
  vehicleKind: "future",
  vehicleInstrument: "TY-DEC26",
};

const suggestion: HedgeSuggestion = {
  suggestionId: "SUG-0003",
  book: "fi-credit-emea",
  instrument: "XS2034-ACME-4H",
  desk: "emea",
  raisedAt: 1_700_000_000_123,
  band: "breach",
  netRisk: 24_840,
  threshold: 18_000,
  utilization: 1.38,
  action,
  policyPath: [0, 2, 3],
  externalSize: 24_840,
  vehiclePlan: plan,
  headline: "Sell 318 contracts of TY-DEC26",
  rationale: "Corporate 9y is 138% of its DV01 budget.",
  parentPositionId: 8_814n,
  lps: ["LP-1", "LP-3"],
  midAtRaise: 111.42,
};

describe("hedge vehicle enum ordinals", () => {
  it("HedgeVehicleKindEnum maps self=0 / benchmark=1 / instrument=2 / future=3", () => {
    expect(hedgeVehicleKindToWire("self")).toBe(0);
    expect(hedgeVehicleKindToWire("benchmark")).toBe(1);
    expect(hedgeVehicleKindToWire("instrument")).toBe(2);
    expect(hedgeVehicleKindToWire("future")).toBe(3);
    expect(hedgeVehicleKindFromWire(3)).toBe("future");
    expect(hedgeVehicleKindFromWire(99)).toBe("self"); // out-of-range ⇒ proto3 zero
  });
  it("HedgeExitModeEnum maps auto=0 / suggest=1", () => {
    expect(hedgeExitModeToWire("auto")).toBe(0);
    expect(hedgeExitModeToWire("suggest")).toBe(1);
    expect(hedgeExitModeFromWire(1)).toBe("suggest");
    expect(hedgeExitModeFromWire(99)).toBe("auto");
  });
});

describe("the pre-vehicle policy must decode unchanged", () => {
  it("an action wire object with NO vehicle keys decodes to the self vehicle", () => {
    // Exactly what a policy authored before the feature looks like on the wire.
    const legacy = {
      kind: 3,
      instrument: "",
      size: { kind: 0, fixed: 0 },
      to_edge: false,
      style: 0,
      lps: [],
      internal_first: false,
      reason: "",
    };
    const back = exitActionFromWire(legacy);
    expect(back.vehicleKind).toBe("self");
    expect(back.vehicleInstrument).toBe("");
  });
  it("a provenance / intent with NO vehicle_plan decodes to null, never a fabricated plan", () => {
    expect(hedgeProvenanceFromWire({ hedge_id: "H-1" }).vehiclePlan).toBeNull();
    expect(hedgeIntentFromWire({ book: "b" }).vehiclePlan).toBeNull();
  });
  it("an intent with NO exit_mode decodes to auto (the pre-feature behaviour)", () => {
    expect(hedgeIntentFromWire({ book: "b" }).exitMode).toBe("auto");
  });
  it("a config with NO vehicles / exit_modes decodes to empty rosters", () => {
    const c = hedgeConfigFromWire({ kill_switch: false });
    expect(c.vehicles).toEqual([]);
    expect(c.exitModes).toEqual([]);
  });
});

describe("exit action carries the vehicle", () => {
  it("ALWAYS writes both vehicle keys and round-trips them", () => {
    const wire = exitActionToWire(action);
    expect(wire["vehicle_kind"]).toBe(3);
    expect(wire["vehicle_instrument"]).toBe("TY-DEC26");
    expect(exitActionFromWire(wire)).toEqual(action);
  });
  it("writes an EMPTY vehicle_instrument for the kinds that name nothing", () => {
    const self: ExitAction = { ...action, vehicleKind: "self", vehicleInstrument: "" };
    const wire = exitActionToWire(self);
    expect(wire["vehicle_kind"]).toBe(0);
    expect(wire["vehicle_instrument"]).toBe("");
    expect(exitActionFromWire(wire)).toEqual(self);
  });
});

describe("hedge vehicle plan + registry round-trip", () => {
  it("round-trips a whole-lot plan including the honesty flags", () => {
    const wire = hedgeVehiclePlanToWire(plan);
    expect(wire["duration_correct"]).toBe(false);
    expect(wire["dv01_basis"]).toBe("exposure-proxy");
    expect(wire["residual_dv01"]).toBe(36);
    expect(hedgeVehiclePlanFromWire(wire)).toEqual(plan);
  });
  it("round-trips a registry row", () => {
    const rule: HedgeVehicleRule = {
      id: "us-corp-7-10y",
      instrumentId: "",
      product: "BOND",
      ccy: "USD",
      minMaturityYears: 7,
      maxMaturityYears: 10,
      hedgeInstrumentId: "TY-DEC26",
      isFuture: true,
      dv01PerUnit: 78,
      unitLabel: "contract",
    };
    const wire = hedgeVehicleRuleToWire(rule);
    expect(wire["min_maturity_years"]).toBe(7);
    expect(wire["is_future"]).toBe(true);
    expect(hedgeVehicleRuleFromWire(wire)).toEqual(rule);
  });
  it("round-trips an exit-mode binding on the SHARED HedgeScopeKind ordinals", () => {
    const b: HedgeExitModeBinding = { scopeKind: "book", scopeId: "fi-credit-emea", mode: "suggest" };
    const wire = hedgeExitModeBindingToWire(b);
    expect(wire["scope_kind"]).toBe(1); // book — the same enum the LP panels use
    expect(wire["mode"]).toBe(1);
    expect(hedgeExitModeBindingFromWire(wire)).toEqual(b);
  });
});

describe("config carries the registry + the exit-mode bindings", () => {
  it("round-trips both new repeated fields through the SAME config message", () => {
    const c: HedgeConfig = {
      killSwitch: false,
      execution: "lp_panel_then_composite",
      deskEnabled: [],
      maxClip: 1_000,
      maxHedgesPerInterval: 5,
      dailyExternalNotionalCap: 0,
      lpPanels: [],
      compositeSpreadBp: 0.5,
      vehicles: [
        {
          id: "uk-gilt-3-7y",
          instrumentId: "",
          product: "BOND",
          ccy: "GBP",
          minMaturityYears: 3,
          maxMaturityYears: 7,
          hedgeInstrumentId: "G-MAR27",
          isFuture: true,
          dv01PerUnit: 64,
          unitLabel: "contract",
        },
      ],
      exitModes: [{ scopeKind: "book", scopeId: "fi-credit-emea", mode: "suggest" }],
      hedgingModels: [],
    };
    const wire = hedgeConfigToWire(c);
    expect((wire["vehicles"] as unknown[]).length).toBe(1);
    expect((wire["exit_modes"] as unknown[]).length).toBe(1);
    expect(hedgeConfigFromWire(wire)).toEqual(c);
  });
});

describe("provenance + intent carry the vehicle plan", () => {
  it("round-trips a provenance whose shed was sized in a vehicle", () => {
    const prov: HedgeProvenance = {
      hedgeId: "H-1",
      book: "fi-credit-emea",
      instrument: "XS2034-ACME-4H",
      firedAt: 1_700_000_000_000,
      metric: "dv01",
      threshold: 18_000,
      netRisk: 24_840,
      utilization: 1.38,
      band: "breach",
      policyPath: [0, 2, 3],
      action,
      internalCrossed: 0,
      externalHedged: 24_804,
      residual: 36,
      hedgePrice: 111.43,
      midAtFire: 111.42,
      slippageBp: 0.5,
      lpWon: "LP-1",
      advisory: false,
      lps: ["LP-1", "LP-3"],
      vehiclePlan: plan,
    };
    expect(hedgeProvenanceFromWire(hedgeProvenanceToWire(prov))).toEqual(prov);
  });
  it("round-trips an intent raised in SUGGEST mode", () => {
    const intent: HedgeIntent = {
      book: "fi-credit-emea",
      instrument: "XS2034-ACME-4H",
      action,
      band: "breach",
      netRisk: 24_840,
      threshold: 18_000,
      utilization: 1.38,
      overflow: 24_840,
      size: 24_840,
      internalCrossed: 0,
      externalHedged: 0,
      advisory: false,
      firedAt: 1_700_000_000_000,
      policyPath: [0, 2, 3],
      reason: "breach · suggest",
      lps: ["LP-1"],
      vehiclePlan: plan,
      exitMode: "suggest",
    };
    const wire = hedgeIntentToWire(intent);
    expect(wire["exit_mode"]).toBe(1);
    expect(hedgeIntentFromWire(wire)).toEqual(intent);
  });
});

describe("hedge suggestion round-trip + RPC framing", () => {
  it("rides raised_at as epoch NANOS while the GUI domain stays in millis", () => {
    const wire = hedgeSuggestionToWire(suggestion);
    expect(wire["raised_at"]).toBe(1_700_000_000_123_000_000);
    expect(hedgeSuggestionFromWire(wire).raisedAt).toBe(1_700_000_000_123);
  });
  it("round-trips the whole suggestion including the plan and the parent link", () => {
    expect(hedgeSuggestionFromWire(hedgeSuggestionToWire(suggestion))).toEqual(suggestion);
  });
  it("nulls parent_position_id when there is no single parent (never fabricated)", () => {
    const orphan: HedgeSuggestion = { ...suggestion, parentPositionId: null };
    const wire = hedgeSuggestionToWire(orphan);
    expect(wire["parent_position_id"]).toBeNull();
    expect(hedgeSuggestionFromWire(wire).parentPositionId).toBeNull();
  });
  it("frames list_hedge_suggestions with an OPTIONAL book filter", () => {
    expect(listHedgeSuggestionsRequestToWire()).toEqual({});
    expect(listHedgeSuggestionsRequestToWire("")).toEqual({});
    expect(listHedgeSuggestionsRequestToWire("fi-credit-emea")).toEqual({ book: "fi-credit-emea" });
  });
  it("decodes the suggestions reply envelope", () => {
    const reply = { suggestions: [hedgeSuggestionToWire(suggestion)] };
    expect(hedgeSuggestionsResponseFromWire(reply)).toEqual([suggestion]);
  });
  it("frames execute_hedge_suggestion with the dismiss flag", () => {
    expect(executeHedgeSuggestionRequestToWire("SUG-1", true)).toEqual({
      suggestion_id: "SUG-1",
      dismiss: true,
    });
  });
  it("decodes a DISMISS reply as a null provenance plus the remaining rows", () => {
    const back = executeHedgeSuggestionResponseFromWire({
      provenance: null,
      suggestions: [hedgeSuggestionToWire(suggestion)],
    });
    expect(back.provenance).toBeNull();
    expect(back.suggestions).toHaveLength(1);
  });
});
