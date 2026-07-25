/**
 * FI Aggregated Book (ADR-0022) — the GUI half of the single wire contract. Two
 * concerns, gated here without a server:
 *
 *   1. wsCodec round-trip — the admin CRUD request/response encoders + the live
 *      composite subscribe/snapshot/update codecs produce and read the EXACT
 *      snake_case, numeric-enum JSON the server codec
 *      (`crates/celnet-server/src/ws/codec.rs`) decodes/encodes: scope_mode as a
 *      numeric enum, params nested under `params`, `subscription.value`,
 *      instrument identity + per-LP contributions + the `stale` flag.
 *
 *   2. mock transport + stream — the offline `createMockTransport()` genuinely
 *      stores an aggregated book (create → list) and its stream session emits a
 *      baseline `aggregatedBookSnapshot` then live `aggregatedBookUpdate`s over
 *      the SAME `StreamSession` seam the live WS transport implements, so the GUI
 *      price view is exercisable end-to-end offline.
 */

import { describe, expect, it } from "vitest";

import type { AggregatedBookSpec } from "../src/data/contract";
import {
  aggregatedBookDescFromWire,
  aggregatedBookResponseFromWire,
  aggregatedBookSnapshotFromWire,
  aggregatedBookSubscribeToWire,
  aggregatedBookUpdateFromWire,
  aggregatedBooksResponseFromWire,
  createAggregatedBookRequestToWire,
  deleteAggregatedBookRequestToWire,
  updateAggregatedBookRequestToWire,
} from "../src/data/wsCodec";
import { createMockTransport, MockTransport } from "../src/data/mockSource";
import type { StreamEvent } from "../src/data/transport";

const SPEC: AggregatedBookSpec = {
  id: "",
  name: "US Treasuries",
  memberConnectionIds: ["LP-SIM-01", "LP-SIM-02", "LP-SIM-03"],
  scopeMode: "EXPLICIT",
  instrumentIds: ["91282CJL6", "91282CJP7"],
  params: {
    stalenessTauMs: 2000,
    maxQuoteAgeMs: 5000,
    divergenceGating: true,
    minContributors: 2,
    depthLevels: 1,
  },
  enabled: true,
  tiering: null,
};

describe("aggregated-book CRUD codec", () => {
  it("encodes a create request in the server's snake_case, numeric-enum shape", () => {
    const wire = createAggregatedBookRequestToWire(SPEC);
    // The nested `spec` object is exactly what `aggregated_book_spec_from_json` reads.
    expect(wire).toEqual({
      spec: {
        id: "",
        name: "US Treasuries",
        member_connection_ids: ["LP-SIM-01", "LP-SIM-02", "LP-SIM-03"],
        scope_mode: 1, // EXPLICIT
        instrument_ids: ["91282CJL6", "91282CJP7"],
        params: {
          staleness_tau_ms: 2000,
          max_quote_age_ms: 5000,
          divergence_gating: true,
          min_contributors: 2,
          depth_levels: 1,
        },
        enabled: true,
        tiering: null,
      },
    });
  });

  it("encodes ALL_MEMBERS_QUOTE as scope_mode 0", () => {
    const wire = createAggregatedBookRequestToWire({
      ...SPEC,
      scopeMode: "ALL_MEMBERS_QUOTE",
    });
    expect((wire.spec as Record<string, unknown>).scope_mode).toBe(0);
  });

  it("encodes update (id + spec) and delete (id) bodies", () => {
    const upd = updateAggregatedBookRequestToWire("us-treasuries", SPEC);
    expect(upd.id).toBe("us-treasuries");
    expect((upd.spec as Record<string, unknown>).name).toBe("US Treasuries");
    expect(deleteAggregatedBookRequestToWire("us-treasuries")).toEqual({
      id: "us-treasuries",
    });
  });

  it("decodes a book desc (numeric scope_mode → union; params → nested)", () => {
    const desc = aggregatedBookDescFromWire({
      id: "us-treasuries",
      name: "US Treasuries",
      member_connection_ids: ["LP-SIM-01", "LP-SIM-02"],
      scope_mode: 1,
      instrument_ids: ["91282CJL6"],
      params: {
        staleness_tau_ms: 2000,
        max_quote_age_ms: 5000,
        divergence_gating: true,
        min_contributors: 2,
        depth_levels: 1,
      },
      enabled: true,
    });
    expect(desc.scopeMode).toBe("EXPLICIT");
    expect(desc.memberConnectionIds).toEqual(["LP-SIM-01", "LP-SIM-02"]);
    expect(desc.params.stalenessTauMs).toBe(2000);
    expect(desc.enabled).toBe(true);
  });

  it("decodes a null params (server renders absent params as null) to defaults", () => {
    const desc = aggregatedBookDescFromWire({
      id: "x",
      name: "X",
      member_connection_ids: [],
      scope_mode: 0,
      instrument_ids: [],
      params: null,
      enabled: false,
    });
    expect(desc.params.minContributors).toBeGreaterThanOrEqual(1);
    expect(desc.scopeMode).toBe("ALL_MEMBERS_QUOTE");
  });

  it("decodes the roster + single-book responses", () => {
    const roster = aggregatedBooksResponseFromWire({
      books: [
        {
          id: "a",
          name: "A",
          member_connection_ids: ["LP-SIM-01"],
          scope_mode: 0,
          instrument_ids: [],
          params: {
            staleness_tau_ms: 1,
            max_quote_age_ms: 2,
            divergence_gating: false,
            min_contributors: 1,
            depth_levels: 1,
          },
          enabled: true,
        },
      ],
    });
    expect(roster).toHaveLength(1);
    expect(roster[0]!.id).toBe("a");
    const single = aggregatedBookResponseFromWire({
      book: { ...roster[0], id: "a", name: "A", scope_mode: 0 },
    });
    expect(single.id).toBe("a");
  });
});

describe("aggregated-book live composite codec", () => {
  it("encodes a subscribe frame with subscription.value + book_id + throttle", () => {
    const wire = aggregatedBookSubscribeToWire({
      subscriptionId: 7n,
      bookId: "us-treasuries",
      throttleNanos: 250_000n,
      correlationId: 42n,
    });
    expect(wire).toEqual({
      subscription: { value: 7 },
      book_id: "us-treasuries",
      throttle_nanos: 250000,
      correlation_id: 42,
    });
  });

  it("omits an absent correlation id and defaults throttle to 0", () => {
    const wire = aggregatedBookSubscribeToWire({ subscriptionId: 1n, bookId: "b" });
    expect(wire.throttle_nanos).toBe(0);
    expect("correlation_id" in wire).toBe(false);
  });

  it("decodes a snapshot frame (identity + best two-way + contributions + stale)", () => {
    const snap = aggregatedBookSnapshotFromWire({
      subscription: { value: 3 },
      sequence: 1,
      correlation_id: 9,
      epoch_nanos: 1_700_000_000_000_000_000,
      book: {
        book_id: "us-treasuries",
        instruments: [
          {
            instrument_id: "91282CJP7",
            display_name: "UST 10Y 4.375%",
            isin: "US91282CJP77",
            cusip: "91282CJP7",
            best_bid: 97.4,
            best_offer: 97.44,
            bid_size: 3_000_000,
            offer_size: 2_000_000,
            confidence: 0.92,
            contributions: [
              { lp_name: "LP-SIM-01", bid: 97.4, offer: 97.45, stale: false },
              { lp_name: "LP-SIM-02", bid: 97.38, offer: 97.44, stale: false },
              { lp_name: "LP-SIM-03", bid: 97.2, offer: 97.6, stale: true },
            ],
          },
        ],
      },
    });
    expect(snap.subscriptionId).toBe(3n);
    expect(snap.sequence).toBe(1n);
    expect(snap.correlationId).toBe(9n);
    expect(snap.book.bookId).toBe("us-treasuries");
    expect(snap.book.instruments).toHaveLength(1);
    const inst = snap.book.instruments[0]!;
    expect(inst.displayName).toBe("UST 10Y 4.375%");
    expect(inst.isin).toBe("US91282CJP77");
    expect(inst.cusip).toBe("91282CJP7");
    expect(inst.bestBid).toBeCloseTo(97.4);
    expect(inst.bestOffer).toBeCloseTo(97.44);
    expect(inst.contributions).toHaveLength(3);
    expect(inst.contributions[2]!.stale).toBe(true);
    expect(inst.contributions[2]!.lpName).toBe("LP-SIM-03");
  });

  it("decodes an update frame (no correlation id)", () => {
    const upd = aggregatedBookUpdateFromWire({
      subscription: { value: 3 },
      sequence: 2,
      epoch_nanos: 1_700_000_000_000_000_001,
      book: { book_id: "b", instruments: [] },
    });
    expect(upd.subscriptionId).toBe(3n);
    expect(upd.sequence).toBe(2n);
    expect(upd.book.instruments).toEqual([]);
  });
});

describe("mock transport aggregated-book CRUD + stream", () => {
  it("stores a created book and lists it back", async () => {
    const t = createMockTransport();
    const created = await t.createAggregatedBook({ ...SPEC, name: "Gilts" });
    expect(created.id).toBe("gilts"); // slug of the name
    expect(created.memberConnectionIds).toEqual(SPEC.memberConnectionIds);
    const list = await t.listAggregatedBooks();
    expect(list.some((b) => b.id === "gilts")).toBe(true);
    // Update is by immutable id; delete removes it.
    const renamed = await t.updateAggregatedBook("gilts", { ...SPEC, name: "UK Gilts" });
    expect(renamed.name).toBe("UK Gilts");
    expect(await t.deleteAggregatedBook("gilts")).toBe(true);
    expect((await t.listAggregatedBooks()).some((b) => b.id === "gilts")).toBe(false);
  });

  it("rejects a duplicate book name", async () => {
    const t = createMockTransport();
    await t.createAggregatedBook({ ...SPEC, name: "Dup" });
    await expect(t.createAggregatedBook({ ...SPEC, name: "Dup" })).rejects.toThrow();
  });

  it("emits a baseline snapshot then live updates for a subscribed book", async () => {
    const t = new MockTransport({ tickMs: 5 });
    const book = await t.createAggregatedBook({ ...SPEC, name: "Stream Book" });
    const session = t.openStreamSession();
    const events: StreamEvent[] = [];
    session.onEvent((e) => events.push(e));
    session.subscribeAggregatedBook(book.id);

    // The baseline snapshot is emitted synchronously inside subscribe.
    const snapshot = events.find((e) => e.kind === "aggregatedBookSnapshot");
    expect(snapshot).toBeDefined();
    if (snapshot && snapshot.kind === "aggregatedBookSnapshot") {
      expect(snapshot.snapshot.sequence).toBe(1n);
      // EXPLICIT scope over the two seed instruments the spec names.
      expect(snapshot.snapshot.book.instruments.length).toBeGreaterThan(0);
      const inst = snapshot.snapshot.book.instruments[0]!;
      expect(inst.contributions.length).toBe(SPEC.memberConnectionIds.length);
      expect(inst.bestOffer).toBeGreaterThanOrEqual(inst.bestBid);
    }

    // A live update lands on the next tick.
    await new Promise((r) => setTimeout(r, 30));
    const update = events.find((e) => e.kind === "aggregatedBookUpdate");
    expect(update).toBeDefined();
    if (update && update.kind === "aggregatedBookUpdate") {
      expect(update.update.sequence).toBeGreaterThan(1n);
    }
    session.close();
  });
});
