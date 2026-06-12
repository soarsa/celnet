/**
 * The class-aware instrument BUILDER — pure, headless STATE + LOGIC for the task
 * pane's ticket (no DOM, no transport, no Office globals — exhaustively
 * unit-testable under node). It replaces the pane's hardcoded FX-vanilla flow
 * (`shapeVanillaInstrument({ pair, tenor, strikeOrDelta, callPut, notional })`)
 * with a model that walks the trader from an ASSET CLASS down to a priceable
 * product on the ONE contract:
 *
 *   1. an asset-class selection (FX / METAL / EQUITY / COMMODITY / CRYPTO);
 *   2. the per-class UNDERLIER inputs (an FX/metal pair; an equity ticker @ venue
 *      : ccy; a commodity ticker : ccy; a crypto base/quote + linear/inverse);
 *   3. the PRICEABLE product ARMS for that class — the executable capability
 *      matrix (FX/metal price all 24 arms; equity/commodity/crypto price only the
 *      cost-of-carry leaves VANILLA + PERPETUAL + FUTUREOPTION), ported from the
 *      GUI's `gui/src/products/capability.ts`;
 *   4. the per-product TERMS (the same order-free key/value pairs the
 *      `CELNET.INSTRUMENT` cell reads).
 *
 * It composes the canonical one-string underlier (the grammar `parseUnderlier`
 * already speaks) from the structured per-class inputs, then DELEGATES to the
 * EXISTING production seam `shapeSpecInstrument` (`functions/instrumentSpec.ts`)
 * to produce the wire instrument and its opaque token. NO wire-building, NO
 * underlier parsing, and NO cross-asset guard is reimplemented here — the one
 * contract stays `celnet.proto`, and a class-aware ticket emits the
 * byte-identical frame the cell would. In particular, the FX-vanilla path stays
 * byte-identical to `shapeVanillaInstrument` (proven in the colocated test).
 *
 * The controller (`taskpane.ts`) owns the DOM; it holds ONE {@link BuilderState},
 * re-renders the underlier fields + the arm `<select>` from
 * {@link availableArms}, mutates the state through the immutable {@link update*}
 * helpers on each input event, and calls {@link buildInstrument} when the trader
 * requests a quote — surfacing a {@link BuildResult.error} (the same typed
 * `ShapingError` message the cell shows) inline, never a server round-trip for a
 * mistake the client can name first.
 */

import type { Instrument, SettlementStyle } from "../contract/contract";
import {
  encodeInstrumentToken,
  shapeSpecInstrument,
  type InstrumentSpecArgs,
  type TermsCell,
} from "../functions/instrumentSpec";
import { ShapingError } from "../functions/shaping";

// ---------------------------------------------------------------------------
// asset class + the executable capability matrix (ported from the GUI's
// gui/src/products/capability.ts — the SAME authoritative boundary the server's
// `price_cross_asset` and the Excel build-time guard enforce, made selectable so
// the trader is GUIDED away from an unpriceable combination before the request)
// ---------------------------------------------------------------------------

/**
 * The trader-facing asset class of an underlier (mirrors
 * `gui/src/products/types.ts` `AssetClass`). FX is the origin class; the four
 * cross-asset classes book through the same `Instrument.product` oneof, carrying
 * their identity on `Instrument.underlying` (and, for CRYPTO,
 * `Instrument.settlementStyle`).
 */
export type AssetClass = "FX" | "METAL" | "EQUITY" | "COMMODITY" | "CRYPTO";

/** The asset classes in trader-facing order, for the class selector. */
export const ASSET_CLASSES: readonly AssetClass[] = [
  "FX",
  "METAL",
  "EQUITY",
  "COMMODITY",
  "CRYPTO",
];

/**
 * The 24 product-family arms, by their canonical trader-facing name — exactly the
 * family tokens `shapeSpecInstrument` resolves (`functions/instrumentSpec.ts`
 * `FAMILIES`), in catalogue order. Kept as the literal source here so the arm
 * `<select>` and the capability sets read ONE list; the build delegates the token
 * back to that same family table, so a new arm needs no second registration.
 */
export const ALL_ARMS: readonly string[] = [
  "VANILLA",
  "STRATEGY",
  "RISKREVERSAL",
  "STRADDLE",
  "STRANGLE",
  "SEAGULL",
  "BARRIER",
  "WINDOWBARRIER",
  "DIGITAL",
  "TOUCH",
  "VARSWAP",
  "VOLSWAP",
  "ASIAN",
  "FORWARDSTART",
  "CLIQUET",
  "QUANTO",
  "TARF",
  "PIVOT",
  "ACCUMULATOR",
  "LOOKBACK",
  "AMERICAN",
  "BASKET",
  "FORWARD",
  "SWAP",
  "NDF",
  "PERPETUAL",
  "FUTUREOPTION",
];

/**
 * The cross-asset (equity / commodity / crypto) priceable arms: the generalized
 * cost-of-carry leaf (VANILLA) plus the two asset-class-AGNOSTIC arms — the
 * perpetual stationary-ODE (PERPETUAL) and Black-76 on a quoted future
 * (FUTUREOPTION). Every other arm is an FX/metal-only engine. This is the exact
 * set the server's `price_cross_asset` accepts and the SAME boundary
 * `shapeSpecInstrument`'s build-time guard refuses outside of — ported here so the
 * arm list is filtered BEFORE the request, never dimmed after a server refusal.
 */
export const CROSS_ASSET_ARMS: readonly string[] = ["VANILLA", "PERPETUAL", "FUTUREOPTION"];

/**
 * The priceable arms per asset class (the executable capability matrix). FX &
 * METAL route to the full FX engine (a metal's lease rate is the FX foreign rate),
 * so they price all 24; equity/commodity/crypto price only the carry leaves +
 * agnostic arms. Mirrors `gui/src/products/capability.ts` `PRICEABLE`.
 */
export const PRICEABLE_ARMS: Record<AssetClass, readonly string[]> = {
  FX: ALL_ARMS,
  METAL: ALL_ARMS,
  EQUITY: CROSS_ASSET_ARMS,
  COMMODITY: CROSS_ASSET_ARMS,
  CRYPTO: CROSS_ASSET_ARMS,
};

/**
 * The product arms a trader can build end-to-end on an asset class (the arm
 * `<select>` options for the class). Returns the canonical family names from
 * {@link PRICEABLE_ARMS}; the first entry (always VANILLA) is the sensible
 * default selection for a freshly-chosen class.
 */
export function availableArms(cls: AssetClass): readonly string[] {
  return PRICEABLE_ARMS[cls];
}

/** Is a product arm priceable on an asset class (end-to-end, per the matrix)? */
export function isArmPriceable(arm: string, cls: AssetClass): boolean {
  return PRICEABLE_ARMS[cls].includes(arm.trim().toUpperCase());
}

// ---------------------------------------------------------------------------
// the per-class underlier inputs + the composed one-string underlier
// ---------------------------------------------------------------------------

/**
 * The crypto contract-settlement selection: LINEAR (quote-margined, the default)
 * or INVERSE_COIN (the coin-margined `1/S_T` convention). Meaningful ONLY for a
 * crypto underlier; the composed underlier string carries it as the `:inverse`
 * suffix the grammar reads.
 */
export type CryptoSettlement = SettlementStyle;

/**
 * The structured per-class underlier inputs. Every class keeps its own fields so
 * a class switch never silently re-interprets the previous class's text; the
 * composer reads only the active class's fields. Each maps to one canonical
 * underlier string the {@link parseUnderlier} grammar speaks:
 *
 *   FX        `pair` ("EURUSD")                          → "EURUSD"
 *   METAL     `pair` ("XAUUSD")                          → "XAUUSD"
 *   EQUITY    `ticker` @ `venue` : `currency`            → "AAPL@XNAS:USD"
 *   COMMODITY `ticker` @ (empty) : `currency`            → "BRENT@:USD"
 *   CRYPTO    `base` / `quote` [ + `settlement` suffix ] → "BTC/USD" / "…:inverse"
 */
export interface UnderlierInputs {
  /** FX / METAL: the pair leg-string, e.g. "EURUSD", "XAUUSD" (also "EUR/USD"). */
  readonly pair: string;
  /** EQUITY / COMMODITY: the instrument ticker, e.g. "AAPL", "BRENT". */
  readonly ticker: string;
  /** EQUITY: the listing venue MIC, e.g. "XNAS" (COMMODITY leaves it empty). */
  readonly venue: string;
  /** EQUITY / COMMODITY: the quote/settlement currency (a 3-letter code), e.g. "USD". */
  readonly currency: string;
  /** CRYPTO: the coin/asset base leg, e.g. "BTC". */
  readonly base: string;
  /** CRYPTO: the numeraire quote leg — fiat ("USD") or stablecoin ("USDT"). */
  readonly quote: string;
  /** CRYPTO: linear (default) or the coin-margined inverse settlement convention. */
  readonly settlement: CryptoSettlement;
}

/** The empty per-class underlier inputs (every field blank; crypto linear). */
export const EMPTY_UNDERLIER: UnderlierInputs = {
  pair: "",
  ticker: "",
  venue: "",
  currency: "",
  base: "",
  quote: "",
  settlement: "LINEAR",
};

/**
 * One row of the terms table: a key cell and its value cells. A scalar term is one
 * value cell (`["strike", 1.12]`); the matrix keys (`legs` / `correlations`)
 * carry their full row to the right (`["legs", "C", "25dC", "BUY"]`) — exactly the
 * 2-D range `CELNET.INSTRUMENT` reads, so the ticket and the cell share one grammar.
 */
export type TermsRow = readonly TermsCell[];

/**
 * The full headless builder state. Immutable; the controller threads it through
 * the `update*` helpers and re-renders from it. `underlier` keeps a full set of
 * per-class inputs so a class switch is non-destructive; `arm` is one of
 * {@link availableArms}`(assetClass)`; `terms` is the order-free key/value table;
 * `tenor`/`notional` are the cell-equivalent positional inputs (the no-expiry
 * PERPETUAL arm takes no tenor — a supplied one is a typed error, exactly as in
 * the cell).
 */
export interface BuilderState {
  /** The selected asset class — drives the underlier fields + the arm options. */
  readonly assetClass: AssetClass;
  /** The structured per-class underlier inputs. */
  readonly underlier: UnderlierInputs;
  /** The selected product family (a canonical arm name from {@link availableArms}). */
  readonly arm: string;
  /** The per-product terms as key/value rows (scalars + the `legs`/`correlations` matrices). */
  readonly terms: readonly TermsRow[];
  /** The tenor (e.g. "1Y"); blank for the no-expiry PERPETUAL arm. */
  readonly tenor: string;
  /** The base/asset-leg notional as the trader typed it (a string cell). */
  readonly notional: string;
}

/**
 * The initial builder state: FX vanilla — the exact flow the pane shipped before
 * the class-aware ticket, so the default ticket is byte-unchanged. Defaults mirror
 * the legacy FX-vanilla inputs (a 1Y EURUSD ATM call, unit notional) so the pane
 * opens on a complete, priceable ticket.
 */
export const INITIAL_BUILDER_STATE: BuilderState = {
  assetClass: "FX",
  underlier: { ...EMPTY_UNDERLIER, pair: "EURUSD" },
  arm: "VANILLA",
  terms: [
    ["strike", "ATM"],
    ["callPut", "C"],
  ],
  tenor: "1Y",
  notional: "1",
};

// ---------------------------------------------------------------------------
// immutable state updates (the controller calls one per input event)
// ---------------------------------------------------------------------------

/**
 * Select an asset class. If the currently-selected arm is not priceable on the
 * new class (e.g. switching an FX BARRIER ticket to EQUITY), the arm falls back to
 * the class's first priceable arm (VANILLA) so the state is never left on an arm
 * the class cannot build. The per-class underlier inputs are preserved (a class
 * switch is non-destructive — the trader's other-class text is not discarded).
 */
export function selectAssetClass(state: BuilderState, assetClass: AssetClass): BuilderState {
  // VANILLA is the universal cost-of-carry leaf — priceable on EVERY class and the
  // first arm of each class's list — so it is the total, always-priceable fallback
  // when the current arm is FX/metal-only and the new class can't build it.
  const arm = isArmPriceable(state.arm, assetClass) ? state.arm : "VANILLA";
  return { ...state, assetClass, arm };
}

/** Patch the per-class underlier inputs (the controller passes only the changed fields). */
export function updateUnderlier(
  state: BuilderState,
  patch: Partial<UnderlierInputs>,
): BuilderState {
  return { ...state, underlier: { ...state.underlier, ...patch } };
}

/**
 * Select a product arm. A non-priceable arm for the current class is rejected
 * loudly (a typed error), never silently coerced — the arm `<select>` only ever
 * offers {@link availableArms}, so this guards a programmatic mis-set.
 */
export function selectArm(state: BuilderState, arm: string): BuilderState {
  const canon = arm.trim().toUpperCase();
  if (!isArmPriceable(canon, state.assetClass)) {
    throw new ShapingError(
      `${canon || arm} is not priceable on ${state.assetClass} ` +
        `(available: ${availableArms(state.assetClass).join(", ")})`,
    );
  }
  return { ...state, arm: canon };
}

/** Replace the terms table (the controller owns the per-row editor). */
export function setTerms(state: BuilderState, terms: readonly TermsRow[]): BuilderState {
  return { ...state, terms };
}

/** Set the tenor string (blank for the no-expiry PERPETUAL arm). */
export function setTenor(state: BuilderState, tenor: string): BuilderState {
  return { ...state, tenor };
}

/** Set the notional string (the build coerces + validates it). */
export function setNotional(state: BuilderState, notional: string): BuilderState {
  return { ...state, notional };
}

// ---------------------------------------------------------------------------
// composing the canonical one-string underlier
// ---------------------------------------------------------------------------

/**
 * Compose the canonical one-string underlier the {@link parseUnderlier} grammar
 * speaks from the active class's structured inputs. This is the ONLY place the
 * structured ticket meets the string grammar; the parsing/validation itself stays
 * in `parseUnderlier` (we never re-derive metal/crypto classification here):
 *
 *   FX/METAL  → the pair verbatim ("EURUSD"); the grammar classifies metal by its
 *              X-code base, so "XAUUSD" routes to the metal arm with no marker here.
 *   EQUITY    → "TICKER@VENUE:CCY"  (venue PRESENT)
 *   COMMODITY → "TICKER@:CCY"       (venue EMPTY — the listed-vs-contract rule)
 *   CRYPTO    → "BASE/QUOTE"[ + ":inverse" ]  (the suffix only for INVERSE_COIN)
 *
 * Blank required fields raise a typed error NAMING the field, so an empty ticket
 * fails with a trader-readable message rather than an opaque grammar error.
 */
export function composeUnderlier(state: BuilderState): string {
  const u = state.underlier;
  switch (state.assetClass) {
    case "FX":
    case "METAL": {
      const pair = u.pair.trim();
      if (pair === "") {
        throw new ShapingError(
          `${state.assetClass} underlier requires a pair (e.g. ${state.assetClass === "FX" ? "EURUSD" : "XAUUSD"})`,
        );
      }
      return pair;
    }
    case "EQUITY": {
      const ticker = u.ticker.trim();
      const venue = u.venue.trim();
      const currency = u.currency.trim();
      if (ticker === "") throw new ShapingError("equity underlier requires a ticker (e.g. AAPL)");
      if (venue === "") {
        throw new ShapingError("equity underlier requires a listing venue MIC (e.g. XNAS)");
      }
      if (currency === "") {
        throw new ShapingError("equity underlier requires a quote currency (e.g. USD)");
      }
      return `${ticker}@${venue}:${currency}`;
    }
    case "COMMODITY": {
      const ticker = u.ticker.trim();
      const currency = u.currency.trim();
      if (ticker === "") throw new ShapingError("commodity underlier requires a symbol (e.g. BRENT)");
      if (currency === "") {
        throw new ShapingError("commodity underlier requires a quote currency (e.g. USD)");
      }
      // The empty venue selects the commodity arm (TICKER@:CCY, the grammar rule).
      return `${ticker}@:${currency}`;
    }
    case "CRYPTO": {
      const base = u.base.trim();
      const quote = u.quote.trim();
      if (base === "") throw new ShapingError("crypto underlier requires a base coin (e.g. BTC)");
      if (quote === "") {
        throw new ShapingError("crypto underlier requires a quote leg (e.g. USD or USDT)");
      }
      const body = `${base}/${quote}`;
      // LINEAR is the proto3 zero — emit NO suffix (so a linear crypto ticket is
      // the unmarked pair the grammar defaults to LINEAR). Only INVERSE_COIN needs
      // the explicit `:inverse` marker.
      return u.settlement === "INVERSE_COIN" ? `${body}:inverse` : body;
    }
  }
}

// ---------------------------------------------------------------------------
// the terms / tenor / notional → CELNET.INSTRUMENT spec args
// ---------------------------------------------------------------------------

/**
 * The 2-D terms range the way `CELNET.INSTRUMENT` receives it: rows of cells. The
 * builder stores `terms` as exactly this shape, so the spec args carry it
 * verbatim — no re-keying, the ticket and the cell share one terms grammar.
 */
function termsRange(terms: readonly TermsRow[]): TermsCell[][] {
  return terms.map((row) => [...row]);
}

/**
 * The no-expiry PERPETUAL family takes no tenor; every other family requires one.
 * We mirror the spec's contract here so the SPEC ARGS carry `tenor: undefined` for
 * the perpetual (rather than a blank string the grammar would reject as a bad
 * tenor) and the trader-typed tenor otherwise. (`shapeSpecInstrument` is the
 * authority on the tenor rule; this only decides whether to forward the field.)
 */
function tenorArg(state: BuilderState): string | undefined {
  if (state.arm === "PERPETUAL") return undefined;
  const tenor = state.tenor.trim();
  return tenor === "" ? undefined : tenor;
}

/**
 * Coerce the notional cell. Blank ⇒ undefined (the spec defaults it to 1, exactly
 * as the cell does); a non-numeric value is a typed error NAMING it, never a
 * silent NaN that the wire shaper would reject downstream with a vaguer message.
 */
function notionalArg(raw: string): number | undefined {
  const s = raw.trim();
  if (s === "") return undefined;
  const n = Number(s);
  if (!Number.isFinite(n)) {
    throw new ShapingError(`invalid notional \`${raw}\` (expected a positive number, e.g. 1)`);
  }
  return n;
}

/**
 * Assemble the {@link InstrumentSpecArgs} the EXISTING `shapeSpecInstrument`
 * consumes from the builder state: the composed underlier string, the selected
 * arm as the product token, the terms range, and the tenor/notional. Exposed for
 * the controller/tests to inspect the exact cell-equivalent arguments a ticket
 * would emit (the spec path itself owns all validation + the cross-asset guard).
 */
export function toSpecArgs(state: BuilderState): InstrumentSpecArgs {
  return {
    underlier: composeUnderlier(state),
    product: state.arm,
    terms: termsRange(state.terms),
    tenor: tenorArg(state),
    notional: notionalArg(state.notional),
  };
}

// ---------------------------------------------------------------------------
// build → { instrument, token } | typed error
// ---------------------------------------------------------------------------

/** A successful build: the wire instrument + its opaque `CELNET.INSTRUMENT` token. */
export interface BuiltInstrument {
  readonly ok: true;
  /** The typed wire instrument the ticket prices/RFQs (byte-identical to the cell). */
  readonly instrument: Instrument;
  /** The opaque spec token (canonical wire JSON) the polymorphic verbs price. */
  readonly token: string;
}

/** A failed build: the typed `ShapingError` message, ready to show inline. */
export interface BuildFailure {
  readonly ok: false;
  /** The trader-readable failure message (the same one the cell would surface). */
  readonly error: string;
}

/** The total build outcome — a discriminated union the controller branches on. */
export type BuildResult = BuiltInstrument | BuildFailure;

/**
 * Build the wire instrument (and its token) from the headless state — the ticket's
 * one impure-free entry point. It composes the canonical underlier and DELEGATES
 * to `shapeSpecInstrument`, so the cross-asset capability guard, the underlier
 * grammar, the per-family term parsing, and the tenor/notional rules are all the
 * SAME ones the `CELNET.INSTRUMENT` cell runs — a class-aware ticket emits the
 * byte-identical frame the cell would.
 *
 * A trader mistake (a missing term, an FX-only arm on a cross-asset underlier, a
 * blank pair) is returned as a {@link BuildFailure} carrying the typed
 * `ShapingError` message, NOT thrown — the controller renders it inline. A
 * non-`ShapingError` (a genuine bug) is re-thrown unchanged, never swallowed as a
 * trader error.
 */
export function buildInstrument(state: BuilderState): BuildResult {
  try {
    const instrument = shapeSpecInstrument(toSpecArgs(state));
    return { ok: true, instrument, token: encodeInstrumentToken(instrument) };
  } catch (err) {
    if (err instanceof ShapingError) {
      return { ok: false, error: err.message };
    }
    throw err;
  }
}
