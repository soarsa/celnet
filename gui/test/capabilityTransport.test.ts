/**
 * The offline MockTransport's capability methods must behave identically to the
 * live contract: get resolves `role bundle ∪ grants ∖ denies` (deny-wins), set
 * replaces the overlay wholesale and persists, and an unknown label is rejected.
 * This guards the "offline GUI behaves like the wire" requirement.
 */

import { describe, expect, it } from "vitest";

import { MockTransport } from "../src/data/mockSource";
import { capKey } from "../src/lib/capabilityMatrix";

async function freshTrader(transport: MockTransport): Promise<string> {
  const created = await transport.createUser({
    email: "trader@celnet.com",
    displayName: "Trader",
    role: "TRADER",
    password: "longenoughpw1",
  });
  return created.id;
}

describe("MockTransport capability overlay", () => {
  it("a fresh trader resolves to every action except administer on both assets", async () => {
    const t = new MockTransport();
    const id = await freshTrader(t);
    const caps = await t.getUserCapabilities(id);
    expect(caps.grants).toEqual([]);
    expect(caps.denies).toEqual([]);
    expect(caps.effective.length).toBe(16);
    const keys = new Set(caps.effective.map((c) => capKey(c.action, c.asset)));
    expect(keys.has(capKey("administer", "fx_options"))).toBe(false);
    expect(keys.has(capKey("execute", "fixed_income"))).toBe(true);
  });

  it("set widens with a grant, narrows with a deny (deny-wins), and persists", async () => {
    const t = new MockTransport();
    const id = await freshTrader(t);
    const saved = await t.setUserCapabilities(
      id,
      [{ action: "administer", asset: "fixed_income" }],
      [{ action: "execute", asset: "fx_options" }],
    );
    const savedKeys = new Set(saved.effective.map((c) => capKey(c.action, c.asset)));
    expect(savedKeys.has(capKey("administer", "fixed_income"))).toBe(true);
    expect(savedKeys.has(capKey("execute", "fx_options"))).toBe(false);

    // Persisted: a re-read returns the same overlay + effective.
    const reread = await t.getUserCapabilities(id);
    expect(reread.grants).toEqual([{ action: "administer", asset: "fixed_income" }]);
    expect(reread.denies).toEqual([{ action: "execute", asset: "fx_options" }]);
    expect(new Set(reread.effective.map((c) => capKey(c.action, c.asset)))).toEqual(savedKeys);
  });

  it("the admin (grant-all) minus a deny removes exactly that capability", async () => {
    const t = new MockTransport();
    const saved = await t.setUserCapabilities(
      "admin",
      [],
      [{ action: "book", asset: "fx_options" }],
    );
    const keys = new Set(saved.effective.map((c) => capKey(c.action, c.asset)));
    expect(keys.has(capKey("book", "fx_options"))).toBe(false);
    expect(keys.has(capKey("book", "fixed_income"))).toBe(true);
    expect(keys.has(capKey("administer", "fx_options"))).toBe(true);
  });

  it("rejects an unknown capability label (server invalid_argument parity)", async () => {
    const t = new MockTransport();
    const id = await freshTrader(t);
    await expect(
      t.setUserCapabilities(
        id,
        [{ action: "teleport" as never, asset: "fx_options" }],
        [],
      ),
    ).rejects.toThrow(/unknown capability/);
  });

  it("rejects an unknown user id", async () => {
    const t = new MockTransport();
    await expect(t.getUserCapabilities("nobody")).rejects.toThrow(/no user/);
  });
});
