/**
 * deskAssignment — the pure transforms behind the Admin inline desk picker.
 * Asserts the roster update is immutable (new array; only the target row changes;
 * every other row keeps its identity) and that "unassigned" is encoded as an
 * OMITTED `deskId`, never a blank string, on both the roster row and the
 * `UpdateUserInput` payload — with the other user fields preserved verbatim.
 */

import { describe, expect, it } from "vitest";

import type { UserDesc } from "../src/data/contract";
import { updateInputForDeskChange, withUserDesk } from "../src/lib/deskAssignment";

function user(overrides: Partial<UserDesc> = {}): UserDesc {
  return {
    id: "u1",
    email: "trader@celnet.com",
    displayName: "Jane Trader",
    role: "TRADER",
    disabled: false,
    ...overrides,
  };
}

const roster: UserDesc[] = [
  user({ id: "u1", email: "one@celnet.com" }),
  user({ id: "u2", email: "two@celnet.com", deskId: "g10" }),
  user({ id: "u3", email: "three@celnet.com", role: "ADMIN" }),
];

describe("withUserDesk — immutable roster update", () => {
  it("returns a NEW array (never mutates the input)", () => {
    const next = withUserDesk(roster, "u1", "g10");
    expect(next).not.toBe(roster);
    // The original roster row is untouched (still unassigned).
    expect(roster[0].deskId).toBeUndefined();
  });

  it("sets the target user's deskId and leaves other rows referentially unchanged", () => {
    const next = withUserDesk(roster, "u1", "g10");
    expect(next[0]).not.toBe(roster[0]); // target replaced
    expect(next[0].deskId).toBe("g10");
    expect(next[1]).toBe(roster[1]); // untouched rows keep identity (memo-safe)
    expect(next[2]).toBe(roster[2]);
  });

  it("removes deskId when assigning undefined (unassigned)", () => {
    const next = withUserDesk(roster, "u2", undefined);
    expect(next[1].deskId).toBeUndefined();
    expect("deskId" in next[1]).toBe(false); // property omitted, not set to undefined
  });

  it("treats an empty / whitespace deskId as unassigned (omits the property)", () => {
    const next = withUserDesk(roster, "u2", "");
    expect("deskId" in next[1]).toBe(false);
    const nextWs = withUserDesk(roster, "u2", "   ");
    expect("deskId" in nextWs[1]).toBe(false);
  });

  it("is a no-op (equivalent new array) for an unknown user id", () => {
    const next = withUserDesk(roster, "nope", "g10");
    expect(next).not.toBe(roster);
    next.forEach((u, i) => expect(u).toBe(roster[i]));
  });
});

describe("updateInputForDeskChange — the UpdateUser payload", () => {
  it("preserves displayName, role and disabled and sets a non-empty deskId", () => {
    const u = user({ displayName: "Kai", role: "ADMIN", disabled: true });
    const input = updateInputForDeskChange(u, "g10");
    expect(input).toEqual({
      displayName: "Kai",
      role: "ADMIN",
      disabled: true,
      deskId: "g10",
    });
  });

  it("OMITS deskId for an unassigning change (undefined)", () => {
    const input = updateInputForDeskChange(user({ deskId: "g10" }), undefined);
    expect("deskId" in input).toBe(false);
    expect(input).toEqual({ displayName: "Jane Trader", role: "TRADER", disabled: false });
  });

  it("OMITS deskId for an empty / whitespace desk", () => {
    expect("deskId" in updateInputForDeskChange(user(), "")).toBe(false);
    expect("deskId" in updateInputForDeskChange(user(), "  ")).toBe(false);
  });

  it("does not mutate the input user", () => {
    const u = user({ deskId: "g10" });
    updateInputForDeskChange(u, undefined);
    expect(u.deskId).toBe("g10");
  });
});
