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
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
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

  // Hermetic per-run persistence. The edge persists its operator config — users +
  // desks + role bundles (identity.json) and inbound FIX acceptors
  // (fix-connections.json) — and reloads it on boot. The production default writes
  // these CWD-relative (here, the repo root), so a SHARED store leaks state across
  // runs and makes mutating specs non-idempotent: a re-run would inherit a prior
  // run's narrowed Trader role bundle (roleBundleEditing's precondition gone) or an
  // already-running connection bound to the wizard's default address (fixDialects'
  // "Next" stuck disabled on an address clash). Point the store at a fresh temp dir
  // per boot via the documented `CELNET_IDENTITY_CONFIG` / `CELNET_FIX_CONFIG`
  // knobs, so every run starts from the SEEDED defaults and never persists into the
  // repo. The dir is removed on teardown.
  const storeDir = mkdtempSync(join(tmpdir(), "celnet-e2e-edge-"));

  const cmd = `source "$HOME/.cargo/env" && exec cargo run -q -p celnet-server --example demo_edge`;
  const child: ChildProcess = spawn("/bin/sh", ["-c", cmd], {
    cwd: REPO_ROOT,
    env: {
      ...process.env,
      CELNET_WS_ADDR: wsAddr,
      CELNET_GRPC_ADDR: grpcAddr,
      CELNET_IDENTITY_CONFIG: join(storeDir, "identity.json"),
      CELNET_FIX_CONFIG: join(storeDir, "fix-connections.json"),
      // Run the demo edge under the PRODUCTION deny-by-default posture, not its
      // friendly Permissive dev default — so this live suite verifies the GUI's
      // entitlement default end-to-end: the Book/Risk views send an explicit
      // grant-all principal and are served, exactly as against a real edge. (A
      // genuinely absent principal would be denied; the GUI never sends one.)
      CELNET_ACCESS_MODE: "enforce",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  const stop = (): Promise<void> =>
    new Promise<void>((res) => {
      const done = (): void => {
        // Drop the throwaway per-run store (best-effort; tmp is reclaimed anyway).
        try {
          rmSync(storeDir, { recursive: true, force: true });
        } catch {
          /* ignore — the OS reclaims tmp */
        }
        res();
      };
      if (child.exitCode !== null || child.signalCode !== null) return done();
      child.once("exit", () => done());
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
