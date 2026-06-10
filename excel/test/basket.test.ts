// PC-BASKET — Excel client parity for the correlated multi-asset basket /
// best-of / worst-of option (proto `BasketOption`, product field 25). These tests
// prove the add-in:
//   1. parses the trader-facing aggregation-kind selector (BASKET / BEST_OF /
//      WORST_OF with aliases like MAX/MIN), case-insensitive, rejecting garbage;
//   2. shapes the legs matrix `[pair, weight, spot, vol, rFor]` and the N×N
//      correlation range into the typed BasketOption, validating leg/matrix shape
//      locally (the SPD check is the server's);
//   3. encodes the oneof under the EXACT `basket` product key the server's
//      crates/celnet-server/src/ws/codec.rs `basket_from_json` decodes — a `legs`
//      array of `{pair, weight, spot, vol, r_for}`, a row-major `correlations`
//      array, the option/kind enum numbers, the strike, and the MC knobs (64-bit
//      seed as a JSON number, the codec's convention).
// No pricing math lives in the add-in — the numbers are the server's libm-core
// multi-asset Monte-Carlo values (gated by celnet-server's
// degenerate_basket_matches_vanilla_on_the_wire + celnet-parity's basket row
// against an independent Levy moment-matched oracle), so a cell is bit-identical
// to the SDK/CLI.

import { describe, expect, it } from "vitest";
import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  formatPremiumSpill,
  parseBasketKind,
  shapeBasket,
} from "../src/functions/shaping";
import { instrumentToWire } from "../src/contract/wsCodec";
import { basketKind as basketKindCodec, optionType as optionTypeCodec } from "../src/contract/enums";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import type { BasketArgs } from "../src/functions/shaping";
import type { Instrument } from "../src/contract/contract";

describe("basket-kind argument parsing", () => {
  it("defaults to BASKET and accepts the aliases case-insensitively", () => {
    expect(parseBasketKind(undefined)).toBe("BASKET");
    expect(parseBasketKind("")).toBe("BASKET");
    expect(parseBasketKind("weighted")).toBe("BASKET");
    expect(parseBasketKind("Best-Of")).toBe("BEST_OF");
    expect(parseBasketKind("MAX")).toBe("BEST_OF");
    expect(parseBasketKind("worst")).toBe("WORST_OF");
    expect(parseBasketKind("min")).toBe("WORST_OF");
  });

  it("rejects garbage with a ShapingError", () => {
    expect(() => parseBasketKind("rainbow-of-doom")).toThrow(ShapingError);
  });
});

describe("basket-kind wire codec (proto enum numbers)", () => {
  it("maps BASKET↔0, BEST_OF↔1, WORST_OF↔2 reversibly", () => {
    expect(basketKindCodec.toWire("BASKET")).toBe(0);
    expect(basketKindCodec.toWire("BEST_OF")).toBe(1);
    expect(basketKindCodec.toWire("WORST_OF")).toBe(2);
    expect(basketKindCodec.fromWire(2)).toBe("WORST_OF");
    // Unknown clamps to the proto3 zero value (BASKET).
    expect(basketKindCodec.fromWire(9)).toBe("BASKET");
  });
});

describe("basket shaping + wire encoding (proto field-25 product contract)", () => {
  const args: BasketArgs = {
    pair: "EURUSD",
    tenor: "1Y",
    notional: 1_000_000,
    callPut: "call",
    strike: 1.18,
    kind: "worst-of",
    legs: [
      ["EURUSD", 0.5, 1.1, 0.11, 0.015],
      ["GBPUSD", 0.5, 1.27, 0.13, 0.02],
    ],
    correlations: [
      [1.0, 0.4],
      [0.4, 1.0],
    ],
    mcPaths: 8192,
    mcReplications: 16,
    mcSteps: 1,
    mcSeed: 0xc0ffee,
  };

  it("shapes the typed BasketOption from the legs matrix + correlation range", () => {
    const instr = shapeBasket(args);
    expect(instr.product.kind).toBe("basket");
    if (instr.product.kind !== "basket") throw new Error("unreachable");
    const b = instr.product.basket;
    expect(b.legs).toHaveLength(2);
    expect(b.legs[0]!.pair).toEqual({ base: "EUR", quote: "USD" });
    expect(b.legs[1]!.spot).toBe(1.27);
    expect(b.correlations).toEqual([1.0, 0.4, 0.4, 1.0]);
    expect(b.kind).toBe("WORST_OF");
    expect(b.optionType).toBe("CALL");
    expect(b.strike).toBe(1.18);
  });

  it("encodes the EXACT `basket` wire shape the server's basket_from_json decodes", () => {
    const wire = instrumentToWire(shapeBasket(args)) as Record<string, unknown>;
    const basket = wire["basket"] as Record<string, unknown>;
    expect(basket).toBeDefined();
    const legs = basket["legs"] as Array<Record<string, unknown>>;
    expect(legs).toHaveLength(2);
    expect(legs[0]).toEqual({
      pair: { base: "EUR", quote: "USD" },
      weight: 0.5,
      spot: 1.1,
      vol: 0.11,
      r_for: 0.015,
    });
    expect(basket["correlations"]).toEqual([1.0, 0.4, 0.4, 1.0]);
    expect(basket["option_type"]).toBe(optionTypeCodec.toWire("CALL"));
    expect(basket["kind"]).toBe(basketKindCodec.toWire("WORST_OF"));
    expect(basket["strike"]).toBe(1.18);
    expect(basket["mc_paths"]).toBe(8192);
    expect(basket["mc_replications"]).toBe(16);
    expect(basket["mc_steps"]).toBe(1);
    expect(basket["mc_seed"]).toBe(0xc0ffee);
  });

  it("rejects a non-square correlation matrix locally", () => {
    expect(() => shapeBasket({ ...args, correlations: [[1.0, 0.4]] })).toThrow(ShapingError);
  });

  it("rejects a malformed leg row locally", () => {
    expect(() => shapeBasket({ ...args, legs: [["EURUSD", 0.5, 1.1]] as never })).toThrow(
      ShapingError,
    );
  });
});

// ---------------------------------------------------------------------------
// End-to-end over the WS mirror — the server emits the multi-asset MC std-error
// and a price-only (zeroed) Greek strip; the spill renders premium + std_error +
// the 13 Greeks (honestly 0) + a convention footer.
// ---------------------------------------------------------------------------

class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((data: string) => void) | null = null;
  readonly sent: Record<string, unknown>[] = [];
  send(data: string): void {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close(): void {
    this.readyState = 3;
    this.onclose?.();
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.();
  }
  sentOfType(type: string): Record<string, unknown>[] {
    return this.sent.filter((f) => f["type"] === type);
  }
}

function makeWiredConnection(): { conn: Connection; sock: FakeSocket } {
  const sock = new FakeSocket();
  const conn = new Connection({ url: "ws://test", factory: () => sock, requestTimeoutMs: 5000 });
  sock.open();
  return { conn, sock };
}

/**
 * Reply to the outstanding request_quote with a SERVER-SHAPED quote — the exact
 * `quote` frame the server's WS mirror emits. A basket is multi-asset Monte-Carlo
 * priced, so the server genuinely stamps `price_std_error` (proto field 7); the
 * Greek strip is the server's price-only strip (multi-asset Greeks deferred), so
 * `greeks` carries only `price` and the decoder fills the rest as 0.
 */
function replyQuote(sock: FakeSocket, body: Record<string, unknown>): void {
  const sent = sock.sentOfType("request_quote").at(-1)!;
  sock.onmessage?.(
    JSON.stringify({
      type: "quote",
      correlation_id: sent["correlation_id"],
      quote_id: 1,
      idempotency_key: "k",
      price: { bid: 0, offer: 0 },
      greeks: {},
      conventions: {},
      resolved_strike: 0,
      epoch_nanos: 0,
      valid_until_nanos: 0,
      ...body,
    }),
  );
}

describe("basket end-to-end over the WS mirror — server emits the MC std-error", () => {
  const args: BasketArgs = {
    pair: "EURUSD",
    tenor: "1Y",
    notional: 1_000_000,
    callPut: "call",
    strike: 1.18,
    kind: "basket",
    legs: [
      ["EURUSD", 0.5, 1.1, 0.11, 0.015],
      ["GBPUSD", 0.5, 1.27, 0.13, 0.02],
    ],
    correlations: [
      [1.0, 0.4],
      [0.4, 1.0],
    ],
    mcPaths: 8192,
    mcReplications: 16,
    mcSteps: 1,
    mcSeed: 0xc0ffee,
  };

  it("sends a basket oneof (field 25) and renders premium + MC std-error + zeroed Greeks", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr: Instrument = shapeBasket(args);
    const p = conn.requestQuote(instr, DEFAULT_CONVENTIONS, "basket");

    // The wire carries the EXACT `basket` body the server's basket_from_json decodes.
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<
      string,
      unknown
    >;
    const basket = wireInstr["basket"] as Record<string, unknown>;
    expect(basket).toBeDefined();
    const legs = basket["legs"] as Array<Record<string, unknown>>;
    expect(legs).toHaveLength(2);
    expect(legs[1]).toEqual({
      pair: { base: "GBP", quote: "USD" },
      weight: 0.5,
      spot: 1.27,
      vol: 0.13,
      r_for: 0.02,
    });
    expect(basket["correlations"]).toEqual([1.0, 0.4, 0.4, 1.0]);
    expect(basket["kind"]).toBe(basketKindCodec.toWire("BASKET"));
    expect(basket["option_type"]).toBe(optionTypeCodec.toWire("CALL"));
    expect(basket["strike"]).toBe(1.18);
    expect(basket["mc_paths"]).toBe(8192);
    expect(basket["mc_seed"]).toBe(0xc0ffee);

    // ---------------------------------------------------------------------
    // Hand-pinned 2-asset reference (Lesson c): the server's multi-asset MC
    // premium is gated by celnet-parity's `basket` row against an INDEPENDENT
    // Levy (1992) lognormal moment-matched basket-call oracle. For the legs
    // (0.5, 1.10, 0.11, 0.015) + (0.5, 1.27, 0.13, 0.020), rho = 0.4, r_d = 0.02,
    // T = 1, K = 1.18 the Levy oracle is PINNED_LEVY = 0.0508837560 and the engine
    // MC is 0.0508708 (gap ~ 1.3e-5 ~ 0.025%, inside the documented 0.5%-relative
    // Levy approximation band + MC stderr). The add-in does NO pricing math — it
    // renders whatever the server stamped, bit-identical to the SDK/CLI — so we
    // reply with the pinned reference premium and assert it spills verbatim.
    //   E. Levy, "Pricing European average rate currency options,"
    //   J. Int. Money & Finance 11 (1992).
    // ---------------------------------------------------------------------
    const PINNED_LEVY = 0.050_883_756_0;
    const stdErr = 1.4e-4;
    replyQuote(sock, {
      greeks: { price: PINNED_LEVY },
      resolved_strike: 1.18,
      price_std_error: stdErr,
    });
    const quote = await p;
    expect(quote.greeks.price).toBe(PINNED_LEVY);
    expect(quote.priceStdError).toBe(stdErr);
    // The Greek strip is the server's price-only (zeroed) strip — honestly 0.
    expect(quote.greeks.deltaSpot).toBe(0);
    expect(quote.greeks.vega).toBe(0);
    expect(quote.greeks.gamma).toBe(0);

    const m = formatPremiumSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
    // premium, then the honest MC std-error row, then the zeroed Greek rows.
    expect(m[0]).toEqual(["premium", PINNED_LEVY]);
    expect(m[1]).toEqual(["std_error", stdErr]);
    expect(m[2]).toEqual(["delta_spot", 0]);
    // A footer row trails the 13 Greeks (premium + std_error + 13 + footer = 16).
    expect(m).toHaveLength(16);
  });

  it("a worst-of basket sends kind=2 (WORST_OF) on the wire", async () => {
    const { conn, sock } = makeWiredConnection();
    const instr = shapeBasket({ ...args, kind: "worst-of" });
    void conn.requestQuote(instr, DEFAULT_CONVENTIONS, "basket-worst");
    const wireInstr = sock.sentOfType("request_quote").at(-1)!["instrument"] as Record<
      string,
      unknown
    >;
    const basket = wireInstr["basket"] as Record<string, unknown>;
    expect(basket["kind"]).toBe(2);
    expect(basket["kind"]).toBe(basketKindCodec.toWire("WORST_OF"));
  });
});
