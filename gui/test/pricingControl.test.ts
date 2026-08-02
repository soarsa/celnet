/**
 * Firm-wide pricing kill-switch — the data layer: the `pricing_control` codec, the
 * version-gated reconcile + halt classification, and the offline mock transport's
 * `set_pricing_control` round-trip (version bump + broadcast + connect-time replay).
 */

import { describe, expect, it, vi } from "vitest";

import {
  pricingControlFromWire,
  setPricingControlRequestToWire,
} from "../src/data/wsCodec";
import {
  reconcilePricingControl,
  haltLevel,
} from "../src/app/PricingControlProvider";
import { createMockTransport } from "../src/data/mockSource";
import type { PricingControl } from "../src/data/contract";

describe("pricing_control codec", () => {
  it("encodes the set request as snake_case bools (no token/correlation — the conn injects them)", () => {
    expect(setPricingControlRequestToWire(false, true)).toEqual({
      outbound_enabled: false,
      inbound_enabled: true,
    });
  });

  it("decodes a push / response frame, defaulting missing bools to false", () => {
    expect(
      pricingControlFromWire({
        outbound_enabled: false,
        inbound_enabled: true,
        version: 7,
      }),
    ).toEqual({ outboundEnabled: false, inboundEnabled: true, version: 7 });
    // A bool that is not literally `true` decodes to false (deny-by-default read).
    expect(
      pricingControlFromWire({ outbound_enabled: 1, version: 3 }),
    ).toEqual({ outboundEnabled: false, inboundEnabled: false, version: 3 });
  });
});

describe("reconcilePricingControl — monotonic version gate", () => {
  const at = (v: number, out = true, inb = true): PricingControl => ({
    outboundEnabled: out,
    inboundEnabled: inb,
    version: v,
  });

  it("accepts a strictly-newer version", () => {
    expect(reconcilePricingControl(at(1), at(2, false, true))).toEqual(at(2, false, true));
  });

  it("ignores an equal-or-older version (no regression from a stale/replayed push)", () => {
    const cur = at(5, false, false);
    expect(reconcilePricingControl(cur, at(5, true, true))).toBe(cur);
    expect(reconcilePricingControl(cur, at(4, true, true))).toBe(cur);
  });
});

describe("haltLevel classification", () => {
  it("maps each gate combination to its severity", () => {
    expect(haltLevel({ outboundEnabled: true, inboundEnabled: true, version: 1 })).toBe("none");
    expect(haltLevel({ outboundEnabled: false, inboundEnabled: true, version: 1 })).toBe("outbound");
    expect(haltLevel({ outboundEnabled: true, inboundEnabled: false, version: 1 })).toBe("inbound");
    expect(haltLevel({ outboundEnabled: false, inboundEnabled: false, version: 1 })).toBe("all");
  });
});

describe("mock transport — set/subscribe pricing control", () => {
  it("replays the current state on subscribe (connect-time push, no subscribe verb)", () => {
    const t = createMockTransport();
    const seen: PricingControl[] = [];
    t.subscribePricingControl((c) => seen.push(c));
    expect(seen).toHaveLength(1);
    expect(seen[0]).toEqual({ outboundEnabled: true, inboundEnabled: true, version: 1 });
  });

  it("bumps the monotonic version and broadcasts to every subscriber on set", async () => {
    const t = createMockTransport();
    const a: PricingControl[] = [];
    const b: PricingControl[] = [];
    t.subscribePricingControl((c) => a.push(c));
    t.subscribePricingControl((c) => b.push(c));

    const committed = await t.setPricingControl(false, true);
    expect(committed).toEqual({ outboundEnabled: false, inboundEnabled: true, version: 2 });
    // Both subscribers received the connect-time frame (v1) then the change (v2).
    expect(a.map((c) => c.version)).toEqual([1, 2]);
    expect(b.map((c) => c.version)).toEqual([1, 2]);

    const resumed = await t.setPricingControl(true, true);
    expect(resumed.version).toBe(3);
  });

  it("stops broadcasting after the disposer runs", async () => {
    const t = createMockTransport();
    const seen: PricingControl[] = [];
    const dispose = t.subscribePricingControl((c) => seen.push(c));
    dispose();
    await t.setPricingControl(false, false);
    expect(seen).toHaveLength(1); // only the connect-time replay
  });

  it("never throws for a spy subscriber that is later removed", () => {
    const t = createMockTransport();
    const cb = vi.fn();
    const off = t.subscribePricingControl(cb);
    expect(cb).toHaveBeenCalledTimes(1);
    off();
    expect(() => off()).not.toThrow();
  });
});
