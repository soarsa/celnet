/**
 * The FIX session monitor data path: the wire codec, the offline mock feed, and
 * the cursored `useFixMessages` tail.
 *
 *  - the codec maps a captured frame (direction enum int ⇄ string union, 64-bit
 *    seq/epoch as bigint) and a cursored page;
 *  - `MockTransport.listFixMessages` seeds a transcript for a running acceptor and
 *    advances its cursor so a second poll returns only newer frames;
 *  - `useFixMessages` accumulates frames across polls and resets on a session
 *    change.
 */

import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MockTransport } from "../src/data/mockSource";
import {
  fixMessageFromWire,
  fixMsgDirectionFromWire,
  listFixMessagesResponseFromWire,
} from "../src/data/wsCodec";
import { useFixMessages } from "../src/hooks/useFixMessages";

describe("fix monitor wire codec", () => {
  it("maps direction and parses 64-bit fields as bigint", () => {
    expect(fixMsgDirectionFromWire(0)).toBe("INBOUND");
    expect(fixMsgDirectionFromWire(1)).toBe("OUTBOUND");
    const m = fixMessageFromWire({
      seq: 1780000000000000001,
      connection_id: "c1",
      direction: 1,
      msg_type: "S",
      summary: "Quote",
      epoch_nanos: 1780000000000000000,
      raw: "8=FIX.4.4|35=S|10=000",
    });
    expect(typeof m.seq).toBe("bigint");
    expect(m.direction).toBe("OUTBOUND");
    expect(m.summary).toBe("Quote");
    expect(typeof m.epochNanos).toBe("bigint");
  });

  it("reads a cursored page with its latest_seq", () => {
    const page = listFixMessagesResponseFromWire({
      messages: [{ seq: 5, connection_id: "c1", direction: 0, msg_type: "0", summary: "Heartbeat", epoch_nanos: 1, raw: "x" }],
      latest_seq: 5,
    });
    expect(page.messages).toHaveLength(1);
    expect(page.latestSeq).toBe(5n);
  });
});

describe("MockTransport.listFixMessages (offline feed)", () => {
  it("seeds a transcript for the running seed acceptor and tails by cursor", async () => {
    const t = new MockTransport();
    const first = await t.listFixMessages("demo-options", 0n);
    expect(first.messages.length).toBeGreaterThan(0);
    expect(first.messages.every((m) => m.connectionId === "demo-options")).toBe(true);
    expect(first.messages.some((m) => m.direction === "INBOUND")).toBe(true);

    const second = await t.listFixMessages("demo-options", first.latestSeq);
    // Every returned frame is strictly newer than the cursor.
    expect(second.messages.every((m) => m.seq > first.latestSeq)).toBe(true);
    expect(second.latestSeq).toBeGreaterThan(first.latestSeq);
  });

  it("returns nothing for a disabled connection", async () => {
    const t = new MockTransport();
    await t.setFixConnectionEnabled("demo-options", false);
    const page = await t.listFixMessages("demo-options", 0n);
    expect(page.messages).toHaveLength(0);
  });
});

describe("useFixMessages", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("accumulates frames across polls and resets on session change", async () => {
    const t = new MockTransport();
    const { result, rerender } = renderHook(
      ({ id }: { id: string | null }) => useFixMessages(t, id, false),
      { initialProps: { id: "demo-options" as string | null } },
    );

    // The immediate first poll fills the buffer.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const afterFirst = result.current.messages.length;
    expect(afterFirst).toBeGreaterThan(0);

    // A later poll appends more (the mock adds a heartbeat each call).
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1300);
    });
    expect(result.current.messages.length).toBeGreaterThan(afterFirst);

    // Switching to no session clears the buffer (the reset effect runs on commit).
    await act(async () => {
      rerender({ id: null });
    });
    expect(result.current.messages).toHaveLength(0);
  });
});
