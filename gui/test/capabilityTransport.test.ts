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
    expect(caps.effective.length).toBe(18);
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

describe("MockTransport role bundles", () => {
  it("the default Trader bundle is every action except administer on both assets", async () => {
    const t = new MockTransport();
    const bundle = await t.getRoleCapabilities("TRADER");
    // 9 non-admin actions x 2 assets.
    expect(bundle.capabilities.length).toBe(18);
    const keys = new Set(bundle.capabilities.map((c) => capKey(c.action, c.asset)));
    expect(keys.has(capKey("book", "fixed_income"))).toBe(true);
    expect(keys.has(capKey("administer", "fx_options"))).toBe(false);
  });

  it("the Admin role reports grant-all and rejects a Set", async () => {
    const t = new MockTransport();
    const bundle = await t.getRoleCapabilities("ADMIN");
    // 10 actions x 2 assets.
    expect(bundle.capabilities.length).toBe(20);
    await expect(
      t.setRoleCapabilities("ADMIN", [{ action: "view", asset: "fx_options" }]),
    ).rejects.toThrow(/cannot be narrowed/);
  });

  it("narrowing the Trader bundle narrows a fresh trader's effective set", async () => {
    const t = new MockTransport();
    // Baseline: a trader holds book.fixed_income from the default role bundle.
    const before = await t.getUserCapabilities(await freshTrader(t));
    expect(
      new Set(before.effective.map((c) => capKey(c.action, c.asset))).has(
        capKey("book", "fixed_income"),
      ),
    ).toBe(true);

    // Narrow the Trader bundle: drop book.fixed_income.
    const narrowed = (await t.getRoleCapabilities("TRADER")).capabilities.filter(
      (c) => !(c.action === "book" && c.asset === "fixed_income"),
    );
    const saved = await t.setRoleCapabilities("TRADER", narrowed);
    expect(
      new Set(saved.capabilities.map((c) => capKey(c.action, c.asset))).has(
        capKey("book", "fixed_income"),
      ),
    ).toBe(false);

    // A NEW trader created after the change no longer holds book.fixed_income.
    const created = await t.createUser({
      email: "after@celnet.com",
      displayName: "After",
      role: "TRADER",
      password: "longenoughpw1",
    });
    const after = await t.getUserCapabilities(created.id);
    const afterKeys = new Set(after.effective.map((c) => capKey(c.action, c.asset)));
    expect(afterKeys.has(capKey("book", "fixed_income"))).toBe(false);
    // Other capabilities still resolve from the (narrowed) bundle.
    expect(afterKeys.has(capKey("execute", "fx_options"))).toBe(true);
  });

  it("a per-user grant still widens beyond a narrowed role bundle (deny-wins algebra holds)", async () => {
    const t = new MockTransport();
    const id = await freshTrader(t);
    // Narrow the role bundle to drop execute.fixed_income for every trader.
    const narrowed = (await t.getRoleCapabilities("TRADER")).capabilities.filter(
      (c) => !(c.action === "execute" && c.asset === "fixed_income"),
    );
    await t.setRoleCapabilities("TRADER", narrowed);
    // This specific user is then granted it back via the per-user overlay.
    const saved = await t.setUserCapabilities(
      id,
      [{ action: "execute", asset: "fixed_income" }],
      [],
    );
    const keys = new Set(saved.effective.map((c) => capKey(c.action, c.asset)));
    expect(keys.has(capKey("execute", "fixed_income"))).toBe(true);
  });

  it("rejects an unknown capability label in a role bundle", async () => {
    const t = new MockTransport();
    await expect(
      t.setRoleCapabilities("TRADER", [{ action: "teleport" as never, asset: "fx_options" }]),
    ).rejects.toThrow(/unknown capability/);
  });
});
