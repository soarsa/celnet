/**
 * The managed-FIX-connections feature, end to end through the offline transport.
 *
 * Covers the three layers the Connections workspace + wizard rest on:
 *  - the WS codec round-trips a connection descriptor and maps the kind enum
 *    (string union ⇄ wire int 0), and builds the admin request envelopes with an
 *    explicit grant-all principal;
 *  - the `MockTransport` registry honours the server's semantics offline — create
 *    mints a slug, rejects duplicate names + enabled-address clashes, enable/
 *    disable flips `running`/`boundAddr`, delete removes;
 *  - the `useFixConnections` hook loads, then re-fetches after each mutation so
 *    the table reflects the authoritative set.
 */

import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { FixConnectionSpec } from "../src/data/contract";
import { MockTransport } from "../src/data/mockSource";
import {
  createFixConnectionRequestToWire,
  fixConnectionFromWire,
  fixConnectionKindFromWire,
  fixConnectionKindToWire,
} from "../src/data/wsCodec";
import { useFixConnections } from "../src/hooks/useFixConnections";

function spec(overrides: Partial<FixConnectionSpec> = {}): FixConnectionSpec {
  return {
    name: "Bank B — Options",
    kind: "OPTIONS",
    bindAddr: "127.0.0.1:9200",
    senderCompId: "CELNET",
    targetCompId: "CELNET-CPTY",
    enabled: true,
    // Every managed connection belongs to a desk (no unowned "house" acceptors).
    desk: "g10",
    ...overrides,
  };
}

describe("fix-admin wire codec", () => {
  it("round-trips a descriptor and maps the kind enum", () => {
    const wire = {
      id: "opt-1",
      name: "Bank A",
      kind: 0,
      bind_addr: "127.0.0.1:9099",
      sender_comp_id: "CELNET",
      target_comp_id: "CELNET-CPTY",
      enabled: true,
      running: true,
      bound_addr: "127.0.0.1:9099",
    };
    const c = fixConnectionFromWire(wire);
    expect(c.kind).toBe("OPTIONS");
    expect(c.bindAddr).toBe("127.0.0.1:9099");
    expect(c.running).toBe(true);
    expect(fixConnectionKindToWire("OPTIONS")).toBe(0);
    expect(fixConnectionKindFromWire(0)).toBe("OPTIONS");
  });

  it("builds a create request with a spec and an asserted principal", () => {
    const body = createFixConnectionRequestToWire(spec());
    expect((body.spec as Record<string, unknown>).bind_addr).toBe("127.0.0.1:9200");
    expect((body.spec as Record<string, unknown>).kind).toBe(0);
    expect((body.principal as Record<string, unknown>).grant_all).toBe(true);
  });

  it("carries the owning desk both ways (descriptor + spec)", () => {
    const c = fixConnectionFromWire({
      id: "opt-1",
      name: "Bank A",
      kind: 0,
      bind_addr: "127.0.0.1:9099",
      sender_comp_id: "CELNET",
      target_comp_id: "CELNET-CPTY",
      enabled: true,
      running: true,
      bound_addr: "127.0.0.1:9099",
      desk: "g10",
    });
    expect(c.desk).toBe("g10");
    // The owning desk rides through to the wire spec (every connection has one).
    const body = createFixConnectionRequestToWire(spec({ desk: "em" }));
    expect((body.spec as Record<string, unknown>).desk).toBe("em");
  });

  it("the mock rejects a deskless connection (no unowned house acceptors)", async () => {
    const t = new MockTransport();
    await expect(
      t.createFixConnection(spec({ name: "Deskless", bindAddr: "127.0.0.1:9600", desk: "" })),
    ).rejects.toThrow(/must belong to a desk/);
  });
});

describe("MockTransport fix registry (offline parity)", () => {
  it("mints a slug id and lists the new connection", async () => {
    const t = new MockTransport();
    const created = await t.createFixConnection(spec({ name: "EUR/USD Bank!" }));
    expect(created.id).toBe("eur-usd-bank");
    const list = await t.listFixConnections();
    expect(list.some((c) => c.id === "eur-usd-bank")).toBe(true);
  });

  it("rejects a duplicate name and an enabled-address clash", async () => {
    const t = new MockTransport();
    await t.createFixConnection(spec({ name: "Dup", bindAddr: "127.0.0.1:9300" }));
    await expect(
      t.createFixConnection(spec({ name: "Dup", bindAddr: "127.0.0.1:9301" })),
    ).rejects.toThrow(/already exists/);
    // The seed acceptor is enabled on :9099 — a second enabled one there clashes.
    await expect(
      t.createFixConnection(spec({ name: "Clash", bindAddr: "127.0.0.1:9099" })),
    ).rejects.toThrow(/already used/);
  });

  it("enable/disable flips running + boundAddr, and delete removes", async () => {
    const t = new MockTransport();
    const c = await t.createFixConnection(spec({ name: "Toggle", bindAddr: "127.0.0.1:9400" }));
    const off = await t.setFixConnectionEnabled(c.id, false);
    expect(off.running).toBe(false);
    expect(off.boundAddr).toBe("");
    const on = await t.setFixConnectionEnabled(c.id, true);
    expect(on.running).toBe(true);
    expect(on.boundAddr).toBe("127.0.0.1:9400");
    await t.deleteFixConnection(c.id);
    const list = await t.listFixConnections();
    expect(list.some((x) => x.id === c.id)).toBe(false);
  });
});

describe("useFixConnections", () => {
  it("loads the seed list then re-fetches after a create", async () => {
    const t = new MockTransport();
    const { result } = renderHook(() => useFixConnections(t));

    await waitFor(() => expect(result.current.isLoading).toBe(false));
    const seedCount = result.current.connections.length;
    expect(seedCount).toBeGreaterThan(0);

    await act(async () => {
      await result.current.create(spec({ name: "Hooked", bindAddr: "127.0.0.1:9500" }));
    });

    expect(result.current.connections.length).toBe(seedCount + 1);
    expect(result.current.connections.some((c) => c.name === "Hooked")).toBe(true);
    expect(result.current.error).toBeNull();
  });

  it("surfaces a rejected mutation without corrupting the list", async () => {
    const t = new MockTransport();
    const { result } = renderHook(() => useFixConnections(t));
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    const before = result.current.connections.length;

    await expect(
      act(async () => {
        // Clashes with the enabled seed acceptor on :9099.
        await result.current.create(spec({ name: "Bad", bindAddr: "127.0.0.1:9099" }));
      }),
    ).rejects.toThrow(/already used/);

    expect(result.current.connections.length).toBe(before);
  });
});
