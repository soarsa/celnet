/**
 * saved-views codec + localStorage (GW1-S4).
 *
 * Oracles:
 *   • ROUND-TRIP IDENTITY — `decode(encode(s))` deep-equals `s` over a generated
 *     state space (the codec's structural contract).
 *   • FROZEN FIXTURES — ≥3 hand-pinned LITERAL URL ⇄ state pairs (external truth,
 *     like a golden vector): a SYMMETRIC encoder/decoder bug that round-trips would
 *     still be caught here because the frozen string is asserted both ways.
 *   • FORWARD-COMPAT — unknown params ignored, missing params default, no throw.
 *   • PERSISTENCE — the named set survives a store→load via the SAME canonical
 *     codec the URL uses (storage mirror == URL form).
 */

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  decodeView,
  encodeViewString,
  loadSavedViews,
  storeSavedViews,
  type SavedView,
  type ViewState,
} from "../src/lib/savedViews";
import { FIRM_SCOPE_ROOT } from "../src/lib/scope";

// jsdom here has no origin, so localStorage is absent; install a standards-shaped
// in-memory Storage (an environment gap fill, like the density test — not a mock
// of any Celnet behaviour; the codec calls the real Web Storage API against it).
function installLocalStorage(): void {
  const store = new Map<string, string>();
  const storage: Storage = {
    get length() {
      return store.size;
    },
    clear: () => store.clear(),
    getItem: (k) => (store.has(k) ? store.get(k)! : null),
    key: (i) => [...store.keys()][i] ?? null,
    removeItem: (k) => store.delete(k),
    setItem: (k, v) => store.set(k, String(v)),
  };
  Object.defineProperty(globalThis, "localStorage", { value: storage, configurable: true });
}

beforeEach(() => {
  installLocalStorage();
});
afterEach(() => {
  localStorage.clear();
});

/** A generated spread of view states (the round-trip space). */
const STATES: ViewState[] = [
  { workspace: "stream", scope: { path: [FIRM_SCOPE_ROOT], groupBy: "none" }, analytics: {} },
  {
    workspace: "risk",
    scope: {
      path: [FIRM_SCOPE_ROOT, { level: "desk", label: "EM Vol" }],
      groupBy: "book",
    },
    analytics: { measures: "var-es" },
  },
  {
    workspace: "surface",
    scope: {
      path: [
        FIRM_SCOPE_ROOT,
        { level: "desk", label: "G10 Vol" },
        { level: "book", label: "EUR Vol" },
        { level: "pair", label: "EUR/USD" },
      ],
      groupBy: "pair",
    },
    analytics: { model: "EXTENDED_SURFACE", trend: "1m", axes: "tenor-delta" },
  },
  {
    workspace: "book",
    scope: { path: [FIRM_SCOPE_ROOT, { level: "desk", label: "FX/Rates" }], groupBy: "none" },
    analytics: { model: "MARKET_HEDGE" },
  },
];

describe("round-trip identity", () => {
  it("decode(encode(s)) deep-equals s for every generated state", () => {
    for (const s of STATES) {
      expect(decodeView(encodeViewString(s))).toEqual(s);
    }
  });

  it("encoding is stable (equal states → byte-identical query string)", () => {
    const s = STATES[2]!;
    expect(encodeViewString(s)).toBe(encodeViewString(structuredClone(s)));
  });
});

describe("frozen literal URL ⇄ state fixtures (external truth)", () => {
  // Each fixture is a hand-written canonical query string and its expected state.
  // Asserted BOTH ways: decode(url)==state AND encode(state)==url — a symmetric
  // codec bug can't pass because the frozen string is the external reference.
  const FIXTURES: { url: string; state: ViewState }[] = [
    {
      url: "view=stream",
      state: { workspace: "stream", scope: { path: [FIRM_SCOPE_ROOT], groupBy: "none" }, analytics: {} },
    },
    {
      url: "view=book&scope=desk%3AEM+Vol&group=book",
      state: {
        workspace: "book",
        scope: { path: [FIRM_SCOPE_ROOT, { level: "desk", label: "EM Vol" }], groupBy: "book" },
        analytics: {},
      },
    },
    {
      url: "view=surface&scope=desk%3AG10+Vol%3Ebook%3AEUR+Vol%3Epair%3AEUR%2FUSD&model=EXTENDED_SURFACE&trend=1m",
      state: {
        workspace: "surface",
        scope: {
          path: [
            FIRM_SCOPE_ROOT,
            { level: "desk", label: "G10 Vol" },
            { level: "book", label: "EUR Vol" },
            { level: "pair", label: "EUR/USD" },
          ],
          groupBy: "none",
        },
        analytics: { model: "EXTENDED_SURFACE", trend: "1m" },
      },
    },
  ];

  it("decodes each frozen URL to its expected state", () => {
    for (const f of FIXTURES) {
      expect(decodeView(f.url)).toEqual(f.state);
    }
  });

  it("encodes each expected state to its frozen URL", () => {
    for (const f of FIXTURES) {
      expect(encodeViewString(f.state)).toBe(f.url);
    }
  });
});

describe("forward-compatibility (never throws; defaults applied)", () => {
  it("ignores unknown params and keeps the known ones", () => {
    const s = decodeView("view=risk&group=book&future_axis=xyz&zz=1");
    expect(s.workspace).toBe("risk");
    expect(s.scope.groupBy).toBe("book");
  });

  it("defaults a missing/invalid workspace to stream and a bad group-by to none", () => {
    expect(decodeView("").workspace).toBe("stream");
    expect(decodeView("view=bogus").workspace).toBe("stream");
    expect(decodeView("group=bogus").scope.groupBy).toBe("none");
  });

  it("an empty string decodes to the bare firm-root stream view", () => {
    expect(decodeView("")).toEqual({
      workspace: "stream",
      scope: { path: [FIRM_SCOPE_ROOT], groupBy: "none" },
      analytics: {},
    });
  });
});

describe("named-set persistence (localStorage mirror == URL canonical form)", () => {
  it("store then load round-trips the named views", () => {
    const views: SavedView[] = [
      { id: "v1", name: "EM book", state: STATES[1]! },
      { id: "v2", name: "EUR surface", state: STATES[2]! },
    ];
    storeSavedViews(views);
    expect(loadSavedViews()).toEqual(views);
  });

  it("drops a structurally-invalid stored entry (forward-compat at the store)", () => {
    localStorage.setItem(
      "celnet.savedViews",
      JSON.stringify([{ id: "ok", name: "fine", q: "view=risk" }, { id: 42 }, "garbage"]),
    );
    const loaded = loadSavedViews();
    expect(loaded).toHaveLength(1);
    expect(loaded[0]!.name).toBe("fine");
    expect(loaded[0]!.state.workspace).toBe("risk");
  });

  it("a corrupt store yields an empty set (never throws)", () => {
    localStorage.setItem("celnet.savedViews", "{not json");
    expect(loadSavedViews()).toEqual([]);
  });
});
