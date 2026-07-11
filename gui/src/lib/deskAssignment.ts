/**
 * deskAssignment — the pure, immutable transforms behind the Admin workspace's
 * inline desk membership editor.
 *
 * Desk membership is what scopes a trader's inbound quote/deal reception: a trader
 * receives quotes and executed deals for ANY desk they belong to. Membership is
 * MANY-TO-MANY — a user may belong to zero, one, or many desks, or to EVERY desk
 * (`allDesks`). A deskless user (no desks, not all-desks) receives nothing
 * (deny-by-default). Editing membership is therefore a routing/permissioning
 * action, not cosmetic. These helpers keep that one mutation total,
 * referentially-honest, and independently testable: the roster update is immutable
 * (untouched rows keep their identity so React skips them), and the
 * `UpdateUserInput` payload preserves every other field the wire contract requires
 * while encoding the three membership states exactly:
 *   - All      → `allDesks:true`,  `deskIds:[]`
 *   - Set      → `allDesks:false`, `deskIds:[…]` (non-empty)
 *   - Deskless → `allDesks:false`, `deskIds:[]`
 */

import type { UpdateUserInput, UserDesc } from "../data/contract";

/**
 * Normalise a desk-id set: drop blanks and duplicates (preserving first-seen
 * order). An `allDesks` membership carries no explicit desks, so it collapses to
 * `[]`.
 */
export function normalizeDeskIds(
  deskIds: readonly string[],
  allDesks: boolean,
): string[] {
  if (allDesks) return [];
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of deskIds) {
    const id = raw.trim();
    if (id.length === 0 || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
}

/**
 * Return a new roster with `userId`'s membership set to (`deskIds`, `allDesks`).
 * Immutable: a fresh array, only the target user replaced with a new object; every
 * other user is referentially unchanged so a memoized row never re-renders. The id
 * set is normalised (blanks/dupes dropped; `[]` when `allDesks`). An unknown
 * `userId` returns an equivalent new array (no-op change).
 */
export function withUserDesks(
  users: readonly UserDesc[],
  userId: string,
  deskIds: readonly string[],
  allDesks: boolean,
): UserDesc[] {
  const normalized = normalizeDeskIds(deskIds, allDesks);
  return users.map((user) =>
    user.id === userId ? { ...user, deskIds: normalized, allDesks } : user,
  );
}

/**
 * Build the `UpdateUser` payload for a membership change: preserve the user's
 * `displayName`, `role`, and `disabled` verbatim, and set the normalised
 * (`deskIds`, `allDesks`) pair. Never mutates the input user.
 */
export function updateInputForDeskChange(
  user: UserDesc,
  deskIds: readonly string[],
  allDesks: boolean,
): UpdateUserInput {
  return {
    displayName: user.displayName,
    role: user.role,
    disabled: user.disabled,
    deskIds: normalizeDeskIds(deskIds, allDesks),
    allDesks,
  };
}
