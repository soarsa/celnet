/**
 * Density token-axis cascade (GW0 lane A).
 *
 * Proves the third orthogonal token axis behaves exactly like `data-appearance`:
 *  1. `data-density="compact"` collapses the density-scale tokens (`--row-h`,
 *     `--cell-pad-*`, `--control-h`) below their `comfortable` defaults, purely in
 *     the CSS cascade — using the REAL `tokens.css` loaded into the document.
 *  2. The axis composes: it is orthogonal to appearance + contrast (a dark/high
 *     combination still flips density independently).
 *  3. NO component reads density in JS — the contract is "attribute + tokens move,
 *     nothing else". This is the lint-style guard the plan calls for: only
 *     `design/density.ts` (the axis owner) and `App.tsx` (the boot wiring) may
 *     reference the density attribute / hook.
 *  4. `useDensity` drives the `data-density` attribute + localStorage, mirroring
 *     `useAppearance`.
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { render } from "@testing-library/react";

import { applyDensity, DENSITY_ATTR, useDensity, type Density } from "../src/design/density";

// Load the REAL design tokens into the test document so the cascade resolves
// against the shipped CSS (jsdom resolves custom properties from author
// stylesheet attribute selectors — verified by these assertions).
const TOKENS_CSS = readFileSync(join(__dirname, "../src/design/tokens.css"), "utf8");

function installTokens(): HTMLStyleElement {
  const style = document.createElement("style");
  style.textContent = TOKENS_CSS;
  document.head.appendChild(style);
  return style;
}

function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function px(value: string): number {
  const n = Number.parseFloat(value);
  if (Number.isNaN(n)) throw new Error(`token value is not a px length: "${value}"`);
  return n;
}

// jsdom in this config has no configured origin, so `localStorage` is absent
// (the production hook already swallows that via try/catch — see density.ts). To
// exercise the persistence path the hook DOES take when storage IS present, give
// the test a standards-shaped in-memory Storage. This is an environment gap fill
// (mirroring the ResizeObserver stub in test/setup.ts), not a mock of any Celnet
// behaviour — the hook calls the real Web Storage API against it.
function installLocalStorage(): void {
  if (typeof globalThis.localStorage !== "undefined") return;
  const store = new Map<string, string>();
  const storage: Storage = {
    get length() {
      return store.size;
    },
    clear: () => store.clear(),
    getItem: (k) => (store.has(k) ? store.get(k)! : null),
    key: (i) => Array.from(store.keys())[i] ?? null,
    removeItem: (k) => {
      store.delete(k);
    },
    setItem: (k, v) => {
      store.set(k, String(v));
    },
  };
  Object.defineProperty(globalThis, "localStorage", { value: storage, configurable: true });
}

let styleEl: HTMLStyleElement | null = null;

beforeEach(() => {
  installLocalStorage();
  styleEl = installTokens();
  // Reset the orthogonal axes to a known baseline for each case.
  document.documentElement.removeAttribute(DENSITY_ATTR);
  document.documentElement.setAttribute("data-appearance", "dark");
  document.documentElement.setAttribute("data-contrast", "normal");
});

afterEach(() => {
  styleEl?.remove();
  styleEl = null;
  document.documentElement.removeAttribute(DENSITY_ATTR);
  try {
    localStorage.clear();
  } catch {
    /* ignore */
  }
});

describe("density token cascade", () => {
  it("comfortable (default) is the relaxed scale; compact collapses the px density tokens", () => {
    // The direct-px tokens — `--row-h`/`--control-h` — are the load-bearing grid
    // metrics; assert they shrink numerically in compact. (jsdom does not resolve
    // nested `var()`, so the spacing-referenced pad tokens are checked separately.)
    const comfortable = { rowH: px(token("--row-h")), controlH: px(token("--control-h")) };
    expect(comfortable.rowH).toBeGreaterThan(0);

    document.documentElement.setAttribute(DENSITY_ATTR, "compact");
    const compact = { rowH: px(token("--row-h")), controlH: px(token("--control-h")) };

    expect(compact.rowH).toBeLessThan(comfortable.rowH);
    expect(compact.controlH).toBeLessThan(comfortable.controlH);
  });

  it("compact re-points the cell-padding tokens to a tighter spacing-scale step", () => {
    // The pad tokens reference the shared spacing scale (good token hygiene), so
    // the cascade SELECTS a different `--space-N` expression in compact. Assert the
    // selected expression changes AND that the spacing step it points to is smaller
    // (resolving one level of indirection the way the browser does).
    function resolveSpace(expr: string): number {
      const m = /var\((--space-\d+)\)/.exec(expr);
      if (!m) return px(expr); // a literal px value
      return px(token(m[1]!));
    }

    document.documentElement.removeAttribute(DENSITY_ATTR);
    const comfX = token("--cell-pad-x");
    const comfY = token("--cell-pad-y");
    const comfXpx = resolveSpace(comfX);
    const comfYpx = resolveSpace(comfY);

    document.documentElement.setAttribute(DENSITY_ATTR, "compact");
    const compX = token("--cell-pad-x");
    const compY = token("--cell-pad-y");

    // The cascade picked a different expression for compact.
    expect(compX).not.toBe(comfX);
    expect(compY).not.toBe(comfY);
    // And the spacing step it resolves to is strictly tighter.
    expect(resolveSpace(compX)).toBeLessThan(comfXpx);
    expect(resolveSpace(compY)).toBeLessThan(comfYpx);
  });

  it("is orthogonal to appearance + contrast (composes with a dark/high combination)", () => {
    document.documentElement.setAttribute("data-appearance", "dark");
    document.documentElement.setAttribute("data-contrast", "high");

    document.documentElement.removeAttribute(DENSITY_ATTR);
    const comfortableRow = px(token("--row-h"));
    document.documentElement.setAttribute(DENSITY_ATTR, "compact");
    const compactRow = px(token("--row-h"));

    expect(compactRow).toBeLessThan(comfortableRow);

    // And appearance/contrast still resolve while density is set (no axis clobbers
    // another): a contrast-tuned text token is present and non-empty.
    expect(token("--text-primary")).not.toBe("");
  });

  it("light appearance keeps the same density behaviour (axes do not interfere)", () => {
    document.documentElement.setAttribute("data-appearance", "light");

    document.documentElement.removeAttribute(DENSITY_ATTR);
    const comfortableRow = px(token("--row-h"));
    document.documentElement.setAttribute(DENSITY_ATTR, "compact");
    const compactRow = px(token("--row-h"));

    expect(compactRow).toBeLessThan(comfortableRow);
  });
});

describe("useDensity (mirrors useAppearance)", () => {
  function Probe(): React.ReactElement {
    const { density, toggleDensity, setDensity } = useDensity();
    return (
      <div>
        <span data-testid="density">{density}</span>
        <button onClick={toggleDensity}>toggle</button>
        <button onClick={() => setDensity("compact")}>compact</button>
      </div>
    );
  }

  it("defaults to comfortable and applies the data-density attribute on mount", () => {
    let container: HTMLElement;
    act(() => {
      ({ container } = render(<Probe />));
    });
    expect(container!.querySelector("[data-testid=density]")?.textContent).toBe("comfortable");
    expect(document.documentElement.getAttribute(DENSITY_ATTR)).toBe("comfortable");
  });

  it("toggle flips comfortable↔compact and updates the attribute + localStorage", () => {
    let container: HTMLElement;
    act(() => {
      ({ container } = render(<Probe />));
    });
    const toggle = container!.querySelectorAll("button")[0]!;
    act(() => {
      toggle.click();
    });
    expect(container!.querySelector("[data-testid=density]")?.textContent).toBe("compact");
    expect(document.documentElement.getAttribute(DENSITY_ATTR)).toBe("compact");
    expect(localStorage.getItem("celnet.density")).toBe("compact");
  });

  it("rehydrates a persisted density from localStorage", () => {
    localStorage.setItem("celnet.density", "compact");
    let container: HTMLElement;
    act(() => {
      ({ container } = render(<Probe />));
    });
    expect(container!.querySelector("[data-testid=density]")?.textContent).toBe("compact");
    expect(document.documentElement.getAttribute(DENSITY_ATTR)).toBe("compact");
  });

  it("applyDensity sets the attribute without React", () => {
    const d: Density = "compact";
    applyDensity(d);
    expect(document.documentElement.getAttribute(DENSITY_ATTR)).toBe("compact");
  });
});

describe("density is JS-free everywhere except its owner + boot wiring", () => {
  // The plan's lint-style guard: nothing reads density in JS. Only density.ts
  // (the axis owner) and App.tsx (the one-line boot wiring) may reference the
  // density attribute or the useDensity hook. A grid hardcoding a row height
  // instead of reading `--row-h` would be a regression this catches as it lands.
  // Storybook stories (`*.stories.tsx`) are exempt: they exist to *demonstrate*
  // the density design axis (density.stories.tsx drives the toggle; tokens.stories
  // .tsx renders the live density tokens), so referencing the hook/attribute is
  // their whole point — they ship no production grid that could hardcode a height.
  function* walk(dir: string): Generator<string> {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      if (statSync(full).isDirectory()) {
        yield* walk(full);
      } else if (/\.(ts|tsx)$/.test(entry) && !/\.stories\.tsx$/.test(entry)) {
        yield full;
      }
    }
  }

  it("only design/density.ts and App.tsx reference data-density / useDensity", () => {
    const srcRoot = join(__dirname, "../src");
    const allowed = new Set([join(srcRoot, "design/density.ts"), join(srcRoot, "App.tsx")]);
    const offenders: string[] = [];
    for (const file of walk(srcRoot)) {
      const text = readFileSync(file, "utf8");
      if (/data-density|useDensity|applyDensity/.test(text) && !allowed.has(file)) {
        offenders.push(file.replace(srcRoot, "src"));
      }
    }
    expect(offenders).toEqual([]);
  });
});
