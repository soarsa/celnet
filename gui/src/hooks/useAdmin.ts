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

import type {
  BookDesc,
  BookInput,
  CreateUserInput,
  DeskDesc,
  EntityDesc,
  EntityInput,
  UpdateUserInput,
  UserDesc,
} from "../data/contract";
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
  /** The current legal-entity registry. */
  entities: EntityDesc[];
  /** The current netting-book registry. */
  books: BookDesc[];
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
  /** Create a legal entity; resolves to the created entity or rejects. */
  createEntity: (input: EntityInput) => Promise<EntityDesc>;
  /** Update a legal entity's name/code (key immutable). */
  updateEntity: (key: number, input: EntityInput) => Promise<EntityDesc>;
  /** Delete a legal entity (rejected if any book references it). */
  deleteEntity: (key: number) => Promise<void>;
  /** Create a netting book under an entity; resolves to the created book or rejects. */
  createBook: (input: BookInput) => Promise<BookDesc>;
  /** Update a netting book's name/owning entity (key immutable). */
  updateBook: (key: number, input: BookInput) => Promise<BookDesc>;
  /** Delete a netting book. */
  deleteBook: (key: number) => Promise<void>;
}

export function useAdmin(transport: CelnetTransport, enabled: boolean): AdminApi {
  const [users, setUsers] = useState<UserDesc[]>([]);
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  const [entities, setEntities] = useState<EntityDesc[]>([]);
  const [books, setBooks] = useState<BookDesc[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refetch = useCallback(async (): Promise<void> => {
    if (!enabled) {
      setUsers([]);
      setDesks([]);
      setEntities([]);
      setBooks([]);
      setError(null);
      return;
    }
    setIsLoading(true);
    try {
      // Independent rosters — fetch in parallel (no request waterfall).
      const [nextUsers, nextDesks, nextEntities, nextBooks] = await Promise.all([
        transport.listUsers(),
        transport.listDesks(),
        transport.listEntities(),
        transport.listBooks(),
      ]);
      setUsers(nextUsers);
      setDesks(nextDesks);
      setEntities(nextEntities);
      setBooks(nextBooks);
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

  const createEntity = useCallback(
    async (input: EntityInput): Promise<EntityDesc> => {
      const created = await transport.createEntity(input);
      await refetch();
      return created;
    },
    [transport, refetch],
  );

  const updateEntity = useCallback(
    async (key: number, input: EntityInput): Promise<EntityDesc> => {
      const updated = await transport.updateEntity(key, input);
      await refetch();
      return updated;
    },
    [transport, refetch],
  );

  const deleteEntity = useCallback(
    async (key: number): Promise<void> => {
      await transport.deleteEntity(key);
      await refetch();
    },
    [transport, refetch],
  );

  const createBook = useCallback(
    async (input: BookInput): Promise<BookDesc> => {
      const created = await transport.createBook(input);
      await refetch();
      return created;
    },
    [transport, refetch],
  );

  const updateBook = useCallback(
    async (key: number, input: BookInput): Promise<BookDesc> => {
      const updated = await transport.updateBook(key, input);
      await refetch();
      return updated;
    },
    [transport, refetch],
  );

  const deleteBook = useCallback(
    async (key: number): Promise<void> => {
      await transport.deleteBook(key);
      await refetch();
    },
    [transport, refetch],
  );

  return {
    users,
    desks,
    entities,
    books,
    isLoading,
    error,
    refetch,
    createUser,
    updateUser,
    deleteUser,
    resetPassword,
    createDesk,
    deleteDesk,
    createEntity,
    updateEntity,
    deleteEntity,
    createBook,
    updateBook,
    deleteBook,
  };
}
