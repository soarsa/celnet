/**
 * AppSettings — the client-side, trader-owned preferences for the notification
 * surface. Persisted to `localStorage` under a VERSIONED key so a schema addition
 * is forward-safe: `loadSettings` MERGES a stored (possibly partial / older) blob
 * over the current defaults, so an added field always resolves to its default
 * rather than `undefined`.
 *
 * v2 (this schema) replaces the three coarse global toggles with a PER-EVENT
 * configuration map (`perEvent`), keyed by {@link NotificationEventType} — each
 * entry owns its own enable/sound/channels/volume — plus the master gates
 * (`masterMute`, `masterVolume`) and the streak/reduced-motion knobs. A v1 blob
 * (`celnet.settings.v1`) upgrades cleanly: its scalar prefs carry over, its
 * `volume` seeds `masterVolume`, and every `perEvent` entry falls back to its
 * default (so an existing trader loses nothing).
 *
 * Pure module — no React, no `any`; the parsed blob is `unknown`, narrowed
 * field-by-field.
 */

import type { NotificationKind } from "../data/contract";
import { SOUND_IDS, type SoundChoice } from "../lib/soundKit";

/**
 * The configurable trader event families (§3 of NOTIFICATIONS-REQUIREMENTS). The
 * lifecycle/RFQ/manual kinds map onto the wire {@link NotificationKind}. The server
 * now emits `ORDER_RECEIVED`/`FILL` (phase 5 — a FIX-venue firm-order lift emits
 * both), so `OrderReceived`/`Fill` are live; `FillBlock` is a CLIENT-side derivation
 * (a large/block `FILL` picks the `fill-block` cue) with no distinct wire kind.
 */
export type NotificationEventType =
  | "OrderReceived"
  | "Fill"
  | "FillBlock"
  | "RfqReceived"
  | "IoiReceived"
  | "ManualIntervention"
  | "QuoteAccepted"
  | "QuoteRejected"
  | "RequestLapsed";

/** Every configurable event type, in the stable order the settings table renders. */
export const NOTIFICATION_EVENT_TYPES: readonly NotificationEventType[] = [
  "RfqReceived",
  "IoiReceived",
  "ManualIntervention",
  "QuoteAccepted",
  "QuoteRejected",
  "RequestLapsed",
  "OrderReceived",
  "Fill",
  "FillBlock",
];

/**
 * Map an inbound wire {@link NotificationKind} to the {@link NotificationEventType}
 * whose per-event config governs it. Every existing kind resolves; the deferred
 * `ORDER_RECEIVED`/`FILL` kinds are not in the contract yet, so their config rows
 * stay forward-ready until phase 5 emits them.
 */
export function eventTypeForKind(kind: NotificationKind): NotificationEventType {
  switch (kind) {
    case "RFQ_RECEIVED":
      return "RfqReceived";
    case "IOI_RECEIVED":
      return "IoiReceived";
    case "MANUAL_INTERVENTION_REQUIRED":
      return "ManualIntervention";
    case "QUOTE_ACCEPTED":
      return "QuoteAccepted";
    case "QUOTE_REJECTED":
      return "QuoteRejected";
    case "REQUEST_WITHDRAWN":
    case "REQUEST_EXPIRED":
      return "RequestLapsed";
    case "ORDER_RECEIVED":
      return "OrderReceived";
    case "FILL":
      return "Fill";
  }
}

/** The two delivery channels, layered ON TOP of the focus-aware rule (§5.3). */
export interface EventChannels {
  /** In-app "growl" toast — shown when the tab is focused/visible. */
  toast: boolean;
  /** OS desktop banner — shown ONLY when the tab is hidden/unfocused. */
  desktop: boolean;
}

/** The per-event trader preference (§6.1). */
export interface PerEventPref {
  /** Master on/off for THIS event — off ⇒ no toast/desktop/sound (still logged). */
  enabled: boolean;
  /** The cue to play (or `"none"` for a silent-but-visible event). */
  sound: SoundChoice;
  /** Which channels deliver it (each still bound by the focus-aware rule). */
  channels: EventChannels;
  /** Per-event volume trim, 0–100, multiplied into the master volume. */
  volume: number;
}

/** Reduced-motion policy for toast entrance/exit animation. */
export type ReducedMotionPref = "auto" | "on" | "off";

/** The persisted trader preferences. All fields are client-side only. */
export interface AppSettings {
  /** Master in-app alerts switch. Off ⇒ no item, toast, sound, or desktop growl. */
  alertsEnabled: boolean;
  /** Whether inbound alerts play the audio cue (the sound-feature master switch). */
  soundsEnabled: boolean;
  /** Legacy master volume, 0–100 (retained; `masterVolume` seeds from it on v1→v2). */
  volume: number;
  /** Raw notional threshold: events with a derived notional < this raise nothing. */
  minQty: number;
  /** Desktop ("growl") browser notifications master switch. */
  growlEnabled: boolean;
  /** Whether completed / aged notifications are auto-cleared from the list. */
  autoClearCompleted: boolean;
  /** Age (seconds) after which auto-clear sweeps a notification. */
  autoClearTtlSeconds: number;
  /** Do-not-disturb: mute EVERY cue regardless of per-event sound (§6.1 master). */
  masterMute: boolean;
  /** Master volume, 0–100 — the ceiling every per-event volume is scaled against. */
  masterVolume: number;
  /** Per-event configuration, one entry per {@link NotificationEventType}. */
  perEvent: Record<NotificationEventType, PerEventPref>;
  /** Coalesce consecutive fills within this window (ms) into `fill-streak`. */
  streakWindowMs: number;
  /** Toast-animation motion policy (`auto` ⇒ honour `prefers-reduced-motion`). */
  reducedMotion: ReducedMotionPref;
}

/** The out-of-the-box per-event defaults (urgent ⇒ desktop+toast; routine ⇒ toast). */
export const DEFAULT_PER_EVENT: Record<NotificationEventType, PerEventPref> = {
  RfqReceived: { enabled: true, sound: "rfq-work", channels: { toast: true, desktop: false }, volume: 100 },
  IoiReceived: { enabled: true, sound: "rfq-work", channels: { toast: true, desktop: false }, volume: 100 },
  ManualIntervention: { enabled: true, sound: "needs-you", channels: { toast: true, desktop: true }, volume: 100 },
  QuoteAccepted: { enabled: true, sound: "won", channels: { toast: true, desktop: false }, volume: 100 },
  QuoteRejected: { enabled: true, sound: "lost", channels: { toast: true, desktop: true }, volume: 100 },
  RequestLapsed: { enabled: true, sound: "lapsed", channels: { toast: true, desktop: false }, volume: 100 },
  OrderReceived: { enabled: true, sound: "order-in", channels: { toast: true, desktop: false }, volume: 100 },
  Fill: { enabled: true, sound: "fill-confirm", channels: { toast: true, desktop: false }, volume: 100 },
  FillBlock: { enabled: true, sound: "fill-block", channels: { toast: true, desktop: false }, volume: 100 },
};

/** Clone the default per-event map (never share the nested objects). */
function defaultPerEvent(): Record<NotificationEventType, PerEventPref> {
  const out = {} as Record<NotificationEventType, PerEventPref>;
  for (const et of NOTIFICATION_EVENT_TYPES) {
    const d = DEFAULT_PER_EVENT[et];
    out[et] = { enabled: d.enabled, sound: d.sound, channels: { ...d.channels }, volume: d.volume };
  }
  return out;
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
  masterMute: false,
  masterVolume: 70,
  perEvent: defaultPerEvent(),
  streakWindowMs: 1500,
  reducedMotion: "auto",
};

/** The current versioned localStorage key. */
export const SETTINGS_STORAGE_KEY = "celnet.settings.v2";
/** The prior versioned key, migrated forward on first v2 load. */
export const SETTINGS_STORAGE_KEY_V1 = "celnet.settings.v1";

const VOLUME_MIN = 0;
const VOLUME_MAX = 100;

/** Clamp `n` into `[lo, hi]`. */
function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}

/** Read a boolean field, falling back to the default. */
function bool(v: unknown, fallback: boolean): boolean {
  return typeof v === "boolean" ? v : fallback;
}

/** Read a finite-number field, falling back to the default. */
function num(v: unknown, fallback: number): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

/** Narrow a value to a valid {@link SoundChoice}, else the default. */
function soundChoice(v: unknown, fallback: SoundChoice): SoundChoice {
  if (v === "none") return "none";
  return typeof v === "string" && (SOUND_IDS as readonly string[]).includes(v)
    ? (v as SoundChoice)
    : fallback;
}

/** Narrow a value to a valid {@link ReducedMotionPref}, else the default. */
function motion(v: unknown, fallback: ReducedMotionPref): ReducedMotionPref {
  return v === "auto" || v === "on" || v === "off" ? v : fallback;
}

/** Merge one stored per-event entry over its default (defaults fill any gap). */
function mergeEventPref(raw: unknown, def: PerEventPref): PerEventPref {
  const b = typeof raw === "object" && raw !== null ? (raw as Record<string, unknown>) : {};
  const ch = typeof b.channels === "object" && b.channels !== null
    ? (b.channels as Record<string, unknown>)
    : {};
  return {
    enabled: bool(b.enabled, def.enabled),
    sound: soundChoice(b.sound, def.sound),
    channels: {
      toast: bool(ch.toast, def.channels.toast),
      desktop: bool(ch.desktop, def.channels.desktop),
    },
    volume: clamp(num(b.volume, def.volume), VOLUME_MIN, VOLUME_MAX),
  };
}

/** Merge a stored `perEvent` blob over the defaults — missing entries default. */
function mergePerEvent(raw: unknown): Record<NotificationEventType, PerEventPref> {
  const b = typeof raw === "object" && raw !== null ? (raw as Record<string, unknown>) : {};
  const out = {} as Record<NotificationEventType, PerEventPref>;
  for (const et of NOTIFICATION_EVENT_TYPES) {
    out[et] = mergeEventPref(b[et], DEFAULT_PER_EVENT[et]);
  }
  return out;
}

/**
 * Merge a parsed settings blob over {@link DEFAULT_SETTINGS}. When `legacy` (a v1
 * blob), `masterVolume` seeds from the blob's `volume` if it carries no explicit
 * `masterVolume`, so a v1 trader's volume survives the upgrade.
 */
function mergeBlob(blob: Record<string, unknown>, legacy: boolean): AppSettings {
  const legacyVolume = clamp(num(blob.volume, DEFAULT_SETTINGS.volume), VOLUME_MIN, VOLUME_MAX);
  const masterVolume = legacy && blob.masterVolume === undefined
    ? legacyVolume
    : clamp(num(blob.masterVolume, DEFAULT_SETTINGS.masterVolume), VOLUME_MIN, VOLUME_MAX);
  return {
    alertsEnabled: bool(blob.alertsEnabled, DEFAULT_SETTINGS.alertsEnabled),
    soundsEnabled: bool(blob.soundsEnabled, DEFAULT_SETTINGS.soundsEnabled),
    volume: legacyVolume,
    minQty: Math.max(0, num(blob.minQty, DEFAULT_SETTINGS.minQty)),
    growlEnabled: bool(blob.growlEnabled, DEFAULT_SETTINGS.growlEnabled),
    autoClearCompleted: bool(blob.autoClearCompleted, DEFAULT_SETTINGS.autoClearCompleted),
    autoClearTtlSeconds: Math.max(0, num(blob.autoClearTtlSeconds, DEFAULT_SETTINGS.autoClearTtlSeconds)),
    masterMute: bool(blob.masterMute, DEFAULT_SETTINGS.masterMute),
    masterVolume,
    perEvent: mergePerEvent(blob.perEvent),
    streakWindowMs: Math.max(0, num(blob.streakWindowMs, DEFAULT_SETTINGS.streakWindowMs)),
    reducedMotion: motion(blob.reducedMotion, DEFAULT_SETTINGS.reducedMotion),
  };
}

/** Parse a stored JSON string into settings; malformed input ⇒ the defaults. */
function parseStored(raw: string, legacy: boolean): AppSettings {
  const parsed: unknown = JSON.parse(raw);
  if (typeof parsed !== "object" || parsed === null) return { ...DEFAULT_SETTINGS, perEvent: defaultPerEvent() };
  return mergeBlob(parsed as Record<string, unknown>, legacy);
}

/**
 * Load the persisted settings. Prefers the v2 key; when absent, a v1 blob is
 * MIGRATED forward (scalars carry over, `volume` seeds `masterVolume`, every
 * `perEvent` entry defaults). Any error ⇒ the defaults. Never throws.
 */
export function loadSettings(): AppSettings {
  try {
    const rawV2 = window.localStorage.getItem(SETTINGS_STORAGE_KEY);
    if (rawV2 !== null) return parseStored(rawV2, false);
    const rawV1 = window.localStorage.getItem(SETTINGS_STORAGE_KEY_V1);
    if (rawV1 !== null) return parseStored(rawV1, true);
    return { ...DEFAULT_SETTINGS, perEvent: defaultPerEvent() };
  } catch {
    return { ...DEFAULT_SETTINGS, perEvent: defaultPerEvent() };
  }
}

/**
 * Persist `s` to the v2 key. Quota / private-mode failures are swallowed — a
 * preference write is best-effort and never fatal to the render tree.
 */
export function saveSettings(s: AppSettings): void {
  try {
    window.localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(s));
  } catch {
    /* storage unavailable (quota / private mode) — best-effort, never fatal */
  }
}

/**
 * The effective per-event cue volume (0–100): the master volume scaled by the
 * event's own volume trim. Both are clamped; the product stays in 0–100.
 */
export function effectiveEventVolume(masterVolume: number, eventVolume: number): number {
  return (clamp(masterVolume, VOLUME_MIN, VOLUME_MAX) * clamp(eventVolume, VOLUME_MIN, VOLUME_MAX)) / 100;
}
