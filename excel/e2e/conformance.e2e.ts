/**
 * Excel REAL-EDGE conformance suite — the WIRE-conformance gate the in-process
 * FakeSocket unit tests (`test/connection.test.ts`) cannot prove.
 *
 * For every FROZEN golden vector (`crates/celnet-golden/vectors/*.json`) whose
 * product family the Excel `CELNET.*` worksheet functions expose, this:
 *   1. builds the EXACT `Instrument` the add-in's production polymorphic spec
 *      produces (the same `CELNET.INSTRUMENT` shaping + token codec a
 *      `CELNET.PRICE(token)` cell runs, token round-trip included);
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
 * Every Excel-exposed family is asserted reachable, including the three
 * cross-asset vanilla arms (now WS-priced — the server WS decoder routes by the
 * authoritative `underlying` oneof rather than the legacy FX `pair` projection;
 * see `corpus.ts`). The not-exposed set is empty and asserted so, not silently
 * skipped. No assertion is lowered.
 */
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { Connection } from "../src/transport/connection";
import {
  DEFAULT_CONVENTIONS,
  formatRfqPanelSpill,
  shapeVanillaInstrument,
} from "../src/functions/shaping";
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
  // Authenticate as the always-seeded admin (`AuthService.Login`) for a
  // capability-complete session token, and install it on the connection (which
  // re-authenticates the open stream). The demo edge runs under the PRODUCTION
  // `Enforce` posture (demoEdge.ts), where the click-to-trade `accept_quote` is
  // gated on `Execute·FxOptions` — a capability resolved ONLY from an authenticated
  // session, never a body-asserted principal (finding #3). The read-side `price`
  // and `request_multi_dealer_quote` are admitted by the grant-all principal alone,
  // but booking a panel line needs the real session. This mirrors the bench/SDK fix
  // (the Rust wire-load logs in as the seed admin) and the GUI's login flow.
  await conn.login("admin@celnet.com", "password");
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

  // ---- multi-dealer ranked panel (RFQ-to-many) over the REAL ≥3-LP edge -----
  //
  // The demo edge boots with `CELNET_DEMO_LPS=3` (pinned in demoEdge.ts): the
  // native maker + 3 labeled DETERMINISTIC SYNTHETIC dealers quoting around the
  // SAME edge mid (live LP connectivity is environment-provided, never claimed
  // here). The spec drives the exact path a `=CELNET.RFQ(…, TRUE)` cell runs —
  // `request_multi_dealer_quote` → ranked `multi_dealer_quote` → spill — then
  // books a chosen line via `accept_quote` carrying `(quote_id, lp_id)`.
  describe("multi-dealer ranked panel (real ≥3-LP demo edge)", () => {
    const PANEL_INSTRUMENT = shapeVanillaInstrument({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: 1.12,
      callPut: "C",
      notional: 1_000_000,
    });

    it("returns a 4-line ranked panel (native + 3 synthetic LPs) with coherent touch winners", async () => {
      const md = await conn.requestMultiDealerQuote(
        PANEL_INSTRUMENT,
        DEFAULT_CONVENTIONS,
        `e2e-panel:${Date.now()}`,
      );
      // Exactly the pinned breadth: the native maker line + SYNTH-LP-1..3.
      expect(md.dealers.length).toBe(4);
      const ids = md.dealers.map((d) => d.lpId);
      expect(new Set(ids).size).toBe(4);
      for (const k of [1, 2, 3]) expect(ids).toContain(`SYNTH-LP-${k}`);
      const native = md.dealers.filter((d) => !d.lpId.startsWith("SYNTH-LP-"));
      expect(native.length).toBe(1);
      // Only the native maker discloses greeks; a synthetic LP discloses a price.
      expect(native[0]!.greeks).toBeDefined();
      for (const d of md.dealers.filter((x) => x.lpId.startsWith("SYNTH-LP-"))) {
        expect(d.greeks).toBeUndefined();
      }
      // Every dealer line is a coherent two-way with a live last-look window.
      for (const d of md.dealers) {
        expect(d.price.bid).toBeGreaterThan(0);
        expect(d.price.offer).toBeGreaterThanOrEqual(d.price.bid);
        expect(d.validUntilNanos > md.epochNanos).toBe(true);
      }
      // The aggregator's touch winners are real panel rows holding the touch.
      const byId = new Map(md.dealers.map((d) => [d.lpId, d]));
      const bestBid = byId.get(md.bestBidLpId);
      const bestOffer = byId.get(md.bestOfferLpId);
      expect(bestBid, `best_bid_lp_id ${md.bestBidLpId} not on the panel`).toBeDefined();
      expect(bestOffer, `best_offer_lp_id ${md.bestOfferLpId} not on the panel`).toBeDefined();
      expect(bestBid!.price.bid).toBe(Math.max(...md.dealers.map((d) => d.price.bid)));
      expect(bestOffer!.price.offer).toBe(Math.min(...md.dealers.map((d) => d.price.offer)));

      // The spill a `=CELNET.RFQ(…, TRUE)` cell renders preserves the server's
      // ranking order row-for-row and exposes the (quote_id, lp_id) accept key.
      const spill = formatRfqPanelSpill({
        quoteId: md.quoteId,
        lines: md.dealers.map((d) => ({
          lpId: d.lpId,
          bid: d.price.bid,
          offer: d.price.offer,
          validUntilNanos: d.validUntilNanos,
        })),
        bestBidLpId: md.bestBidLpId,
        bestOfferLpId: md.bestOfferLpId,
        conventions: md.conventions,
        surfaceVersion: md.surfaceVersion,
        epochNanos: md.epochNanos,
      });
      expect(spill.length).toBe(1 + 4 + 2); // header + 4 LP rows + quote_id + footer
      expect(spill.slice(1, 5).map((r) => r[0])).toEqual(ids);
      // The quote_id row carries its label + id; `rectangular()` then pads every
      // row to the table width (an Excel dynamic-array spill must be rectangular),
      // so the row trails empties out to the header width.
      const width = spill[0].length;
      expect(spill[5]).toEqual(["quote_id", md.quoteId.toString(), ...Array(width - 2).fill("")]);
    });

    it("books a chosen panel line by (quote_id, lp_id) at exactly the shown price", async () => {
      const key = `e2e-panel-accept:${Date.now()}`;
      const md = await conn.requestMultiDealerQuote(PANEL_INSTRUMENT, DEFAULT_CONVENTIONS, key);
      // Lift the best OFFER dealer's line (BUY) — the pinned panel row books at
      // the price the client was shown, never a re-price.
      const chosen = md.dealers.find((d) => d.lpId === md.bestOfferLpId)!;
      const exec = await conn.acceptQuote({
        quoteId: md.quoteId,
        side: "BUY",
        idempotencyKey: md.idempotencyKey,
        lpId: chosen.lpId,
      });
      expect(exec.quoteId).toBe(md.quoteId);
      expect(exec.side).toBe("BUY");
      expect(exec.tradedPremium).toBe(chosen.price.offer);

      // A different dealer line on the SAME quote is a different trade intent,
      // not a retry — the server refuses it rather than double-booking.
      const other = md.dealers.find((d) => d.lpId !== chosen.lpId)!;
      await expect(
        conn.acceptQuote({
          quoteId: md.quoteId,
          side: "BUY",
          idempotencyKey: md.idempotencyKey,
          lpId: other.lpId,
        }),
      ).rejects.toThrow(/already booked/);
    });
  });

  it("exposes every corpus family — no family is excluded from the Excel WS path", () => {
    // Documentation-as-assertion (CLAUDE.md rule 2 — no silent gap). Every corpus
    // family is now WS-priced above, including the three cross-asset vanilla arms
    // (`equity_option` / `commodity_option` / `crypto_option`): the polymorphic
    // `CELNET.INSTRUMENT` underlier grammar shapes each onto the wire (the
    // `Underlying` oneof + `settlement_style` beside the legacy FX `pair`
    // projection), and the server WS decoder now routes by the authoritative
    // `underlying` rather than `pair` — proven end-to-end by
    // `crates/celnet-server/tests/cross_asset_ws.rs`
    // (`ws_underlying_precedence_routes_client_shaped_frames_to_the_cross_asset_arm`).
    // The not-exposed set is therefore empty; if a future family is genuinely not
    // WS-priceable it lands there with a TRUE reason (never the routing bug), and
    // this pins that the gap cannot silently reappear.
    expect(FAMILIES_NOT_EXPOSED).toEqual([]);
  });
});
