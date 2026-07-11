/**
 * deskAssignment — the pure, immutable transforms behind the Admin workspace's
 * MANY-TO-MANY desk membership editor. Membership has three states: All
 * (`allDesks:true`, `deskIds:[]`), a Set (`allDesks:false`, non-empty), and
 * Deskless (`allDesks:false`, `[]`). These helpers keep the roster update immutable
 * (untouched rows keep identity) and the `UpdateUser` payload contract-faithful.
 */

import { describe, expect, it } from "vitest";

import type { UserDesc } from "../src/data/contract";
import {
  normalizeDeskIds,
  updateInputForDeskChange,
  withUserDesks,
} from "../src/lib/deskAssignment";

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

const roster = (): UserDesc[] => [
  makeUser(),
  makeUser({ id: "u2", email: "other@celnet.com", deskIds: ["em"] }),
];

describe("normalizeDeskIds", () => {
  it("collapses to [] when allDesks is set (the set is then ignored)", () => {
    expect(normalizeDeskIds(["g10", "em"], true)).toEqual([]);
  });

  it("drops blanks and duplicates, preserving first-seen order", () => {
    expect(normalizeDeskIds([" g10 ", "em", "g10", "", "  "], false)).toEqual(["g10", "em"]);
  });
});

describe("withUserDesks", () => {
  it("sets a non-empty desk set on the target user only (Set state)", () => {
    const before = roster();
    const after = withUserDesks(before, "u1", ["g10", "em"], false);
    expect(after[0]).toMatchObject({ id: "u1", deskIds: ["g10", "em"], allDesks: false });
    // Untouched row keeps referential identity so a memoized row never re-renders.
    expect(after[1]).toBe(before[1]);
    // Immutable — the input roster is unchanged.
    expect(before[0].deskIds).toEqual([]);
  });

  it("encodes All desks as allDesks:true with an empty set", () => {
    const after = withUserDesks(roster(), "u1", ["g10"], true);
    expect(after[0]).toMatchObject({ deskIds: [], allDesks: true });
  });

  it("encodes deskless as allDesks:false with an empty set", () => {
    const after = withUserDesks([makeUser({ deskIds: ["g10"] })], "u1", [], false);
    expect(after[0]).toMatchObject({ deskIds: [], allDesks: false });
  });

  it("returns an equivalent new array for an unknown user id (no-op)", () => {
    const before = roster();
    const after = withUserDesks(before, "nope", ["g10"], false);
    expect(after).not.toBe(before);
    expect(after).toEqual(before);
  });
});

describe("updateInputForDeskChange", () => {
  it("preserves displayName/role/disabled and carries the normalised set", () => {
    const user = makeUser({ displayName: "Jane", role: "TRADER", disabled: false });
    expect(updateInputForDeskChange(user, [" g10 ", "g10", "em"], false)).toEqual({
      displayName: "Jane",
      role: "TRADER",
      disabled: false,
      deskIds: ["g10", "em"],
      allDesks: false,
    });
  });

  it("emits allDesks:true with an empty set when All desks is chosen", () => {
    expect(updateInputForDeskChange(makeUser(), ["g10"], true)).toMatchObject({
      deskIds: [],
      allDesks: true,
    });
  });

  it("does not mutate the input user", () => {
    const user = makeUser({ deskIds: ["g10"] });
    updateInputForDeskChange(user, ["em"], false);
    expect(user.deskIds).toEqual(["g10"]);
  });
});
