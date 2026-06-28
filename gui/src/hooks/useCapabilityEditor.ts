/**
 * useCapabilityEditor — the data hook behind the Admin capability matrix.
 *
 * A focused sibling to {@link useAdmin}: for one selected user it loads the
 * capability overlay + resolved effective set (`AuthService.GetUserCapabilities`),
 * holds the editable overlay locally (tri-state per cell), and saves it wholesale
 * (`AuthService.SetUserCapabilities`). On a successful save it refreshes from the
 * returned effective set and surfaces the server's session-revocation as a note —
 * the admin must know the target's live sessions were ended.
 *
 * Errors surface as a human-readable string; the matrix renders explicit loading,
 * error and saved states. When no user is selected the hook stays idle.
 */

import { useCallback, useEffect, useState } from "react";

import type { Capability, UserDesc } from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import {
  type OverlayMap,
  overlayFromCapabilities,
  overlaysDiffer,
  overlayToCapabilities,
  nextOverlay,
} from "../lib/capabilityMatrix";

/** Narrow an unknown thrown value to a display string. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : "unexpected error";
}

/** The success note shown after a save — the contract's session-revocation rule. */
export const SAVED_NOTE = "Saved — the user's active sessions were ended.";

/** The capability-editor API the matrix consumes. */
export interface CapabilityEditor {
  /** True while the initial load (or a reload) is in flight. */
  isLoading: boolean;
  /** True while a save is in flight (controls disable). */
  isSaving: boolean;
  /** The last load/save error as a display string, or `null`. */
  error: string | null;
  /** The success note after a save, or `null` (cleared on the next edit). */
  savedNote: string | null;
  /** The current editable overlay (tri-state per cell). */
  overlay: OverlayMap;
  /** The server's last-resolved effective set (refreshed after each save). */
  effective: Capability[];
  /** Whether the overlay has unsaved edits. */
  isDirty: boolean;
  /** Cycle one cell inherit → grant → deny → inherit. */
  cycle: (key: string) => void;
  /** Discard unsaved edits back to the last-loaded overlay. */
  reset: () => void;
  /** Persist the overlay wholesale; refresh from the returned effective set. */
  save: () => Promise<void>;
  /** Re-load the overlay + effective set from the server. */
  reload: () => Promise<void>;
}

export function useCapabilityEditor(
  transport: CelnetTransport,
  user: UserDesc | null,
): CapabilityEditor {
  const [overlay, setOverlay] = useState<OverlayMap>(() => new Map());
  const [loadedOverlay, setLoadedOverlay] = useState<OverlayMap>(() => new Map());
  const [effective, setEffective] = useState<Capability[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedNote, setSavedNote] = useState<string | null>(null);

  const userId = user?.id ?? null;

  const reload = useCallback(async (): Promise<void> => {
    if (userId === null) {
      setOverlay(new Map());
      setLoadedOverlay(new Map());
      setEffective([]);
      setError(null);
      setSavedNote(null);
      return;
    }
    setIsLoading(true);
    setSavedNote(null);
    try {
      const caps = await transport.getUserCapabilities(userId);
      const next = overlayFromCapabilities(caps.grants, caps.denies);
      setOverlay(new Map(next));
      setLoadedOverlay(new Map(next));
      setEffective(caps.effective);
      setError(null);
    } catch (e: unknown) {
      setError(messageOf(e));
    } finally {
      setIsLoading(false);
    }
  }, [transport, userId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const cycle = useCallback((key: string): void => {
    setSavedNote(null);
    setOverlay((prev) => {
      const next = new Map(prev);
      const current = prev.get(key) ?? "inherit";
      const cycled = nextOverlay(current);
      if (cycled === "inherit") next.delete(key);
      else next.set(key, cycled);
      return next;
    });
  }, []);

  const reset = useCallback((): void => {
    setSavedNote(null);
    setError(null);
    setOverlay(new Map(loadedOverlay));
  }, [loadedOverlay]);

  const save = useCallback(async (): Promise<void> => {
    if (userId === null) return;
    setIsSaving(true);
    setError(null);
    setSavedNote(null);
    try {
      const { grants, denies } = overlayToCapabilities(overlay);
      const caps = await transport.setUserCapabilities(userId, grants, denies);
      const next = overlayFromCapabilities(caps.grants, caps.denies);
      setOverlay(new Map(next));
      setLoadedOverlay(new Map(next));
      setEffective(caps.effective);
      setSavedNote(SAVED_NOTE);
    } catch (e: unknown) {
      setError(messageOf(e));
    } finally {
      setIsSaving(false);
    }
  }, [transport, userId, overlay]);

  return {
    isLoading,
    isSaving,
    error,
    savedNote,
    overlay,
    effective,
    isDirty: overlaysDiffer(overlay, loadedOverlay),
    cycle,
    reset,
    save,
    reload,
  };
}
