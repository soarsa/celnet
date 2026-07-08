/**
 * deskAssignment — the pure, immutable transforms behind the Admin workspace's
 * inline desk picker.
 *
 * A desk is what scopes a trader's inbound quote/deal reception: a trader only
 * receives quotes and executed deals for the desk they are assigned to
 * (deny-by-default — an unassigned trader receives nothing). Assigning a desk is
 * therefore a routing/permissioning action, not cosmetic. These helpers keep that
 * one mutation total, referentially-honest, and independently testable: the roster
 * update is immutable (untouched rows keep their identity so React skips them), and
 * the `UpdateUserInput` payload preserves every other field the wire contract
 * requires while encoding "unassigned" as an OMITTED `deskId` (the proto's
 * presence-tracked absence), never an empty string.
 */

import type { UpdateUserInput, UserDesc } from "../data/contract";

/**
 * Return a new roster with `userId`'s desk set to `deskId` (or UNASSIGNED when
 * `deskId` is `undefined`/empty). Immutable: a fresh array, only the target user
 * replaced with a new object; every other user is referentially unchanged so a
 * memoized row never re-renders. An empty/whitespace `deskId` omits the property
 * entirely (the contract's "unassigned" encoding — never a blank string on the
 * wire). An unknown `userId` returns an equivalent new array (no-op change).
 */
export function withUserDesk(
  users: readonly UserDesc[],
  userId: string,
  deskId: string | undefined,
): UserDesc[] {
  const normalized = deskId?.trim() ? deskId : undefined;
  return users.map((user) => {
    if (user.id !== userId) return user;
    if (normalized === undefined) {
      // Omit the property so an unassigned user carries no `deskId` key.
      const { deskId: _dropped, ...rest } = user;
      return rest;
    }
    return { ...user, deskId: normalized };
  });
}

/**
 * Build the `UpdateUser` payload for a desk (re)assignment: preserve the user's
 * `displayName`, `role`, and `disabled` verbatim, and set `deskId` ONLY when a
 * non-empty desk is chosen — an unassigning change omits the field so the server
 * clears the membership (proto presence-tracked absence, guardrail #9's one
 * contract). Never mutates the input user.
 */
export function updateInputForDeskChange(
  user: UserDesc,
  deskId: string | undefined,
): UpdateUserInput {
  const normalized = deskId?.trim() ? deskId : undefined;
  const input: UpdateUserInput = {
    displayName: user.displayName,
    role: user.role,
    disabled: user.disabled,
  };
  if (normalized !== undefined) input.deskId = normalized;
  return input;
}
