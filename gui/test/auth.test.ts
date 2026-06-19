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

  it("round-trips a user descriptor (presence-tracked desk_id)", () => {
    const assigned = userDescFromWire({
      id: "u-1",
      email: "jane@celnet.com",
      display_name: "Jane Trader",
      role: 0,
      desk_id: "g10",
      disabled: false,
    });
    expect(assigned).toEqual({
      id: "u-1",
      email: "jane@celnet.com",
      displayName: "Jane Trader",
      role: "TRADER",
      deskId: "g10",
      disabled: false,
    });
    // An absent/empty desk_id decodes to an unassigned user (no `deskId` key).
    const unassigned = userDescFromWire({
      id: "admin",
      email: "admin@celnet.com",
      display_name: "Administrator",
      role: 1,
      disabled: false,
    });
    expect(unassigned.deskId).toBeUndefined();
    expect(unassigned.role).toBe("ADMIN");
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
    });
    expect(result.token).toBe("tok-abc");
    expect(result.user.role).toBe("ADMIN");
    expect(result.expiresNanos).toBe(1234567890n);

    // create: a supplied desk rides through; the role maps to its wire tag.
    const create = createUserRequestToWire({
      email: "jane@celnet.com",
      displayName: "Jane",
      role: "TRADER",
      deskId: "g10",
      password: "longenoughpw1",
    });
    expect(create.role).toBe(0);
    expect(create.desk_id).toBe("g10");
    expect(create.password).toBe("longenoughpw1");
    // create without a desk omits desk_id (unassigned).
    const houseCreate = createUserRequestToWire({
      email: "bob@celnet.com",
      displayName: "Bob",
      role: "ADMIN",
      password: "longenoughpw1",
    });
    expect("desk_id" in houseCreate).toBe(false);
    expect(houseCreate.role).toBe(1);

    const update = updateUserRequestToWire("u-1", {
      displayName: "Jane R",
      role: "ADMIN",
      disabled: true,
    });
    expect(update.id).toBe("u-1");
    expect(update.role).toBe(1);
    expect(update.disabled).toBe(true);
    expect("desk_id" in update).toBe(false);
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

  it("creates users with unique emails and the 12-char minimum", async () => {
    const t = new MockTransport();
    await expect(
      t.createUser({ email: "weak@celnet.com", displayName: "W", role: "TRADER", password: "short" }),
    ).rejects.toThrow(/at least 12 characters/);
    const jane = await t.createUser({
      email: "jane@celnet.com",
      displayName: "Jane",
      role: "TRADER",
      password: "longenoughpw1",
    });
    expect(jane.role).toBe("TRADER");
    const users = await t.listUsers();
    expect(users.some((u) => u.email === "jane@celnet.com")).toBe(true);
    await expect(
      t.createUser({ email: "JANE@celnet.com", displayName: "Dup", role: "TRADER", password: "longenoughpw1" }),
    ).rejects.toThrow(/already exists/);
  });

  it("groups users on a desk and unassigns members when the desk is deleted", async () => {
    const t = new MockTransport();
    const desk = await t.createDesk("G10 Options");
    expect(desk.id).toBe("g10-options");
    const jane = await t.createUser({
      email: "jane@celnet.com",
      displayName: "Jane",
      role: "TRADER",
      deskId: desk.id,
      password: "longenoughpw1",
    });
    expect(jane.deskId).toBe("g10-options");
    await t.deleteDesk(desk.id);
    const users = await t.listUsers();
    expect(users.find((u) => u.id === jane.id)?.deskId).toBeUndefined();
    expect(await t.listDesks()).toHaveLength(0);
  });

  it("refuses to remove the last administrator", async () => {
    const t = new MockTransport();
    const [admin] = await t.listUsers();
    await expect(t.deleteUser(admin!.id)).rejects.toThrow(/last administrator/);
    // Demoting the only admin to trader is also blocked.
    await expect(
      t.updateUser(admin!.id, { displayName: "Administrator", role: "TRADER", disabled: false }),
    ).rejects.toThrow(/last administrator/);
    // With a second admin present, the first can be removed.
    await t.createUser({
      email: "admin2@celnet.com",
      displayName: "Second",
      role: "ADMIN",
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
