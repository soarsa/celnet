/**
 * The auth feature (server-enforced sessions + user/desk admin), end to end
 * through the offline transport, plus the wire codec round-trip.
 *
 * Covers two layers:
 *  - the WS codec maps `UserRole` (string union ⇄ wire int 0/1), round-trips a
 *    user descriptor (presence-tracked `desk_id`), decodes a login result, and
 *    builds the create/update request envelopes;
 *  - the `MockTransport` honours the server's `AuthService` semantics offline —
 *    the seeded `admin@celnet.com` / `password` signs in, a wrong password is
 *    rejected, create enforces unique emails + the 12-char minimum, a desk groups
 *    users (and deleting it unassigns members), and the last administrator cannot
 *    be removed.
 */

import { describe, expect, it } from "vitest";

import { MockTransport } from "../src/data/mockSource";
import {
  createUserRequestToWire,
  loginResultFromWire,
  updateUserRequestToWire,
  userDescFromWire,
  userRoleFromWire,
  userRoleToWire,
} from "../src/data/wsCodec";

describe("auth wire codec", () => {
  it("maps the UserRole enum both ways (admin=1, trader=0 default)", () => {
    expect(userRoleToWire("ADMIN")).toBe(1);
    expect(userRoleToWire("TRADER")).toBe(0);
    expect(userRoleFromWire(1)).toBe("ADMIN");
    expect(userRoleFromWire(0)).toBe("TRADER");
    // Any unknown tag is the least-privileged role — never accidental admin.
    expect(userRoleFromWire(7)).toBe("TRADER");
  });

  it("round-trips a user descriptor (repeated desk_ids + all_desks)", () => {
    const assigned = userDescFromWire({
      id: "u-1",
      email: "jane@celnet.com",
      display_name: "Jane Trader",
      role: 0,
      desk_ids: ["g10", "em"],
      all_desks: false,
      disabled: false,
    });
    expect(assigned).toEqual({
      id: "u-1",
      email: "jane@celnet.com",
      displayName: "Jane Trader",
      role: "TRADER",
      deskIds: ["g10", "em"],
      allDesks: false,
      disabled: false,
    });
    // An all-desks user carries `all_desks:true` (desk_ids empty).
    const everyDesk = userDescFromWire({
      id: "u-2",
      email: "chief@celnet.com",
      display_name: "Chief",
      role: 0,
      desk_ids: [],
      all_desks: true,
      disabled: false,
    });
    expect(everyDesk.allDesks).toBe(true);
    expect(everyDesk.deskIds).toEqual([]);
    // An absent desk_ids decodes to a deskless user (empty array, never absent).
    const deskless = userDescFromWire({
      id: "admin",
      email: "admin@celnet.com",
      display_name: "Administrator",
      role: 1,
      disabled: false,
    });
    expect(deskless.deskIds).toEqual([]);
    expect(deskless.allDesks).toBe(false);
    expect(deskless.role).toBe("ADMIN");
  });

  it("decodes a login result and builds the user request envelopes", () => {
    const result = loginResultFromWire({
      session_token: "tok-abc",
      user: {
        id: "admin",
        email: "admin@celnet.com",
        display_name: "Administrator",
        role: 1,
        disabled: false,
      },
      expires_nanos: 1234567890,
      // The caller's OWN effective set rides on the login reply (snake_case
      // action/asset labels), driving the client's affordance gating.
      capabilities: [
        { action: "price", asset: "fx_options" },
        { action: "execute", asset: "fixed_income" },
      ],
    });
    expect(result.token).toBe("tok-abc");
    expect(result.user.role).toBe("ADMIN");
    expect(result.expiresNanos).toBe(1234567890n);
    expect(result.capabilities).toEqual([
      { action: "price", asset: "fx_options" },
      { action: "execute", asset: "fixed_income" },
    ]);

    // An absent `capabilities` array decodes to an empty (deny-everything) set —
    // never an accidental grant.
    const noCaps = loginResultFromWire({
      session_token: "tok-x",
      user: { id: "u", email: "u@celnet.com", display_name: "U", role: 0, disabled: false },
      expires_nanos: 1,
    });
    expect(noCaps.capabilities).toEqual([]);

    // create: a multi-desk set rides through as repeated desk_ids + all_desks:false.
    const create = createUserRequestToWire({
      email: "jane@celnet.com",
      displayName: "Jane",
      role: "TRADER",
      deskIds: ["g10", "em"],
      allDesks: false,
      password: "longenoughpw1",
    });
    expect(create.role).toBe(0);
    expect(create.desk_ids).toEqual(["g10", "em"]);
    expect(create.all_desks).toBe(false);
    expect(create.password).toBe("longenoughpw1");
    // create with All desks: all_desks:true supersedes the set (empty desk_ids).
    const allDeskCreate = createUserRequestToWire({
      email: "chief@celnet.com",
      displayName: "Chief",
      role: "ADMIN",
      deskIds: ["g10"],
      allDesks: true,
      password: "longenoughpw1",
    });
    expect(allDeskCreate.all_desks).toBe(true);
    expect(allDeskCreate.desk_ids).toEqual([]);
    // create with no desks: a deskless user sends an empty set, all_desks:false.
    const houseCreate = createUserRequestToWire({
      email: "bob@celnet.com",
      displayName: "Bob",
      role: "ADMIN",
      deskIds: [],
      allDesks: false,
      password: "longenoughpw1",
    });
    expect(houseCreate.desk_ids).toEqual([]);
    expect(houseCreate.all_desks).toBe(false);
    expect(houseCreate.role).toBe(1);

    // update: the new membership rides through as desk_ids + all_desks.
    const update = updateUserRequestToWire("u-1", {
      displayName: "Jane R",
      role: "ADMIN",
      deskIds: ["em"],
      allDesks: false,
      disabled: true,
    });
    expect(update.id).toBe("u-1");
    expect(update.role).toBe(1);
    expect(update.disabled).toBe(true);
    expect(update.desk_ids).toEqual(["em"]);
    expect(update.all_desks).toBe(false);
  });
});

describe("MockTransport auth (offline parity)", () => {
  it("signs in the seeded admin and rejects a wrong password", async () => {
    const t = new MockTransport();
    const result = await t.login("admin@celnet.com", "password");
    expect(result.user.email).toBe("admin@celnet.com");
    expect(result.user.role).toBe("ADMIN");
    expect(result.token.length).toBeGreaterThan(0);
    // A single opaque error for a bad password (no factor leak).
    await expect(t.login("admin@celnet.com", "wrong")).rejects.toThrow(/invalid email or password/);
    // Email match is case-insensitive.
    const upper = await t.login("ADMIN@CELNET.COM", "password");
    expect(upper.user.id).toBe("admin");
  });

  it("returns the caller's effective capability set on login (offline gating)", async () => {
    const t = new MockTransport();
    // The seeded admin's role bundle is grant-all (18 actions × 2 assets = 36),
    // so offline affordance gating is coherent real behaviour, not a stub.
    const result = await t.login("admin@celnet.com", "password");
    expect(result.capabilities.length).toBe(36);
    expect(
      result.capabilities.some((c) => c.action === "execute" && c.asset === "fixed_income"),
    ).toBe(true);
    expect(
      result.capabilities.some((c) => c.action === "administer" && c.asset === "fx_options"),
    ).toBe(true);

    // A freshly-created TRADER holds every action EXCEPT the six held-back
    // authorities (administer, risk_transfer, the three manage caps, and
    // view_analytics): 9 × 2 = 18.
    await t.createUser({
      email: "trader@celnet.com",
      displayName: "T",
      role: "TRADER",
      deskIds: [],
      allDesks: false,
      password: "traderpass12",
    });
    const trader = await t.login("trader@celnet.com", "traderpass12");
    expect(trader.capabilities.length).toBe(18);
    expect(trader.capabilities.some((c) => c.action === "administer")).toBe(false);
  });

  it("creates users with unique emails and the 12-char minimum", async () => {
    const t = new MockTransport();
    await expect(
      t.createUser({
        email: "weak@celnet.com",
        displayName: "W",
        role: "TRADER",
        deskIds: [],
        allDesks: false,
        password: "short",
      }),
    ).rejects.toThrow(/at least 12 characters/);
    const jane = await t.createUser({
      email: "jane@celnet.com",
      displayName: "Jane",
      role: "TRADER",
      deskIds: [],
      allDesks: false,
      password: "longenoughpw1",
    });
    expect(jane.role).toBe("TRADER");
    const users = await t.listUsers();
    expect(users.some((u) => u.email === "jane@celnet.com")).toBe(true);
    await expect(
      t.createUser({
        email: "JANE@celnet.com",
        displayName: "Dup",
        role: "TRADER",
        deskIds: [],
        allDesks: false,
        password: "longenoughpw1",
      }),
    ).rejects.toThrow(/already exists/);
  });

  it("groups users across many desks and drops one when it is deleted", async () => {
    const t = new MockTransport();
    const g10 = await t.createDesk("G10 Options");
    const em = await t.createDesk("EM Rates");
    expect(g10.id).toBe("g10-options");
    const jane = await t.createUser({
      email: "jane@celnet.com",
      displayName: "Jane",
      role: "TRADER",
      deskIds: [g10.id, em.id],
      allDesks: false,
      password: "longenoughpw1",
    });
    expect(jane.deskIds).toEqual([g10.id, em.id]);
    // Deleting one desk drops it from membership; the other survives.
    await t.deleteDesk(g10.id);
    const users = await t.listUsers();
    expect(users.find((u) => u.id === jane.id)?.deskIds).toEqual([em.id]);
    expect(await t.listDesks()).toHaveLength(1);
  });

  it("assigns a user to every desk via all_desks", async () => {
    const t = new MockTransport();
    await t.createDesk("G10 Options");
    const chief = await t.createUser({
      email: "chief@celnet.com",
      displayName: "Chief",
      role: "TRADER",
      deskIds: ["g10-options"],
      allDesks: true,
      password: "longenoughpw1",
    });
    // all_desks supersedes the explicit set — it is stored empty.
    expect(chief.allDesks).toBe(true);
    expect(chief.deskIds).toEqual([]);
  });

  it("renames a desk (label changes; the id/routing key is immutable)", async () => {
    const t = new MockTransport();
    const desk = await t.createDesk("G10 Options");
    const jane = await t.createUser({
      email: "jane@celnet.com",
      displayName: "Jane",
      role: "TRADER",
      deskIds: [desk.id],
      allDesks: false,
      password: "longenoughpw1",
    });

    const renamed = await t.updateDesk(desk.id, "G10 Vol");
    expect(renamed).toEqual({ id: desk.id, name: "G10 Vol" });
    // The member keeps its desk — routing keys on the immutable id, not the label.
    const users = await t.listUsers();
    expect(users.find((u) => u.id === jane.id)?.deskIds).toEqual([desk.id]);
    expect((await t.listDesks()).find((d) => d.id === desk.id)?.name).toBe("G10 Vol");
  });

  it("rejects a desk rename that is unknown, blank, or a duplicate label", async () => {
    const t = new MockTransport();
    const g10 = await t.createDesk("G10 Options");
    await t.createDesk("EM Rates");

    await expect(t.updateDesk("ghost", "X")).rejects.toThrow(/no desk/); // NotFound
    await expect(t.updateDesk(g10.id, "   ")).rejects.toThrow(/required/); // InvalidArgument
    // Duplicate is case-insensitive vs OTHER desks (AlreadyExists).
    await expect(t.updateDesk(g10.id, "em rates")).rejects.toThrow(/already exists/);
    // Renaming a desk to its OWN current label (any case) is allowed.
    await expect(t.updateDesk(g10.id, "G10 OPTIONS")).resolves.toEqual({
      id: g10.id,
      name: "G10 OPTIONS",
    });
  });

  it("refuses to remove the last administrator", async () => {
    const t = new MockTransport();
    const [admin] = await t.listUsers();
    await expect(t.deleteUser(admin!.id)).rejects.toThrow(/last administrator/);
    // Demoting the only admin to trader is also blocked.
    await expect(
      t.updateUser(admin!.id, {
        displayName: "Administrator",
        role: "TRADER",
        // the update changes ONLY the role — carry the admin's existing membership
        deskIds: [...admin!.deskIds],
        allDesks: admin!.allDesks,
        disabled: false,
      }),
    ).rejects.toThrow(/last administrator/);
    // With a second admin present, the first can be removed.
    await t.createUser({
      email: "admin2@celnet.com",
      displayName: "Second",
      role: "ADMIN",
      deskIds: [],
      allDesks: false,
      password: "longenoughpw1",
    });
    expect(await t.deleteUser(admin!.id)).toBe(true);
  });

  it("resets a password so the new credential signs in", async () => {
    const t = new MockTransport();
    const [admin] = await t.listUsers();
    await expect(t.resetPassword(admin!.id, "short")).rejects.toThrow(/at least 12 characters/);
    await t.resetPassword(admin!.id, "brandnewpass1");
    await expect(t.login("admin@celnet.com", "password")).rejects.toThrow(/invalid/);
    const ok = await t.login("admin@celnet.com", "brandnewpass1");
    expect(ok.user.id).toBe("admin");
  });
});
