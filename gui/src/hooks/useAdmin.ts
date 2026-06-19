/**
 * useAdmin — the data hook backing the Admin workspace's user + desk rosters.
 *
 * A thin layer over the transport's `AuthService` admin surface: it loads the
 * users and desks when `enabled` (an admin is signed in), exposes the mutating
 * operations (create/update/delete user, reset password, create/delete desk),
 * and re-fetches after each mutation so the tables reflect the server's
 * authoritative state. Errors surface as a human-readable string for the table;
 * mutations reject so a caller (a dialog) can show a per-action failure inline.
 *
 * When `enabled` is false (anonymous, or a non-admin session) it stays idle with
 * empty rosters — the admin RPCs would be `permission_denied` server-side, so it
 * never issues them.
 */

import { useCallback, useEffect, useState } from "react";

import type { CreateUserInput, DeskDesc, UpdateUserInput, UserDesc } from "../data/contract";
import type { CelnetTransport } from "../data/transport";

/** Narrow an unknown thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "unexpected error";
}

/** The Admin-workspace data API. */
export interface AdminApi {
  /** The current user roster (empty until the first load resolves). */
  users: UserDesc[];
  /** The current desk roster. */
  desks: DeskDesc[];
  /** True while a load (or refetch) is in flight. */
  isLoading: boolean;
  /** The last load error as a display string, or `null`. */
  error: string | null;
  /** Re-load users + desks from the server. */
  refetch: () => Promise<void>;
  /** Create a user; resolves to the created account or rejects. */
  createUser: (input: CreateUserInput) => Promise<UserDesc>;
  /** Update a user's profile/role/desk/disabled flag. */
  updateUser: (id: string, input: UpdateUserInput) => Promise<UserDesc>;
  /** Delete a user. */
  deleteUser: (id: string) => Promise<void>;
  /** Set a user's password (the seeded-admin rotation + general reset path). */
  resetPassword: (id: string, newPassword: string) => Promise<void>;
  /** Create a desk; resolves to the created desk or rejects. */
  createDesk: (name: string) => Promise<DeskDesc>;
  /** Delete a desk (its members become unassigned). */
  deleteDesk: (id: string) => Promise<void>;
}

export function useAdmin(transport: CelnetTransport, enabled: boolean): AdminApi {
  const [users, setUsers] = useState<UserDesc[]>([]);
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refetch = useCallback(async (): Promise<void> => {
    if (!enabled) {
      setUsers([]);
      setDesks([]);
      setError(null);
      return;
    }
    setIsLoading(true);
    try {
      // Independent rosters — fetch in parallel (no request waterfall).
      const [nextUsers, nextDesks] = await Promise.all([
        transport.listUsers(),
        transport.listDesks(),
      ]);
      setUsers(nextUsers);
      setDesks(nextDesks);
      setError(null);
    } catch (e: unknown) {
      setError(messageOf(e));
    } finally {
      setIsLoading(false);
    }
  }, [transport, enabled]);

  useEffect(() => {
    void refetch();
  }, [refetch]);

  const createUser = useCallback(
    async (input: CreateUserInput): Promise<UserDesc> => {
      const created = await transport.createUser(input);
      await refetch();
      return created;
    },
    [transport, refetch],
  );

  const updateUser = useCallback(
    async (id: string, input: UpdateUserInput): Promise<UserDesc> => {
      const updated = await transport.updateUser(id, input);
      await refetch();
      return updated;
    },
    [transport, refetch],
  );

  const deleteUser = useCallback(
    async (id: string): Promise<void> => {
      await transport.deleteUser(id);
      await refetch();
    },
    [transport, refetch],
  );

  const resetPassword = useCallback(
    async (id: string, newPassword: string): Promise<void> => {
      await transport.resetPassword(id, newPassword);
    },
    [transport],
  );

  const createDesk = useCallback(
    async (name: string): Promise<DeskDesc> => {
      const created = await transport.createDesk(name);
      await refetch();
      return created;
    },
    [transport, refetch],
  );

  const deleteDesk = useCallback(
    async (id: string): Promise<void> => {
      await transport.deleteDesk(id);
      await refetch();
    },
    [transport, refetch],
  );

  return {
    users,
    desks,
    isLoading,
    error,
    refetch,
    createUser,
    updateUser,
    deleteUser,
    resetPassword,
    createDesk,
    deleteDesk,
  };
}
