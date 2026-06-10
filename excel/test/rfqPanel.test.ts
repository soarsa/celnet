// PHASE-4b — Excel client parity for the multi-dealer ranked panel (RFQ-to-many)
// the server's WS mirror carries: `request_multi_dealer_quote` → the
// `multi_dealer_quote` frame (one `DealerQuote` line per LP, pre-ranked
// best-first by the server aggregator), and the `accept_quote` that books a
// chosen line by `(quote_id, lp_id)`. These tests prove the add-in decodes the
// frame field-for-field against the server's `multi_dealer_quote_to_json`
// (snake_case, presence-tracked nulls), spills the panel in the SERVER's ranking
// order (never a client re-sort), and emits the accept frame exactly — with
// `lp_id` present only when a panel line is selected, so the single-dealer
// accept stays byte-identical to the pre-panel contract. The in-repo panel
// sources are the native maker plus labeled deterministic synthetic dealers
// (`SYNTH-LP-k`); live LP connectivity is environment-provided, never claimed.

import { describe, expect, it } from "vitest";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import { multiDealerQuoteFromWire } from "../src/contract/wsCodec";
import {
  DEFAULT_CONVENTIONS,
  formatRfqPanelSpill,
  parseRfqPanelFlag,
  shapeVanillaInstrument,
  ShapingError,
  type RfqPanelResult,
} from "../src/functions/shaping";
import type { MultiDealerQuote } from "../src/contract/contract";

// ---------------------------------------------------------------------------
// test transport (the same in-memory socket idiom as connection.test.ts)
// ---------------------------------------------------------------------------

class FakeSocket implements WebSocketLike {
  readyState = 0;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((data: string) => void) | null = null;
  readonly sent: Record<string, unknown>[] = [];
  /** The exact wire text of every sent frame (64-bit literal assertions). */
  readonly rawSent: string[] = [];
  send(data: string): void {
    this.rawSent.push(data);
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
  deliver(frame: Record<string, unknown>): void {
    this.onmessage?.(JSON.stringify(frame));
  }
  /** Deliver an exact wire text (frames carrying integers beyond MAX_SAFE). */
  deliverRaw(raw: string): void {
    this.onmessage?.(raw);
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

const INSTR = shapeVanillaInstrument({
  pair: "EURUSD",
  tenor: "1Y",
  strikeOrDelta: 1.12,
  callPut: "C",
  notional: 1e6,
});

/**
 * A SERVER-SHAPED `multi_dealer_quote` body — the exact snake_case JSON the
 * server's `multi_dealer_quote_to_json` emits for a native maker + 3 synthetic
 * dealers panel (the demo edge's ≥3-LP shape). Ranking is the server's: the
 * dealers array is best-first; only the native line carries greeks / std-error
 * (synthetic lines carry `null` — presence-tracked proto optionals).
 */
function serverPanelBody(): Record<string, unknown> {
  return {
    quote_id: 42,
    idempotency_key: "panel:rfq:EURUSD",
    dealers: [
      {
        lp_id: "SYNTH-LP-2",
        price: { bid: 0.0405, offer: 0.0419 },
        greeks: null,
        resolved_strike: 1.12,
        valid_until_nanos: 5_000_000_000,
        attribution: { quotedBy: { book: "SYNTH", owner: { autoPricer: "SYNTH-LP-2" } } },
        price_std_error: null,
      },
      {
        lp_id: "celnet-auto-pricer",
        price: { bid: 0.0401, offer: 0.0421 },
        greeks: { price: 0.0411, delta_spot: 0.52, vega: 0.39 },
        resolved_strike: 1.12,
        valid_until_nanos: 5_000_000_000,
        attribution: { quotedBy: { book: "AUTO-MM", owner: { autoPricer: "celnet-auto-pricer" } } },
        price_std_error: null,
      },
      {
        lp_id: "SYNTH-LP-1",
        price: { bid: 0.0399, offer: 0.0423 },
        greeks: null,
        resolved_strike: 1.12,
        valid_until_nanos: 5_000_000_000,
        attribution: { quotedBy: { book: "SYNTH", owner: { autoPricer: "SYNTH-LP-1" } } },
        price_std_error: null,
      },
      {
        lp_id: "SYNTH-LP-3",
        price: { bid: 0.0395, offer: 0.0427 },
        greeks: null,
        resolved_strike: 1.12,
        valid_until_nanos: 5_000_000_000,
        attribution: { quotedBy: { book: "SYNTH", owner: { autoPricer: "SYNTH-LP-3" } } },
        price_std_error: null,
      },
    ],
    best_bid_lp_id: "SYNTH-LP-2",
    best_offer_lp_id: "SYNTH-LP-2",
    conventions: {},
    epoch_nanos: 1_000_000_000,
    correlation_id: null,
    surface_version: 7,
  };
}

/** The decoded panel projected onto the spill formatter's input. */
function panelResultOf(md: MultiDealerQuote): RfqPanelResult {
  return {
    quoteId: md.quoteId,
    lines: md.dealers.map((d) => ({
      lpId: d.lpId,
      bid: d.price.bid,
      offer: d.price.offer,
      validUntilNanos: d.validUntilNanos,
    })),
    bestBidLpId: md.bestBidLpId,
    bestOfferLpId: md.bestOfferLpId,
    conventions: md.conventions,
    surfaceVersion: md.surfaceVersion,
    epochNanos: md.epochNanos,
  };
}

// ---------------------------------------------------------------------------
// codec: the multi_dealer_quote frame round-trips field-for-field
// ---------------------------------------------------------------------------

describe("multi_dealer_quote codec (the exact mirror of multi_dealer_quote_to_json)", () => {
  it("decodes every field of a server-shaped panel frame", () => {
    const md = multiDealerQuoteFromWire(serverPanelBody());
    expect(md.quoteId).toBe(42n);
    expect(md.idempotencyKey).toBe("panel:rfq:EURUSD");
    expect(md.dealers.length).toBe(4);
    expect(md.bestBidLpId).toBe("SYNTH-LP-2");
    expect(md.bestOfferLpId).toBe("SYNTH-LP-2");
    expect(md.epochNanos).toBe(1_000_000_000n);
    expect(md.surfaceVersion).toBe(7n);
    // Presence-tracked: a `null` correlation_id decodes to undefined.
    expect(md.correlationId).toBeUndefined();

    const native = md.dealers.find((d) => d.lpId === "celnet-auto-pricer")!;
    expect(native.price).toEqual({ bid: 0.0401, offer: 0.0421 });
    expect(native.resolvedStrike).toBe(1.12);
    expect(native.validUntilNanos).toBe(5_000_000_000n);
    // Only the native maker line carries greeks (an LP discloses a price, not
    // its greeks) — `null` decodes to undefined, never a fabricated zero strip.
    expect(native.greeks?.price).toBe(0.0411);
    expect(native.greeks?.deltaSpot).toBe(0.52);
    const synth = md.dealers.find((d) => d.lpId === "SYNTH-LP-1")!;
    expect(synth.greeks).toBeUndefined();
    expect(synth.priceStdError).toBeUndefined();
  });

  it("preserves the server's ranking order verbatim (no client re-sort)", () => {
    const md = multiDealerQuoteFromWire(serverPanelBody());
    // The server orders best-first; here that order is deliberately NOT
    // alphabetical, so any client-side sort would be caught.
    expect(md.dealers.map((d) => d.lpId)).toEqual([
      "SYNTH-LP-2",
      "celnet-auto-pricer",
      "SYNTH-LP-1",
      "SYNTH-LP-3",
    ]);
  });
});

// ---------------------------------------------------------------------------
// spill geometry: header + one ranked row per LP + quote_id + footer
// ---------------------------------------------------------------------------

describe("CELNET.RFQ panel spill", () => {
  it("spills header, one row per LP in rank order, BEST markers, quote_id and footer", () => {
    const md = multiDealerQuoteFromWire(serverPanelBody());
    const spill = formatRfqPanelSpill(panelResultOf(md));
    // header + 4 LP rows + quote_id row + convention footer
    expect(spill.length).toBe(7);
    expect(spill[0]).toEqual(["lp_id", "bid", "offer", "valid_until", "best"]);
    // Rank order preserved row-for-row.
    expect(spill.slice(1, 5).map((r) => r[0])).toEqual([
      "SYNTH-LP-2",
      "celnet-auto-pricer",
      "SYNTH-LP-1",
      "SYNTH-LP-3",
    ]);
    // The touch dealer is marked on BOTH sides here (it wins bid AND offer).
    expect(spill[1]).toEqual([
      "SYNTH-LP-2",
      0.0405,
      0.0419,
      new Date(5_000).toISOString(),
      "BEST_BID+BEST_OFFER",
    ]);
    // Non-touch rows carry an empty marker, never a fabricated one.
    expect(spill[2]![4]).toBe("");
    expect(spill[3]![4]).toBe("");
    // The aggregate quote id (a string — 64-bit precision) for the accept path.
    expect(spill[5]).toEqual(["quote_id", "42"]);
    // The convention-transparency footer (docs §3.4) closes the spill.
    expect(String(spill[6]![0])).toContain("conv:");
    expect(String(spill[6]![0])).toContain("surface v7");
  });

  it("marks distinct BEST_BID and BEST_OFFER rows when the touch is split", () => {
    const md = multiDealerQuoteFromWire({
      ...serverPanelBody(),
      best_bid_lp_id: "celnet-auto-pricer",
      best_offer_lp_id: "SYNTH-LP-2",
    });
    const spill = formatRfqPanelSpill(panelResultOf(md));
    expect(spill[1]![4]).toBe("BEST_OFFER");
    expect(spill[2]![4]).toBe("BEST_BID");
  });
});

// ---------------------------------------------------------------------------
// transport: panel request + accept by (quote_id, lp_id) over the wire
// ---------------------------------------------------------------------------

describe("Connection multi-dealer panel round-trip", () => {
  it("sends request_multi_dealer_quote and decodes the multi_dealer_quote reply", async () => {
    const { conn, sock } = makeWiredConnection();
    const p = conn.requestMultiDealerQuote(INSTR, DEFAULT_CONVENTIONS, "panel:rfq:EURUSD");
    const sent = sock.sentOfType("request_multi_dealer_quote")[0]!;
    // The SAME QuoteRequest body as a single-dealer RFQ — one contract, two verbs.
    expect(sent["idempotency_key"]).toBe("panel:rfq:EURUSD");
    expect(sent["instrument"]).toBeTruthy();
    expect(sent["conventions"]).toBeTruthy();
    sock.deliver({
      type: "multi_dealer_quote",
      ...serverPanelBody(),
      correlation_id: sent["correlation_id"],
    });
    const md = await p;
    expect(md.quoteId).toBe(42n);
    expect(md.dealers.map((d) => d.lpId)).toEqual([
      "SYNTH-LP-2",
      "celnet-auto-pricer",
      "SYNTH-LP-1",
      "SYNTH-LP-3",
    ]);
  });

  it("accept of a panel line emits (quote_id, lp_id) with the originating key", async () => {
    const { conn, sock } = makeWiredConnection();
    const p = conn.acceptQuote({
      quoteId: 42n,
      side: "BUY",
      idempotencyKey: "panel:rfq:EURUSD",
      lpId: "SYNTH-LP-2",
    });
    const sent = sock.sentOfType("accept_quote")[0]!;
    expect(sent["quote_id"]).toBe(42);
    expect(sent["lp_id"]).toBe("SYNTH-LP-2");
    expect(sent["idempotency_key"]).toBe("panel:rfq:EURUSD");
    expect(sent["side"]).toBe(0); // BUY — lifts the chosen dealer's offer
    sock.deliver({
      type: "execution",
      correlation_id: sent["correlation_id"],
      execution_id: 9,
      quote_id: 42,
      side: 0,
      traded_premium: 0.0419,
      epoch_nanos: 2_000_000_000,
    });
    const exec = await p;
    expect(exec.executionId).toBe(9n);
    expect(exec.quoteId).toBe(42n);
    expect(exec.side).toBe("BUY");
    expect(exec.tradedPremium).toBe(0.0419);
  });

  it("a single-dealer accept omits lp_id entirely (byte-identical pre-panel frame)", async () => {
    const { conn, sock } = makeWiredConnection();
    const p = conn.acceptQuote({ quoteId: 7n, side: "SELL", idempotencyKey: "rfq:EURUSD" });
    const sent = sock.sentOfType("accept_quote")[0]!;
    expect("lp_id" in sent).toBe(false);
    expect(sent["quote_id"]).toBe(7);
    expect(sent["side"]).toBe(1); // SELL — hits the bid
    sock.deliver({
      type: "execution",
      correlation_id: sent["correlation_id"],
      execution_id: 1,
      quote_id: 7,
      side: 1,
      traded_premium: 0.0401,
      epoch_nanos: 0,
    });
    const exec = await p;
    expect(exec.side).toBe("SELL");
  });

  it("preserves a minted quote_id beyond MAX_SAFE bit-for-bit (panel decode → accept echo)", async () => {
    // The edge mints quote ids over the FULL u64 range (splitmix64), so almost
    // every real id exceeds Number.MAX_SAFE_INTEGER. A plain JSON.parse rounds
    // the panel's id and a `Number()` echo rounds the accept's — the server then
    // refuses the booking as `unknown quote_id` (the failure the live GUI panel
    // e2e caught). Drive the EXACT wire text end-to-end: decode keeps the id
    // exact, and the accept echoes the same bare 64-bit literal.
    const id = "4385739192607958123"; // > 2^53; rounds to …958000 as a double
    const { conn, sock } = makeWiredConnection();
    const p = conn.requestMultiDealerQuote(INSTR, DEFAULT_CONVENTIONS, "panel:big-id");
    const sent = sock.sentOfType("request_multi_dealer_quote")[0]!;
    const raw = JSON.stringify({
      type: "multi_dealer_quote",
      ...serverPanelBody(),
      idempotency_key: "panel:big-id",
      correlation_id: sent["correlation_id"],
    }).replace('"quote_id":42', `"quote_id":${id}`);
    sock.deliverRaw(raw);
    const md = await p;
    expect(md.quoteId).toBe(BigInt(id));

    const accept = conn.acceptQuote({
      quoteId: md.quoteId,
      side: "BUY",
      idempotencyKey: md.idempotencyKey,
      lpId: "SYNTH-LP-2",
    });
    const acceptRaw = sock.rawSent.find((t) => t.includes('"accept_quote"'))!;
    expect(acceptRaw).toContain(`"quote_id":${id}`);
    sock.deliverRaw(
      JSON.stringify({
        type: "execution",
        correlation_id: sock.sentOfType("accept_quote")[0]!["correlation_id"],
        execution_id: 9,
        quote_id: 1, // placeholder; replaced with the exact literal below
        side: 0,
        traded_premium: 0.0419,
        epoch_nanos: 2_000_000_000,
      }).replace('"quote_id":1', `"quote_id":${id}`),
    );
    const exec = await accept;
    expect(exec.quoteId).toBe(BigInt(id));
    expect(exec.tradedPremium).toBe(0.0419);
  });
});

// ---------------------------------------------------------------------------
// the CELNET.RFQ panel flag
// ---------------------------------------------------------------------------

describe("RFQ panel-flag parsing", () => {
  it("absent/FALSE/empty keep the single-dealer RFQ; TRUE/'PANEL' select the panel", () => {
    expect(parseRfqPanelFlag(undefined)).toBe(false);
    expect(parseRfqPanelFlag(false)).toBe(false);
    expect(parseRfqPanelFlag("")).toBe(false);
    expect(parseRfqPanelFlag("false")).toBe(false);
    expect(parseRfqPanelFlag(true)).toBe(true);
    expect(parseRfqPanelFlag("PANEL")).toBe(true);
    expect(parseRfqPanelFlag("panel")).toBe(true);
    expect(parseRfqPanelFlag("true")).toBe(true);
  });

  it("rejects a typo loudly rather than silently degrading to single-dealer", () => {
    expect(() => parseRfqPanelFlag("PANNEL")).toThrow(ShapingError);
  });
});
