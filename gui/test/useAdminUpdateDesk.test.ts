/**
 * useAdmin.updateDesk — the OPTIMISTIC inline desk-rename path.
 *
 * A desk's `id` is the stable routing key (RFQ/deal delivery, `User.deskId` and
 * connection routing all key on it); only its display `name` is editable. A rename
 * must show the new label instantly, reconcile to the server's authoritative desk
 * on success, and roll back (rethrowing) on failure — a duplicate-name rejection
 * being the canonical failure. Driven through a minimal fake transport implementing
 * only the methods the hook touches.
 */

import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DeskDesc } from "../src/data/contract";
import type { CelnetTransport } from "../src/data/transport";
import { useAdmin } from "../src/hooks/useAdmin";

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
}
function defer<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/**
 * A minimal `CelnetTransport` stub: the four list calls the initial load issues +
 * `updateDesk` (whose result the test controls via a deferred). Cast to the full
 * interface — the hook never touches any other method in this test.
 */
function fakeTransport(
  initialDesks: DeskDesc[],
  updateDeferred: Deferred<DeskDesc>,
): { transport: CelnetTransport; updateCalls: Array<{ id: string; name: string }> } {
  const updateCalls: Array<{ id: string; name: string }> = [];
  const transport = {
    listUsers: async () => [],
    listDesks: async () => initialDesks,
    listEntities: async () => [],
    listBooks: async () => [],
    listAggregatedBooks: async () => [],
    updateDesk: async (id: string, name: string) => {
      updateCalls.push({ id, name });
      return updateDeferred.promise;
    },
  } as unknown as CelnetTransport;
  return { transport, updateCalls };
}

describe("useAdmin.updateDesk — optimistic desk rename", () => {
  it("renames optimistically, then reconciles to the returned desk (id unchanged)", async () => {
    const deferred = defer<DeskDesc>();
    const { transport, updateCalls } = fakeTransport(
      [{ id: "g10", name: "G10 Options" }],
      deferred,
    );
    const { result } = renderHook(() => useAdmin(transport, true));

    await waitFor(() => expect(result.current.desks).toHaveLength(1));

    let pending!: Promise<DeskDesc>;
    act(() => {
      pending = result.current.updateDesk("g10", "G10 Vol");
    });

    // Optimistic: the new label shows immediately; the id (routing key) is unchanged.
    expect(result.current.desks[0]).toEqual({ id: "g10", name: "G10 Vol" });
    expect(updateCalls).toEqual([{ id: "g10", name: "G10 Vol" }]);

    // The server returns the canonical desk (distinct object) — reconcile to it.
    const reconciled: DeskDesc = { id: "g10", name: "G10 Vol" };
    await act(async () => {
      deferred.resolve(reconciled);
      await pending;
    });
    expect(result.current.desks[0]).toEqual(reconciled);
  });

  it("rolls back and rejects when the rename collides with another desk", async () => {
    const deferred = defer<DeskDesc>();
    const { transport } = fakeTransport(
      [
        { id: "g10", name: "G10 Options" },
        { id: "em", name: "EM Rates" },
      ],
      deferred,
    );
    const { result } = renderHook(() => useAdmin(transport, true));

    await waitFor(() => expect(result.current.desks).toHaveLength(2));

    let pending!: Promise<DeskDesc>;
    act(() => {
      pending = result.current.updateDesk("g10", "EM Rates");
    });
    // Optimistic label applied before the server replies.
    expect(result.current.desks[0].name).toBe("EM Rates");

    await act(async () => {
      deferred.reject(new Error("a desk named `EM Rates` already exists"));
      await expect(pending).rejects.toThrow(/already exists/);
    });

    // Rolled back to the pre-attempt roster (the original label restored).
    expect(result.current.desks[0]).toEqual({ id: "g10", name: "G10 Options" });
  });
});
