/**
 * soundKit — the trader notification sound library. A registry of short,
 * procedurally-SYNTHESIZED Web-Audio cues (zero license, zero bundle weight —
 * guardrail #7: every cue is our own IP, no recorded/copyrighted audio) that the
 * per-event notification config (settingsSchema `perEvent`) and the settings
 * preview button reference by a stable {@link SoundId}.
 *
 * Each cue is `(ctx, volume, streak?) => void` where `volume` is a 0–1 gain
 * scalar (already normalised + master-scaled by the caller) and `streak` is an
 * optional consecutive-fill counter that the escalating `fill-streak` arpeggio
 * uses to climb its pitch ladder (double → triple → multi). Every cue is capped
 * to a gentle peak so it never blares, is < ~0.6 s, and is fully guarded — an
 * unsupported/suspended AudioContext degrades to a silent no-op and NEVER throws
 * into React.
 *
 * The AudioContext is a single lazily-created shared node graph (one is cheap to
 * reuse and browsers cap how many you may create), unlocked behind the first user
 * gesture via its `suspended → resume()` guard.
 */

/** The stable identifier of a synthesized cue (never free text in config/UI). */
export type SoundId =
  | "fill-confirm"
  | "fill-streak"
  | "fill-block"
  | "order-in"
  | "rfq-work"
  | "needs-you"
  | "won"
  | "lost"
  | "lapsed"
  | "celebrate";

/** A per-event sound choice: a cue id, or `"none"` for a silent event. */
export type SoundChoice = SoundId | "none";

/** A cue: schedule its nodes on `ctx` at `volume` (0–1). `streak` is fill-only. */
export type CueFn = (ctx: AudioContext, volume: number, streak?: number) => void;

/** The gentle peak-gain ceiling (at full volume) so a cue never blares. */
const PEAK_CEILING = 0.14;
/** Cap the `fill-streak` ladder note count (its "how high it climbs" ceiling). */
const STREAK_MAX_NOTES = 6;

// --- shared AudioContext (lazy, guarded, reset-able for tests) ---------------

let sharedAudioCtx: AudioContext | null = null;

/** The AudioContext ctor across browsers (`webkit`-prefixed on old Safari). */
function audioCtxCtor(): (new () => AudioContext) | null {
  if (typeof window === "undefined") return null;
  const w = window as unknown as {
    AudioContext?: new () => AudioContext;
    webkitAudioContext?: new () => AudioContext;
  };
  return w.AudioContext ?? w.webkitAudioContext ?? null;
}

/** The shared AudioContext (created once), or null when unsupported. Never throws. */
export function getSharedAudioContext(): AudioContext | null {
  try {
    if (sharedAudioCtx) return sharedAudioCtx;
    const Ctor = audioCtxCtor();
    if (!Ctor) return null;
    sharedAudioCtx = new Ctor();
    return sharedAudioCtx;
  } catch {
    return null;
  }
}

/** Drop the cached shared AudioContext. TEST-ONLY: lets a suite re-stub the ctor. */
export function resetSharedAudioContextForTests(): void {
  sharedAudioCtx = null;
}

// --- synthesis primitives ----------------------------------------------------

/** Schedule one enveloped oscillator tone. Guarded by the caller's try/catch. */
function tone(
  ctx: AudioContext,
  opts: {
    freq: number;
    start: number;
    dur: number;
    peak: number;
    type?: OscillatorType;
    endFreq?: number;
  },
): void {
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = opts.type ?? "sine";
  osc.frequency.setValueAtTime(opts.freq, opts.start);
  if (opts.endFreq !== undefined) {
    osc.frequency.exponentialRampToValueAtTime(
      Math.max(1, opts.endFreq),
      opts.start + opts.dur,
    );
  }
  const peak = Math.max(0.0002, opts.peak);
  gain.gain.setValueAtTime(0.0001, opts.start);
  gain.gain.exponentialRampToValueAtTime(peak, opts.start + Math.min(0.012, opts.dur / 2));
  gain.gain.exponentialRampToValueAtTime(0.0001, opts.start + opts.dur);
  osc.connect(gain);
  gain.connect(ctx.destination);
  osc.start(opts.start);
  osc.stop(opts.start + opts.dur + 0.02);
}

/** Schedule a short filtered white-noise burst — a percussive "shot"/thud. */
function noiseBurst(
  ctx: AudioContext,
  opts: { start: number; dur: number; peak: number; cutoff: number },
): void {
  const frames = Math.max(1, Math.floor(ctx.sampleRate * opts.dur));
  const buffer = ctx.createBuffer(1, frames, ctx.sampleRate);
  const data = buffer.getChannelData(0);
  for (let i = 0; i < frames; i += 1) data[i] = Math.random() * 2 - 1;
  const src = ctx.createBufferSource();
  src.buffer = buffer;
  const lp = ctx.createBiquadFilter();
  lp.type = "lowpass";
  lp.frequency.setValueAtTime(opts.cutoff, opts.start);
  const gain = ctx.createGain();
  const peak = Math.max(0.0002, opts.peak);
  gain.gain.setValueAtTime(peak, opts.start);
  gain.gain.exponentialRampToValueAtTime(0.0001, opts.start + opts.dur);
  src.connect(lp);
  lp.connect(gain);
  gain.connect(ctx.destination);
  src.start(opts.start);
  src.stop(opts.start + opts.dur + 0.02);
}

/** An ascending pentatonic run whose length/height encodes the streak count. */
function arpeggio(ctx: AudioContext, volume: number, streak: number): void {
  // C D E G A C — a bright, consonant pentatonic ladder.
  const scale = [523.25, 587.33, 659.25, 783.99, 880.0, 1046.5];
  const notes = Math.max(2, Math.min(STREAK_MAX_NOTES, Math.floor(streak) || 2));
  // Beyond the note ceiling, transpose the whole run UP a semitone per extra
  // streak so a very long streak still climbs (the ladder keeps rising).
  const semis = Math.max(0, (Math.floor(streak) || 2) - STREAK_MAX_NOTES);
  const shift = Math.pow(2, semis / 12);
  const now = ctx.currentTime;
  const step = 0.075;
  for (let i = 0; i < notes; i += 1) {
    const base = scale[i] ?? scale[scale.length - 1] ?? 880;
    tone(ctx, {
      freq: base * shift,
      start: now + i * step,
      dur: 0.11,
      peak: volume * PEAK_CEILING * (i === notes - 1 ? 1 : 0.8),
      type: "triangle",
    });
  }
}

// --- the cues ----------------------------------------------------------------

const fillConfirm: CueFn = (ctx, v) => {
  const t = ctx.currentTime;
  tone(ctx, { freq: 660, start: t, dur: 0.09, peak: v * PEAK_CEILING, type: "sine" });
  tone(ctx, { freq: 880, start: t + 0.08, dur: 0.11, peak: v * PEAK_CEILING, type: "sine" });
};

const fillStreak: CueFn = (ctx, v, streak) => arpeggio(ctx, v, streak ?? 2);

const fillBlock: CueFn = (ctx, v) => {
  const t = ctx.currentTime;
  // A low sine "body" thump under a fast, dark noise transient — a "shot"/knock.
  tone(ctx, { freq: 90, start: t, dur: 0.16, peak: v * PEAK_CEILING, type: "sine", endFreq: 55 });
  noiseBurst(ctx, { start: t, dur: 0.14, peak: v * PEAK_CEILING * 0.9, cutoff: 700 });
};

const orderIn: CueFn = (ctx, v) => {
  tone(ctx, { freq: 440, start: ctx.currentTime, dur: 0.14, peak: v * PEAK_CEILING, type: "triangle" });
};

const rfqWork: CueFn = (ctx, v) => {
  tone(ctx, { freq: 523.25, start: ctx.currentTime, dur: 0.12, peak: v * PEAK_CEILING, type: "sine" });
};

const needsYou: CueFn = (ctx, v) => {
  // Insistent two-note, repeated once — attention-grabbing, not alarming.
  const t = ctx.currentTime;
  const peak = v * PEAK_CEILING;
  tone(ctx, { freq: 932, start: t, dur: 0.1, peak, type: "sine" });
  tone(ctx, { freq: 1046.5, start: t + 0.11, dur: 0.1, peak, type: "sine" });
  tone(ctx, { freq: 932, start: t + 0.26, dur: 0.1, peak, type: "sine" });
  tone(ctx, { freq: 1046.5, start: t + 0.37, dur: 0.12, peak, type: "sine" });
};

const won: CueFn = (ctx, v) => {
  // Rising major triad C–E–G — a pleasant success chime.
  const t = ctx.currentTime;
  const peak = v * PEAK_CEILING;
  tone(ctx, { freq: 523.25, start: t, dur: 0.1, peak, type: "sine" });
  tone(ctx, { freq: 659.25, start: t + 0.07, dur: 0.1, peak, type: "sine" });
  tone(ctx, { freq: 783.99, start: t + 0.14, dur: 0.16, peak, type: "sine" });
};

const lost: CueFn = (ctx, v) => {
  // Gentle descending "declined".
  const t = ctx.currentTime;
  const peak = v * PEAK_CEILING;
  tone(ctx, { freq: 523.25, start: t, dur: 0.1, peak, type: "sine" });
  tone(ctx, { freq: 392.0, start: t + 0.08, dur: 0.1, peak, type: "sine" });
  tone(ctx, { freq: 349.23, start: t + 0.16, dur: 0.16, peak, type: "sine" });
};

const lapsed: CueFn = (ctx, v) => {
  tone(ctx, { freq: 392.0, start: ctx.currentTime, dur: 0.1, peak: v * PEAK_CEILING * 0.85, type: "sine" });
};

const celebrate: CueFn = (ctx, v) => {
  // A cheeky pitch-swept "boing" + a pop — the opt-in fun cue.
  const t = ctx.currentTime;
  const peak = v * PEAK_CEILING;
  tone(ctx, { freq: 300, start: t, dur: 0.18, peak, type: "triangle", endFreq: 900 });
  tone(ctx, { freq: 900, start: t + 0.16, dur: 0.14, peak, type: "triangle", endFreq: 500 });
  noiseBurst(ctx, { start: t + 0.02, dur: 0.06, peak: peak * 0.6, cutoff: 2000 });
};

/** The cue registry — every {@link SoundId} maps to exactly one synthesis fn. */
export const SOUND_CUES: Record<SoundId, CueFn> = {
  "fill-confirm": fillConfirm,
  "fill-streak": fillStreak,
  "fill-block": fillBlock,
  "order-in": orderIn,
  "rfq-work": rfqWork,
  "needs-you": needsYou,
  won,
  lost,
  lapsed,
  celebrate,
};

/** Every selectable cue id, in a stable display order (for the config dropdown). */
export const SOUND_IDS: readonly SoundId[] = [
  "fill-confirm",
  "fill-streak",
  "fill-block",
  "order-in",
  "rfq-work",
  "needs-you",
  "won",
  "lost",
  "lapsed",
  "celebrate",
];

/** Human labels for the sound picker (never expose raw ids to the trader). */
export const SOUND_LABELS: Record<SoundChoice, string> = {
  "fill-confirm": "Fill confirm",
  "fill-streak": "Fill streak (multi)",
  "fill-block": "Block fill (shot)",
  "order-in": "Order in",
  "rfq-work": "RFQ to work",
  "needs-you": "Needs you (urgent)",
  won: "Won",
  lost: "Lost",
  lapsed: "Lapsed",
  celebrate: "Celebrate",
  none: "Silent",
};

/**
 * Play a cue by id at `volume` (0–100), scaled into the gentle gain ceiling.
 * `streak` drives the `fill-streak` ladder height. Fully guarded — a zero volume,
 * unsupported/suspended context, or a throwing node call degrades to a silent
 * no-op and NEVER throws.
 */
export function playSound(sound: SoundChoice, volume: number, streak = 0): void {
  try {
    if (sound === "none") return;
    const vol = Math.max(0, Math.min(100, volume)) / 100;
    if (vol <= 0) return;
    const cue = SOUND_CUES[sound];
    if (!cue) return;
    const ctx = getSharedAudioContext();
    if (!ctx) return;
    if (ctx.state === "suspended") void ctx.resume().catch(() => {});
    cue(ctx, vol, streak);
  } catch {
    /* audio unsupported / blocked — never throw into the render tree */
  }
}

/**
 * Preview a cue for the settings picker — plays it at `volume` (0–100). For
 * `fill-streak` a representative streak of 3 is used so the escalating ladder is
 * audible. A silent choice is a no-op.
 */
export function previewSound(sound: SoundChoice, volume: number): void {
  if (sound === "none") return;
  playSound(sound, volume, sound === "fill-streak" ? 3 : 0);
}
