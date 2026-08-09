/**
 * useDebouncedValue — return `value` delayed by `delayMs`, so a fast-changing
 * source (e.g. a search box the operator is typing into) only propagates once
 * typing settles. Used by the FIX Session Monitor's free-text search so filtering
 * a multi-thousand-frame buffer does not run on every keystroke.
 */

import { useEffect, useState } from "react";

export function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState<T>(value);

  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(id);
  }, [value, delayMs]);

  return debounced;
}
