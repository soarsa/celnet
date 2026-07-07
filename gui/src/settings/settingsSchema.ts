/**
 * AppSettings — the client-side, trader-owned preferences for the notification
 * surface (alerts, sound, size thresholds, desktop growl, auto-clear). Persisted
 * to `localStorage` under a versioned key so a schema addition is forward-safe:
 * `loadSettings` MERGES a stored (possibly partial / older) blob over the current
 * defaults, so an added field always resolves to its default rather than
 * `undefined`. Pure module — no React, no `any`; the parsed blob is `unknown` and
 * narrowed field-by-field.
 */

/** The persisted trader preferences. All fields are client-side only. */
export interface AppSettings {
  /** Master in-app alerts switch. Off ⇒ no item, toast, sound, or desktop growl. */
  alertsEnabled: boolean;
  /** Whether an inbound alert plays the audio cue. */
  soundsEnabled: boolean;
  /** Audio-cue volume, 0–100 (clamped on load). */
  volume: number;
  /** Raw notional threshold: events with a derived notional < this raise nothing. */
  minQty: number;
  /** Desktop ("growl") browser notifications master switch. */
  growlEnabled: boolean;
  /** Whether completed / aged notifications are auto-cleared from the list. */
  autoClearCompleted: boolean;
  /** Age (seconds) after which auto-clear sweeps a notification. */
  autoClearTtlSeconds: number;
}

/** The out-of-the-box defaults a fresh trader starts from. */
export const DEFAULT_SETTINGS: AppSettings = {
  alertsEnabled: true,
  soundsEnabled: true,
  volume: 60,
  minQty: 0,
  growlEnabled: true,
  autoClearCompleted: true,
  autoClearTtlSeconds: 60,
};

/** The versioned localStorage key (bump the suffix on an incompatible change). */
export const SETTINGS_STORAGE_KEY = "celnet.settings.v1";

/** The lowest a volume may be. */
const VOLUME_MIN = 0;
/** The highest a volume may be. */
const VOLUME_MAX = 100;

/** Clamp `n` into `[lo, hi]`. */
function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}

/** Read a boolean field from a parsed blob, falling back to the default. */
function pickBool(blob: Record<string, unknown>, key: keyof AppSettings, fallback: boolean): boolean {
  const v = blob[key];
  return typeof v === "boolean" ? v : fallback;
}

/** Read a finite-number field from a parsed blob, falling back to the default. */
function pickNum(blob: Record<string, unknown>, key: keyof AppSettings, fallback: number): number {
  const v = blob[key];
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

/**
 * Load the persisted settings, MERGED over {@link DEFAULT_SETTINGS} so a stored
 * subset (or an older schema) is safe. Volume is clamped to 0–100 and minQty to
 * `>= 0`. Any error (absent key, malformed JSON, private-mode throw) ⇒ the
 * defaults. Never throws.
 */
export function loadSettings(): AppSettings {
  try {
    const raw = window.localStorage.getItem(SETTINGS_STORAGE_KEY);
    if (raw === null) return { ...DEFAULT_SETTINGS };
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return { ...DEFAULT_SETTINGS };
    const blob = parsed as Record<string, unknown>;
    return {
      alertsEnabled: pickBool(blob, "alertsEnabled", DEFAULT_SETTINGS.alertsEnabled),
      soundsEnabled: pickBool(blob, "soundsEnabled", DEFAULT_SETTINGS.soundsEnabled),
      volume: clamp(pickNum(blob, "volume", DEFAULT_SETTINGS.volume), VOLUME_MIN, VOLUME_MAX),
      minQty: Math.max(0, pickNum(blob, "minQty", DEFAULT_SETTINGS.minQty)),
      growlEnabled: pickBool(blob, "growlEnabled", DEFAULT_SETTINGS.growlEnabled),
      autoClearCompleted: pickBool(
        blob,
        "autoClearCompleted",
        DEFAULT_SETTINGS.autoClearCompleted,
      ),
      autoClearTtlSeconds: Math.max(
        0,
        pickNum(blob, "autoClearTtlSeconds", DEFAULT_SETTINGS.autoClearTtlSeconds),
      ),
    };
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

/**
 * Persist `s` to the versioned key. Quota / private-mode failures are swallowed —
 * a preference write is best-effort and never fatal to the render tree.
 */
export function saveSettings(s: AppSettings): void {
  try {
    window.localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(s));
  } catch {
    /* storage unavailable (quota / private mode) — best-effort, never fatal */
  }
}
