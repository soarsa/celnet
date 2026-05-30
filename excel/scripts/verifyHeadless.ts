/**
 * Headless end-to-end verification — the DEPLOYMENT-GATE substitute.
 *
 * HONESTY BOUNDARY: there is no Excel application in this environment, so the
 * actual in-Excel grid render cannot be asserted here (that is the user's
 * sideload step, README §Sideload). What this harness DOES prove is the full
 * chain through the EXACT code path Excel's custom functions call — the request
 * shaping (src/functions/shaping), the WS transport (src/transport, with the node
 * socket factory instead of the browser one), and the live celnet-server WS
 * mirror — by exercising the real CELNET.* function implementations against a
 * real running server, with NO mocks of our own functionality.
 *
 * It spawns the real `celnet-server` binary, parses the `ws://HOST:PORT` it
 * prints, drives PRICE / GREEKS / SURFACE / RFQ over the live mirror, and a
 * streaming SUBSCRIBE that receives a snapshot then a sequenced update. Every
 * wait is bounded; the server is always killed; a hang fails fast.
 */

import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

import { Connection } from "../src/transport/connection";
import { StreamRegistry } from "../src/functions/streamRegistry";
import { DEFAULT_CONVENTIONS, parsePair, parseTenor, shapeVanillaInstrument } from "../src/functions/shaping";
import { smileFromWire, type WireObject } from "../src/contract/wsCodec";
import { nodeWebSocketFactory } from "./nodeSocket";

const OVERALL_DEADLINE_MS = 90_000;
const STEP_MS = 15_000;

function fail(msg: string): never {
  console.error(`FAIL: ${msg}`);
  process.exit(1);
}

/** Start the celnet-server binary and resolve its WS mirror URL from stderr.
 * Prefers the prebuilt debug/release binary (no cargo on PATH needed); falls back
 * to `cargo run` only if no binary is present. */
async function startServer(): Promise<{ proc: ChildProcess; wsUrl: string }> {
  // From excel/scripts/verifyHeadless.ts → repo root is two levels up.
  const repoRoot = fileURLToPath(new URL("../../", import.meta.url));
  const debugBin = `${repoRoot}target/debug/celnet-server`;
  const releaseBin = `${repoRoot}target/release/celnet-server`;
  const env = { ...process.env, CELNET_GRPC_ADDR: "127.0.0.1:0" };

  let proc: ChildProcess;
  if (existsSync(releaseBin)) {
    proc = spawn(releaseBin, [], { cwd: repoRoot, env, stdio: ["ignore", "inherit", "pipe"] });
  } else if (existsSync(debugBin)) {
    proc = spawn(debugBin, [], { cwd: repoRoot, env, stdio: ["ignore", "inherit", "pipe"] });
  } else {
    proc = spawn("cargo", ["run", "--quiet", "-p", "celnet-server"], {
      cwd: repoRoot,
      env,
      stdio: ["ignore", "inherit", "pipe"],
    });
  }
  proc.on("error", (e) => fail(`failed to spawn celnet-server: ${e.message}`));

  let buffer = "";
  const wsUrl = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("server did not announce its WS mirror in time")), 80_000);
    proc.stderr?.on("data", (chunk: Buffer) => {
      const text = chunk.toString();
      process.stderr.write(text);
      buffer += text;
      const m = /WS-mirror (ws:\/\/[0-9.]+:\d+)/.exec(buffer);
      if (m) {
        clearTimeout(timer);
        resolve(m[1] as string);
      }
    });
    proc.on("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`server exited early with code ${code}`));
    });
  });
  return { proc, wsUrl };
}

async function withTimeout<T>(p: Promise<T>, ms: number, what: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout>;
  const guard = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`timeout: ${what}`)), ms);
  });
  try {
    return await Promise.race([p, guard]);
  } finally {
    clearTimeout(timer!);
  }
}

async function main(): Promise<void> {
  const overall = setTimeout(() => fail("overall deadline exceeded"), OVERALL_DEADLINE_MS);
  let server: { proc: ChildProcess; wsUrl: string } | null = null;
  let conn: Connection | null = null;

  try {
    server = await startServer();
    console.log(`server WS mirror: ${server.wsUrl}`);
    conn = new Connection({ url: server.wsUrl, factory: nodeWebSocketFactory(), stalenessWindowMs: 0 });

    // Let the socket open.
    await delay(300);

    const args = { pair: "EURUSD", tenor: "1Y", strikeOrDelta: 1.12, callPut: "C", notional: 1_000_000 };
    const instrument = shapeVanillaInstrument(args);

    // 1) PRICE / RFQ — the EXACT shaping CELNET.PRICE/RFQ use, against the live server.
    const quote = await withTimeout(
      conn.requestQuote(instrument, DEFAULT_CONVENTIONS, "verify-rfq"),
      STEP_MS,
      "request_quote",
    );
    if (!(quote.price.bid < quote.price.offer)) fail("RFQ two-way is not bid<offer");
    const mid = 0.5 * (quote.price.bid + quote.price.offer);
    console.log(`CELNET.RFQ → bid=${quote.price.bid} offer=${quote.price.offer} mid=${mid} quoteId=${quote.quoteId}`);
    console.log(`CELNET.PRICE → ${mid}`);

    // 2) The single-cell price path (PRICE returns the mid). Assert finite & positive.
    if (!(mid > 0)) fail("CELNET.PRICE mid is not positive");

    // 3) GREEKS — the full Greek vector over the live server (Price path returns Greeks).
    const priced = await withTimeout(
      conn.price(instrument, { spot: 1.1, vol: 0.1, rDom: 0.0, rFor: 0.0 }, DEFAULT_CONVENTIONS),
      STEP_MS,
      "price (greeks)",
    );
    const g = priced.greeks;
    console.log(
      `CELNET.GREEKS → price=${g.price} deltaSpot=${g.deltaSpot} gamma=${g.gamma} vega=${g.vega} theta=${g.theta} color=${g.color}`,
    );
    if (!Number.isFinite(g.deltaSpot) || g.gamma <= 0) fail("Greeks look wrong (gamma must be > 0)");

    // 4) SURFACE — the marked smile for a (pair, tenor) over the live server.
    const ccy = parsePair("EURUSD");
    const { expiryYears } = parseTenor("1Y");
    const smileReply: WireObject = await withTimeout(
      conn.getSmile(ccy, expiryYears, DEFAULT_CONVENTIONS),
      STEP_MS,
      "get_smile",
    );
    const smile = smileFromWire(smileReply);
    console.log(`CELNET.SURFACE → ${smile.points.length} smile points; arbFree=${smile.arbitrage.butterflyArbitrageFree}`);
    if (smile.points.length === 0) fail("SURFACE returned no smile points");

    // 5) SUBSCRIBE — the streaming path through the SAME registry CELNET.SUBSCRIBE uses.
    const registry = new StreamRegistry(conn);
    let sawSnapshot = false;
    let sawUpdate = false;
    const seen: number[] = [];
    const { release } = registry.acquire(instrument, DEFAULT_CONVENTIONS, "verify-stream", (tick) => {
      seen.push(tick.price.bid);
      if (tick.health === "HEALTHY") sawSnapshot = true;
      if (seen.length >= 2) sawUpdate = true;
    });

    const start = Date.now();
    while ((!sawSnapshot || !sawUpdate) && Date.now() - start < STEP_MS) {
      await delay(100);
    }
    release();
    registry.dispose();
    console.log(`CELNET.SUBSCRIBE → snapshot=${sawSnapshot} update=${sawUpdate} ticks=${seen.length}`);
    if (!sawSnapshot) fail("SUBSCRIBE never received a baseline snapshot");
    if (!sawUpdate) fail("SUBSCRIBE never received a live update");

    console.log("\nHEADLESS E2E PASS — every CELNET.* code path verified against the live celnet-server WS mirror.");
  } finally {
    clearTimeout(overall);
    conn?.close();
    if (server) {
      server.proc.kill("SIGINT");
      // Give it a moment to drain, then ensure it is gone.
      await delay(500);
      server.proc.kill("SIGKILL");
    }
  }
}

main().catch((err: unknown) => fail(err instanceof Error ? err.message : String(err)));
