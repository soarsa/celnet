/**
 * useRoleCapabilityEditor — the data hook behind the Admin **role-bundle** editor.
 *
 * A sibling to {@link useCapabilityEditor} (which edits a single user's overlay):
 * for one selected role it loads the role's capability bundle — the *base* a role
 * confers before any per-user overlay (`AuthService.GetRoleCapabilities`) — holds
 * the editable set locally (a flat on/off per action × asset cell), and saves it
 * wholesale (`AuthService.SetRoleCapabilities`). On a successful save it refreshes
 * from the returned bundle and surfaces the server's session-revocation as a note:
 * the change ended the live sessions of every user holding the role.
 *
 * The `ADMIN` role is grant-all and immutable — the hook loads its full surface
 * read-only and never offers a save (the server rejects a Set with
 * `failed_precondition`). Errors surface as a human-readable string.
 */

import { useCallback, useEffect, useState } from "react";

import type { Capability, UserRole } from "../data/contract";
import { CAPABILITY_ACTIONS, CAPABILITY_ASSETS } from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import { capKey } from "../lib/capabilityMatrix";

/** Narrow an unknown thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "unexpected error";
}

/** The success note shown after a save — the contract's session-revocation rule. */
export const ROLE_SAVED_NOTE =
  "Saved — every signed-in user with this role had their session ended.";

/** The set of cell keys present in a capability bundle. */
function selectionOf(caps: readonly Capability[]): Set<string> {
  return new Set(caps.map((c) => capKey(c.action, c.asset)));
}

/** Whether two selections differ (the editor's dirty flag). */
function selectionsDiffer(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  if (a.size !== b.size) return true;
  for (const k of a) if (!b.has(k)) return true;
  return false;
}

/** Serialize a selection to the wire bundle in canonical action × asset order. */
function selectionToCapabilities(selected: ReadonlySet<string>): Capability[] {
  const caps: Capability[] = [];
  for (const action of CAPABILITY_ACTIONS) {
    for (const asset of CAPABILITY_ASSETS) {
      if (selected.has(capKey(action, asset))) caps.push({ action, asset });
    }
  }
  return caps;
}

/** The role-bundle-editor API the matrix consumes. */
export interface RoleCapabilityEditor {
  /** True while the initial load (or a reload) is in flight. */
  isLoading: boolean;
  /** True while a save is in flight (controls disable). */
  isSaving: boolean;
  /** The last load/save error as a display string, or `null`. */
  error: string | null;
  /** The success note after a save, or `null` (cleared on the next edit). */
  savedNote: string | null;
  /** The current editable bundle as a set of `action asset` cell keys. */
  selected: ReadonlySet<string>;
  /** Whether the selected role is the immutable grant-all `ADMIN` role. */
  isAdminRole: boolean;
  /** Whether the bundle has unsaved edits. */
  isDirty: boolean;
  /** Toggle one cell in/out of the bundle (no-op for the immutable `ADMIN` role). */
  toggle: (key: string) => void;
  /** Discard unsaved edits back to the last-loaded bundle. */
  reset: () => void;
  /** Persist the bundle wholesale; refresh from the returned bundle. */
  save: () => Promise<void>;
  /** Re-load the bundle from the server. */
  reload: () => Promise<void>;
}

export function useRoleCapabilityEditor(
  transport: CelnetTransport,
  role: UserRole,
): RoleCapabilityEditor {
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [loaded, setLoaded] = useState<Set<string>>(() => new Set());
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedNote, setSavedNote] = useState<string | null>(null);

  const isAdminRole = role === "ADMIN";

  const reload = useCallback(async (): Promise<void> => {
    setIsLoading(true);
    setSavedNote(null);
    try {
      const bundle = await transport.getRoleCapabilities(role);
      const next = selectionOf(bundle.capabilities);
      setSelected(new Set(next));
      setLoaded(new Set(next));
      setError(null);
    } catch (e: unknown) {
      setError(messageOf(e));
    } finally {
      setIsLoading(false);
    }
  }, [transport, role]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const toggle = useCallback(
    (key: string): void => {
      if (isAdminRole) return; // grant-all, immutable
      setSavedNote(null);
      setSelected((prev) => {
        const next = new Set(prev);
        if (next.has(key)) next.delete(key);
        else next.add(key);
        return next;
      });
    },
    [isAdminRole],
  );

  const reset = useCallback((): void => {
    setSavedNote(null);
    setError(null);
    setSelected(new Set(loaded));
  }, [loaded]);

  const save = useCallback(async (): Promise<void> => {
    if (isAdminRole) return; // the Admin role cannot be narrowed
    setIsSaving(true);
    setError(null);
    setSavedNote(null);
    try {
      const bundle = await transport.setRoleCapabilities(role, selectionToCapabilities(selected));
      const next = selectionOf(bundle.capabilities);
      setSelected(new Set(next));
      setLoaded(new Set(next));
      setSavedNote(ROLE_SAVED_NOTE);
    } catch (e: unknown) {
      setError(messageOf(e));
    } finally {
      setIsSaving(false);
    }
  }, [transport, role, selected, isAdminRole]);

  return {
    isLoading,
    isSaving,
    error,
    savedNote,
    selected,
    isAdminRole,
    isDirty: selectionsDiffer(selected, loaded),
    toggle,
    reset,
    save,
    reload,
  };
}
