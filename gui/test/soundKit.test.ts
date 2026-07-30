import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  SOUND_CUES,
  SOUND_IDS,
  SOUND_LABELS,
  playSound,
  previewSound,
  resetSharedAudioContextForTests,
  type SoundId,
} from "../src/lib/soundKit";

/** A minimal but complete fake AudioContext covering every node a cue may touch. */
function fakeAudioContextClass(spies: { osc: () => void; noise: () => void }) {
  const param = () => ({
    value: 0,
    setValueAtTime: vi.fn(),
    exponentialRampToValueAtTime: vi.fn(),
  });
  return class FakeAudioContext {
    state = "running";
    currentTime = 0;
    sampleRate = 48_000;
    destination = {};
    createOscillator() {
      spies.osc();
      return { type: "", frequency: param(), connect: vi.fn(), start: vi.fn(), stop: vi.fn() };
    }
    createGain() {
      return { gain: param(), connect: vi.fn() };
    }
    createBuffer(_ch: number, frames: number) {
      return { getChannelData: () => new Float32Array(frames) };
    }
    createBufferSource() {
      spies.noise();
      return { buffer: null, connect: vi.fn(), start: vi.fn(), stop: vi.fn() };
    }
    createBiquadFilter() {
      return { type: "", frequency: param(), connect: vi.fn() };
    }
    resume() {
      return Promise.resolve();
    }
  };
}

describe("soundKit registry", () => {
  it("has a cue function for every SoundId", () => {
    for (const id of SOUND_IDS) {
      expect(typeof SOUND_CUES[id]).toBe("function");
    }
    // The registry and the id list agree exactly.
    expect(Object.keys(SOUND_CUES).sort()).toEqual([...SOUND_IDS].sort());
  });

  it("labels every id plus the silent choice", () => {
    for (const id of SOUND_IDS) expect(SOUND_LABELS[id]).toBeTruthy();
    expect(SOUND_LABELS.none).toBeTruthy();
  });
});

describe("playSound (guarded)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    resetSharedAudioContextForTests();
  });

  it("never throws when AudioContext is unavailable", () => {
    resetSharedAudioContextForTests();
    vi.stubGlobal("AudioContext", undefined);
    for (const id of SOUND_IDS) {
      expect(() => playSound(id, 60, 3)).not.toThrow();
    }
  });

  it("is a silent no-op at zero volume (no nodes created)", () => {
    const osc = vi.fn();
    const noise = vi.fn();
    resetSharedAudioContextForTests();
    vi.stubGlobal("AudioContext", fakeAudioContextClass({ osc, noise }));
    playSound("won", 0);
    playSound("none", 80);
    expect(osc).not.toHaveBeenCalled();
    expect(noise).not.toHaveBeenCalled();
  });

  it("builds nodes for every cue without throwing when a context exists", () => {
    const osc = vi.fn();
    const noise = vi.fn();
    resetSharedAudioContextForTests();
    vi.stubGlobal("AudioContext", fakeAudioContextClass({ osc, noise }));
    for (const id of SOUND_IDS) {
      expect(() => playSound(id as SoundId, 80, 4)).not.toThrow();
    }
    // Tonal cues built oscillators; the noise-based cues built buffer sources.
    expect(osc).toHaveBeenCalled();
    expect(noise).toHaveBeenCalled();
  });

  it("clamps an out-of-range volume without throwing", () => {
    const osc = vi.fn();
    resetSharedAudioContextForTests();
    vi.stubGlobal("AudioContext", fakeAudioContextClass({ osc, noise: vi.fn() }));
    expect(() => playSound("rfq-work", 999)).not.toThrow();
  });
});

describe("previewSound", () => {
  beforeEach(() => {
    resetSharedAudioContextForTests();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    resetSharedAudioContextForTests();
  });

  it("is a no-op for the silent choice", () => {
    const osc = vi.fn();
    vi.stubGlobal("AudioContext", fakeAudioContextClass({ osc, noise: vi.fn() }));
    previewSound("none", 80);
    expect(osc).not.toHaveBeenCalled();
  });

  it("plays a cue without throwing", () => {
    vi.stubGlobal("AudioContext", fakeAudioContextClass({ osc: vi.fn(), noise: vi.fn() }));
    expect(() => previewSound("fill-streak", 70)).not.toThrow();
  });
});
