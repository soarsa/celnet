/**
 * useSettings — read the trader's persisted {@link AppSettings} and the immutable
 * `update` patcher from the surrounding {@link SettingsProvider}. Throws when used
 * outside the provider so a mis-wired tree fails loudly rather than silently
 * reading stale defaults.
 */

import { useContext } from "react";
import {
  SettingsContext,
  type SettingsContextValue,
} from "../settings/SettingsProvider";

/** Consume the settings context; throws outside a {@link SettingsProvider}. */
export function useSettings(): SettingsContextValue {
  const v = useContext(SettingsContext);
  if (!v) throw new Error("useSettings outside SettingsProvider");
  return v;
}
