/**
 * Per-family REAL-EDGE conformance for the trader GUI — the wire-level gate the
 * GUI-self round-trips (`gui/test/products/*.test.ts`, `gui/test/conformance.test.ts`)
 * cannot prove. Two halves, one REAL booted `celnet-server` demo edge (the
 * Playwright globalSetup edge — no mock):
 *
 *  1. BROWSER half — gallery → ticket → live RFQ, per registered family: select
 *     each product family in the real StructureGallery (its registry
 *     `ProductSpec` defaults, exactly as a trader structures it), Request quote
 *     against the live edge, and assert a SERVER-priced result renders (the
 *     Greeks strip's delta cell, or the fair-strike panel for the variance/vol
 *     swaps). This proves every family the gallery offers actually books over
 *     the wire and prices end-to-end — previously proven only for the default
 *     vanilla/RR ticket in `workflows.e2e.ts`.
 *
 *  2. WIRE half — frozen golden corpus over the GUI's production codec: for
 *     every vector of every WS-priced family, build the EXACT `Instrument` the
 *     GUI books (`goldenCorpus.ts`), price it via the production
 *     `instrumentToWire`/`marketToWire`/`conventionsToWire` encoding over a REAL
 *     WebSocket (`edgeClient.ts`) against the vector's OWN market context, and
 *     assert the server price equals the vector's independent oracle within the
 *     vector's FROZEN tolerance: `(rel, abs)` for closed forms, the
 *     `k·(oracle_se + server_se)` band (`k = 4`) for the Monte-Carlo families,
 *     and the structural + wide flat-GBM band for the LSV-only window barrier —
 *     bit-for-bit the same gate as the Rust SDK (`celnet-client/tests/
 *     conformance.rs`) and Excel (`excel/e2e/conformance.e2e.ts`), so
 *     "server == GUI == oracle" is a wire-level fact for the GUI too.
 *
 * Honesty (CLAUDE.md rules 2 & 5): no assertion is lowered; every skipped
 *  family/flow carries a concrete, ASSERTED reason:
 *  - browser half: the NDF cannot be live-quoted from the ticket today — the
 *    seeded watched-pair set (`src/data/seed.ts` PAIRS) carries only deliverable
 *    majors and the server's validity matrix refuses an NDF on a deliverable
 *    pair. That reason is PROVEN below (the edge refuses an EURUSD NDF with the
 *    typed message), and the NDF's numerical conformance runs in the wire half
 *    on its real USDBRL/USDCOP/USDINR underlyings.
 *  - wire half: the three cross-asset vanilla arms are not priceable over the
 *    FX-two-rate `MarketContext` (their vectors pin the generalized carry);
 *    asserted as an exact set, mirroring `excel/e2e/corpus.ts`.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";

import { EdgeClient, EdgeRefusal, priceOnEdge } from "./edgeClient";
import {
  CONFORMANCE_CONVENTIONS,
  FAMILIES_NOT_EXPOSED_ON_FX_WS,
  WS_PRICED_FAMILIES,
  instrumentOfVector,
  loadCorpus,
  marketOf,
  type GoldenVector,
} from "./goldenCorpus";
import { gotoWorkspace, openLive } from "./helpers";
import { readWsUrl } from "./wsUrl";

// ---------------------------------------------------------------------------
// the GUI-bookable structure catalogue (pinned to the live gallery by a law test)
// ---------------------------------------------------------------------------

/** What a successful live quote renders for a family on the ticket face. */
type PricedMarker = "greeks" | "fair-variance" | "fair-volatility";

/**
 * Every registered `ProductSpec` (id + trader-facing gallery label, in catalogue
 * order) with the corpus families it books and the priced-render marker. The e2e
 * tsconfig type-checks under the node lib, so the `.tsx` registry cannot be
 * imported here; instead this table IS asserted against the LIVE gallery (count
 * + every label, uniquely) — a registry edit that misses this suite fails
 * loudly, exactly like the unit conformance's "every family accounted for" law.
 */
const GUI_STRUCTURES: readonly {
  id: string;
  label: string;
  corpusFamilies: readonly string[];
  marker: PricedMarker;
  /**
   * The edge prices this family's registry-DEFAULT ticket by Monte-Carlo/LSM
   * (200k-pair MC strips for TARF/accumulator; LSM + FD-grid Greeks for the
   * American) — its rendered-quote wait is the MC hang-detector deadline, not
   * the closed-form one.
   */
  mcPriced?: true;
}[] = [
  { id: "VANILLA", label: "Vanilla", corpusFamilies: ["vanilla"], marker: "greeks" },
  { id: "RISK_REVERSAL", label: "Risk Reversal", corpusFamilies: ["strategy"], marker: "greeks" },
  { id: "STRANGLE", label: "Strangle", corpusFamilies: ["strategy"], marker: "greeks" },
  { id: "STRADDLE", label: "Straddle", corpusFamilies: ["strategy"], marker: "greeks" },
  { id: "SEAGULL", label: "Seagull", corpusFamilies: ["strategy"], marker: "greeks" },
  {
    id: "LISTED_FUTURE_OPTION",
    label: "Future option (listed)",
    corpusFamilies: ["listed_future_option"],
    marker: "greeks",
  },
  { id: "FX_FORWARD", label: "Forward (outright)", corpusFamilies: ["fx_forward"], marker: "greeks" },
  { id: "FX_SWAP", label: "Swap (near/far)", corpusFamilies: ["fx_swap"], marker: "greeks" },
  { id: "NDF", label: "NDF (non-deliverable)", corpusFamilies: ["ndf"], marker: "greeks" },
  { id: "SINGLE_BARRIER", label: "Single Barrier", corpusFamilies: ["single_barrier"], marker: "greeks" },
  { id: "DOUBLE_BARRIER", label: "Double Barrier", corpusFamilies: ["double_barrier"], marker: "greeks" },
  { id: "DIGITAL", label: "Digital", corpusFamilies: ["digital"], marker: "greeks" },
  { id: "TOUCH", label: "Touch", corpusFamilies: ["touch"], marker: "greeks" },
  {
    id: "VARIANCE_SWAP",
    label: "Variance Swap",
    corpusFamilies: ["variance_swap"],
    marker: "fair-variance",
  },
  {
    id: "VOLATILITY_SWAP",
    label: "Volatility Swap",
    corpusFamilies: ["volatility_swap"],
    marker: "fair-volatility",
  },
  { id: "ASIAN", label: "Asian (average-rate)", corpusFamilies: ["asian_option"], marker: "greeks" },
  { id: "FORWARD_START", label: "Forward Start", corpusFamilies: ["forward_start"], marker: "greeks" },
  { id: "CLIQUET", label: "Cliquet", corpusFamilies: ["cliquet"], marker: "greeks" },
  { id: "QUANTO", label: "Quanto", corpusFamilies: ["quanto"], marker: "greeks" },
  { id: "TARF", label: "TARF", corpusFamilies: ["tarf"], marker: "greeks", mcPriced: true },
  {
    id: "ACCUMULATOR",
    label: "Accumulator",
    corpusFamilies: ["accumulator"],
    marker: "greeks",
    mcPriced: true,
  },
  { id: "LOOKBACK", label: "Lookback", corpusFamilies: ["lookback"], marker: "greeks" },
  { id: "WINDOW_BARRIER", label: "Window Barrier", corpusFamilies: ["window_barrier"], marker: "greeks" },
  {
    id: "AMERICAN",
    label: "American / Bermudan",
    corpusFamilies: ["american"],
    marker: "greeks",
    mcPriced: true,
  },
  { id: "PERPETUAL", label: "Perpetual (no expiry)", corpusFamilies: ["perpetual_option"], marker: "greeks" },
  {
    id: "BASKET",
    label: "Basket / Best-of / Worst-of",
    corpusFamilies: ["basket"],
    marker: "greeks",
  },
  {
    id: "CROSS_ASSET_VANILLA",
    label: "Cross-asset vanilla",
    corpusFamilies: ["equity_option", "commodity_option", "crypto_option"],
    marker: "greeks",
  },
];

/**
 * The structures the BROWSER half does not live-quote, each with a concrete
 * reason proven by an assertion below (never assumed):
 *  - NDF: the server's linear validity matrix refuses an NDF on a deliverable
 *    pair, and the GUI's seeded watched-pair set carries only deliverable majors
 *    — so a defaults ticket quote would be a server refusal, not a price. The
 *    refusal is asserted on the live edge (the typed `deliverable` message), and
 *    the family's numerical conformance runs in the wire half on its real
 *    non-deliverable underlyings.
 */
const BROWSER_DECLARED_SKIPS: Readonly<Record<string, string>> = {
  NDF: "server refuses an NDF on a deliverable pair; the seeded watched pairs are all deliverable majors",
};

/** A rendered-quote wait generous enough for the LSV window barrier on a warm edge. */
const QUOTE_RENDER_TIMEOUT_MS = 30_000;

/**
 * The rendered-quote wait for the `mcPriced` default tickets (TARF, accumulator,
 * American). Like `PRICE_DEADLINE` in the Rust SDK gate
 * (`celnet-client/tests/conformance.rs`), this is a HANG detector, not a latency
 * gate — latency budgets are gated in `celnet-bench` on a quiet machine. The e2e
 * edge shares one box with the dev server, the browser and any parallel gates,
 * and the heaviest MC/LSM defaults (200k antithetic pairs + bump/FD Greeks; the
 * wire half clocked a single contended American price at ~33s) legitimately
 * exceed the closed-form wait there. 120s still fails fast on a genuine hang
 * (stalled stream, deadlock) while never failing a merely-contended box.
 */
const MC_QUOTE_RENDER_TIMEOUT_MS = 120_000;

/** The Monte-Carlo standard-error multiplier — identical to the Rust SDK / Excel gates. */
const K_STDERR = 4.0;

// The frozen corpus, loaded once at module scope (test generation is static).
const CORPUS = loadCorpus();

// ---------------------------------------------------------------------------
// browser half — gallery → ticket → live RFQ, per registered family
// ---------------------------------------------------------------------------

test.describe("gallery → ticket → live RFQ: every registered family quotes on the real edge", () => {
  // ONE warm page + live WS session for the whole catalogue sweep (workers = 1,
  // sequential within the file) — the bounded-runtime warm-edge pattern.
  let page: Page;
  let pane: Locator;

  test.beforeAll(async ({ browser }) => {
    page = await browser.newPage();
    await openLive(page);
    pane = await gotoWorkspace(page, "ticket");
  });

  test.afterAll(async () => {
    await page?.close();
  });

  /** The gallery card (role=option) whose label span is exactly `label`. */
  function galleryCard(label: string): Locator {
    return pane.getByRole("option").filter({ has: page.getByText(label, { exact: true }) });
  }

  test("the live gallery catalogue is exactly the declared structure table", async () => {
    // Count first: a registry entry ADDED without extending this suite (or
    // removed without pruning it) fails here — the same accounted-for law the
    // unit conformance applies to corpus families.
    await expect(pane.getByRole("option")).toHaveCount(GUI_STRUCTURES.length);
    for (const s of GUI_STRUCTURES) {
      await expect(galleryCard(s.label), `gallery offers exactly one "${s.label}" card`).toHaveCount(1);
    }
  });

  test("declared browser-half skips are exactly the NDF (reason proven in the wire half)", () => {
    expect(Object.keys(BROWSER_DECLARED_SKIPS)).toEqual(["NDF"]);
    // The skipped family is still numerically gated: the corpus carries NDF
    // vectors and the wire half prices them on their real non-deliverable pairs.
    expect(CORPUS.get("ndf")?.length ?? 0).toBeGreaterThan(0);
  });

  for (const s of GUI_STRUCTURES) {
    if (s.id in BROWSER_DECLARED_SKIPS) continue;
    test(`${s.id}: structure via gallery defaults → live server quote renders`, async () => {
      // MC/LSM-priced defaults get the hang-detector deadline (rationale on the
      // constant); the test budget tracks it with interaction headroom.
      const renderTimeoutMs = s.mcPriced ? MC_QUOTE_RENDER_TIMEOUT_MS : QUOTE_RENDER_TIMEOUT_MS;
      if (s.mcPriced) test.setTimeout(MC_QUOTE_RENDER_TIMEOUT_MS + 30_000);

      // Select the family card; the shell clears any prior priced state on a
      // structure change, so the previous family's render cannot satisfy this
      // family's assertion (asserted before requesting). The swap heads are
      // matched EXACTLY: the swap tickets' own field hint legitimately CONTAINS
      // the phrase ("Fair variance strike K_var = strike_vol². Leave 0 …"),
      // while the priced `SwapResult` head span is exactly the phrase.
      const card = galleryCard(s.label);
      await card.click();
      await expect(card).toHaveAttribute("aria-selected", "true");
      await expect(pane.getByTitle("delta (spot)")).toHaveCount(0);
      await expect(pane.getByText("Fair variance strike", { exact: true })).toHaveCount(0);
      await expect(pane.getByText("Fair volatility strike", { exact: true })).toHaveCount(0);

      // RFQ the registry-default structure against the live edge.
      await pane.getByRole("button", { name: /Request quote/ }).click();

      // A REAL server-priced result renders: the Greeks strip's live delta cell
      // (it mounts only once a quote lands), or the priced fair strike for the
      // variance/volatility swaps (quoted as a fair strike, not a premium).
      if (s.marker === "fair-variance") {
        // The TRUE variance-swap render (`TicketWorkspace` `SwapResult`): the
        // exact head, the server-resolved fair variance strike K_var to six
        // decimals (non-zero — a 0.000000 means the fair strike never resolved),
        // and its √K_var vol-terms sub-line. The fair-strike panel REPLACES the
        // premium two-way: no premium-unit label, no Greeks strip.
        await expect(pane.getByText("Fair variance strike", { exact: true })).toBeVisible({
          timeout: renderTimeoutMs,
        });
        await expect(pane.getByText(/^K_var (?!0\.000000$)\d+\.\d{6}$/)).toBeVisible();
        await expect(pane.getByText(/^√K_var = \d+\.\d{2}$/)).toBeVisible();
        await expect(pane.getByText(/% .* prem/)).toHaveCount(0);
        await expect(pane.getByTitle("delta (spot)")).toHaveCount(0);
      } else if (s.marker === "fair-volatility") {
        // Same contract in vol-swap shape: the exact head, the non-zero fair
        // (convexity-adjusted) volatility strike K_vol in vol points, and the
        // convexity-adjusted marker — again a fair strike, never a premium.
        await expect(pane.getByText("Fair volatility strike", { exact: true })).toBeVisible({
          timeout: renderTimeoutMs,
        });
        await expect(pane.getByText(/^K_vol (?!0\.00$)\d+\.\d{2}$/)).toBeVisible();
        await expect(pane.getByText("convexity-adjusted", { exact: true })).toBeVisible();
        await expect(pane.getByText(/% .* prem/)).toHaveCount(0);
        await expect(pane.getByTitle("delta (spot)")).toHaveCount(0);
      } else {
        await expect(pane.getByTitle("delta (spot)")).toBeVisible({
          timeout: renderTimeoutMs,
        });
        // The priced premium-unit label renders beside the two-way.
        await expect(pane.getByText(/% .* prem/).first()).toBeVisible();
      }
    });
  }
});

// ---------------------------------------------------------------------------
// wire half — frozen golden corpus through the GUI's production codec
// ---------------------------------------------------------------------------

/**
 * Assert one server-priced result against a vector — the SDK/Excel conformance
 * gate, mirrored. `window_barrier` (LSV-only, no flat-GBM closed form) gates
 * structural invariants plus its documented wide flat-GBM band; MC families use
 * `k·(oracle_se + server_se)`; the rest use the frozen `(rel, abs)` plus the
 * Greek set where the oracle quotes one (vanilla).
 */
function assertConforms(
  v: GoldenVector,
  got: number,
  serverStdErr: number | undefined,
  greeks: Record<string, number>,
): void {
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
      `MC vector ${v.id}: GUI-priced ${got} vs oracle ${want} |Δ|=${diff} > band ${band} ` +
        `(oracle_se=${oracleSe}, server_se=${serverSe})`,
    ).toBeLessThanOrEqual(band);
    return;
  }

  const scale = Math.max(Math.abs(got), Math.abs(want));
  const tol = v.tolerance.abs + v.tolerance.rel * scale;
  expect(
    Math.abs(got - want),
    `vector ${v.id}: GUI-priced ${got} vs oracle ${want} (rel ${v.tolerance.rel}, abs ${v.tolerance.abs})`,
  ).toBeLessThanOrEqual(tol);

  // Greeks where the oracle provides them (vanilla) — the same relative gate as
  // the SDK / Excel suites; keys map the wire snake_case onto the GUI `Greeks`.
  const expectedGreeks = v.expected.greeks ?? {};
  const keyMap: Record<string, string> = {
    delta_spot: "deltaSpot",
    gamma: "gamma",
    vega: "vega",
    theta: "theta",
    rho_dom: "rhoDom",
    rho_for: "rhoFor",
  };
  for (const [name, expected] of Object.entries(expectedGreeks)) {
    const key = keyMap[name];
    if (!key) continue;
    const g = greeks[key]!;
    const gscale = Math.max(Math.abs(g), Math.abs(expected));
    const ok = Math.abs(g - expected) <= 1e-7 + v.tolerance.rel * Math.max(gscale, 1);
    expect(ok, `vector ${v.id} greek ${name}: GUI-priced ${g} vs oracle ${expected}`).toBe(true);
  }
}

test.describe("golden-vector wire conformance: GUI codec → real edge == frozen oracle", () => {
  let client: EdgeClient;

  test.beforeAll(async () => {
    client = await EdgeClient.connect(readWsUrl());
  });

  test.afterAll(() => {
    client?.close();
  });

  test("the corpus has vectors for every WS-priced family", () => {
    for (const family of WS_PRICED_FAMILIES) {
      expect(
        CORPUS.get(family)?.length ?? 0,
        `no corpus vectors for WS-priced family \`${family}\``,
      ).toBeGreaterThan(0);
    }
  });

  test("every corpus family is either WS-priced or explicitly declared not-exposed", () => {
    const declared = new Set<string>([...WS_PRICED_FAMILIES, ...FAMILIES_NOT_EXPOSED_ON_FX_WS]);
    for (const family of CORPUS.keys()) {
      expect(declared.has(family), `corpus family \`${family}\` is accounted for`).toBe(true);
    }
  });

  for (const family of WS_PRICED_FAMILIES) {
    const vectors = CORPUS.get(family) ?? [];
    test.describe(`family: ${family} (${vectors.length} vectors)`, () => {
      for (const v of vectors) {
        test(`${v.id}${v.expected.oracle ? ` (oracle: ${v.expected.oracle.slice(0, 60)})` : ""}`, async () => {
          // Headroom over the per-call deadline of the heaviest MC families.
          test.setTimeout(120_000);
          const instrument = instrumentOfVector(v);
          const priced = await priceOnEdge(client, instrument, marketOf(v), CONFORMANCE_CONVENTIONS);

          // Whenever the server prices by Monte-Carlo it MUST surface a genuine,
          // positive stderr (the honest precision band the MC gate consumes).
          if (priced.priceStdError !== undefined) {
            expect(priced.priceStdError, `${v.id}: server MC stderr is positive`).toBeGreaterThan(0);
          }

          // The oracle-quoted Greek subset (vanilla) as a plain keyed record.
          const greekValues: Record<string, number> = {
            deltaSpot: priced.greeks.deltaSpot,
            gamma: priced.greeks.gamma,
            vega: priced.greeks.vega,
            theta: priced.greeks.theta,
            rhoDom: priced.greeks.rhoDom,
            rhoFor: priced.greeks.rhoFor,
          };
          assertConforms(v, priced.price, priced.priceStdError, greekValues);
        });
      }
    });
  }

  test("the NDF browser-half skip reason is REAL: the edge refuses an NDF on a deliverable pair", async () => {
    // Re-book a genuine corpus NDF onto a deliverable major (EURUSD) — exactly
    // what a defaults ticket would send today — and assert the server's typed
    // validity refusal. This pins the declared skip to a server-asserted fact,
    // so it cannot silently rot if the validity matrix (or the seeded pair
    // universe) changes: the moment an NDF quotes on a deliverable pair, this
    // fails and the browser half must cover the NDF flow instead.
    const ndf = (CORPUS.get("ndf") ?? [])[0];
    expect(ndf, "the corpus carries at least one NDF vector").toBeDefined();
    const instrument = {
      ...instrumentOfVector(ndf!),
      pair: { base: "EUR", quote: "USD" },
    };
    const outcome = await priceOnEdge(
      client,
      instrument,
      marketOf(ndf!),
      CONFORMANCE_CONVENTIONS,
    ).then(
      () => null,
      (err: unknown) => err,
    );
    // A TYPED server refusal (`error` frame), never a transport failure/timeout —
    // and the refusal names the deliverability law, not some incidental error.
    expect(outcome, "the edge must refuse an NDF on a deliverable pair").toBeInstanceOf(EdgeRefusal);
    expect(String((outcome as Error).message)).toMatch(/deliverable/);
  });

  test("reports (does not skip) the corpus families not priced over the FX WS path", () => {
    // Documentation-as-assertion (CLAUDE.md rule 2 — no silent gap): the three
    // cross-asset vanilla arms pin the GENERALIZED cost-of-carry their leaf
    // crates gate server-side; the FX-two-rate `MarketContext` cannot transport
    // it. The GUI still BOOKS and live-quotes them (the cross-asset spec is
    // covered in the browser half above; booking gated by
    // `gui/test/crossAssetProducts.test.ts`). Identical boundary to the Excel
    // gate. If a family becomes FX-WS-priceable, this pins the honest gap so it
    // cannot drift.
    expect([...FAMILIES_NOT_EXPOSED_ON_FX_WS]).toEqual([
      "equity_option",
      "commodity_option",
      "crypto_option",
    ]);
  });
});
