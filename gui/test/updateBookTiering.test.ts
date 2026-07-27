/**
 * Trader-accessible per-book tiering retune (`AuthService.UpdateBookTiering`,
 * server commit 8404bc9) — the GUI half of the single wire contract.
 *
 *   1. wsCodec round-trip — `updateBookTieringRequestToWire` produces the EXACT
 *      snake_case body the server's `update_book_tiering_request_from_json`
 *      decodes (`{ book_id, tiering }`, an absent tiering ⇒ `null` ⇒ disable),
 *      REUSING the SAME `tieringConfigToWire` shape the aggregated-book spec
 *      already encodes; `bookTieringUpdatedResponseFromWire` reads the server's
 *      `{ book }` envelope back into an `AggregatedBookDesc`.
 *
 *   2. mock transport — the offline `createMockTransport()` genuinely retunes ONLY
 *      the selected book's tiering (structure untouched) and returns the retuned
 *      book, over the SAME transport seam the live WS transport implements.
 */

import { describe, expect, it } from "vitest";

import type { TieringConfig } from "../src/data/contract";
import { defaultTieringConfig } from "../src/lib/tiering";
import {
  bookTieringUpdatedResponseFromWire,
  tieringConfigToWire,
  updateBookTieringRequestToWire,
} from "../src/data/wsCodec";
import { createMockTransport } from "../src/data/mockSource";

const CONFIG: TieringConfig = defaultTieringConfig();

describe("update-book-tiering request codec", () => {
  it("encodes { book_id, tiering } with the shared tieringConfig shape", () => {
    const wire = updateBookTieringRequestToWire("us-treasuries", CONFIG);
    expect(wire).toEqual({
      book_id: "us-treasuries",
      // Byte-identical to the shape the aggregated-book spec's `tiering` uses.
      tiering: tieringConfigToWire(CONFIG),
    });
    // Spot-check the nested wire is the server's snake_case, numeric-enum shape.
    const t = wire.tiering as Record<string, unknown>;
    expect(t.unit).toBe(0); // PRICE_BPS
    expect(t.stale_policy).toBe(0); // SUPPRESS
    const strat = (t.strategies as Record<string, unknown>[])[0]!;
    expect(strat.kind).toBe(0); // FLAT_MARKUP
    expect(strat.half_spread).toBe(25);
  });

  it("encodes an absent tiering as null (⇒ disable the book's tiering)", () => {
    expect(updateBookTieringRequestToWire("us-treasuries", null)).toEqual({
      book_id: "us-treasuries",
      tiering: null,
    });
  });

  it("decodes the book_tiering_updated { book } envelope back to a desc", () => {
    const book = bookTieringUpdatedResponseFromWire({
      book: {
        id: "us-treasuries",
        name: "US Treasuries",
        member_connection_ids: ["LP-SIM-01"],
        scope_mode: 0,
        instrument_ids: [],
        params: {
          staleness_tau_ms: 2000,
          max_quote_age_ms: 5000,
          divergence_gating: true,
          min_contributors: 2,
          depth_levels: 1,
        },
        enabled: true,
        tiering: tieringConfigToWire(CONFIG),
      },
      correlation_id: 7,
    });
    expect(book.id).toBe("us-treasuries");
    expect(book.tiering).not.toBeNull();
    expect(book.tiering!.strategies[0]!.kind).toBe("FLAT_MARKUP");
    expect(book.tiering!.strategies[0]!.halfSpread).toBe(25);
  });
});

describe("mock transport updateBookTiering", () => {
  it("retunes ONLY the selected book's tiering and returns it", async () => {
    const t = createMockTransport();
    const before = await t.listAggregatedBooks();
    const target = before[0]!; // the seeded `us-treasuries` book (tiering: null)
    expect(target.tiering).toBeNull();

    const updated = await t.updateBookTiering(target.id, CONFIG);
    expect(updated.id).toBe(target.id);
    expect(updated.tiering).not.toBeNull();
    expect(updated.tiering!.strategies[0]!.kind).toBe("FLAT_MARKUP");
    // Structure is untouched — only tiering changed.
    expect(updated.memberConnectionIds).toEqual(target.memberConnectionIds);
    expect(updated.scopeMode).toBe(target.scopeMode);

    // The change persisted in the store (a re-list shows it).
    const after = await t.listAggregatedBooks();
    expect(after.find((b) => b.id === target.id)!.tiering).not.toBeNull();

    // Passing null disables tiering again.
    const disabled = await t.updateBookTiering(target.id, null);
    expect(disabled.tiering).toBeNull();
  });

  it("rejects an unknown book id", async () => {
    const t = createMockTransport();
    await expect(t.updateBookTiering("no-such-book", CONFIG)).rejects.toThrow();
  });
});
