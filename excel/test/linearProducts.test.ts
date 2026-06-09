// W2 — Excel client parity for the linear FX book (outright forward, FX swap, NDF)
// the Rust slice put on the ONE wire contract. These tests prove the add-in shapes
// each product into the EXACT wire shape the server's
// `crates/celnet-server/src/ws/codec.rs` decodes — the product oneof keys
// `fx_forward` / `fx_swap` / `ndf` (proto field numbers 26 / 27 / 28), each body in
// snake_case with the directional `side` (and the NDF `fixing` / `settlement_ccy`)
// the server reads. No pricing math lives in the add-in: a cell is bit-identical to
// the SDK/CLI (whose own gates reconcile the server linear pricer to the
// `celnet-linear` closed forms vs an independent oracle).

import { describe, expect, it } from "vitest";
import {
  ShapingError,
  shapeForward,
  shapeForwardSide,
  shapeFixingSource,
  shapeNdf,
  shapeSwap,
} from "../src/functions/shaping";
import { instrumentToWire } from "../src/contract/wsCodec";
import * as e from "../src/contract/enums";

// ---------------------------------------------------------------------------
// argument parsing
// ---------------------------------------------------------------------------

describe("linear side / fixing parsing", () => {
  it("parses the directional side with a BUY default and rejects garbage", () => {
    expect(shapeForwardSide(undefined)).toBe("BUY");
    expect(shapeForwardSide("buy")).toBe("BUY");
    expect(shapeForwardSide("B")).toBe("BUY");
    expect(shapeForwardSide("sell")).toBe("SELL");
    expect(shapeForwardSide("S")).toBe("SELL");
    // A linear product needs a definite side — TWO_WAY is not a direction.
    expect(() => shapeForwardSide("two_way")).toThrow(ShapingError);
  });

  it("parses the fixing identity from the code / token / wire member", () => {
    expect(shapeFixingSource("BRL.PTAX")).toBe("BRL_PTAX");
    expect(shapeFixingSource("brlptax")).toBe("BRL_PTAX");
    expect(shapeFixingSource("PTAX")).toBe("BRL_PTAX");
    expect(shapeFixingSource("BRL_PTAX")).toBe("BRL_PTAX");
    expect(shapeFixingSource("COP.TRM")).toBe("COP_TRM");
    expect(shapeFixingSource("INR.RBIB")).toBe("INR_RBI_REF");
    expect(() => shapeFixingSource("not-a-fixing")).toThrow(ShapingError);
  });
});

// ---------------------------------------------------------------------------
// instrument shaping → wire encoding (the contract field-number contract)
// ---------------------------------------------------------------------------

describe("FX outright forward shaping + wire encoding", () => {
  it("shapes a directional BUY forward with the product side mirroring the top-level side", () => {
    const i = shapeForward({ pair: "EURUSD", tenor: "1Y", contractRate: 1.1, notional: 1e6 });
    expect(i.pair).toEqual({ base: "EUR", quote: "USD" });
    // A linear product carries the DIRECTIONAL side at the top level (not TWO_WAY).
    expect(i.side).toBe("BUY");
    expect(i.quantity).toEqual({ notional: 1e6, baseCcy: true });
    expect(i.product).toEqual({
      kind: "fxForward",
      fxForward: { contractRate: 1.1, notional: 1e6, side: "BUY" },
    });
  });

  it("encodes the oneof under `fx_forward` (proto field 26) with snake_case body + numeric side", () => {
    const i = shapeForward({
      pair: "EURUSD",
      tenor: "1Y",
      contractRate: 1.08,
      notional: 1e6,
      side: "SELL",
    });
    const wire = instrumentToWire(i);
    expect(wire["fx_forward"]).toEqual({
      contract_rate: 1.08,
      notional: 1e6,
      side: e.side.toWire("SELL"), // SELL = 1
    });
    // Exactly one product arm is present (oneof discipline).
    expect("vanilla" in wire).toBe(false);
    expect("fx_swap" in wire).toBe(false);
    expect("ndf" in wire).toBe(false);
  });

  it("rejects a non-positive notional / contract rate", () => {
    expect(() => shapeForward({ pair: "EURUSD", tenor: "1Y", contractRate: 1.1, notional: 0 })).toThrow(
      ShapingError,
    );
    expect(() => shapeForward({ pair: "EURUSD", tenor: "1Y", contractRate: 0, notional: 1e6 })).toThrow(
      ShapingError,
    );
  });
});

describe("FX swap shaping + wire encoding", () => {
  it("forms the far leg as the opposite side at the same contract rate / notional", () => {
    const i = shapeSwap({
      pair: "EURUSD",
      tenor: "1Y",
      contractRate: 1.3,
      notional: 2.5e6,
      side: "BUY",
    });
    expect(i.side).toBe("BUY");
    expect(i.product).toEqual({
      kind: "fxSwap",
      fxSwap: {
        near: { contractRate: 1.3, notional: 2.5e6, side: "BUY" },
        far: { contractRate: 1.3, notional: 2.5e6, side: "SELL" },
      },
    });
  });

  it("encodes the oneof under `fx_swap` (proto field 27) with near + far forward bodies", () => {
    const i = shapeSwap({ pair: "USDJPY", tenor: "18M", contractRate: 150, notional: 1e6, side: "SELL" });
    const wire = instrumentToWire(i);
    expect(wire["fx_swap"]).toEqual({
      near: { contract_rate: 150, notional: 1e6, side: e.side.toWire("SELL") },
      far: { contract_rate: 150, notional: 1e6, side: e.side.toWire("BUY") },
    });
    expect("fx_forward" in wire).toBe(false);
    expect("variance_swap" in wire).toBe(false);
  });
});

describe("NDF shaping + wire encoding", () => {
  it("shapes an NDF with the fixing identity + default USD settlement", () => {
    const i = shapeNdf({
      pair: "USDBRL",
      tenor: "6M",
      contractRate: 5.1,
      notional: 1e6,
      fixing: "BRL.PTAX",
    });
    expect(i.side).toBe("BUY");
    expect(i.product).toEqual({
      kind: "ndf",
      ndf: {
        contractRate: 5.1,
        notional: 1e6,
        side: "BUY",
        fixing: "BRL_PTAX",
        settlementCcy: "USD",
      },
    });
  });

  it("encodes the oneof under `ndf` (proto field 28) with numeric side + fixing tags", () => {
    const i = shapeNdf({
      pair: "USDINR",
      tenor: "1Y",
      contractRate: 84,
      notional: 1e6,
      fixing: "INR.RBIB",
      settlementCcy: "USD",
      side: "SELL",
    });
    const wire = instrumentToWire(i);
    expect(wire["ndf"]).toEqual({
      contract_rate: 84,
      notional: 1e6,
      side: e.side.toWire("SELL"),
      fixing: e.fixingSource.toWire("INR_RBI_REF"),
      settlement_ccy: "USD",
    });
    expect("fx_forward" in wire).toBe(false);
  });

  it("rejects an unknown fixing", () => {
    expect(() =>
      shapeNdf({ pair: "USDBRL", tenor: "6M", contractRate: 5.1, notional: 1e6, fixing: "NOPE" }),
    ).toThrow(ShapingError);
  });
});
