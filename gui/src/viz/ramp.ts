/**
 * The perceptually-uniform diverging colour ramp (GUI-DESIGN §3.2): cool → light
 * neutral → warm, NEVER rainbow (rainbow ramps mislead magnitude). Implemented
 * directly in OKLCH and interpolated in OKLCH space so equal data steps read as
 * equal perceptual steps and the ramp stays colourblind-distinguishable.
 *
 * `t` in [0,1] maps 0 → cool (negative), 0.5 → neutral, 1 → warm (positive).
 * `shade` in [0,1] scales lightness for depth shading on the 3D mesh.
 */

interface Oklch {
  l: number;
  c: number;
  h: number;
}

// Stops mirror the rebranded --ramp-* design tokens (dark appearance): the
// cool (negative) pole tracks the indigo accent hue (~280), the mid is a
// near-neutral on the navy hue (264), the warm (positive) pole stays green.
// The negative (cool) pole is kept LIGHT ENOUGH that the heatmap's near-black cell
// ink clears WCAG AA 4.5:1 even at the depth-shaded (0.92) darkest cell — the old
// L0.55 negative pole composited to ~L0.50 and left black text at 4.40:1. Lifting
// the two cool stops keeps the diverging direction (cool↔warm) and chroma intact.
const STOPS: { t: number; col: Oklch }[] = [
  { t: 0.0, col: { l: 0.63, c: 0.14, h: 280 } }, // --ramp-neg-2
  { t: 0.25, col: { l: 0.71, c: 0.09, h: 278 } }, // --ramp-neg-1
  { t: 0.5, col: { l: 0.8, c: 0.01, h: 264 } }, // --ramp-mid
  { t: 0.75, col: { l: 0.74, c: 0.1, h: 145 } }, // --ramp-pos-1
  { t: 1.0, col: { l: 0.66, c: 0.16, h: 150 } }, // --ramp-pos-2
];

function lerp(a: number, b: number, w: number): number {
  return a + (b - a) * w;
}

/** Sample the diverging ramp at `t∈[0,1]`, optionally depth-shaded. */
export function rampColor(t: number, shade = 1): string {
  const clamped = Math.max(0, Math.min(1, t));
  let lo = STOPS[0]!;
  let hi = STOPS[STOPS.length - 1]!;
  for (let i = 0; i < STOPS.length - 1; i += 1) {
    if (clamped >= STOPS[i]!.t && clamped <= STOPS[i + 1]!.t) {
      lo = STOPS[i]!;
      hi = STOPS[i + 1]!;
      break;
    }
  }
  const span = hi.t - lo.t || 1;
  const w = (clamped - lo.t) / span;
  const l = lerp(lo.col.l, hi.col.l, w) * shade;
  const c = lerp(lo.col.c, hi.col.c, w);
  // Hue interpolation is fine here (no wrap across the chosen stops).
  const h = lerp(lo.col.h, hi.col.h, w);
  return `oklch(${l.toFixed(3)} ${c.toFixed(3)} ${h.toFixed(1)})`;
}

/** A CSS gradient string for legends/cells using the same stops. */
export function rampGradient(): string {
  const parts = STOPS.map((s) => `${rampColor(s.t)} ${(s.t * 100).toFixed(0)}%`);
  return `linear-gradient(90deg, ${parts.join(", ")})`;
}
