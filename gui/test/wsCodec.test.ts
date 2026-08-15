/**
 * wsCodec round-trip identity tests — the GUI end of the ONE `celnet.wire`
 * contract (CLAUDE.md rule 9). For every value type with both an encoder and a
 * decoder, `fromWire(toWire(x)) === x`: the snake_case / numeric-enum JSON the
 * server speaks is reconstructed losslessly on the way back in. We also pin the
 * frame (de)serialization that carries 64-bit `bigint` tokens without the
 * `JSON.parse` f64-rounding that would corrupt a click-to-trade token.
 *
 * These exercise the REAL `src/data/wsCodec.ts` + `src/data/enums.ts` through
 * their public surface with NO server and NO mocks.
 */
import { describe, expect, it } from "vitest";

import {
  aggregateRiskRequestToWire,
  attributionFromWire,
  attributionToWire,
  booksResponseFromWire,
  bookResponseFromWire,
  ccyPairFromWire,
  ccyPairToWire,
  conventionsFromWire,
  conventionsToWire,
  createBookRequestToWire,
  createDeskRequestToWire,
  createEntityRequestToWire,
  dealFromWire,
  deskResponseFromWire,
  updateDeskRequestToWire,
  deleteBookRequestToWire,
  deleteEntityRequestToWire,
  drillRiskRequestToWire,
  entitiesResponseFromWire,
  entityResponseFromWire,
  limitStatusRequestToWire,
  listBooksRequestToWire,
  listEntitiesRequestToWire,
  listLiquidityProvidersResponseFromWire,
  listPositionsRequestToWire,
  marketFromWire,
  marketToWire,
  multiDealerQuoteFromWire,
  parseFrame,
  quoteAcceptToWire,
  ratesCurveSetToWire,
  ratesInstrumentToWire,
  ratesInstrumentUnionToWire,
  ratesStreamSnapshotFromWire,
  ratesStreamUpdateFromWire,
  ratesSubscribeToWire,
  serializeFrame,
  smileModelToWire,
  updateBookRequestToWire,
  updateEntityRequestToWire,
  type WireObject,
} from "../src/data/wsCodec";
import type {
  AttributionRecord,
  CcyPair,
  Conventions,
  DeltaConvention,
  MarketContext,
  OisInstrument,
  PremiumStyle,
  RatesCurveSet,
  RatesInstrument,
  RatesPricingResult,
  ReportingNumeraire,
  RiskScope,
  SmileModel,
} from "../src/data/contract";
import * as e from "../src/data/enums";
import { DEFAULT_CONVENTIONS, PAIRS } from "../src/data/seed";

describe("wsCodec — CcyPair round-trip", () => {
  it("reconstructs every seeded pair identically", () => {
    for (const { pair } of PAIRS) {
      expect(ccyPairFromWire(ccyPairToWire(pair))).toEqual(pair);
    }
  });

  it("encodes to the wire's bare base/quote shape", () => {
    const p: CcyPair = { base: "EUR", quote: "USD" };
    expect(ccyPairToWire(p)).toEqual({ base: "EUR", quote: "USD" });
  });
});

describe("wsCodec — Conventions round-trip", () => {
  it("reconstructs the default conventions identically", () => {
    expect(conventionsFromWire(conventionsToWire(DEFAULT_CONVENTIONS))).toEqual(
      DEFAULT_CONVENTIONS,
    );
  });

  it("round-trips every enum member across the whole convention space", () => {
    // Cartesian-ish sweep: vary each axis through all its members.
    const deltas: DeltaConvention[] = [
      "SPOT_UNADJUSTED",
      "FORWARD_UNADJUSTED",
      "SPOT_PREMIUM_ADJUSTED",
      "FORWARD_PREMIUM_ADJUSTED",
    ];
    const premiums: PremiumStyle[] = [
      "DOMESTIC_PIPS",
      "PERCENT_FOREIGN",
      "PERCENT_DOMESTIC",
      "FOREIGN_PIPS",
    ];
    for (const deltaConvention of deltas) {
      for (const premiumStyle of premiums) {
        const c: Conventions = {
          ...DEFAULT_CONVENTIONS,
          deltaConvention,
          premiumStyle,
        };
        expect(conventionsFromWire(conventionsToWire(c))).toEqual(c);
      }
    }
  });

  it("emits canonical snake_case keys with numeric enum tags", () => {
    const w = conventionsToWire(DEFAULT_CONVENTIONS);
    expect(Object.keys(w).sort()).toEqual([
      "atm_convention",
      "cut",
      "day_count",
      "delta_convention",
      "premium_style",
      "settlement",
    ]);
    expect(w["premium_style"]).toBe(e.premiumStyle.toWire("PERCENT_FOREIGN"));
  });
});

describe("wsCodec — MarketContext round-trip", () => {
  it("reconstructs every seeded market identically", () => {
    for (const { market } of PAIRS) {
      expect(marketFromWire(marketToWire(market))).toEqual(market);
    }
  });

  it("maps the GUI camelCase rate fields to the wire's r_dom/r_for", () => {
    const m: MarketContext = { spot: 1.0768, vol: 0.0755, rDom: 0.0432, rFor: 0.0218 };
    const w = marketToWire(m);
    expect(w).toEqual({ spot: 1.0768, vol: 0.0755, r_dom: 0.0432, r_for: 0.0218 });
    expect(marketFromWire(w)).toEqual(m);
  });
});

describe("wsCodec — AttributionRecord round-trip", () => {
  it("round-trips a full attribution chain", () => {
    const a: AttributionRecord = {
      quotedBy: { book: "FX-VOL", owner: { kind: "autoPricer", autoPricer: "edge-1" } },
      heldBy: { book: "EMEA-DESK", owner: { kind: "trader", trader: "jdoe" } },
      won: true,
      lpCount: 4,
    };
    // attributionToWire emits the bare body; attributionFromWire reads it nested
    // under an "attribution" key (its place on a quoted line) — so wrap to decode.
    expect(attributionFromWire({ attribution: attributionToWire(a) })).toEqual(a);
  });

  it("treats an empty body as honestly absent (undefined, not {})", () => {
    expect(attributionFromWire({ attribution: attributionToWire({}) })).toBeUndefined();
    expect(attributionFromWire({})).toBeUndefined();
  });

  it("preserves a partial chain (heldBy only)", () => {
    const a: AttributionRecord = { heldBy: { book: "APAC" }, lpCount: 0 };
    const round = attributionFromWire({ attribution: attributionToWire(a) });
    expect(round?.heldBy).toEqual({ book: "APAC" });
    expect(round?.lpCount).toBe(0);
    expect(round?.quotedBy).toBeUndefined();
  });
});

describe("wsCodec — frame (de)serialization", () => {
  it("round-trips a plain JSON object frame", () => {
    const frame = { kind: 1, pair: { base: "EUR", quote: "USD" }, spot: 1.0768 };
    expect(parseFrame(serializeFrame(frame))).toEqual(frame);
  });

  it("carries a 64-bit token through ser→parse without f64 rounding", () => {
    // A token beyond Number.MAX_SAFE_INTEGER would be corrupted by a naive
    // JSON.parse; serializeFrame writes the bigint bare and parseFrame requotes
    // the oversized literal so numToBigInt-style consumers recover it exactly.
    const token = 9_007_199_254_740_993n; // MAX_SAFE_INTEGER + 2
    const text = serializeFrame({ token });
    // The wire literal is a bare integer (no quotes) — JSON has no bigint.
    expect(text).toBe('{"token":9007199254740993}');
    const parsed = parseFrame(text) as { token: unknown };
    // parseFrame requotes it into a string to survive the parse losslessly.
    expect(BigInt(parsed.token as string)).toBe(token);
  });

  it("recovers LP-panel nanosecond timestamps through the real parseFrame path", () => {
    // REGRESSION (observed on a live UAT edge): parseFrame requotes any integer
    // above MAX_SAFE_INTEGER so it survives the parse losslessly, so a 19-digit
    // *_nanos field never reaches the decoder as a `number`. Read with the plain
    // `num` helper it collapsed to 0, and the LP panel rendered "last quote:
    // never" / "Rate —" / "unresolved id" over four demonstrably healthy feeds.
    const text =
      '{"as_of_nanos":1786739476487987184,"inbound_enabled":true,' +
      '"providers":[{"connection_id":"jpm-sim","quote_updates":5890,' +
      '"last_quote_nanos":1786739385101072245,"instruments_quoted":155,' +
      '"fresh_quotes":155,"connection_defined":false}],"quotes":[]}';
    const panel = listLiquidityProvidersResponseFromWire(
      parseFrame(text) as Parameters<typeof listLiquidityProvidersResponseFromWire>[0],
    );
    expect(panel.asOfNanos).toBeGreaterThan(0);
    const provider = panel.providers[0];
    expect(provider?.lastQuoteNanos).toBeGreaterThan(0);
    expect(provider?.quoteUpdates).toBe(5890);
    // The age the panel actually renders: ~91s, NOT null (which prints "never").
    const ageSecs = (panel.asOfNanos - (provider?.lastQuoteNanos ?? 0)) / 1e9;
    expect(ageSecs).toBeGreaterThan(90);
    expect(ageSecs).toBeLessThan(92);
  });

  it("leaves small integers and non-integers untouched on parse", () => {
    const parsed = parseFrame('{"seq":42,"px":1.0768,"neg":-7}') as Record<string, unknown>;
    expect(parsed).toEqual({ seq: 42, px: 1.0768, neg: -7 });
  });

  it("does not requote digits that live inside a string value", () => {
    const parsed = parseFrame('{"label":"99999999999999999999 lots"}') as {
      label: string;
    };
    expect(parsed.label).toBe("99999999999999999999 lots");
  });
});

describe("wsCodec — SmileModel codec (proto-number alignment)", () => {
  // The array index MUST equal the proto enum number, or the GUI would mark a
  // surface under a different family than the trader picked.
  const PROTO_NUMBER: Record<SmileModel, number> = {
    MARKET_HEDGE: 0,
    STOCHASTIC_VOL: 1,
    PARAMETRIC: 2,
    PARAMETRIC_SURFACE: 3,
    EXTENDED_SURFACE: 4,
  };

  it("maps every SmileModel to its proto enum number on the wire", () => {
    for (const [model, number] of Object.entries(PROTO_NUMBER) as [SmileModel, number][]) {
      expect(smileModelToWire(model)).toBe(number);
    }
  });

  it("round-trips eSSVI (EXTENDED_SURFACE) through index 4 (the parity-break fix)", () => {
    expect(smileModelToWire("EXTENDED_SURFACE")).toBe(4);
    expect(e.smileModel.toWire("EXTENDED_SURFACE")).toBe(4);
    expect(e.smileModel.fromWire(4)).toBe("EXTENDED_SURFACE");
  });
});

describe("wsCodec — MultiDealerQuote panel frame (the server-emitted shape)", () => {
  // The exact snake_case Greeks body the server's `greeks_to_json` emits.
  const GREEKS_WIRE = {
    price: 0.0123,
    delta_spot: 0.51,
    delta_forward: 0.52,
    gamma: 0.03,
    vega: 0.21,
    theta: -0.01,
    rho_dom: 0.05,
    rho_for: -0.04,
    vanna: 0.002,
    volga: 0.011,
    charm: -0.0005,
    speed: 0.0001,
    zomma: 0.0004,
    color: -0.0002,
  };

  /**
   * The exact panel frame shape `multi_dealer_quote_to_json` emits: dealers in
   * the server's deterministic audit order, the native maker row carrying the
   * greeks / MC std-error, and a synthetic dealer row carrying `null` for both
   * (an LP discloses a price, not its greeks).
   */
  function panelFrame(): WireObject {
    return {
      quote_id: 42,
      idempotency_key: "tkt-1",
      dealers: [
        {
          lp_id: "SYNTH-LP-1",
          price: { bid: 0.1206, offer: 0.1296 },
          greeks: null,
          resolved_strike: 1.0921,
          valid_until_nanos: 1_700_000_008_000_000_000,
          attribution: {
            quotedBy: { book: "SYNTH-LP-1", owner: { autoPricer: "SYNTH-LP-1" } },
          },
          price_std_error: null,
        },
        {
          lp_id: "SYNTH-LP-2",
          price: { bid: 0.1182, offer: 0.1292 },
          greeks: null,
          resolved_strike: 1.0921,
          valid_until_nanos: 1_700_000_008_000_000_000,
          attribution: {
            quotedBy: { book: "SYNTH-LP-2", owner: { autoPricer: "SYNTH-LP-2" } },
          },
          price_std_error: null,
        },
        {
          lp_id: "celnet-auto-pricer",
          price: { bid: 0.12, offer: 0.13 },
          greeks: GREEKS_WIRE,
          resolved_strike: 1.0921,
          valid_until_nanos: 1_700_000_008_000_000_000,
          attribution: {
            quotedBy: { book: "AUTO", owner: { autoPricer: "celnet-auto-pricer" } },
          },
          price_std_error: 0.0004,
        },
      ],
      best_bid_lp_id: "SYNTH-LP-1",
      best_offer_lp_id: "SYNTH-LP-2",
      conventions: conventionsToWire(DEFAULT_CONVENTIONS),
      epoch_nanos: 1_700_000_000_000_000_000,
      correlation_id: null,
      surface_version: 7,
    };
  }

  it("decodes the panel field-for-field, keeping the dealers in FRAME order", () => {
    const m = multiDealerQuoteFromWire(panelFrame());
    expect(m.quoteId).toBe(42n);
    expect(m.idempotencyKey).toBe("tkt-1");
    expect(m.dealers.map((d) => d.lpId)).toEqual([
      "SYNTH-LP-1",
      "SYNTH-LP-2",
      "celnet-auto-pricer",
    ]);
    expect(m.bestBidLpId).toBe("SYNTH-LP-1");
    expect(m.bestOfferLpId).toBe("SYNTH-LP-2");
    expect(m.conventions).toEqual(DEFAULT_CONVENTIONS);
    expect(m.epochNanos).toBe(1_700_000_000_000_000_000n);
    // Presence-tracked optionals: a `null` correlation is honestly absent.
    expect(m.correlationId).toBeUndefined();
    expect(m.surfaceVersion).toBe(7n);
  });

  it("keeps the native row's greeks/std-error and a synthetic row's honest absences", () => {
    const m = multiDealerQuoteFromWire(panelFrame());
    const native = m.dealers.find((d) => d.lpId === "celnet-auto-pricer")!;
    expect(native.greeks?.deltaSpot).toBe(0.51);
    expect(native.greeks?.rhoDom).toBe(0.05);
    expect(native.priceStdError).toBe(0.0004);
    expect(native.price).toEqual({ bid: 0.12, offer: 0.13 });
    const synth = m.dealers.find((d) => d.lpId === "SYNTH-LP-2")!;
    // `null` on the wire ⇒ undefined in the GUI — never a fabricated zero row.
    expect(synth.greeks).toBeUndefined();
    expect(synth.priceStdError).toBeUndefined();
    expect(synth.attribution?.quotedBy?.owner).toEqual({
      kind: "autoPricer",
      autoPricer: "SYNTH-LP-2",
    });
    expect(synth.validUntilNanos).toBe(1_700_000_008_000_000_000n);
  });

  it("recovers a 64-bit valid_until_nanos beyond MAX_SAFE through parseFrame", () => {
    // Build the raw frame TEXT (a JS number literal would already have rounded),
    // exactly as the server serializes it: a bare 64-bit integer literal.
    const big = "9223372036854775806"; // i64::MAX - 1
    const raw =
      `{"quote_id":42,"idempotency_key":"k","dealers":[{"lp_id":"SYNTH-LP-1",` +
      `"price":{"bid":0.1,"offer":0.2},"greeks":null,"resolved_strike":1.09,` +
      `"valid_until_nanos":${big},"attribution":null,"price_std_error":null}],` +
      `"best_bid_lp_id":"SYNTH-LP-1","best_offer_lp_id":"SYNTH-LP-1",` +
      `"conventions":{},"epoch_nanos":1,"correlation_id":null,"surface_version":null}`;
    const m = multiDealerQuoteFromWire(parseFrame(raw) as WireObject);
    expect(m.dealers[0]!.validUntilNanos).toBe(BigInt(big));
    expect(m.surfaceVersion).toBeUndefined();
  });
});

describe("wsCodec — accept_quote body (the multi-dealer line selector)", () => {
  // The grant-all default principal the accept carries (item B §2 caller-authz):
  // the SAME explicit grant-all the risk requests and the stream `authenticate`
  // frame default to, so the accept presents a caller and the server's `Enforce`
  // posture admits it (and binds it to the recording requester).
  const GRANT_ALL = { grant_all: true, grants: [], denies: [] };

  it("emits the pre-panel body plus the grant-all caller when no lpId is named", () => {
    const w = quoteAcceptToWire(42n, "BUY", "tkt-1");
    // EXACT key set: no `lp_id` key at all (the server reads absent as ""); the
    // `principal` rides verbatim so QuoteService gates the accept under Enforce.
    expect(Object.keys(w).sort()).toEqual([
      "idempotency_key",
      "principal",
      "quote_id",
      "side",
    ]);
    // The quote_id stays the exact 64-bit identity (a `bigint`); on the wire
    // `serializeFrame` writes it as the same bare integer literal as before.
    expect(w).toEqual({
      quote_id: 42n,
      idempotency_key: "tkt-1",
      side: 0,
      principal: GRANT_ALL,
    });
    expect(serializeFrame(w)).toBe(
      '{"quote_id":42,"idempotency_key":"tkt-1","side":0,' +
        '"principal":{"grant_all":true,"grants":[],"denies":[]}}',
    );
  });

  it("treats an empty lpId exactly like an absent one (single-dealer accept)", () => {
    expect(quoteAcceptToWire(42n, "SELL", "tkt-1", "")).toEqual({
      quote_id: 42n,
      idempotency_key: "tkt-1",
      side: 1,
      principal: GRANT_ALL,
    });
  });

  it("carries a named panel row's lp_id (book exactly that dealer line)", () => {
    expect(quoteAcceptToWire(42n, "BUY", "tkt-1", "SYNTH-LP-2")).toEqual({
      quote_id: 42n,
      idempotency_key: "tkt-1",
      side: 0,
      lp_id: "SYNTH-LP-2",
      principal: GRANT_ALL,
    });
  });

  it("preserves a minted quote_id beyond MAX_SAFE bit-for-bit through the wire text", () => {
    // The server mints quote ids over the FULL u64 range (splitmix64), so almost
    // every real id exceeds Number.MAX_SAFE_INTEGER. A lossy `Number()` here
    // rounds the id and the server refuses the accept as `unknown quote_id` —
    // the exact failure the live LP-panel e2e caught. Round-trip the literal.
    const id = 4385739192607958123n; // > 2^53; rounds to …958000 as a double
    const w = quoteAcceptToWire(id, "BUY", "tkt-1", "SYNTH-LP-2");
    expect(w["quote_id"]).toBe(id);
    expect(serializeFrame(w)).toContain('"quote_id":4385739192607958123,');
  });
});

// The regression guard for the entitlements client-default propagation: with the
// server now deny-by-default (`AccessMode::Enforce`), every risk request the GUI
// sends MUST carry a principal. When the caller asserts none, the encoder emits an
// EXPLICIT grant-all (the audited show-all-now default), so the Book/Risk view
// clears the production boundary — never relying on a removed server-side
// absent-⇒-grant-all. A genuinely absent principal would be denied server-side.
describe("wsCodec — risk requests always carry an explicit grant-all principal by default", () => {
  const GRANT_ALL = { grant_all: true, grants: [], denies: [] };
  const usd: ReportingNumeraire = { numeraire: "USD", rates: [{ ccy: "EUR", rate: 1.1 }] };
  const firm: RiskScope = { dimension: "FIRM", value: 0n };

  it("aggregate_risk defaults to grant-all when no principal is asserted", () => {
    const w = aggregateRiskRequestToWire({
      dimension: "FIRM",
      numeraire: usd,
      vegaPillars: [],
      varSpotShocks: [],
      varAlpha: 0,
      curvatureRiskWeight: 0,
    });
    expect(w["principal"]).toEqual(GRANT_ALL);
  });

  it("list_positions / drill_risk / limit_status all default to grant-all", () => {
    expect(listPositionsRequestToWire({})["principal"]).toEqual(GRANT_ALL);
    expect(
      drillRiskRequestToWire({
        node: firm,
        childDimension: "BOOK",
        numeraire: usd,
        vegaPillars: [],
        includeChildren: true,
        includePositions: false,
      })["principal"],
    ).toEqual(GRANT_ALL);
    expect(
      limitStatusRequestToWire({
        scope: firm,
        numeraire: usd,
        vegaPillars: [],
        varSpotShocks: [],
        varAlpha: 0,
      })["principal"],
    ).toEqual(GRANT_ALL);
  });

  it("honors an asserted scoped principal instead of the grant-all default", () => {
    const w = aggregateRiskRequestToWire({
      dimension: "FIRM",
      numeraire: usd,
      principal: { grantAll: false, grants: [{ scopes: [{ dimension: "BOOK", value: 7n }] }], denies: [] },
      vegaPillars: [],
      varSpotShocks: [],
      varAlpha: 0,
      curvatureRiskWeight: 0,
    });
    expect(w["principal"]).toEqual({
      grant_all: false,
      grants: [{ scopes: [{ dimension: 2, value: 7n }] }],
      denies: [],
    });
  });
});

describe("wsCodec — legal-entity / netting-book registry", () => {
  it("encodes list requests as an empty body (session_token is auto-injected)", () => {
    expect(listEntitiesRequestToWire()).toEqual({});
    expect(listBooksRequestToWire()).toEqual({});
  });

  it("decodes an `entities` frame into the camelCase EntityDesc roster", () => {
    const frame: WireObject = {
      entities: [
        { key: 1, name: "Celnet Global Markets", code: "CGM" },
        { key: 2, name: "Celnet Securities", code: "CSEC" },
      ],
    };
    expect(entitiesResponseFromWire(frame)).toEqual([
      { key: 1, name: "Celnet Global Markets", code: "CGM" },
      { key: 2, name: "Celnet Securities", code: "CSEC" },
    ]);
  });

  it("decodes an absent `entities` array as an empty roster", () => {
    expect(entitiesResponseFromWire({})).toEqual([]);
  });

  it("encodes create_entity with key 0 (server auto-assigns the lowest free key)", () => {
    expect(createEntityRequestToWire({ name: "ACME Capital", code: "ACME" })).toEqual({
      name: "ACME Capital",
      code: "ACME",
      key: 0,
    });
  });

  it("encodes update_entity carrying the immutable key", () => {
    expect(updateEntityRequestToWire(7, { name: "ACME Capital", code: "ACME" })).toEqual({
      key: 7,
      name: "ACME Capital",
      code: "ACME",
    });
  });

  it("decodes a single `entity_created` / `entity_updated` frame", () => {
    const frame: WireObject = { entity: { key: 5, name: "ACME Capital", code: "ACME" } };
    expect(entityResponseFromWire(frame)).toEqual({
      key: 5,
      name: "ACME Capital",
      code: "ACME",
    });
  });

  it("encodes delete_entity by key", () => {
    expect(deleteEntityRequestToWire(3)).toEqual({ key: 3 });
  });

  it("decodes a `books` frame mapping snake_case entity_key → camelCase entityKey", () => {
    const frame: WireObject = {
      books: [
        { key: 1, name: "Rates Trading", entity_key: 1 },
        { key: 3, name: "Government Bonds", entity_key: 2 },
      ],
    };
    expect(booksResponseFromWire(frame)).toEqual([
      { key: 1, name: "Rates Trading", entityKey: 1 },
      { key: 3, name: "Government Bonds", entityKey: 2 },
    ]);
  });

  it("encodes create_book mapping entityKey → wire entity_key with key 0 (auto)", () => {
    expect(createBookRequestToWire({ name: "Rates Trading", entityKey: 4 })).toEqual({
      name: "Rates Trading",
      entity_key: 4,
      key: 0,
    });
  });

  it("encodes update_book carrying the immutable key + re-homed entity_key", () => {
    expect(updateBookRequestToWire(9, { name: "Rates Vol", entityKey: 2 })).toEqual({
      key: 9,
      name: "Rates Vol",
      entity_key: 2,
    });
  });

  it("decodes a single `book_created` / `book_updated` frame", () => {
    const frame: WireObject = { book: { key: 11, name: "Rates Trading", entity_key: 1 } };
    expect(bookResponseFromWire(frame)).toEqual({
      key: 11,
      name: "Rates Trading",
      entityKey: 1,
    });
  });

  it("encodes delete_book by key", () => {
    expect(deleteBookRequestToWire(11)).toEqual({ key: 11 });
  });

  it("round-trips a book through the create encoder and the response decoder", () => {
    // The wire `entity_key` the encoder emits is exactly what the response codec
    // reads back as `entityKey` (the only snake↔camel rename on this surface).
    const encoded = createBookRequestToWire({ name: "Swaps", entityKey: 2 });
    const decoded = bookResponseFromWire({ book: { ...encoded, key: 4 } });
    expect(decoded).toEqual({ key: 4, name: "Swaps", entityKey: 2 });
  });
});

describe("wsCodec — RatesInstrument oneof arms (server decoder contract)", () => {
  // The exact snake_case field names + integer enum codes the server
  // `rates_instrument_from_json` (crates/celnet-server/src/ws/codec.rs) reads. Wire
  // codes: Side BUY=0/SELL=1; PaymentFrequency ANNUAL=0/SEMI=1/QUARTERLY=2; leg
  // DayCount ACT_365_FIXED=0/ACT_360=1; AccrualBasis ACT_360=0/ACT_365F=1/30_360=2.

  it("encodes the OIS arm identically to the OIS-only encoder", () => {
    const ois: OisInstrument = {
      tenorYears: 5,
      fixedRate: 0.0405,
      notional: 100_000_000,
      direction: "RECEIVE_FIXED",
    };
    const wire = ratesInstrumentUnionToWire({ kind: "ois", ois });
    expect(wire).toEqual(ratesInstrumentToWire(ois));
    expect(wire).toEqual({
      ois: { tenor_years: 5, fixed_rate: 0.0405, notional: 100_000_000, side: 1 },
    });
  });

  it("encodes the IRS arm with the server's field names + enum codes", () => {
    const wire = ratesInstrumentUnionToWire({
      kind: "irs",
      irs: {
        tenorYears: 5,
        fixedRate: 0.041,
        notional: 100_000_000,
        direction: "PAY_FIXED",
        fixedFrequency: "SEMI_ANNUAL",
        fixedDayCount: "ACT_360",
        floatFrequency: "QUARTERLY",
        floatDayCount: "ACT_365_FIXED",
      },
    });
    expect(wire).toEqual({
      irs: {
        tenor_years: 5,
        fixed_rate: 0.041,
        notional: 100_000_000,
        side: 0,
        fixed_frequency: 1,
        fixed_day_count: 1,
        float_frequency: 2,
        float_day_count: 0,
      },
    });
  });

  it("encodes the FRA arm with the server's field names + enum codes", () => {
    const wire = ratesInstrumentUnionToWire({
      kind: "fra",
      fra: {
        startMonths: 3,
        endMonths: 6,
        fixedRate: 0.033,
        notional: 100_000_000,
        direction: "RECEIVE_FIXED",
        accrualBasis: "THIRTY_360_BOND_BASIS",
      },
    });
    expect(wire).toEqual({
      fra: {
        start_months: 3,
        end_months: 6,
        fixed_rate: 0.033,
        notional: 100_000_000,
        side: 1,
        accrual_basis: 2,
      },
    });
  });

  it("encodes the bond arm with a nested maturity_date + long/short side code", () => {
    const long = ratesInstrumentUnionToWire({
      kind: "bond",
      bond: {
        couponRate: 0.06,
        couponFrequency: "SEMI_ANNUAL",
        dayCount: "THIRTY_360_BOND_BASIS",
        maturityDate: { year: 2035, month: 6, day: 15 },
        redemption: 100,
        position: "LONG",
      },
    });
    expect(long).toEqual({
      bond: {
        coupon_rate: 0.06,
        coupon_frequency: 1,
        day_count: 2,
        maturity_date: { year: 2035, month: 6, day: 15 },
        redemption: 100,
        side: 0,
      },
    });
    // SHORT flips only the side code (SELL = 1).
    const short = ratesInstrumentUnionToWire({
      kind: "bond",
      bond: {
        couponRate: 0.06,
        couponFrequency: "SEMI_ANNUAL",
        dayCount: "THIRTY_360_BOND_BASIS",
        maturityDate: { year: 2035, month: 6, day: 15 },
        redemption: 100,
        position: "SHORT",
      },
    });
    expect((short.bond as { side: number }).side).toBe(1);
  });

  it("emits exactly one oneof arm key per instrument", () => {
    const arms = [
      ratesInstrumentUnionToWire({
        kind: "ois",
        ois: { tenorYears: 2, fixedRate: 0.04, notional: 1, direction: "PAY_FIXED" },
      }),
      ratesInstrumentUnionToWire({
        kind: "fra",
        fra: {
          startMonths: 3,
          endMonths: 6,
          fixedRate: 0.03,
          notional: 1,
          direction: "PAY_FIXED",
          accrualBasis: "ACT_360",
        },
      }),
    ];
    expect(arms.map((a) => Object.keys(a))).toEqual([["ois"], ["fra"]]);
  });
});

describe("wsCodec — dealFromWire decodes each RatesInstrument arm to its productKind", () => {
  // A minimal wire `Deal` carrying the given instrument arm; the curve reference year
  // (2026) anchors a BOND arm's whole-year term.
  function wireDeal(instrument: WireObject): WireObject {
    return {
      deal_id: "d-1",
      request_id: "r-1",
      kind: 0,
      counterparty: "CP",
      desk: "g10-rates",
      instrument,
      curve_set: {
        currency: "USD",
        reference_date: { year: 2026, month: 1, day: 1 },
        ois_pillars: [],
      },
      side: 0,
      notional: 100_000_000,
      price: 0.04,
      executed_at_nanos: 0,
      trader: "T",
    };
  }

  it("decodes the OIS arm → productKind OIS with the tenor/notional projection", () => {
    const d = dealFromWire(
      wireDeal({ ois: { tenor_years: 5, fixed_rate: 0.0405, notional: 1e7, side: 1 } }),
    );
    expect(d.productKind).toBe("OIS");
    expect(d.instrument).toEqual({
      tenorYears: 5,
      fixedRate: 0.0405,
      notional: 1e7,
      direction: "RECEIVE_FIXED",
    });
  });

  it("decodes the IRS arm → productKind IRS (tenor from tenor_years)", () => {
    const d = dealFromWire(
      wireDeal({
        irs: {
          tenor_years: 7,
          fixed_rate: 0.041,
          notional: 5e7,
          side: 0,
          fixed_frequency: 1,
          fixed_day_count: 1,
          float_frequency: 2,
          float_day_count: 0,
        },
      }),
    );
    expect(d.productKind).toBe("IRS");
    expect(d.instrument.tenorYears).toBe(7);
    expect(d.instrument.direction).toBe("PAY_FIXED");
  });

  it("decodes the FRA arm → productKind FRA (tenor = end_months / 12)", () => {
    const d = dealFromWire(
      wireDeal({
        fra: { start_months: 3, end_months: 6, fixed_rate: 0.033, notional: 2e7, side: 1, accrual_basis: 2 },
      }),
    );
    expect(d.productKind).toBe("FRA");
    expect(d.instrument.tenorYears).toBe(0.5);
  });

  it("decodes the BOND arm → productKind BOND (tenor = maturity year − curve ref year)", () => {
    const d = dealFromWire(
      wireDeal({
        bond: {
          coupon_rate: 0.06,
          coupon_frequency: 1,
          day_count: 2,
          maturity_date: { year: 2036, month: 6, day: 15 },
          redemption: 100,
          side: 0,
        },
      }),
    );
    expect(d.productKind).toBe("BOND");
    // 2036 − 2026 = 10y; the projection reports the coupon + face for display.
    expect(d.instrument.tenorYears).toBe(10);
    expect(d.instrument.fixedRate).toBe(0.06);
    expect(d.instrument.notional).toBe(100);
  });
});

describe("wsCodec — fixed-income LIVE STREAMING (rates_subscribe + snapshot/update)", () => {
  // The server contract: `rates_subscribe_from_json` reads { subscription,
  // instrument, curve_set, throttle_nanos, correlation_id? }; the snapshot/update
  // frames are `rates_stream_snapshot_to_json` / `rates_stream_update_to_json`
  // (subscription, sequence, result{pv,par_rate,pv01,dv01,key_rate_ladder},
  // curve_shift, epoch_nanos, correlation_id?). BYTE-MATCH those exactly.

  const curve: RatesCurveSet = {
    currency: "USD",
    referenceDate: { year: 2026, month: 6, day: 30 },
    pillars: [
      { tenor: { kind: "years", years: 2 }, parRate: 0.0418 },
      { tenor: { kind: "years", years: 5 }, parRate: 0.0405 },
    ],
  };
  const instrument: RatesInstrument = {
    kind: "ois",
    ois: { tenorYears: 5, fixedRate: 0.0405, notional: 100_000_000, direction: "PAY_FIXED" },
  };

  it("encodes rates_subscribe with the server decoder's exact field shape", () => {
    const body = ratesSubscribeToWire({ subscriptionId: 7n, instrument, curveSet: curve });
    expect(body).toEqual({
      subscription: { value: 7 },
      // reuses the SHARED unary encoders verbatim (one encoding, no duplication)
      instrument: ratesInstrumentUnionToWire(instrument),
      curve_set: ratesCurveSetToWire(curve),
      throttle_nanos: 0,
    });
    // correlation_id is presence-tracked — absent unless supplied.
    expect("correlation_id" in body).toBe(false);
  });

  it("carries the correlation id + throttle when supplied", () => {
    const body = ratesSubscribeToWire({
      subscriptionId: 9n,
      instrument,
      curveSet: curve,
      throttleNanos: 250_000n,
      correlationId: 4242n,
    });
    expect(body["throttle_nanos"]).toBe(250_000);
    expect(body["correlation_id"]).toBe(4242);
  });

  it("decodes a rates_stream_snapshot frame (server rates_stream_snapshot_to_json)", () => {
    const result: RatesPricingResult = {
      pv: -123456.789,
      parRate: 0.04093,
      pv01: -4821.5,
      dv01: -4830.1,
      keyRateLadder: [-1200.3, -3630.2],
    };
    // The exact server frame shape (snake_case, nested `result`).
    const frame = {
      subscription: { value: 7 },
      sequence: 1,
      result: {
        pv: result.pv,
        par_rate: result.parRate,
        pv01: result.pv01,
        dv01: result.dv01,
        key_rate_ladder: [...result.keyRateLadder],
      },
      curve_shift: 0,
      correlation_id: 4242,
      epoch_nanos: 1_700_000_000_000_000_000,
    };
    const snap = ratesStreamSnapshotFromWire(frame);
    expect(snap.subscriptionId).toBe(7n);
    expect(snap.sequence).toBe(1n);
    expect(snap.result).toEqual(result);
    expect(snap.curveShift).toBe(0);
    expect(snap.correlationId).toBe(4242n);
    expect(snap.epochNanos).toBe(1_700_000_000_000_000_000n);
  });

  it("decodes a rates_stream_update frame (no correlation, non-zero shift)", () => {
    const frame = {
      subscription: { value: 7 },
      sequence: 5,
      result: { pv: 42.0, par_rate: 0.041, pv01: 10.0, dv01: 11.0, key_rate_ladder: [] },
      curve_shift: 0.00007,
      // A JSON-safe-integer epoch (the WireObject the decoder receives is already
      // parsed; the bigint-safe frame path is exercised by parseFrame elsewhere).
      epoch_nanos: 1_700_000_000_000,
    };
    const upd = ratesStreamUpdateFromWire(frame);
    expect(upd.subscriptionId).toBe(7n);
    expect(upd.sequence).toBe(5n);
    expect(upd.result).toEqual({
      pv: 42.0,
      parRate: 0.041,
      pv01: 10.0,
      dv01: 11.0,
      keyRateLadder: [],
    });
    expect(upd.curveShift).toBe(0.00007);
    expect(upd.epochNanos).toBe(1_700_000_000_000n);
  });

  it("snapshot omits correlationId when the frame carries none (present-tracked)", () => {
    const frame = {
      subscription: { value: 1 },
      sequence: 1,
      result: { pv: 0, par_rate: 0.04, pv01: 0, dv01: 0, key_rate_ladder: [] },
      curve_shift: 0,
      epoch_nanos: 1,
    };
    const snap = ratesStreamSnapshotFromWire(frame);
    expect("correlationId" in snap).toBe(false);
  });
});

describe("desk create/rename codecs", () => {
  it("createDeskRequestToWire carries only the name (token auto-injected)", () => {
    expect(createDeskRequestToWire("G10 Options")).toEqual({ name: "G10 Options" });
  });

  it("updateDeskRequestToWire carries (id, name) — id is the routing key", () => {
    expect(updateDeskRequestToWire("g10", "G10 Vol")).toEqual({ id: "g10", name: "G10 Vol" });
  });

  it("deskResponseFromWire decodes the desk_updated reply `{ desk: {...} }`", () => {
    const desk = deskResponseFromWire({ desk: { id: "g10", name: "G10 Vol" } });
    expect(desk).toEqual({ id: "g10", name: "G10 Vol" });
  });
});
