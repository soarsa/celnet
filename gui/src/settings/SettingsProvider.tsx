/**
 * SettingsProvider — the React context that owns the trader's {@link AppSettings}
 * (see settingsSchema.ts). State is seeded once from `loadSettings()`; `update`
 * applies an immutable patch, persists it, and re-renders consumers. The value is
 * read via the `useSettings` hook (hooks/useSettings.ts), which throws when used
 * outside this provider.
 */

import { createContext, useCallback, useMemo, useState } from "react";
import {
  loadSettings,
  saveSettings,
  type AppSettings,
} from "./settingsSchema";

/** The context surface: the current settings and an immutable patch applier. */
export interface SettingsContextValue {
  /** The current, effective settings. */
  readonly settings: AppSettings;
  /** Merge a partial patch → new settings object, persist, and re-render. */
  update: (patch: Partial<AppSettings>) => void;
}

/** The context. `null` until a provider mounts (the hook guards this). */
export const SettingsContext = createContext<SettingsContextValue | null>(null);

/** Wrap the app so every descendant can read/patch the persisted settings. */
export function SettingsProvider({
  children,
}: {
  children: React.ReactNode;
}): React.ReactElement {
  const [settings, setSettings] = useState<AppSettings>(() => loadSettings());

  const update = useCallback((patch: Partial<AppSettings>) => {
    setSettings((prev) => {
      // Immutable: a fresh object every time (never mutate `prev`).
      const next: AppSettings = { ...prev, ...patch };
      saveSettings(next);
      return next;
    });
  }, []);

  const value = useMemo<SettingsContextValue>(
    () => ({ settings, update }),
    [settings, update],
  );

  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}
