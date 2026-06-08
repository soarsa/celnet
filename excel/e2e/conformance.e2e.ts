/**
 * Excel REAL-EDGE conformance suite — the WIRE-conformance gate the in-process
 * FakeSocket unit tests (`test/connection.test.ts`) cannot prove.
 *
 * For every FROZEN golden vector (`crates/celnet-golden/vectors/*.json`) whose
 * product family the Excel `CELNET.*` worksheet functions expose, this:
 *   1. builds the EXACT `Instrument` the add-in's production `shape*` function
 *      produces (the same code `CELNET.PRICE` / `CELNET.BARRIER` / … run);
 *   2. prices it through the REAL add-in `Connection` (the same transport + wire
 *      codec the worksheet functions use) over a REAL WebSocket to a REAL booted
 *      `celnet-server` demo edge — NO FakeSocket, NO mock — against the vector's
 *      OWN market context;
 *   3. asserts the server-returned price equals the vector's independent oracle
 *      within the vector's FROZEN tolerance: `(rel, abs)` for closed-form families,
 *      and the `k·(oracle_se + server_se)` band (`k = 4`) for the Monte-Carlo
 *      families — bit-for-bit the same gate as the Rust SDK conformance
 *      (`celnet-client/tests/conformance.rs`), so "server == Excel == oracle" is a
 *      wire-level gate, not a documentation claim.
 *
 * Every Excel-exposed family is asserted reachable; the one corpus family Excel does
 * not expose (`strategy`) is reported, not silently skipped. No assertion is lowered.
 */
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { Connection } from "../src/transport/connection";
import { DEFAULT_CONVENTIONS } from "../src/functions/shaping";
import {
  EXCEL_FAMILIES,
  FAMILIES_NOT_EXPOSED,
  type GoldenVector,
  instrumentOf,
  loadVectors,
  marketOf,
} from "./corpus";
import { nodeWebSocketFactory } from "./nodeSocket";
import { readWsUrl } from "./wsUrl";

/** The Monte-Carlo standard-error multiplier — identical to the Rust SDK gate. */
const K_STDERR = 4.0;
/** Per-call deadline covering the heaviest MC family on the server. */
const PRICE_TIMEOUT_MS = 90_000;

const exposed = new Set<string>(EXCEL_FAMILIES);

let conn: Connection;

beforeAll(async () => {
  const url = readWsUrl();
  conn = new Connection({
    url,
    factory: nodeWebSocketFactory(),
    requestTimeoutMs: PRICE_TIMEOUT_MS,
    // A long staleness window: this suite issues request/reply pricing calls, not
    // streaming subscriptions, so the staleness monitor is irrelevant here.
    stalenessWindowMs: 10 * 60_000,
  });
  // Wait for the real socket to reach OPEN before pricing (bounded).
  const deadline = Date.now() + 30_000;
  while (!conn.isOpen()) {
    if (Date.now() > deadline) throw new Error(`e2e: socket to ${url} never opened`);
    await new Promise((r) => setTimeout(r, 25));
  }
});

afterAll(() => {
  conn?.close();
});

/** The frozen corpus, restricted to the families Excel exposes. */
const vectors: GoldenVector[] = loadVectors().filter((v) => exposed.has(v.family));

/** Group vectors by family so each family is its own `describe` block + reachability check. */
const byFamily = new Map<string, GoldenVector[]>();
for (const v of vectors) {
  const list = byFamily.get(v.family) ?? [];
  list.push(v);
  byFamily.set(v.family, list);
}

/**
 * Assert one server-priced result against a vector — the SDK conformance gate,
 * mirrored. `window_barrier` (LSV-only, no closed form) gates STRUCTURAL invariants
 * plus the documented wide flat-GBM model band; MC families use `k·stderr`; the rest
 * use the frozen `(rel, abs)`.
 */
function assertConforms(v: GoldenVector, got: number, serverStdErr: number | undefined): void {
  const want = v.expected.price;

  if (v.family === "window_barrier") {
    const vanilla = Number(v.terms["unbarriered_vanilla"]);
    expect(Number.isFinite(got), `window_barrier ${v.id} price finite`).toBe(true);
    expect(got, `window_barrier ${v.id} price non-negative`).toBeGreaterThanOrEqual(-1e-9);
    expect(got, `window_barrier ${v.id} ≤ unbarriered vanilla`).toBeLessThanOrEqual(vanilla + 1e-9);
    const scale = Math.max(Math.abs(got), Math.abs(want));
    const band = v.tolerance.abs + v.tolerance.rel * scale;
    expect(
      Math.abs(got - want),
      `window_barrier ${v.id}: LSV price ${got} outside wide flat-GBM band of ${want} (band ${band})`,
    ).toBeLessThanOrEqual(band);
    return;
  }

  const oracleSe = v.expected.price_std_error;
  if (oracleSe !== null && oracleSe !== undefined) {
    const serverSe = serverStdErr ?? 0;
    const band = K_STDERR * Math.max(oracleSe + serverSe, 1e-12);
    const diff = Math.abs(got - want);
    expect(
      diff,
      `MC vector ${v.id}: Excel price ${got} vs oracle ${want} |Δ|=${diff} > band ${band} ` +
        `(oracle_se=${oracleSe}, server_se=${serverSe})`,
    ).toBeLessThanOrEqual(band);
  } else {
    const scale = Math.max(Math.abs(got), Math.abs(want));
    const tol = v.tolerance.abs + v.tolerance.rel * scale;
    expect(
      Math.abs(got - want),
      `vector ${v.id}: Excel price ${got} vs oracle ${want} (rel ${v.tolerance.rel}, abs ${v.tolerance.abs})`,
    ).toBeLessThanOrEqual(tol);

    // Greeks where the oracle provides them (vanilla) — same relative gate as the SDK.
    const greeks = v.expected.greeks ?? {};
    // greeksFromWire keys: deltaSpot/gamma/vega/theta/rhoDom/rhoFor (camelCase).
    const map: Record<string, keyof typeof lastGreeks> = {
      delta_spot: "deltaSpot",
      gamma: "gamma",
      vega: "vega",
      theta: "theta",
      rho_dom: "rhoDom",
      rho_for: "rhoFor",
    };
    for (const [name, expected] of Object.entries(greeks)) {
      const key = map[name];
      if (!key) continue;
      const g = lastGreeks[key];
      const gscale = Math.max(Math.abs(g), Math.abs(expected));
      const ok = Math.abs(g - expected) <= 1e-7 + v.tolerance.rel * Math.max(gscale, 1);
      expect(ok, `vector ${v.id} greek ${name}: Excel ${g} vs oracle ${expected}`).toBe(true);
    }
  }
}

// The most-recent priced Greeks, captured so `assertConforms` can validate the
// vanilla Greek set the oracle quotes. Scoped per `it` (sequential within a family).
let lastGreeks: {
  deltaSpot: number;
  gamma: number;
  vega: number;
  theta: number;
  rhoDom: number;
  rhoFor: number;
};

describe("Excel real-edge conformance (frozen golden corpus over a REAL WebSocket)", () => {
  it("the corpus has vectors for every Excel-exposed family", () => {
    for (const fam of EXCEL_FAMILIES) {
      expect(byFamily.get(fam)?.length ?? 0, `no corpus vectors for Excel family \`${fam}\``).toBeGreaterThan(0);
    }
  });

  for (const fam of EXCEL_FAMILIES) {
    const fvectors = byFamily.get(fam) ?? [];
    describe(`family: ${fam} (${fvectors.length} vectors)`, () => {
      for (const v of fvectors) {
        it(`${v.id}`, async () => {
          const instrument = instrumentOf(v);
          const market = marketOf(v);
          const priced = await conn.price(instrument, market, DEFAULT_CONVENTIONS);
          lastGreeks = {
            deltaSpot: priced.greeks.deltaSpot,
            gamma: priced.greeks.gamma,
            vega: priced.greeks.vega,
            theta: priced.greeks.theta,
            rhoDom: priced.greeks.rhoDom,
            rhoFor: priced.greeks.rhoFor,
          };
          assertConforms(v, priced.greeks.price, priced.priceStdError);
        }, PRICE_TIMEOUT_MS + 10_000);
      }
    });
  }

  it("reports (does not skip) the corpus families Excel does not expose", () => {
    // Documentation-as-assertion: `strategy` is the only corpus family without a
    // `CELNET.*` worksheet function. If a future family becomes Excel-exposed, this
    // pins the honest gap so it cannot silently drift.
    expect(FAMILIES_NOT_EXPOSED).toEqual(["strategy"]);
  });
});
