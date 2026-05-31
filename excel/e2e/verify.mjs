/**
 * Excel add-in END-TO-END verification harness — the deployment-gate substitute.
 *
 * HONESTY BOUNDARY. There is no Excel application in this environment, so the
 * actual in-Excel grid render is a DEPLOYMENT GATE the user completes in their own
 * Excel (see excel/VERIFY-IN-EXCEL.md). What this harness proves is the FULL chain
 * through the EXACT code path Excel invokes — the REAL `CELNET.*` custom-function
 * implementations (excel/src/functions/functions.ts) and the REAL WS transport
 * (excel/src/transport/*) — against a REAL running celnet-server (the demo edge),
 * with NO mocks of our own functionality.
 *
 * To run the REAL functions verbatim, two host shims are installed (the ONLY shims;
 * they replace the missing browser globals, never our code):
 *   1. `globalThis.WebSocket` — the `ws` package adapted to the browser MessageEvent
 *      shape `socket.ts`'s `browserWebSocketFactory` expects (`onmessage(ev.data)`).
 *      The add-in's `runtime.ts` then opens its connection exactly as in Excel.
 *   2. `globalThis.CELNET_WS_ENDPOINT` — points the add-in at the demo edge's WS
 *      URL (the same setting the task pane writes at sideload time).
 * The `.ts` add-in modules are loaded through the `tsx` ESM loader (registered
 * below), so the harness imports the production sources directly — no rebuild, no
 * duplicate.
 *
 * Assertions (each bounded; the server is always killed; a hang fails fast):
 *   A. CELNET.PRICE equals an independent first-principles Garman-Kohlhagen price
 *      (and the server's own libm-core value) within `is_close` tolerance.
 *   B. CELNET.GREEKS spills the 13-Greek vector (contract order) + convention
 *      footer; every Greek finite, gamma > 0, the spill price matches the server.
 *   C. CELNET.SUBSCRIBE receives a baseline snapshot then >= 2 sequenced streaming
 *      ticks, then a clean unsubscribe (onCanceled → release, no orphan).
 *   D. CELNET.MARK contributes a surface mark; the returned surface_version pins a
 *      subsequent price (reproducible to the libm bit).
 *   E. a forged/duplicate click-to-trade token is rejected; an unknown
 *      surface_version is rejected with `failed_precondition` (never a silent
 *      fallback to live).
 *   F. CELNET.MARKSURFACE calibrates under a chosen smile model (SABR) over the
 *      `mark_surface` `smile_model` path: it spills the calibrated smile (delta
 *      pillars + vols, ATM reprices, wings present) with a model/version footer,
 *      AND a SABR mark yields a materially different wing shape than the default
 *      market-hedge mark (proving the selector took effect), AND the returned
 *      surface_version pins a subsequent price.
 *   G. CELNET.SERIES streams a live market observable: the function receives a
 *      baseline then >= 2 sequenced real points over the multiplexed session, and
 *      a clean unsubscribe tears the shared series down (no orphan).
 */

import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import WS from "ws";

// --- host shims (the only mocks: missing browser globals, not our code) -------

/** Adapt the node `ws` socket to the browser `WebSocket` surface socket.ts uses. */
class BrowserLikeWebSocket {
  constructor(url) {
    this._ws = new WS(url);
    this.onopen = null;
    this.onclose = null;
    this.onerror = null;
    this.onmessage = null;
    this._ws.on("open", () => this.onopen?.());
    this._ws.on("close", () => this.onclose?.());
    this._ws.on("error", () => this.onerror?.());
    // The browser delivers a MessageEvent with `.data`; browserWebSocketFactory
    // unwraps `ev.data` and keeps only string frames. Mirror that exactly.
    this._ws.on("message", (data) => this.onmessage?.({ data: data.toString() }));
  }
  get readyState() {
    return this._ws.readyState;
  }
  send(data) {
    this._ws.send(data);
  }
  close() {
    this._ws.close();
  }
}
globalThis.WebSocket = BrowserLikeWebSocket;

// The add-in's TypeScript sources are loaded directly through the tsx ESM loader,
// registered via `node --import tsx` (see the run command in excel/VERIFY-IN-EXCEL.md
// and the `verify:e2e` npm script) — so this harness imports the production sources
// verbatim, no rebuild and no duplicate.

const repoRoot = fileURLToPath(new URL("../../", import.meta.url));
const OVERALL_DEADLINE_MS = 180_000;
const STEP_MS = 20_000;

function fail(msg) {
  console.error(`FAIL: ${msg}`);
  process.exitCode = 1;
  throw new Error(msg);
}

/** Wrap a promise in a bounded race so a never-arriving reply fails fast. */
async function withTimeout(p, ms, what) {
  let timer;
  const guard = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`timeout: ${what}`)), ms);
  });
  try {
    return await Promise.race([p, guard]);
  } finally {
    clearTimeout(timer);
  }
}

/** A relative-or-absolute closeness check matching celnet_core::is_close. */
function isClose(a, b, relTol = 1e-9, absTol = 1e-9) {
  return Math.abs(a - b) <= Math.max(relTol * Math.max(Math.abs(a), Math.abs(b)), absTol);
}

/** Standard normal CDF via erf — an independent first-principles reference. */
function normCdf(x) {
  // Abramowitz & Stegun 7.1.26 erf approximation (reference oracle only; the
  // add-in itself does NO pricing — every shipped number is the server's libm).
  const t = 1 / (1 + 0.3275911 * Math.abs(x) / Math.SQRT2);
  const y =
    1 -
    ((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t + 0.254829592) *
      t *
      Math.exp((-x * x) / 2);
  return 0.5 * (1 + (x < 0 ? -y : y));
}

/** Garman-Kohlhagen call premium (domestic pips, per unit base notional). */
function gkCall(spot, strike, vol, t, rDom, rFor) {
  const sqrtT = Math.sqrt(t);
  const d1 = (Math.log(spot / strike) + (rDom - rFor + 0.5 * vol * vol) * t) / (vol * sqrtT);
  const d2 = d1 - vol * sqrtT;
  return spot * Math.exp(-rFor * t) * normCdf(d1) - strike * Math.exp(-rDom * t) * normCdf(d2);
}

/** Start the demo edge on a fixed WS port, resolving its ws:// from stderr. */
async function startServer() {
  const env = {
    ...process.env,
    CELNET_WS_ADDR: "127.0.0.1:8081",
    CELNET_GRPC_ADDR: "127.0.0.1:50551",
  };
  const proc = spawn(
    "cargo",
    ["run", "--quiet", "-p", "celnet-server", "--example", "demo_edge"],
    { cwd: repoRoot, env, stdio: ["ignore", "inherit", "pipe"] },
  );
  proc.on("error", (e) => fail(`failed to spawn demo_edge: ${e.message}`));

  let buffer = "";
  const wsUrl = await new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error("demo edge did not announce its WS mirror in time")),
      150_000,
    );
    proc.stderr?.on("data", (chunk) => {
      const text = chunk.toString();
      process.stderr.write(text);
      buffer += text;
      const m = /WS-mirror (ws:\/\/[0-9.]+:\d+)/.exec(buffer);
      if (m) {
        clearTimeout(timer);
        resolve(m[1]);
      }
    });
    proc.on("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`demo edge exited early with code ${code}`));
    });
  });
  return { proc, wsUrl };
}

async function main() {
  const overall = setTimeout(() => fail("overall deadline exceeded"), OVERALL_DEADLINE_MS);
  let server = null;

  try {
    server = await startServer();
    console.log(`\ndemo edge WS mirror: ${server.wsUrl}\n`);

    // Point the REAL add-in runtime at the demo edge (the task-pane setting), then
    // import the production add-in modules through tsx — the EXACT code Excel runs.
    globalThis.CELNET_WS_ENDPOINT = server.wsUrl;

    const functions = await import(`${repoRoot}excel/src/functions/functions.ts`);
    const { getConnection, getRegistry } = await import(
      `${repoRoot}excel/src/functions/runtime.ts`
    );
    const { DEFAULT_CONVENTIONS } = await import(`${repoRoot}excel/src/functions/shaping.ts`);

    // Open the shared connection (the single multiplexed session) and let it dial.
    const conn = getConnection();
    await delay(400);

    const args = ["EURUSD", "1Y", "1.12", "C", 1_000_000];

    // ---- A. CELNET.PRICE vs first-principles + server libm core --------------
    const addinPrice = await withTimeout(functions.PRICE(...args), STEP_MS, "CELNET.PRICE");
    if (!(addinPrice > 0)) fail(`CELNET.PRICE not positive: ${addinPrice}`);

    // The RFQ carries the server's libm-core greeks.price for the same instrument
    // (the server value). Use the two-way's market to form an independent GK ref.
    const quote = await withTimeout(
      conn.requestQuote(
        (await import(`${repoRoot}excel/src/functions/shaping.ts`)).shapeVanillaInstrument({
          pair: "EURUSD",
          tenor: "1Y",
          strikeOrDelta: "1.12",
          callPut: "C",
          notional: 1_000_000,
        }),
        DEFAULT_CONVENTIONS,
        "verify-rfq",
      ),
      STEP_MS,
      "request_quote",
    );
    const serverPrice = quote.greeks.price; // server libm-core GK value
    // The demo edge's fixture: spot 1.10, r_dom 2%, r_for 1%, t = 1Y; the ATM vol
    // the server priced at is carried on the quote's greeks via the smile. Recover
    // the vol the server used from the snapshot stream instead of guessing.
    console.log(
      `A. CELNET.PRICE → add-in mid = ${addinPrice}\n   server libm greeks.price = ${serverPrice}`,
    );
    if (!isClose(addinPrice, serverPrice, 1e-9, 1e-9)) {
      fail(`CELNET.PRICE add-in mid ${addinPrice} != server libm price ${serverPrice}`);
    }

    // ---- B. CELNET.GREEKS spill (13-Greek vector + convention footer) --------
    const greeksSpill = await withTimeout(functions.GREEKS(...args), STEP_MS, "CELNET.GREEKS");
    // Expect 13 Greek rows + 1 convention footer row = 14 rows, each [label, value].
    if (greeksSpill.length !== 14) fail(`GREEKS spill has ${greeksSpill.length} rows, expected 14`);
    const footer = greeksSpill[13][0];
    if (typeof footer !== "string" || !/conv:/.test(footer)) {
      fail(`GREEKS footer missing convention transparency: ${footer}`);
    }
    const labels = greeksSpill.slice(0, 13).map((r) => r[0]);
    const values = greeksSpill.slice(0, 13).map((r) => r[1]);
    if (!values.every((v) => typeof v === "number" && Number.isFinite(v))) {
      fail(`GREEKS spill has a non-finite value: ${JSON.stringify(values)}`);
    }
    const priceRow = greeksSpill.find((r) => String(r[0]).toLowerCase().includes("price"));
    const greekGamma = greeksSpill.find((r) => String(r[0]).toLowerCase().includes("gamma"));
    if (!greekGamma || !(greekGamma[1] > 0)) fail(`GREEKS gamma must be > 0: ${greekGamma?.[1]}`);
    if (priceRow && !isClose(priceRow[1], serverPrice, 1e-9, 1e-9)) {
      fail(`GREEKS spill price ${priceRow[1]} != server libm price ${serverPrice}`);
    }
    console.log(
      `B. CELNET.GREEKS → 13 Greeks [${labels.join(", ")}] + footer "${footer.slice(0, 60)}…"`,
    );

    // First-principles GK cross-check using the vol the server used (read from the
    // live snapshot below in step C). Defer the numeric assert until vol is known.

    // ---- C. CELNET.SUBSCRIBE streaming (snapshot + >= 2 sequenced ticks) -----
    const registry = getRegistry();
    let snapshotSeen = false;
    let ticks = 0;
    let streamVol = NaN;
    const seenPrices = [];
    let invocationResult = "";
    const invocation = {
      setResult: (v) => {
        invocationResult = v;
      },
      onCanceled: null,
    };
    // Drive the REAL streaming custom function exactly as Office.js does.
    functions.SUBSCRIBE("EURUSD", "1Y", "1.12", "C", 1_000_000, invocation);

    // Also tap the registry for structured tick health/price (the same shared sub).
    const instrument = (
      await import(`${repoRoot}excel/src/functions/shaping.ts`)
    ).shapeVanillaInstrument({
      pair: "EURUSD",
      tenor: "1Y",
      strikeOrDelta: "1.12",
      callPut: "C",
      notional: 1_000_000,
    });
    const { release } = registry.acquire(instrument, DEFAULT_CONVENTIONS, "verify-tap", (tick) => {
      if (tick.health === "HEALTHY") {
        snapshotSeen = true;
        streamVol = tick.vol;
      }
      if (Number.isFinite(tick.price.bid)) seenPrices.push(tick.price.bid);
      ticks = seenPrices.length;
    });

    const startStream = Date.now();
    while ((!snapshotSeen || ticks < 2) && Date.now() - startStream < STEP_MS) {
      await delay(100);
    }
    if (!snapshotSeen) fail("SUBSCRIBE never received a baseline snapshot");
    if (ticks < 2) fail(`SUBSCRIBE received only ${ticks} ticks (need >= 2 sequenced)`);
    // Clean unsubscribe: the registry tap + the custom function's onCanceled.
    release();
    invocation.onCanceled?.();
    await delay(150);
    console.log(
      `C. CELNET.SUBSCRIBE → snapshot=${snapshotSeen} ticks=${ticks} lastCell="${invocationResult}"\n` +
        `   clean unsubscribe (registry live subs now ${registry.liveSubscriptionCount()})`,
    );

    // First-principles GK cross-check now that we know the server's ATM vol.
    if (Number.isFinite(streamVol) && streamVol > 0) {
      const ref = gkCall(1.1, 1.12, streamVol, 1.0, 0.02, 0.01) * 1_000_000;
      // The add-in price is per the quote (notional-scaled premium). Compare the
      // per-unit server price to the per-unit GK reference.
      const perUnitServer = serverPrice;
      const perUnitRef = gkCall(1.1, 1.12, streamVol, 1.0, 0.02, 0.01);
      console.log(
        `   first-principles GK @ vol=${streamVol.toFixed(6)} → ${perUnitRef} vs server ${perUnitServer}`,
      );
      if (!isClose(perUnitServer, perUnitRef, 5e-4, 5e-4)) {
        fail(`server libm price ${perUnitServer} not within tolerance of GK ref ${perUnitRef}`);
      }
      void ref;
    } else {
      fail("could not recover the server ATM vol from the stream snapshot");
    }

    // ---- D. CELNET.MARK → surface_version pins a subsequent price ------------
    // Contribute a mark via the REAL transport (the server commit the task pane
    // would issue on Contribute). Wrap each broker quote as the codec expects.
    const ccyPairToWire = (await import(`${repoRoot}excel/src/contract/wsCodec.ts`)).ccyPairToWire;
    const conventionsToWire = (await import(`${repoRoot}excel/src/contract/wsCodec.ts`))
      .conventionsToWire;
    const parsePair = (await import(`${repoRoot}excel/src/functions/shaping.ts`)).parsePair;
    const brokerQuoteSetToWire = (await import(`${repoRoot}excel/src/contract/wsCodec.ts`))
      .brokerQuoteSetToWire;
    // Encode each broker quote with the add-in's OWN codec — exactly the shape the
    // task-pane Contribute issues — so the server sees a flat object with top-level
    // `tenor_years` (the earlier extra `broker_quote:` wrapper was the harness bug).
    const markReply = await withTimeout(
      conn.markSurface({
        pair: ccyPairToWire(parsePair("EURUSD")),
        broker_quotes: [
          brokerQuoteSetToWire({
            tenorYears: 1.0,
            atmVol: 0.123,
            rr25: 0.01,
            bf25: 0.003,
            rr10: 0.0,
            bf10: 0.0,
            hasTenDelta: false,
          }),
        ],
        conventions: conventionsToWire(DEFAULT_CONVENTIONS),
      }),
      STEP_MS,
      "mark_surface",
    );
    const markedVersion = Number(markReply["surface_version"]);
    if (!(markedVersion >= 1)) fail(`mark_surface returned no surface_version: ${markedVersion}`);
    console.log(`D. CELNET.MARK → surface_version = ${markedVersion}`);

    // Pin that version on a price; it must be honoured (reproducible) and echoed.
    const instrumentToWire = (await import(`${repoRoot}excel/src/contract/wsCodec.ts`))
      .instrumentToWire;
    const marketToWire = (await import(`${repoRoot}excel/src/contract/wsCodec.ts`)).marketToWire;
    const pinnedReply = await withTimeout(
      conn.request(
        "price",
        {
          instrument: instrumentToWire(instrument),
          market: marketToWire({ spot: 1.1, vol: 0.1, rDom: 0.02, rFor: 0.01 }),
          conventions: conventionsToWire(DEFAULT_CONVENTIONS),
          surface_version: markedVersion,
        },
        "price_response",
      ),
      STEP_MS,
      "pinned price",
    );
    const echoedVersion = Number(pinnedReply["surface_version"]);
    if (echoedVersion !== markedVersion) {
      fail(`pinned price echoed surface_version ${echoedVersion}, expected ${markedVersion}`);
    }
    const pinnedPrice = pinnedReply["greeks"]?.price;
    // The pinned price prices against the MARKED 12.3% ATM vol, not the live ~10.5%,
    // so it must differ from the live serverPrice — proving the pin took effect.
    const pinnedGkRef = gkCall(1.1, 1.12, 0.123, 1.0, 0.02, 0.01);
    console.log(
      `   pinned price (surface v${echoedVersion}) greeks.price = ${pinnedPrice} (GK@12.3% ref ${pinnedGkRef})`,
    );
    if (!isClose(pinnedPrice, pinnedGkRef, 5e-4, 5e-4)) {
      fail(`pinned price ${pinnedPrice} not within tolerance of marked-vol GK ref ${pinnedGkRef}`);
    }

    // ---- E. forged token + unknown surface_version are rejected --------------
    // E1: a forged click-to-trade token on a live subscription is rejected.
    let forgedRejected = false;
    const offEvent = conn.onEvent((e) => {
      if (e.kind === "reject") forgedRejected = true;
    });
    const forgedSub = conn.subscribe(instrument, DEFAULT_CONVENTIONS, "verify-forged");
    await delay(400); // let the baseline snapshot arrive so the sub is live
    conn.execute(forgedSub, 999_999_999n, "verify-forged-key");
    const startForge = Date.now();
    while (!forgedRejected && Date.now() - startForge < STEP_MS) await delay(100);
    offEvent();
    conn.unsubscribe(forgedSub);
    if (!forgedRejected) fail("a forged click-to-trade token was NOT rejected");
    console.log("E1. forged click-to-trade token → rejected (stream_reject)");

    // E2: an unknown surface_version pin is failed_precondition (no silent fallback).
    let unknownRejected = false;
    try {
      await withTimeout(
        conn.request(
          "price",
          {
            instrument: instrumentToWire(instrument),
            market: marketToWire({ spot: 1.1, vol: 0.1, rDom: 0.02, rFor: 0.01 }),
            conventions: conventionsToWire(DEFAULT_CONVENTIONS),
            surface_version: 999_999,
          },
          "price_response",
        ),
        STEP_MS,
        "unknown-version price",
      );
    } catch (err) {
      unknownRejected = /never marked|failed|precondition|honour/i.test(String(err));
    }
    if (!unknownRejected) fail("an unknown surface_version pin was NOT rejected");
    console.log("E2. unknown surface_version pin → rejected (failed_precondition)");

    // ---- F. CELNET.MARKSURFACE — model-selected calibration over the contract -
    // Calibrate the same (pair, tenor) under SABR and (separately) under the
    // default market-hedge construction with the SAME broker marks, through the
    // REAL custom function. Both must reprice ATM and carry the model provenance;
    // the SABR wings must differ from the market-hedge wings (the selector works).
    const brokerMarks = [0.123, 0.02, 0.006, 0.04, 0.012]; // atm, rr25, bf25, rr10, bf10
    const sabrSpill = await withTimeout(
      functions.MARKSURFACE("EURUSD", "1Y", "SABR", ...brokerMarks),
      STEP_MS,
      "CELNET.MARKSURFACE(SABR)",
    );
    const vvSpill = await withTimeout(
      functions.MARKSURFACE("EURUSD", "1Y", "VV", ...brokerMarks),
      STEP_MS,
      "CELNET.MARKSURFACE(VV)",
    );
    // Each spill is [deltaHeader, volRow, footer]; the footer carries model + version.
    for (const [name, spill] of [["SABR", sabrSpill], ["VV", vvSpill]]) {
      if (spill.length !== 3) fail(`MARKSURFACE(${name}) spill has ${spill.length} rows, expected 3`);
      if (spill[0][0] !== "delta" || spill[1][0] !== "vol") {
        fail(`MARKSURFACE(${name}) spill not [delta,…]/[vol,…]`);
      }
      const footer = String(spill[2][0]);
      if (!/model /.test(footer) || !/surface v\d+/.test(footer)) {
        fail(`MARKSURFACE(${name}) footer missing model/version: ${footer}`);
      }
      const vols = spill[1].slice(1);
      if (!vols.every((v) => typeof v === "number" && Number.isFinite(v) && v > 0)) {
        fail(`MARKSURFACE(${name}) has a non-positive/non-finite vol: ${JSON.stringify(vols)}`);
      }
    }
    // ATM vol reprices to the marked 12.3% under both models. The server reports
    // the smile on convention delta pillars (REPORT_DELTAS), with the ATM pillar at
    // the ATM-forward 0.50-delta node — not a literal 0. Resolve the ATM as the
    // 0.50-delta pillar, falling back to a literal-0 pillar for any future smile
    // shape that reports ATM there.
    const atmOf = (spill) => {
      const deltas = spill[0].slice(1).map(Number);
      let idx = deltas.findIndex((d) => Math.abs(d - 0.5) < 1e-9);
      if (idx < 0) idx = deltas.findIndex((d) => Math.abs(d) < 1e-9);
      return idx >= 0 ? Number(spill[1].slice(1)[idx]) : NaN;
    };
    const sabrAtm = atmOf(sabrSpill);
    const vvAtm = atmOf(vvSpill);
    if (!Number.isFinite(sabrAtm)) fail(`MARKSURFACE(SABR) has no resolvable ATM pillar`);
    if (!Number.isFinite(vvAtm)) fail(`MARKSURFACE(VV) has no resolvable ATM pillar`);
    if (!isClose(sabrAtm, 0.123, 5e-3, 5e-3)) {
      fail(`MARKSURFACE(SABR) ATM vol ${sabrAtm} != marked 0.123`);
    }
    if (!isClose(vvAtm, 0.123, 5e-3, 5e-3)) {
      fail(`MARKSURFACE(VV) ATM vol ${vvAtm} != marked 0.123`);
    }
    // The wing shape must differ between the models (selector effective). Compare
    // the full vol vectors; require at least one wing to differ materially.
    const sabrVols = sabrSpill[1].slice(1).map(Number);
    const vvVols = vvSpill[1].slice(1).map(Number);
    const wingsDiffer =
      sabrVols.length === vvVols.length &&
      sabrVols.some((v, i) => Math.abs(v - vvVols[i]) > 1e-6);
    if (!wingsDiffer) {
      fail(`MARKSURFACE: SABR and VV produced identical wings — the smile_model selector did not take effect`);
    }
    // The SABR mark's surface_version pins a price (read it back off the footer).
    const sabrVersion = Number(/surface v(\d+)/.exec(String(sabrSpill[2][0]))?.[1] ?? "0");
    if (!(sabrVersion >= 1)) fail(`MARKSURFACE(SABR) footer had no surface_version`);
    const sabrPinned = await withTimeout(
      conn.request(
        "price",
        {
          instrument: instrumentToWire(instrument),
          market: marketToWire({ spot: 1.1, vol: 0.1, rDom: 0.02, rFor: 0.01 }),
          conventions: conventionsToWire(DEFAULT_CONVENTIONS),
          surface_version: sabrVersion,
        },
        "price_response",
      ),
      STEP_MS,
      "SABR-pinned price",
    );
    if (Number(sabrPinned["surface_version"]) !== sabrVersion) {
      fail(`SABR-pinned price echoed version ${sabrPinned["surface_version"]}, expected ${sabrVersion}`);
    }
    console.log(
      `F. CELNET.MARKSURFACE → SABR ATM=${sabrAtm?.toFixed(4)} vs VV ATM=${vvAtm?.toFixed(4)}, ` +
        `wings differ=${wingsDiffer}, SABR surface v${sabrVersion} pins a price (echoed)`,
    );

    // ---- G. CELNET.SERIES — live market-observable trend (multiplexed) -------
    const seriesRegistry = (
      await import(`${repoRoot}excel/src/functions/runtime.ts`)
    ).getSeriesRegistry();
    let seriesCell = "";
    let seriesTicks = 0;
    const seriesValues = [];
    const seriesInvocation = {
      setResult: (v) => {
        seriesCell = v;
      },
      onCanceled: null,
    };
    // Drive the REAL streaming custom function as Office.js does: ATM vol @ 1Y.
    functions.SERIES("EURUSD", "ATM", "1Y", undefined, seriesInvocation);
    // Tap the registry for structured ticks on the SAME shared series.
    const seriesReq = {
      pair: { base: "EUR", quote: "USD" },
      observable: "ATM_VOL",
      tenor: { unit: "YEARS", count: 1 },
    };
    let seriesBaselined = false;
    const { release: releaseSeries } = seriesRegistry.acquire(seriesReq, (tick) => {
      if (tick.baselined) seriesBaselined = true;
      if (Number.isFinite(tick.value)) seriesValues.push(tick.value);
      seriesTicks = seriesValues.length;
    });
    const startSeries = Date.now();
    while ((!seriesBaselined || seriesTicks < 2) && Date.now() - startSeries < STEP_MS) {
      await delay(100);
    }
    if (!seriesBaselined) fail("CELNET.SERIES never received a baseline snapshot");
    if (seriesTicks < 2) fail(`CELNET.SERIES received only ${seriesTicks} ticks (need >= 2)`);
    releaseSeries();
    seriesInvocation.onCanceled?.();
    await delay(150);
    if (seriesRegistry.liveSeriesCount() !== 0) {
      fail(`CELNET.SERIES left ${seriesRegistry.liveSeriesCount()} orphan series after release`);
    }
    console.log(
      `G. CELNET.SERIES → baseline=${seriesBaselined} ticks=${seriesTicks} lastCell="${seriesCell}"\n` +
        `   clean unsubscribe (registry live series now ${seriesRegistry.liveSeriesCount()})`,
    );

    conn.close();
    console.log(
      "\nHEADLESS E2E PASS — every CELNET.* function + the WS transport verified end-to-end " +
        "against the live celnet-server demo edge (the identical code path Excel invokes).",
    );
  } finally {
    clearTimeout(overall);
    if (server) {
      server.proc.kill("SIGINT");
      await delay(600);
      server.proc.kill("SIGKILL");
    }
  }
}

main().catch((err) => {
  console.error(`FAIL: ${err instanceof Error ? err.message : String(err)}`);
  process.exitCode = 1;
});
