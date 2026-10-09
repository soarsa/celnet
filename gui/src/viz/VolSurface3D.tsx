/**
 * VolSurface3D — the signature shaded, rotatable 3D implied-volatility surface
 * (GUI-DESIGN §4.3, mockup 01-vol-surface). A real WebGL renderer (three.js): a
 * lit, vertex-coloured mesh over the (tenor × delta) plane where BOTH the vertex
 * height AND its Viridis colour encode the implied vol — X = tenor, Z = delta,
 * Y (up) + `--seq-*` colour = vol. OrbitControls give drag-to-rotate and
 * scroll-to-zoom; a `mode` prop swaps between the shaded mesh, a wireframe and a
 * point cloud; arbitrage-flagged vertices are highlighted with a `--danger` flag.
 *
 * Colour policy (GUIDE.md / dataviz contract): quantitative colour is the
 * Viridis sequential ramp (`--seq-1..6`), NEVER brand coral/indigo. WebGL cannot
 * read CSS variables, so the six ramp stops (and the danger/grid chrome colours)
 * are resolved ONCE at mount from the live token cascade. The tokens are authored
 * in `oklch()`, which three r185's `Color` parser does not accept, so each token
 * is normalised to a concrete sRGB triplet through a 1×1 canvas (the browser
 * gamut-maps oklch → sRGB deterministically) and fed to `Color.setRGB(..., SRGB)`.
 *
 * Honesty discipline: a surface needs at least a 2×2 finite grid whose axis
 * labels match its dimensions — anything else renders an explicit empty state
 * rather than a fabricated mesh. The mesh is a QC / visual aid; the marking grid
 * remains the source of truth.
 *
 * Performance / lifecycle: the WebGL context, geometry, materials and controls are
 * lazily created on mount and fully disposed on unmount (no GPU leak across story
 * navigations / hot-reloads). The continuous render loop runs ONLY when damping is
 * active; under `prefers-reduced-motion` damping is off and frames are drawn on
 * demand (one per interaction), so an idle surface costs nothing.
 */

import { useEffect, useMemo, useRef, useState } from "react";
// The three.js runtime is code-split OUT of the main bundle: only its TYPES are
// imported statically (fully erased at build — `verbatimModuleSyntax`), while the
// runtime module + OrbitControls are `await import(…)`-ed inside the mount effect
// (fixes the >500KB main-chunk warning). A lightweight loading state shows until
// they resolve; the public Props and rendered behaviour are unchanged.
import type * as THREE from "three";

/** The three.js runtime module type (for the lazily-imported values). */
type ThreeModule = typeof import("three");

/** A vertex to highlight — e.g. an arbitrage-flagged wing (butterfly < 0). */
export interface VolSurfaceMarker {
  /** Column index into the tenor axis (0 … tenors.length-1). */
  readonly tenorIndex: number;
  /** Row index into the delta axis (0 … deltas.length-1). */
  readonly deltaIndex: number;
  /** Why the vertex is flagged — surfaced in the accessible description / title. */
  readonly reason?: string;
}

/** How the surface is drawn; also flippable live from a parent / Storybook control. */
export type VolSurfaceMode = "shaded" | "wireframe" | "points";

export interface VolSurface3DProps {
  /**
   * Row-major grid of implied vols in vol points, `vols[tenorIndex][deltaIndex]`
   * (e.g. `7.85`). Must be rectangular and at least 2×2 with finite entries.
   */
  readonly vols: ReadonlyArray<ReadonlyArray<number>>;
  /** Tenor axis labels (X); `length` must equal `vols.length`. */
  readonly tenors: ReadonlyArray<string>;
  /** Delta axis labels (Z); `length` must equal each `vols` row length. */
  readonly deltas: ReadonlyArray<string>;
  /** Vertices to highlight (arbitrage flags); out-of-range entries are ignored. */
  readonly markers?: ReadonlyArray<VolSurfaceMarker>;
  /** Render mode — shaded mesh (default), wireframe, or point cloud. */
  readonly mode?: VolSurfaceMode;
  /** Canvas width in CSS px. */
  readonly width?: number;
  /** Canvas height in CSS px. */
  readonly height?: number;
  /** Display-only vertical exaggeration of the vol height (colour is unaffected). */
  readonly heightGain?: number;
}

/** The `--seq-*` Viridis ramp tokens, cool→warm (low vol → high vol). */
const SEQ_TOKENS = ["--seq-1", "--seq-2", "--seq-3", "--seq-4", "--seq-5", "--seq-6"] as const;

/**
 * sRGB fallbacks for the Viridis stops — used only when the token cascade or the
 * browser's oklch parser is unavailable (SSR / typecheck); in a real browser the
 * live `oklch()` tokens are resolved instead. Ordered cool→warm to match SEQ.
 */
const SEQ_FALLBACK = ["#332766", "#3f4d8f", "#3b7f92", "#3fae8c", "#8fce55", "#e2e34f"] as const;

const NO_MARKERS: ReadonlyArray<VolSurfaceMarker> = [];

/** Mesh footprint (half-extents) and vol-height amplitude in scene units. */
const HALF_X = 1.15;
const HALF_Z = 0.95;
const HEIGHT_AMP = 0.85;

interface SurfaceModel {
  readonly ok: true;
  readonly grid: ReadonlyArray<ReadonlyArray<number>>;
  readonly nT: number;
  readonly nD: number;
  readonly vMin: number;
  readonly vMax: number;
}
interface SurfaceModelError {
  readonly ok: false;
  readonly reason: string;
}

/** Validate + normalise the props into a rectangular, finite grid (or an honest error). */
function buildModel(
  vols: ReadonlyArray<ReadonlyArray<number>>,
  tenors: ReadonlyArray<string>,
  deltas: ReadonlyArray<string>,
): SurfaceModel | SurfaceModelError {
  const nT = vols.length;
  if (nT < 2) return { ok: false, reason: "needs at least 2 tenors" };
  const first = vols[0];
  const nD = first ? first.length : 0;
  if (nD < 2) return { ok: false, reason: "needs at least 2 delta pillars" };
  if (tenors.length !== nT) return { ok: false, reason: "tenor labels do not match the grid" };
  if (deltas.length !== nD) return { ok: false, reason: "delta labels do not match the grid" };

  let vMin = Infinity;
  let vMax = -Infinity;
  const grid: number[][] = [];
  for (let ti = 0; ti < nT; ti += 1) {
    const row = vols[ti];
    if (!row || row.length !== nD) return { ok: false, reason: "the vol grid is not rectangular" };
    const out: number[] = [];
    for (let di = 0; di < nD; di += 1) {
      const v = row[di];
      if (v === undefined || !Number.isFinite(v)) {
        return { ok: false, reason: "the vol grid has a non-finite entry" };
      }
      out.push(v);
      if (v < vMin) vMin = v;
      if (v > vMax) vMax = v;
    }
    grid.push(out);
  }
  return { ok: true, grid, nT, nD, vMin, vMax };
}

/**
 * Resolve a CSS colour token to a three `Color`. Reads the live cascade value
 * (an `oklch()` string) and normalises it through a 1×1 canvas so the browser
 * gamut-maps it to sRGB bytes, which are handed to `setRGB(..., SRGBColorSpace)`.
 */
function resolveTokenColor(three: ThreeModule, token: string, fallback: string): THREE.Color {
  const color = new three.Color();
  let css = fallback;
  if (typeof document !== "undefined") {
    const v = getComputedStyle(document.documentElement).getPropertyValue(token).trim();
    if (v) css = v;
  }
  if (typeof document === "undefined") {
    color.setStyle(fallback, three.SRGBColorSpace);
    return color;
  }
  const canvas = document.createElement("canvas");
  canvas.width = 1;
  canvas.height = 1;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) {
    color.setStyle(fallback, three.SRGBColorSpace);
    return color;
  }
  ctx.fillStyle = fallback; // known-parseable seed; kept if `css` is unparseable
  ctx.fillStyle = css; // oklch() → normalised by the browser
  ctx.fillRect(0, 0, 1, 1);
  const data = ctx.getImageData(0, 0, 1, 1).data;
  color.setRGB((data[0] ?? 0) / 255, (data[1] ?? 0) / 255, (data[2] ?? 0) / 255, three.SRGBColorSpace);
  return color;
}

/** Sample a piecewise-linear ramp of `stops` at `t∈[0,1]` into `out`. */
function sampleRamp(stops: ReadonlyArray<THREE.Color>, t: number, out: THREE.Color): void {
  const clamped = Math.max(0, Math.min(1, t));
  const segs = stops.length - 1;
  const scaled = clamped * segs;
  const i = Math.min(segs - 1, Math.floor(scaled));
  const lo = stops[i] ?? stops[0]!;
  const hi = stops[i + 1] ?? stops[stops.length - 1]!;
  out.copy(lo).lerp(hi, scaled - i);
}

function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState<boolean>(() =>
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-reduced-motion: reduce)").matches
      : false,
  );
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onChange = (): void => setReduced(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
  return reduced;
}

export function VolSurface3D({
  vols,
  tenors,
  deltas,
  markers = NO_MARKERS,
  mode = "shaded",
  width = 560,
  height = 400,
  heightGain = 1,
}: VolSurface3DProps): React.ReactElement {
  const mountRef = useRef<HTMLDivElement>(null);
  const reducedMotion = useReducedMotion();
  const [glError, setGlError] = useState<string | null>(null);
  // False until the code-split three.js runtime resolves; drives the loading state.
  const [libLoaded, setLibLoaded] = useState(false);

  const model = useMemo(() => buildModel(vols, tenors, deltas), [vols, tenors, deltas]);

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount || !model.ok) return;

    // The heavy three.js runtime + OrbitControls are code-split out of the main
    // bundle and dynamically imported here on mount; the effect stays sync-return
    // (React requires a cleanup fn, not a Promise), so the async work runs in an
    // IIFE and its teardown is captured into `cleanup`, guarded by `disposed`.
    let disposed = false;
    let cleanup: (() => void) | null = null;

    void (async () => {
      let mods: readonly [ThreeModule, typeof import("three/examples/jsm/controls/OrbitControls.js")];
      try {
        mods = await Promise.all([
          import("three"),
          import("three/examples/jsm/controls/OrbitControls.js"),
        ]);
      } catch {
        if (!disposed) setGlError("3D renderer library failed to load");
        return;
      }
      if (disposed) return;
      const THREE = mods[0];
      const { OrbitControls } = mods[1];
      setLibLoaded(true);

      const { grid, nT, nD, vMin, vMax } = model;
      const vSpan = vMax - vMin || 1;
      const gain = Number.isFinite(heightGain) && heightGain > 0 ? heightGain : 1;

      // Lazy WebGL init — bail to an honest fallback if the context can't be made.
      let renderer: THREE.WebGLRenderer;
      try {
        renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
      } catch {
        if (!disposed) setGlError("WebGL is unavailable in this environment");
        return;
      }
      renderer.setPixelRatio(Math.min(typeof window !== "undefined" ? window.devicePixelRatio : 1, 2));
      renderer.setSize(width, height);
      renderer.setClearColor(0x000000, 0);
      renderer.outputColorSpace = THREE.SRGBColorSpace;
      renderer.domElement.style.display = "block";
      renderer.domElement.style.touchAction = "none";
      mount.appendChild(renderer.domElement);

      const scene = new THREE.Scene();
      const camera = new THREE.PerspectiveCamera(45, width / height, 0.1, 100);
      camera.position.set(1.9, 1.5, 2.35);
      camera.lookAt(0, 0, 0);

      const controls = new OrbitControls(camera, renderer.domElement);
      controls.target.set(0, 0, 0);
      controls.enablePan = false;
      controls.enableZoom = true;
      controls.minDistance = 1.5;
      controls.maxDistance = 7;
      controls.enableDamping = !reducedMotion;
      controls.dampingFactor = 0.08;
      controls.update();

      // Neutral white lighting so the Viridis vertex colours read true, only shaded.
      const ambient = new THREE.AmbientLight(0xffffff, 0.72);
      const key = new THREE.DirectionalLight(0xffffff, 0.9);
      key.position.set(2.2, 3.4, 1.8);
      const fill = new THREE.DirectionalLight(0xffffff, 0.25);
      fill.position.set(-2.4, 1.2, -1.6);
      scene.add(ambient, key, fill);

      const disposables: Array<{ dispose: () => void }> = [];

      const seqStops = SEQ_TOKENS.map((tok, i) => resolveTokenColor(THREE, tok, SEQ_FALLBACK[i]!));
      const dangerColor = resolveTokenColor(THREE, "--danger", "#e5484d");
      const gridColor = resolveTokenColor(THREE, "--grid-line", "rgba(255,255,255,0.16)");

      const posOf = (ti: number, di: number, v: number): { x: number; y: number; z: number; h: number } => {
        const x = (ti / (nT - 1)) * 2 * HALF_X - HALF_X;
        const z = (di / (nD - 1)) * 2 * HALF_Z - HALF_Z;
        const h = (v - vMin) / vSpan;
        const y = (h - 0.5) * HEIGHT_AMP * gain;
        return { x, y, z, h };
      };

      // Vertex buffers: position + Viridis vertex colour (height & colour both = vol).
      const positions: number[] = [];
      const colors: number[] = [];
      const scratch = new THREE.Color();
      for (let ti = 0; ti < nT; ti += 1) {
        for (let di = 0; di < nD; di += 1) {
          const v = grid[ti]![di]!;
          const p = posOf(ti, di, v);
          positions.push(p.x, p.y, p.z);
          sampleRamp(seqStops, p.h, scratch);
          colors.push(scratch.r, scratch.g, scratch.b);
        }
      }
      const indices: number[] = [];
      const vIndex = (ti: number, di: number): number => ti * nD + di;
      for (let ti = 0; ti < nT - 1; ti += 1) {
        for (let di = 0; di < nD - 1; di += 1) {
          const a = vIndex(ti, di);
          const b = vIndex(ti + 1, di);
          const c = vIndex(ti + 1, di + 1);
          const d = vIndex(ti, di + 1);
          indices.push(a, b, c, a, c, d);
        }
      }

      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
      geometry.setAttribute("color", new THREE.Float32BufferAttribute(colors, 3));
      geometry.setIndex(indices);
      geometry.computeVertexNormals();
      disposables.push(geometry);

      // The lit mesh is always present; it dims under wire/points to give the
      // overlay context (matches the mockup's layered shaded→wire→points modes).
      const meshOpacity = mode === "shaded" ? 1 : mode === "wireframe" ? 0.16 : 0.07;
      const surfaceMat = new THREE.MeshStandardMaterial({
        vertexColors: true,
        side: THREE.DoubleSide,
        roughness: 0.68,
        metalness: 0.03,
        flatShading: false,
        transparent: mode !== "shaded",
        opacity: meshOpacity,
      });
      disposables.push(surfaceMat);
      const surface = new THREE.Mesh(geometry, surfaceMat);
      scene.add(surface);

      if (mode === "wireframe") {
        const wireGeo = new THREE.WireframeGeometry(geometry);
        const wireMat = new THREE.LineBasicMaterial({ color: gridColor, transparent: true, opacity: 0.55 });
        disposables.push(wireGeo, wireMat);
        scene.add(new THREE.LineSegments(wireGeo, wireMat));
      } else if (mode === "points") {
        const pointMat = new THREE.PointsMaterial({ vertexColors: true, size: 0.07, sizeAttenuation: true });
        disposables.push(pointMat);
        scene.add(new THREE.Points(geometry, pointMat));
      }

      // Arb-flagged vertices: a danger flag (sphere + stem) at each in-range marker.
      if (markers.length > 0) {
        const flagMat = new THREE.MeshBasicMaterial({ color: dangerColor });
        const stemMat = new THREE.LineBasicMaterial({ color: dangerColor, transparent: true, opacity: 0.85 });
        disposables.push(flagMat, stemMat);
        let anyFlag = false;
        for (const m of markers) {
          if (!Number.isInteger(m.tenorIndex) || m.tenorIndex < 0 || m.tenorIndex >= nT) continue;
          if (!Number.isInteger(m.deltaIndex) || m.deltaIndex < 0 || m.deltaIndex >= nD) continue;
          anyFlag = true;
          const v = grid[m.tenorIndex]![m.deltaIndex]!;
          const p = posOf(m.tenorIndex, m.deltaIndex, v);
          const topY = p.y + 0.12;
          const sphereGeo = new THREE.SphereGeometry(0.035, 16, 16);
          disposables.push(sphereGeo);
          const flag = new THREE.Mesh(sphereGeo, flagMat);
          flag.position.set(p.x, topY, p.z);
          scene.add(flag);
          const stemGeo = new THREE.BufferGeometry().setFromPoints([
            new THREE.Vector3(p.x, p.y, p.z),
            new THREE.Vector3(p.x, topY, p.z),
          ]);
          disposables.push(stemGeo);
          scene.add(new THREE.Line(stemGeo, stemMat));
        }
        if (!anyFlag) {
          flagMat.dispose();
          stemMat.dispose();
        }
      }

      // Draw on demand; run a rAF loop only while damping needs to settle frames.
      let raf = 0;
      const renderFrame = (): void => {
        renderer.render(scene, camera);
      };
      const onControlsChange = (): void => renderFrame();
      if (reducedMotion) {
        controls.addEventListener("change", onControlsChange);
        renderFrame();
      } else {
        const animate = (): void => {
          raf = requestAnimationFrame(animate);
          controls.update();
          renderer.render(scene, camera);
        };
        animate();
      }

      cleanup = () => {
        if (raf) cancelAnimationFrame(raf);
        controls.removeEventListener("change", onControlsChange);
        controls.dispose();
        for (const d of disposables) d.dispose();
        renderer.dispose();
        renderer.forceContextLoss();
        if (renderer.domElement.parentNode === mount) mount.removeChild(renderer.domElement);
      };

      // Unmounted while the dynamic import was in flight — tear straight back down.
      if (disposed) cleanup();
    })();

    return () => {
      disposed = true;
      if (cleanup) cleanup();
    };
  }, [model, markers, mode, width, height, heightGain, reducedMotion]);

  if (!model.ok) {
    return (
      <div
        role="img"
        aria-label={`Vol surface unavailable — ${model.reason}`}
        title={model.reason}
        style={{
          width,
          height,
          display: "grid",
          placeItems: "center",
          borderRadius: "var(--r-md)",
          border: "var(--hairline)",
          background: "var(--bg-inset)",
          color: "var(--text-tertiary)",
          fontFamily: "var(--font-display)",
          fontSize: "var(--type-caption)",
          textAlign: "center",
          padding: "var(--space-5)",
        }}
      >
        <span>
          <span className="num" aria-hidden="true" style={{ fontSize: "var(--type-title)", display: "block" }}>
            —
          </span>
          vol surface unavailable · {model.reason}
        </span>
      </div>
    );
  }

  const flagged = markers.filter(
    (m) =>
      Number.isInteger(m.tenorIndex) &&
      m.tenorIndex >= 0 &&
      m.tenorIndex < model.nT &&
      Number.isInteger(m.deltaIndex) &&
      m.deltaIndex >= 0 &&
      m.deltaIndex < model.nD,
  );
  const label =
    `3D implied-volatility surface — ${model.nT} tenors × ${model.nD} deltas; ` +
    `height and Viridis colour encode implied vol from ${model.vMin.toFixed(1)} to ${model.vMax.toFixed(1)} vol points` +
    (flagged.length > 0
      ? `; ${flagged.length} arbitrage-flagged ${flagged.length === 1 ? "vertex" : "vertices"} highlighted`
      : "") +
    ". Drag to rotate, scroll to zoom.";

  return (
    <figure role="img" aria-label={label} style={{ margin: 0, position: "relative", width, height }}>
      {glError ? (
        <div
          style={{
            width,
            height,
            display: "grid",
            placeItems: "center",
            borderRadius: "var(--r-md)",
            border: "var(--hairline)",
            background: "var(--bg-inset)",
            color: "var(--text-tertiary)",
            fontFamily: "var(--font-display)",
            fontSize: "var(--type-caption)",
          }}
        >
          {glError}
        </div>
      ) : (
        <div
          ref={mountRef}
          style={{
            width,
            height,
            borderRadius: "var(--r-md)",
            border: "var(--hairline)",
            background: "var(--bg-inset)",
            overflow: "hidden",
            touchAction: "none",
            cursor: "grab",
          }}
        />
      )}

      {/* Lightweight loading state while the code-split three.js runtime resolves.
          Decorative (the figure keeps its accessible name); honours reduced motion. */}
      {!glError && !libLoaded && (
        <div
          aria-hidden="true"
          style={{
            position: "absolute",
            inset: 0,
            display: "grid",
            placeItems: "center",
            pointerEvents: "none",
            borderRadius: "var(--r-md)",
            fontFamily: "var(--font-display)",
            fontSize: "var(--type-caption)",
            letterSpacing: "0.04em",
            color: "var(--text-tertiary)",
          }}
        >
          loading 3D surface…
        </div>
      )}

      {/* Axis cues — decorative overlay (the mesh carries the meaning); pointer
          events pass through so the drag/zoom controls stay live underneath. */}
      <div
        aria-hidden="true"
        style={{
          position: "absolute",
          inset: 0,
          pointerEvents: "none",
          fontFamily: "var(--font-display)",
          fontSize: "var(--type-micro)",
          letterSpacing: "0.08em",
          textTransform: "uppercase",
          color: "var(--text-secondary)",
        }}
      >
        <span style={{ position: "absolute", left: "var(--space-3)", bottom: "var(--space-3)" }}>tenor →</span>
        <span style={{ position: "absolute", right: "var(--space-3)", bottom: "var(--space-3)" }}>← delta</span>
        <span style={{ position: "absolute", left: "var(--space-3)", top: "var(--space-3)" }}>vol ▲</span>
      </div>

      {/* Viridis legend — same `--seq-*` stops as the mesh, low → high vol. */}
      <div
        aria-hidden="true"
        style={{
          position: "absolute",
          top: "var(--space-3)",
          right: "var(--space-3)",
          display: "flex",
          gap: "var(--space-2)",
          alignItems: "stretch",
          pointerEvents: "none",
          padding: "var(--space-2)",
          borderRadius: "var(--r-sm)",
          background: "var(--bg-overlay)",
          backdropFilter: "blur(var(--blur-float))",
        }}
      >
        <div
          style={{
            width: 10,
            borderRadius: 3,
            background:
              "linear-gradient(to top, var(--seq-1), var(--seq-2), var(--seq-3), var(--seq-4), var(--seq-5), var(--seq-6))",
          }}
        />
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            justifyContent: "space-between",
            fontFamily: "var(--font-mono)",
            fontSize: "var(--type-micro)",
            color: "var(--text-tertiary)",
          }}
        >
          <span>{model.vMax.toFixed(1)}</span>
          <span>vol %</span>
          <span>{model.vMin.toFixed(1)}</span>
        </div>
      </div>

      <figcaption
        style={{
          position: "absolute",
          left: "var(--space-3)",
          bottom: "calc(var(--space-3) + 14px)",
          fontFamily: "var(--font-display)",
          fontSize: "var(--type-micro)",
          color: "var(--text-tertiary)",
          pointerEvents: "none",
        }}
      >
        drag to rotate · scroll to zoom{flagged.length > 0 ? " · ⚠ arb-flagged wing" : ""}
      </figcaption>
    </figure>
  );
}
