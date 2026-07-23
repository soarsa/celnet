/**
 * useAdmin.setUserDesks — the OPTIMISTIC inline desk-membership path.
 *
 * Desk membership (MANY-TO-MANY) scopes which inbound quotes/deals a trader
 * receives, so the roster must reflect a change instantly, reconcile to the
 * server's authoritative row on success, and roll back (rethrowing) on failure —
 * never a full refetch (which would defeat the optimistic path). Driven through a
 * minimal fake transport implementing only the methods the hook touches.
 */

import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { UserDesc } from "../src/data/contract";
import type { CelnetTransport } from "../src/data/transport";
import { useAdmin } from "../src/hooks/useAdmin";

function makeUser(overrides: Partial<UserDesc> = {}): UserDesc {
  return {
    id: "u1",
    email: "trader@celnet.com",
    displayName: "Jane Trader",
    role: "TRADER",
    deskIds: [],
    allDesks: false,
    disabled: false,
    ...overrides,
  };
}

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
 * A minimal `CelnetTransport` stub: only the four list calls the initial load
 * issues + `updateUser` (whose result the test controls via a deferred). Cast to
 * the full interface — the hook never touches any other method in this test.
 */
function fakeTransport(
  initialUsers: UserDesc[],
  updateDeferred: Deferred<UserDesc>,
): { transport: CelnetTransport; updateCalls: Array<{ id: string; input: unknown }> } {
  const updateCalls: Array<{ id: string; input: unknown }> = [];
  const transport = {
    listUsers: async () => initialUsers,
    listDesks: async () => [
      { id: "g10", name: "G10 Options" },
      { id: "em", name: "EM Rates" },
    ],
    listEntities: async () => [],
    listBooks: async () => [],
    listAggregatedBooks: async () => [],
    updateUser: async (id: string, input: unknown) => {
      updateCalls.push({ id, input });
      return updateDeferred.promise;
    },
  } as unknown as CelnetTransport;
  return { transport, updateCalls };
}

describe("useAdmin.setUserDesks — optimistic multi-desk membership", () => {
  it("applies a two-desk set optimistically, sends desk_ids+all_desks, then reconciles", async () => {
    const deferred = defer<UserDesc>();
    const { transport, updateCalls } = fakeTransport([makeUser()], deferred);
    const { result } = renderHook(() => useAdmin(transport, true));

    await waitFor(() => expect(result.current.users).toHaveLength(1));
    expect(result.current.users[0].deskIds).toEqual([]);

    // Kick off the membership change but DO NOT resolve the transport yet.
    let pending!: Promise<void>;
    act(() => {
      pending = result.current.setUserDesks("u1", ["g10", "em"], false);
    });

    // Optimistic: both desks show immediately, before the server replies.
    expect(result.current.users[0].deskIds).toEqual(["g10", "em"]);
    expect(result.current.users[0].allDesks).toBe(false);
    expect(updateCalls).toEqual([
      {
        id: "u1",
        input: {
          displayName: "Jane Trader",
          role: "TRADER",
          disabled: false,
          deskIds: ["g10", "em"],
          allDesks: false,
        },
      },
    ]);

    // The server returns a canonical row (distinct object) — reconcile to it.
    const reconciled = makeUser({ deskIds: ["g10", "em"], displayName: "Jane Trader (server)" });
    await act(async () => {
      deferred.resolve(reconciled);
      await pending;
    });
    expect(result.current.users[0]).toEqual(reconciled);
    expect(result.current.users[0].displayName).toBe("Jane Trader (server)");
  });

  it("sends allDesks=true with an empty set when All desks is toggled on", async () => {
    const deferred = defer<UserDesc>();
    const { transport, updateCalls } = fakeTransport([makeUser({ deskIds: ["g10"] })], deferred);
    const { result } = renderHook(() => useAdmin(transport, true));

    await waitFor(() => expect(result.current.users).toHaveLength(1));

    let pending!: Promise<void>;
    act(() => {
      pending = result.current.setUserDesks("u1", [], true);
    });

    expect(result.current.users[0].allDesks).toBe(true);
    expect(result.current.users[0].deskIds).toEqual([]);
    expect(updateCalls[0].input).toMatchObject({ deskIds: [], allDesks: true });

    await act(async () => {
      deferred.resolve(makeUser({ allDesks: true }));
      await pending;
    });
    expect(result.current.users[0].allDesks).toBe(true);
  });

  it("rolls the roster back and rejects when the update fails", async () => {
    const deferred = defer<UserDesc>();
    const { transport } = fakeTransport([makeUser({ deskIds: ["g10"] })], deferred);
    const { result } = renderHook(() => useAdmin(transport, true));

    await waitFor(() => expect(result.current.users).toHaveLength(1));

    let pending!: Promise<void>;
    act(() => {
      pending = result.current.setUserDesks("u1", ["g10", "em"], false);
    });
    expect(result.current.users[0].deskIds).toEqual(["g10", "em"]); // optimistic

    await act(async () => {
      deferred.reject(new Error("permission_denied"));
      await expect(pending).rejects.toThrow("permission_denied");
    });

    // Rolled back to the pre-attempt roster.
    expect(result.current.users[0].deskIds).toEqual(["g10"]);
  });
});
