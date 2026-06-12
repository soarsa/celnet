// The multi-dealer ranked-panel task-pane MODEL (src/taskpane/dealerPanel.ts) —
// the headless port of the GUI's `DealerPanel` + `LastLookRing` onto the add-in
// `Connection`. These tests drive the pure projection + last-look math + accept
// with NO Office host and NO live server: a synthetic `MultiDealerQuote` (the same
// shape `multiDealerQuoteFromWire` decodes) feeds `fromMultiDealer`, and the
// accept path is exercised against an in-memory `Connection` over a fake socket,
// asserting the booked `(quote_id, lp_id)` frame is emitted bit-identically.
//
// Parity anchors: (1) the SERVER's ranking order is preserved verbatim — the model
// never re-sorts; (2) the touch winners (`bestBidLpId` / `bestOfferLpId`) badge
// exactly the right lines and an empty winner id badges nothing; (3) the last-look
// fraction depletes 1 → 0 across the window and an elapsed line is expired and not
// tradable; (4) booking a line emits the aggregate `quote_id` (full-precision
// 64-bit literal) + the chosen `lp_id` + the originating idempotency key.

import { describe, expect, it } from "vitest";
import { Connection } from "../src/transport/connection";
import type { WebSocketLike } from "../src/transport/socket";
import {
  DEFAULT_WINDOW_SECONDS,
  accept,
  fromMultiDealer,
  isExpired,
  lastLookRemaining,
  lineFor,
} from "../src/taskpane/dealerPanel";
import type { Conventions, MultiDealerQuote, TwoWayPrice } from "../src/contract/contract";

// ---------------------------------------------------------------------------
// test transport (the same in-memory socket idiom as connection.test.ts /
// rfqPanel.test.ts) — captures the raw wire text of every sent frame so the
// 64-bit `quote_id` literal can be asserted byte-for-byte.
// ---------------------------------------------------------------------------

class FakeSocket implements WebSocketLike {
  readyState = 1;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((data: string) => void) | null = null;
  /** Decoded JSON of every sent frame. */
  readonly sent: Record<string, unknown>[] = [];
  /** The exact wire text of every sent frame (64-bit literal assertions). */
  readonly rawSent: string[] = [];

  constructor() {
    // Open on the next microtask so the Connection's ctor wiring completes first.
    queueMicrotask(() => {
      this.readyState = 1;
      this.onopen?.();
    });
  }

  send(data: string): void {
    this.rawSent.push(data);
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }

  close(): void {
    this.readyState = 3;
    this.onclose?.();
  }

  /** Drive an inbound server frame (snake_case wire text, as the mirror emits). */
  deliver(raw: string): void {
    this.onmessage?.(raw);
  }
}

function makeConnection(): { conn: Connection; socket: FakeSocket } {
  let socket!: FakeSocket;
  const conn = new Connection({
    url: "ws://test",
    factory: () => {
      socket = new FakeSocket();
      return socket;
    },
    stalenessWindowMs: 0,
  });
  return { conn, socket };
}

// ---------------------------------------------------------------------------
// synthetic panel — three LP lines pre-ranked best-first (the server's order),
// the native maker as the touch best-offer winner and SYNTH-LP-1 the best-bid.
// ---------------------------------------------------------------------------

const CONV = {} as Conventions; // conventions are opaque to the panel model.

function price(bid: number, offer: number): TwoWayPrice {
  return { bid, offer };
}

function synthPanel(overrides: Partial<MultiDealerQuote> = {}): MultiDealerQuote {
  return {
    // A 64-bit minted id beyond MAX_SAFE_INTEGER — must round-trip verbatim.
    quoteId: 9_007_199_254_740_993n,
    idempotencyKey: "tk-panel-7",
    dealers: [
      { lpId: "NATIVE", price: price(0.0390, 0.0405), resolvedStrike: 1.085, validUntilNanos: 1_000n },
      { lpId: "SYNTH-LP-1", price: price(0.0392, 0.0410), resolvedStrike: 1.085, validUntilNanos: 2_000n },
      { lpId: "SYNTH-LP-2", price: price(0.0388, 0.0412), resolvedStrike: 1.085, validUntilNanos: 3_000n },
    ],
    bestBidLpId: "SYNTH-LP-1",
    bestOfferLpId: "NATIVE",
    conventions: CONV,
    epochNanos: 0n,
    ...overrides,
  };
}

describe("fromMultiDealer — server ranking preserved verbatim", () => {
  it("projects the dealer lines in the SERVER's order, never re-sorted", () => {
    const panel = fromMultiDealer(synthPanel());
    expect(panel.lines.map((l) => l.lpId)).toEqual(["NATIVE", "SYNTH-LP-1", "SYNTH-LP-2"]);
    // rank is the 0-based frame position, verbatim.
    expect(panel.lines.map((l) => l.rank)).toEqual([0, 1, 2]);
    // Even though SYNTH-LP-2 has the keenest offer (0.0412 is NOT keenest — NATIVE
    // 0.0405 is), the model emits frame order regardless of any local "betterness".
    const reordered = synthPanel({
      dealers: [
        { lpId: "Z", price: price(0.05, 0.06), resolvedStrike: 1, validUntilNanos: 1n },
        { lpId: "A", price: price(0.01, 0.02), resolvedStrike: 1, validUntilNanos: 1n },
      ],
      bestBidLpId: "A",
      bestOfferLpId: "A",
    });
    // "A" is the keener line on both sides but the model keeps the SERVER order Z,A.
    expect(fromMultiDealer(reordered).lines.map((l) => l.lpId)).toEqual(["Z", "A"]);
  });

  it("carries the aggregate quoteId + originating idempotency key", () => {
    const panel = fromMultiDealer(synthPanel());
    expect(panel.quoteId).toBe(9_007_199_254_740_993n);
    expect(panel.idempotencyKey).toBe("tk-panel-7");
  });
});

describe("fromMultiDealer — best markers", () => {
  it("badges exactly the touch best-bid and best-offer lines", () => {
    const panel = fromMultiDealer(synthPanel());
    const native = lineFor(panel, "NATIVE")!;
    const lp1 = lineFor(panel, "SYNTH-LP-1")!;
    const lp2 = lineFor(panel, "SYNTH-LP-2")!;
    expect(native.bestOffer).toBe(true);
    expect(native.bestBid).toBe(false);
    expect(lp1.bestBid).toBe(true);
    expect(lp1.bestOffer).toBe(false);
    expect(lp2.bestBid).toBe(false);
    expect(lp2.bestOffer).toBe(false);
  });

  it("an empty winner id (no dealer quoted that side) badges nothing", () => {
    const panel = fromMultiDealer(synthPanel({ bestBidLpId: "", bestOfferLpId: "NATIVE" }));
    expect(panel.lines.every((l) => l.bestBid === false)).toBe(true);
    expect(lineFor(panel, "NATIVE")!.bestOffer).toBe(true);
  });
});

describe("lastLookRemaining — honest depletion against validUntilNanos", () => {
  // A line whose window is exactly `windowSeconds` long: full → empty as now sweeps.
  const windowSeconds = 4;
  const deadline = BigInt(windowSeconds) * 1_000_000_000n; // 4s in nanos
  const line = fromMultiDealer(
    synthPanel({
      dealers: [
        { lpId: "X", price: price(0.01, 0.02), resolvedStrike: 1, validUntilNanos: deadline },
      ],
      bestBidLpId: "X",
      bestOfferLpId: "X",
    }),
  ).lines[0]!;

  it("is 1 at the window start and depletes to 0 at the deadline", () => {
    expect(lastLookRemaining(line, 0n, windowSeconds)).toBeCloseTo(1, 9);
    expect(lastLookRemaining(line, 1_000_000_000n, windowSeconds)).toBeCloseTo(0.75, 9);
    expect(lastLookRemaining(line, 2_000_000_000n, windowSeconds)).toBeCloseTo(0.5, 9);
    expect(lastLookRemaining(line, deadline, windowSeconds)).toBe(0);
  });

  it("clamps to [0,1]: past the deadline ⇒ 0, before the window ⇒ 1", () => {
    expect(lastLookRemaining(line, deadline + 1n, windowSeconds)).toBe(0);
    // now earlier than (deadline - window) ⇒ more than a full window remains ⇒ 1.
    expect(lastLookRemaining(line, -1_000_000_000n, windowSeconds)).toBe(1);
  });

  it("a degenerate (non-positive) window depletes to 0", () => {
    expect(lastLookRemaining(line, 0n, 0)).toBe(0);
  });

  it("defaults to DEFAULT_WINDOW_SECONDS when the window is omitted", () => {
    const d = BigInt(DEFAULT_WINDOW_SECONDS) * 1_000_000_000n;
    const l = fromMultiDealer(
      synthPanel({
        dealers: [{ lpId: "Y", price: price(0.01, 0.02), resolvedStrike: 1, validUntilNanos: d }],
        bestBidLpId: "Y",
        bestOfferLpId: "Y",
      }),
    ).lines[0]!;
    expect(lastLookRemaining(l, 0n)).toBeCloseTo(1, 9);
    expect(lastLookRemaining(l, d / 2n)).toBeCloseTo(0.5, 9);
  });

  it("marks an elapsed line expired (never tradable on a stale window)", () => {
    expect(isExpired(line, deadline - 1n)).toBe(false);
    expect(isExpired(line, deadline)).toBe(true);
    expect(isExpired(line, deadline + 1n)).toBe(true);
  });
});

describe("accept — books the chosen (quoteId, lpId) line bit-identically", () => {
  it("emits accept_quote with the aggregate quote_id, chosen lp_id, and key", async () => {
    const { conn, socket } = makeConnection();
    await Promise.resolve(); // let the socket open.

    const panel = fromMultiDealer(synthPanel());
    const promise = accept(panel, "SYNTH-LP-1", "SELL", conn);

    // The accept frame the model emitted, in send order.
    const sentFrame = socket.sent.find((f) => f["type"] === "accept_quote")!;
    expect(sentFrame).toBeTruthy();
    expect(sentFrame["lp_id"]).toBe("SYNTH-LP-1");
    expect(sentFrame["idempotency_key"]).toBe("tk-panel-7");
    // The Side enum encodes to its wire index (BUY=0, SELL=1, TWO_WAY=2).
    expect(sentFrame["side"]).toBe(1);

    // The 64-bit quote_id must be written as the full-precision integer LITERAL
    // (no rounding, no exponent) — assert the raw wire text, not the parsed number.
    const rawFrame = socket.rawSent.find((r) => r.includes("accept_quote"))!;
    expect(rawFrame).toContain('"quote_id":9007199254740993');

    // Resolve the in-flight accept with a synthetic Execution so the promise
    // settles. The reply is built as RAW wire text with the 64-bit `quote_id` as a
    // bare integer literal — `JSON.stringify` would round it before `parseFrame`'s
    // `requoteLargeIntegers` could recover it, so the mirror's exact bytes are
    // reproduced here (the same way the server's `execution_to_json` emits them).
    const corr = sentFrame["correlation_id"] as number;
    socket.deliver(
      `{"type":"execution","correlation_id":${corr},"execution_id":42,` +
        `"quote_id":9007199254740993,"side":1,"traded_premium":0.0392,"epoch_nanos":0}`,
    );
    const exec = await promise;
    expect(exec.quoteId).toBe(9_007_199_254_740_993n);
    expect(exec.side).toBe("SELL");
    conn.close();
  });

  it("rejects booking an lpId absent from the panel (stale / re-requested)", async () => {
    const { conn } = makeConnection();
    await Promise.resolve();
    const panel = fromMultiDealer(synthPanel());
    await expect(accept(panel, "GHOST-LP", "BUY", conn)).rejects.toThrow(/no panel line/);
    conn.close();
  });
});
