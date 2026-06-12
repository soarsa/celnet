/**
 * The class-aware instrument BUILDER — headless state + logic gate
 * (`src/taskpane/instrumentBuilder.ts`). This proves the four contract
 * obligations of the ticket model that replaces the pane's hardcoded FX-vanilla
 * flow:
 *
 *  1. CAPABILITY: each asset class builds its priceable arms (FX/metal all 24;
 *     equity/commodity/crypto VANILLA + PERPETUAL + FUTUREOPTION only), and an
 *     FX-only arm on a cross-asset underlier returns the SAME typed guard error
 *     the `CELNET.INSTRUMENT` cell + the server's `price_cross_asset` reject with.
 *
 *  2. PARITY: the FX-vanilla path is BYTE-IDENTICAL to the retired
 *     `shapeVanillaInstrument` flow — the wire frame and its canonical token
 *     match, so the class-aware ticket is a behaviour-preserving refactor.
 *
 *  3. UNDERLIER GRAMMAR: the composed one-string underlier routes each class to
 *     its `Underlying` arm (the metal X-code base, the equity venue, the empty
 *     commodity venue, the crypto inverse suffix) through the EXISTING
 *     `parseUnderlier` — never a reimplemented classification.
 *
 *  4. STATE LOGIC: the immutable updaters are non-destructive, a class switch
 *     re-homes a now-unpriceable arm, blank required fields fail with typed
 *     trader-readable messages, and a build mistake is a {@link BuildFailure}
 *     (never a thrown exception the controller must catch).
 */
import { describe, expect, it } from "vitest";

import type { Instrument } from "../src/contract/contract";
import { canonicalWireJson } from "../src/contract/instrumentCodec";
import { instrumentToWire } from "../src/contract/wsCodec";
import {
  decodeInstrumentToken,
  encodeInstrumentToken,
  shapeSpecInstrument,
} from "../src/functions/instrumentSpec";
import { shapeVanillaInstrument } from "../src/functions/shaping";
import {
  ALL_ARMS,
  ASSET_CLASSES,
  availableArms,
  buildInstrument,
  composeUnderlier,
  CROSS_ASSET_ARMS,
  INITIAL_BUILDER_STATE,
  isArmPriceable,
  selectArm,
  selectAssetClass,
  setNotional,
  setTenor,
  setTerms,
  toSpecArgs,
  updateUnderlier,
  type AssetClass,
  type BuilderState,
} from "../src/taskpane/instrumentBuilder";

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/** A built instrument (asserts the build succeeded, returning the wire shape). */
function built(state: BuilderState): { instrument: Instrument; token: string } {
  const result = buildInstrument(state);
  if (!result.ok) throw new Error(`expected build success, got error: ${result.error}`);
  return { instrument: result.instrument, token: result.token };
}

/** The typed error message a failed build surfaces (asserts the build failed). */
function failure(state: BuilderState): string {
  const result = buildInstrument(state);
  if (result.ok) throw new Error("expected build failure, got a built instrument");
  return result.error;
}

/** A complete, priceable VANILLA ticket on the given class + underlier inputs. */
function vanillaTicket(
  cls: AssetClass,
  underlier: Partial<BuilderState["underlier"]>,
): BuilderState {
  return {
    assetClass: cls,
    underlier: { ...INITIAL_BUILDER_STATE.underlier, ...underlier },
    arm: "VANILLA",
    terms: [
      ["strike", "ATM"],
      ["callPut", "C"],
    ],
    tenor: "1Y",
    notional: "1",
  };
}

// ---------------------------------------------------------------------------
// 1. the executable capability matrix
// ---------------------------------------------------------------------------

describe("capability matrix — available arms per class", () => {
  it("FX and METAL price all 24 arms", () => {
    expect(ALL_ARMS).toHaveLength(27); // 24 product oneof arms + 3 strategy template names
    expect(availableArms("FX")).toBe(ALL_ARMS);
    expect(availableArms("METAL")).toBe(ALL_ARMS);
  });

  it("equity / commodity / crypto price only the cost-of-carry leaves", () => {
    expect(CROSS_ASSET_ARMS).toEqual(["VANILLA", "PERPETUAL", "FUTUREOPTION"]);
    for (const cls of ["EQUITY", "COMMODITY", "CRYPTO"] as const) {
      expect(availableArms(cls)).toBe(CROSS_ASSET_ARMS);
    }
  });

  it("isArmPriceable answers the matrix (case-insensitive)", () => {
    expect(isArmPriceable("barrier", "FX")).toBe(true);
    expect(isArmPriceable("BARRIER", "EQUITY")).toBe(false);
    expect(isArmPriceable("vanilla", "CRYPTO")).toBe(true);
    expect(isArmPriceable("perpetual", "COMMODITY")).toBe(true);
    expect(isArmPriceable("futureoption", "EQUITY")).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// 2. each class builds its priceable arms (end-to-end through shapeSpecInstrument)
// ---------------------------------------------------------------------------

describe("each class builds its priceable arms", () => {
  it("FX builds a vanilla", () => {
    const { instrument } = built(vanillaTicket("FX", { pair: "EURUSD" }));
    expect(instrument.product.kind).toBe("vanilla");
    expect(instrument.pair).toEqual({ base: "EUR", quote: "USD" });
    expect(instrument.underlying).toBeUndefined(); // FX carries no underlying arm
  });

  it("METAL builds a vanilla on the metal arm", () => {
    const { instrument } = built(vanillaTicket("METAL", { pair: "XAUUSD" }));
    expect(instrument.product.kind).toBe("vanilla");
    expect(instrument.underlying?.kind).toBe("metal");
  });

  it("METAL prices the full FX engine arms (a barrier)", () => {
    const state: BuilderState = {
      ...vanillaTicket("METAL", { pair: "XAUUSD" }),
      arm: "BARRIER",
      terms: [
        ["strike", "ATM"],
        ["callPut", "C"],
        ["barrier", 2200],
        ["kind", "KNOCK_OUT"],
        ["side", "UP"],
      ],
    };
    const { instrument } = built(state);
    expect(instrument.product.kind).toBe("singleBarrier");
    expect(instrument.underlying?.kind).toBe("metal");
  });

  it("EQUITY builds a vanilla on the equity arm (ticker@venue:ccy)", () => {
    const { instrument } = built(
      vanillaTicket("EQUITY", { ticker: "AAPL", venue: "XNAS", currency: "USD" }),
    );
    expect(instrument.product.kind).toBe("vanilla");
    expect(instrument.underlying?.kind).toBe("equity");
  });

  it("COMMODITY builds a vanilla on the commodity arm (empty venue)", () => {
    const { instrument } = built(
      vanillaTicket("COMMODITY", { ticker: "BRENT", currency: "USD" }),
    );
    expect(instrument.product.kind).toBe("vanilla");
    expect(instrument.underlying?.kind).toBe("commodity");
  });

  it("CRYPTO builds a vanilla on the digital-asset arm", () => {
    const { instrument } = built(vanillaTicket("CRYPTO", { base: "BTC", quote: "USD" }));
    expect(instrument.product.kind).toBe("vanilla");
    expect(instrument.underlying?.kind).toBe("digitalAsset");
    expect(instrument.settlementStyle).toBeUndefined(); // LINEAR is presence-omitted
  });

  it("CRYPTO carries INVERSE_COIN settlement when selected", () => {
    const state = vanillaTicket("CRYPTO", {
      base: "BTC",
      quote: "USD",
      settlement: "INVERSE_COIN",
    });
    const { instrument } = built(state);
    expect(instrument.settlementStyle).toBe("INVERSE_COIN");
  });

  it("EQUITY builds the asset-class-agnostic arms (perpetual, future-option)", () => {
    const perp: BuilderState = {
      ...vanillaTicket("EQUITY", { ticker: "AAPL", venue: "XNAS", currency: "USD" }),
      arm: "PERPETUAL",
      tenor: "", // the no-expiry arm takes no tenor
      terms: [
        ["strike", 190],
        ["callPut", "C"],
      ],
    };
    expect(built(perp).instrument.product.kind).toBe("perpetualOption");

    const fut: BuilderState = {
      ...vanillaTicket("COMMODITY", { ticker: "BRENT", currency: "USD" }),
      arm: "FUTUREOPTION",
      terms: [
        ["strike", 80],
        ["callPut", "C"],
        ["futureSymbol", "BRN@IFEU"],
        ["futureExpiry", 1.0],
      ],
    };
    expect(built(fut).instrument.product.kind).toBe("listedFutureOption");
  });
});

// ---------------------------------------------------------------------------
// 3. the cross-asset guard — an FX-only arm on a cross-asset underlier
// ---------------------------------------------------------------------------

describe("cross-asset capability guard", () => {
  it("an FX-only arm on a cross-asset underlier yields the typed guard error", () => {
    // The build proceeds (the arm `<select>` would normally filter it, but a
    // programmatic / stale arm reaches the build) and the SAME guard the cell runs
    // refuses it with the honest asset-class sentence.
    const state: BuilderState = {
      ...vanillaTicket("EQUITY", { ticker: "AAPL", venue: "XNAS", currency: "USD" }),
      arm: "BARRIER",
      terms: [
        ["strike", "ATM"],
        ["callPut", "C"],
        ["barrier", 200],
      ],
    };
    const message = failure(state);
    expect(message).toContain("equity");
    expect(message).toContain("VANILLA, PERPETUAL and");
    expect(message).toContain("FX/metal-only");
  });

  it("the guard error matches the cell's `shapeSpecInstrument` rejection verbatim", () => {
    const state: BuilderState = {
      ...vanillaTicket("CRYPTO", { base: "BTC", quote: "USD" }),
      arm: "TARF",
      terms: [
        ["strike", "ATM"],
        ["callPut", "C"],
        ["target", 0.3],
        ["leverage", 2],
        ["fixings", 12],
      ],
    };
    const fromBuilder = failure(state);
    let fromCell = "";
    try {
      shapeSpecInstrument(toSpecArgs(state));
    } catch (err) {
      fromCell = (err as Error).message;
    }
    expect(fromBuilder).toBe(fromCell);
    expect(fromBuilder).toContain("crypto");
  });

  it("selectArm refuses a non-priceable arm for the class (loud, not silent)", () => {
    const equity = selectAssetClass(INITIAL_BUILDER_STATE, "EQUITY");
    expect(() => selectArm(equity, "BARRIER")).toThrow(/not priceable on EQUITY/);
  });
});

// ---------------------------------------------------------------------------
// 4. PARITY — the FX path stays byte-identical to shapeVanillaInstrument
// ---------------------------------------------------------------------------

describe("FX-vanilla parity with the retired shapeVanillaInstrument flow", () => {
  it("emits the byte-identical wire frame + token", () => {
    const state = vanillaTicket("FX", { pair: "EURUSD" });
    state; // (the ticket the pane builds)
    const fromBuilder = built({
      ...state,
      terms: [
        ["strike", 1.12],
        ["callPut", "C"],
      ],
    });

    const legacy = shapeVanillaInstrument({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.12,
      callPut: "C",
      notional: 1,
    });

    // Key-for-key deep equality of the encoded frames…
    expect(instrumentToWire(fromBuilder.instrument)).toEqual(instrumentToWire(legacy));
    // …and byte equality of the canonical serialization (the token form)…
    expect(fromBuilder.token).toBe(canonicalWireJson(instrumentToWire(legacy)));
    // …and the token round-trip is lossless.
    expect(encodeInstrumentToken(decodeInstrumentToken(fromBuilder.token))).toBe(
      fromBuilder.token,
    );
  });

  it("the INITIAL_BUILDER_STATE is a complete FX-vanilla ticket (the legacy default)", () => {
    const { instrument } = built(INITIAL_BUILDER_STATE);
    expect(instrument.product.kind).toBe("vanilla");
    expect(instrument.underlying).toBeUndefined();
    expect(instrument.pair).toEqual({ base: "EUR", quote: "USD" });
  });
});

// ---------------------------------------------------------------------------
// 5. the composed one-string underlier (delegates to the parseUnderlier grammar)
// ---------------------------------------------------------------------------

describe("composeUnderlier — the per-class one-string grammar", () => {
  it("composes each class to its canonical string", () => {
    expect(composeUnderlier(vanillaTicket("FX", { pair: "eur/usd" }))).toBe("eur/usd");
    expect(composeUnderlier(vanillaTicket("METAL", { pair: "XAUUSD" }))).toBe("XAUUSD");
    expect(
      composeUnderlier(
        vanillaTicket("EQUITY", { ticker: "AAPL", venue: "XNAS", currency: "USD" }),
      ),
    ).toBe("AAPL@XNAS:USD");
    expect(
      composeUnderlier(vanillaTicket("COMMODITY", { ticker: "BRENT", currency: "USD" })),
    ).toBe("BRENT@:USD");
    expect(composeUnderlier(vanillaTicket("CRYPTO", { base: "BTC", quote: "USD" }))).toBe(
      "BTC/USD",
    );
    expect(
      composeUnderlier(
        vanillaTicket("CRYPTO", { base: "BTC", quote: "USD", settlement: "INVERSE_COIN" }),
      ),
    ).toBe("BTC/USD:inverse");
  });

  it("a blank required field fails with a typed, field-named message", () => {
    expect(failure(vanillaTicket("FX", { pair: "" }))).toContain("pair");
    expect(failure(vanillaTicket("EQUITY", { ticker: "", venue: "XNAS", currency: "USD" }))).toContain(
      "ticker",
    );
    expect(
      failure(vanillaTicket("EQUITY", { ticker: "AAPL", venue: "", currency: "USD" })),
    ).toContain("venue");
    expect(failure(vanillaTicket("CRYPTO", { base: "BTC", quote: "" }))).toContain("quote leg");
  });
});

// ---------------------------------------------------------------------------
// 6. the immutable state updaters
// ---------------------------------------------------------------------------

describe("builder state updates", () => {
  it("ASSET_CLASSES lists every class", () => {
    expect([...ASSET_CLASSES]).toEqual(["FX", "METAL", "EQUITY", "COMMODITY", "CRYPTO"]);
  });

  it("selecting a class preserves a priceable arm", () => {
    const fx: BuilderState = { ...INITIAL_BUILDER_STATE, arm: "VANILLA" };
    expect(selectAssetClass(fx, "EQUITY").arm).toBe("VANILLA");
  });

  it("selecting a class re-homes a now-unpriceable arm to the class default", () => {
    const fxBarrier: BuilderState = { ...INITIAL_BUILDER_STATE, arm: "BARRIER" };
    const equity = selectAssetClass(fxBarrier, "EQUITY");
    expect(equity.arm).toBe("VANILLA");
    expect(availableArms("EQUITY")[0]).toBe("VANILLA");
  });

  it("a class switch is non-destructive (per-class underlier fields preserved)", () => {
    let s = updateUnderlier(INITIAL_BUILDER_STATE, { ticker: "AAPL", venue: "XNAS" });
    s = selectAssetClass(s, "EQUITY");
    s = selectAssetClass(s, "FX"); // back to FX
    expect(s.underlier.ticker).toBe("AAPL");
    expect(s.underlier.pair).toBe("EURUSD");
  });

  it("the updaters do not mutate the prior state (immutability)", () => {
    const before = INITIAL_BUILDER_STATE;
    updateUnderlier(before, { pair: "GBPUSD" });
    setTenor(before, "3M");
    setNotional(before, "5");
    setTerms(before, [["strike", 1.5]]);
    expect(before.underlier.pair).toBe("EURUSD");
    expect(before.tenor).toBe("1Y");
    expect(before.notional).toBe("1");
  });

  it("a blank tenor on the no-expiry PERPETUAL arm is forwarded as no tenor", () => {
    const perp: BuilderState = {
      ...vanillaTicket("FX", { pair: "EURUSD" }),
      arm: "PERPETUAL",
      tenor: "",
      terms: [
        ["strike", 1.1],
        ["callPut", "C"],
      ],
    };
    expect(toSpecArgs(perp).tenor).toBeUndefined();
    expect(built(perp).instrument.product.kind).toBe("perpetualOption");
  });

  it("a non-numeric notional fails with a typed message", () => {
    expect(failure(setNotional(vanillaTicket("FX", { pair: "EURUSD" }), "abc"))).toContain(
      "notional",
    );
  });
});
