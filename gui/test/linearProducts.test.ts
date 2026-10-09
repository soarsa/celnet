/**
 * W2 linear-product parity for the GUI end of the ONE `celnet.wire` contract: the
 * outright forward, FX swap and NDF are surfaced as registry {@link ProductSpec}s
 * (the 5th client of the W2 wave — SDK/CLI/Excel already carry them) and encoded
 * onto the SAME `Instrument.product` oneof as the option families, with the EXACT
 * appended wire field NAMES (`fx_forward`/`fx_swap`/`ndf`, proto field numbers
 * 26/27/28 — no schema_version, no renumber; GUIDE.md rule 9). These are LINEAR,
 * closed-form discounted cashflows (not option payoffs): no option type / strike /
 * vol — a contract rate, a notional and a Side.
 *
 * The test exercises the REAL `src/data/seed.ts` builders, the three `ProductSpec`
 * `toInstrument` builders and the real `src/data/wsCodec.ts` / `src/data/enums.ts`
 * through their public surface with NO server and NO mocks. It pins the wire shape
 * of each arm, the FX-swap near=spot(t=0)/far=tenor & opposite-side invariant, the
 * BUY/SELL direction, the NDF fixing identity, DEFAULT-model presence-omission,
 * and the new gallery group.
 */
import { describe, expect, it } from "vitest";

import { forwardSpec } from "../src/products/forward";
import { swapSpec } from "../src/products/swap";
import { ndfSpec } from "../src/products/ndf";
import { registryByGroup, type ProductBuildCtx } from "../src/products";
import { forwardInstrument, ndfInstrument, oppositeSide, swapInstrument } from "../src/data/seed";
import { instrumentToWire } from "../src/data/wsCodec";
import * as e from "../src/data/enums";
import type { CcyPair, FxForward, Ndf } from "../src/data/contract";

const PAIR: CcyPair = { base: "USD", quote: "KRW" };

/** A representative ctx with ATM-forward seeded above spot (so defaults ≠ spot). */
const CTX: ProductBuildCtx = {
  pair: PAIR,
  tenor: { unit: "MONTHS", count: 3 },
  tenorYears: 0.25,
  notionalMm: 10,
  pricingModel: "DEFAULT",
  atmForward: 1330.5,
  spot: 1325.0,
  pipDecimals: 2,
  today: { year: 2026, month: 6, day: 8 },
};

// ---------------------------------------------------------------------------
// wire encoding — the appended oneof arms, by their exact proto field NAMES
// ---------------------------------------------------------------------------

describe("W2 linear products — instrumentToWire oneof arms", () => {
  it("encodes an outright forward under `fx_forward` (field 26) with numeric Side", () => {
    const w = instrumentToWire(
      forwardInstrument(PAIR, 0.25, 10, { contractRate: 1330.0, side: "BUY" }),
    );
    expect(w["fx_forward"]).toEqual({
      contract_rate: 1330.0,
      notional: 10e6,
      // Side BUY=0, SELL=1.
      side: e.side.toWire("BUY"),
    });
    // A oneof carries exactly one body.
    expect(w["vanilla"]).toBeUndefined();
    expect(w["fx_swap"]).toBeUndefined();
    expect(w["ndf"]).toBeUndefined();
  });

  it("encodes an FX swap under `fx_swap` (field 27): near=spot, far=tenor, opposite sides", () => {
    const w = instrumentToWire(
      swapInstrument(PAIR, 0.25, 10, { nearRate: 1325.0, farRate: 1330.5, nearSide: "BUY" }),
    );
    expect(w["fx_swap"]).toEqual({
      near: { contract_rate: 1325.0, notional: 10e6, side: e.side.toWire("BUY") },
      far: { contract_rate: 1330.5, notional: 10e6, side: e.side.toWire("SELL") },
    });
  });

  it("encodes an NDF under `ndf` (field 28) with the FixingSource enum + settlement ccy", () => {
    const w = instrumentToWire(
      ndfInstrument(PAIR, 0.25, 10, {
        contractRate: 1330.0,
        side: "SELL",
        fixing: "KRW_KFTC18",
        settlementCcy: "KRW",
      }),
    );
    expect(w["ndf"]).toEqual({
      contract_rate: 1330.0,
      notional: 10e6,
      side: e.side.toWire("SELL"),
      // FixingSource KRW_KFTC18=0.
      fixing: e.fixingSource.toWire("KRW_KFTC18"),
      settlement_ccy: "KRW",
    });
  });

  it("pins the canonical FixingSource enum numbers the server decodes by", () => {
    expect(e.fixingSource.toWire("KRW_KFTC18")).toBe(0);
    expect(e.fixingSource.toWire("TWD_TAIPEI")).toBe(1);
    expect(e.fixingSource.toWire("INR_RBI_REF")).toBe(2);
    expect(e.fixingSource.toWire("BRL_PTAX")).toBe(3);
    expect(e.fixingSource.toWire("CLP_DOLAR_OBS")).toBe(4);
    expect(e.fixingSource.toWire("COP_TRM")).toBe(5);
    expect(e.fixingSource.fromWire(0)).toBe("KRW_KFTC18");
    expect(e.fixingSource.fromWire(5)).toBe("COP_TRM");
  });

  it("oppositeSide is an involution on the directional sides", () => {
    expect(oppositeSide("BUY")).toBe("SELL");
    expect(oppositeSide("SELL")).toBe("BUY");
    expect(oppositeSide(oppositeSide("BUY"))).toBe("BUY");
  });
});

// ---------------------------------------------------------------------------
// ProductSpec round-trip — defaults build the correct wire arm + DEFAULT-omit
// ---------------------------------------------------------------------------

describe("W2 linear products — ProductSpec toInstrument", () => {
  it("the forward spec books a `fxForward` at the ATM-forward default with DEFAULT omitted", () => {
    const inst = forwardSpec.toInstrument(forwardSpec.defaults, CTX);
    expect(inst.product.kind).toBe("fxForward");
    const f = (inst.product as { fxForward: FxForward }).fxForward;
    // A 0 contract rate defaults to the ATM-forward (an at-market, zero-PV trade).
    expect(f.contractRate).toBe(CTX.atmForward);
    expect(f.notional).toBe(CTX.notionalMm * 1e6);
    expect(f.side).toBe("BUY");
    expect(inst.tenor).toEqual(CTX.tenor);
    // DEFAULT model is presence-omitted on the wire (linear products are DCF-only).
    expect(inst.pricingModel).toBeUndefined();
    expect(forwardSpec.allowedModels).toEqual(["DEFAULT"]);
  });

  it("the swap spec books near=spot / far=ATMF with the far side derived opposite", () => {
    const inst = swapSpec.toInstrument(swapSpec.defaults, CTX);
    expect(inst.product.kind).toBe("fxSwap");
    if (inst.product.kind !== "fxSwap") throw new Error("expected fxSwap");
    const { near, far } = inst.product.fxSwap;
    // Near defaults to spot (its settlement t=0 at-market rate), far to the ATM-forward.
    expect(near.contractRate).toBe(CTX.spot);
    expect(far.contractRate).toBe(CTX.atmForward);
    expect(near.side).toBe("BUY");
    expect(far.side).toBe("SELL"); // opposite of near
    expect(near.notional).toBe(CTX.notionalMm * 1e6);
    expect(far.notional).toBe(CTX.notionalMm * 1e6);
    expect(inst.pricingModel).toBeUndefined();
  });

  it("the NDF spec books an `ndf` with the fixing identity + quote-ccy settlement", () => {
    const inst = ndfSpec.toInstrument(ndfSpec.defaults, CTX);
    expect(inst.product.kind).toBe("ndf");
    const n = (inst.product as { ndf: Ndf }).ndf;
    expect(n.contractRate).toBe(CTX.atmForward);
    expect(n.side).toBe("BUY");
    expect(n.fixing).toBe("KRW_KFTC18");
    // Cash-settles in the convertible (quote) leg.
    expect(n.settlementCcy).toBe(CTX.pair.quote);
    expect(inst.pricingModel).toBeUndefined();
  });

  it("a SELL forward flips only the direction, leaving the rate/notional intact", () => {
    const inst = forwardSpec.toInstrument({ contractRate: 1331.0, side: "SELL" }, CTX);
    const f = (inst.product as { fxForward: FxForward }).fxForward;
    expect(f.side).toBe("SELL");
    expect(f.contractRate).toBe(1331.0);
  });

  it("every linear spec round-trips deterministically through the real wire codec", () => {
    const insts = [
      forwardSpec.toInstrument(forwardSpec.defaults, CTX),
      swapSpec.toInstrument(swapSpec.defaults, CTX),
      ndfSpec.toInstrument(ndfSpec.defaults, CTX),
    ];
    for (const inst of insts) {
      const wire = instrumentToWire(inst);
      expect(wire).toBeTruthy();
      expect(instrumentToWire(inst)).toEqual(wire);
    }
  });
});

// ---------------------------------------------------------------------------
// gallery grouping — the new "Linear (forwards & swaps)" group
// ---------------------------------------------------------------------------

describe("W2 linear products — gallery group", () => {
  it("the three specs appear under the new 'Linear (forwards & swaps)' group", () => {
    const grouped = registryByGroup();
    const linear = grouped.find((g) => g.group === "Linear (forwards & swaps)");
    expect(linear).toBeDefined();
    const ids = linear!.specs.map((s) => s.id);
    expect(ids).toEqual(["FX_FORWARD", "FX_SWAP", "NDF"]);
  });

  it("the linear group sits right after 'Vanilla & strategies' in canonical order", () => {
    const groups = registryByGroup().map((g) => g.group);
    expect(groups.indexOf("Linear (forwards & swaps)")).toBe(
      groups.indexOf("Vanilla & strategies") + 1,
    );
  });
});
