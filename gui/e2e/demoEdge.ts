/**
 * Boot the REAL `celnet-server` demo edge as a child process for the Playwright
 * e2e run — no mock, no fake. This is the same `demo_edge` example a trader-facing
 * QA would dial: a real gRPC + WebSocket-mirror edge over the engine's calibrated
 * EURUSD fixture, serving server-computed prices/Greeks/surfaces/scenarios out of
 * the single `celnet.wire` contract.
 *
 * The edge prints a ready line to stderr ("celnet-server demo edge ready — …
 * WS-mirror ws://HOST:PORT …"); we parse the WS URL from it so the e2e dials the
 * exact bound endpoint. The process is killed on teardown. Cargo is invoked
 * through the user's env exactly as CLAUDE.md prescribes (rustup is not on PATH).
 */
import { type ChildProcess, spawn } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** A running demo edge: its WebSocket-mirror URL and the child to tear down. */
export interface DemoEdge {
  wsUrl: string;
  stop: () => Promise<void>;
}

// ESM-safe directory of this module (`__dirname` is undefined under "type":
// "module"). `e2e/` lives at `<repo>/gui/e2e`, so the repo root is two levels up.
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "..", "..");
const READY_RE = /WS-mirror\s+(ws:\/\/[\d.:]+)/;

/**
 * Spawn the demo edge on fixed ports and resolve once it prints its ready line
 * (or reject on a bounded timeout / early exit). Sourcing the cargo env mirrors
 * the project's `source "$HOME/.cargo/env" && cargo …` invariant.
 */
export function startDemoEdge(opts?: {
  wsAddr?: string;
  grpcAddr?: string;
  timeoutMs?: number;
}): Promise<DemoEdge> {
  const wsAddr = opts?.wsAddr ?? "127.0.0.1:8097";
  const grpcAddr = opts?.grpcAddr ?? "127.0.0.1:50597";
  const timeoutMs = opts?.timeoutMs ?? 180_000;

  const cmd = `source "$HOME/.cargo/env" && exec cargo run -q -p celnet-server --example demo_edge`;
  const child: ChildProcess = spawn("/bin/sh", ["-c", cmd], {
    cwd: REPO_ROOT,
    env: {
      ...process.env,
      CELNET_WS_ADDR: wsAddr,
      CELNET_GRPC_ADDR: grpcAddr,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  const stop = (): Promise<void> =>
    new Promise<void>((res) => {
      if (child.exitCode !== null || child.signalCode !== null) return res();
      child.once("exit", () => res());
      child.kill("SIGTERM");
      // Hard-stop if it lingers (the edge holds sockets/threads).
      setTimeout(() => {
        if (child.exitCode === null) child.kill("SIGKILL");
      }, 4_000);
    });

  return new Promise<DemoEdge>((res, reject) => {
    let settled = false;
    let buf = "";
    const onLine = (chunk: Buffer) => {
      buf += chunk.toString();
      const m = buf.match(READY_RE);
      if (m && !settled) {
        settled = true;
        clearTimeout(timer);
        res({ wsUrl: m[1]!, stop });
      }
    };
    child.stdout?.on("data", onLine);
    child.stderr?.on("data", onLine);

    child.once("exit", (code) => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        reject(new Error(`demo_edge exited before ready (code ${code}); output:\n${buf}`));
      }
    });
    child.once("error", (err) => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        reject(err);
      }
    });

    const timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        void stop();
        reject(new Error(`demo_edge did not become ready within ${timeoutMs}ms; output:\n${buf}`));
      }
    }, timeoutMs);
  });
}
