#!/usr/bin/env bash
# run_dev.sh — THE single-command local bring-up of Celnet.
#
# Replaces the previous split between `tools/dev.sh` (demo_edge + GUI, no
# simulators) and a separate full-stack script: one entry point, flags to vary it.
#
# Starts, in dependency order:
#
#   1. celnet-server        gRPC 127.0.0.1:50551 · ws://127.0.0.1:8081
#   2. lp-sim               N liquidity providers -> the LpFeed gRPC ingest
#   3. fix_rfq_client (RFQ) one-shot quote requests against the FIX acceptor
#   4. fix_rfq_client (ESP) Market-Data streaming off reference data
#   5. gui                  Vite dev server on http://localhost:5173
#
# Those four inbound connections are exactly what Administration -> LP Panel
# reports on.
#
# ## It provisions what the fleet needs to actually do anything
#
# Two things previously had to be done by hand before a simulator produced
# anything visible, and getting either wrong looks identical to a broken feed:
#
#   - The FIX acceptor's PORT is operator-created and therefore environment
#     specific. This script asks the running server for it rather than hardcoding
#     a guess, and skips the FIX legs with a message when no acceptor exists.
#   - An lp-sim provider only streams into a book that lists it as a MEMBER. With
#     no such book the feed connects, logs "0 streams", and quotes nothing. This
#     script ensures one exists.
#
# Both are idempotent — an existing acceptor/book is used as-is, never replaced.
#
# ## Teardown
#
# Ctrl-C tears the fleet down, and the EXIT trap also sweeps by process IMAGE. A
# simulator is a long-lived client of the server: one that outlives its recorded
# pid keeps authenticating and pushing quotes into the NEXT run's books, showing
# up on the LP Panel as a provider matching nothing configured. That is the same
# orphan hazard `simsctl` closes on a deployed box.
#
# Usage:
#   ./run_dev.sh                  # build + run the whole stack
#   ./run_dev.sh --skip-build     # use the cached debug binaries
#   ./run_dev.sh --no-sims        # server + GUI only
#   ./run_dev.sh --no-gui         # server + sims only
#   ./run_dev.sh --demo-edge      # run the demo_edge example (what the e2e suites drive)
#
# Env overrides: CELNET_GRPC_ADDR, CELNET_WS_ADDR, CELNET_GUI_PORT,
#   CELNET_DEV_FIX_PORT (skip discovery), CELNET_DEV_SIM_USER/_PASSWORD,
#   CELNET_DEV_LPSIM_MEMBERS, CELNET_DEV_LPSIM_INTERVAL, CELNET_DEV_BOOK.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO_ROOT"

# CLAUDE.md sources $HOME/.cargo/env because rustup-curl installs put cargo there.
# Skip silently when absent (e.g. Homebrew rustup puts cargo on PATH directly).
if [[ -f "$HOME/.cargo/env" ]]; then
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi
command -v cargo >/dev/null 2>&1 || { echo "run_dev.sh: cargo not on PATH." >&2; exit 1; }

LOG_DIR="$REPO_ROOT/target/dev"
mkdir -p "$LOG_DIR"

GRPC_ADDR="${CELNET_GRPC_ADDR:-127.0.0.1:50551}"
WS_ADDR="${CELNET_WS_ADDR:-127.0.0.1:8081}"
GUI_PORT="${CELNET_GUI_PORT:-5173}"
SIM_USER="${CELNET_DEV_SIM_USER:-admin@celnet.com}"
SIM_PASSWORD="${CELNET_DEV_SIM_PASSWORD:-password}"
LPSIM_MEMBERS="${CELNET_DEV_LPSIM_MEMBERS:-4}"
LPSIM_INTERVAL="${CELNET_DEV_LPSIM_INTERVAL:-2}"
DEV_BOOK="${CELNET_DEV_BOOK:-lp-sim-book}"

SKIP_BUILD=0
RUN_GUI=1
RUN_SIMS=1
DEMO_EDGE=0
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=1 ;;
        --no-gui)     RUN_GUI=0 ;;
        --no-sims)    RUN_SIMS=0 ;;
        --demo-edge)  DEMO_EDGE=1 ;;
        -h|--help)
            awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"
            exit 0
            ;;
        *) echo "run_dev.sh: unknown flag: $arg" >&2; exit 2 ;;
    esac
done

# demo_edge is a self-contained example with its own seeded liquidity; layering
# the simulator fleet on top would double-feed its books.
if [[ $DEMO_EDGE -eq 1 && $RUN_SIMS -eq 1 ]]; then
    echo "[dev] --demo-edge implies --no-sims (the example seeds its own liquidity)."
    RUN_SIMS=0
fi

PIDS=()

signal() {
    local pid="$1" sig="$2"
    [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null && kill "-$sig" "$pid" 2>/dev/null || true
}

cleanup() {
    local code=$?
    trap - EXIT INT TERM
    echo
    echo "[dev] tearing down…"
    for pid in "${PIDS[@]:-}"; do signal "$pid" TERM; done
    sleep 1
    for pid in "${PIDS[@]:-}"; do signal "$pid" KILL; done
    pkill -9 -u "$(id -u)" -f 'target/debug/lp-sim|examples/fix_rfq_client' 2>/dev/null || true
    exit "$code"
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------------
# 1. Build
# ---------------------------------------------------------------------------
if [[ $SKIP_BUILD -eq 0 ]]; then
    if [[ $DEMO_EDGE -eq 1 ]]; then
        echo "[dev] building demo_edge (release)…"
        cargo build --release -p celnet-server --example demo_edge
    else
        echo "[dev] building server + simulators (debug — fast iteration)…"
        cargo build -p celnet-server --bin celnet-server
        if [[ $RUN_SIMS -eq 1 ]]; then
            cargo build -p celnet-lp-sim --bin lp-sim
            cargo build -p celnet-fix --example fix_rfq_client
        fi
    fi
fi

if [[ $DEMO_EDGE -eq 1 ]]; then
    SERVER_BIN="$REPO_ROOT/target/release/examples/demo_edge"
else
    SERVER_BIN="$REPO_ROOT/target/debug/celnet-server"
fi
[[ -x "$SERVER_BIN" ]] || { echo "[dev] $SERVER_BIN missing — re-run without --skip-build." >&2; exit 1; }

# ---------------------------------------------------------------------------
# 2. Server. Process substitution keeps $! on the REAL process — a `( … | tee )`
#    pipeline would make $! the subshell and orphan the server on teardown, and
#    that orphan then squats on the WS port so the next run dies with AddrInUse.
# ---------------------------------------------------------------------------
echo "[dev] starting server → gRPC $GRPC_ADDR  ws://$WS_ADDR"
CELNET_GRPC_ADDR="$GRPC_ADDR" CELNET_WS_ADDR="$WS_ADDR" CELNET_LOG_DIR="$LOG_DIR/logs" \
    "$SERVER_BIN" > >(tee "$LOG_DIR/edge.log" | sed -u 's/^/[edge] /') 2>&1 &
PIDS+=($!)

WS_HOST="${WS_ADDR%:*}"; WS_PORT="${WS_ADDR##*:}"
echo -n "[dev] waiting for ws://$WS_ADDR "
ready=0
for _ in $(seq 1 200); do
    if (exec 3<>"/dev/tcp/$WS_HOST/$WS_PORT") 2>/dev/null; then exec 3<&- 3>&- || true; ready=1; break; fi
    sleep 0.2; echo -n "."
done
echo
[[ $ready -eq 1 ]] || { echo "[dev] server never bound ws://$WS_ADDR — see $LOG_DIR/edge.log" >&2; exit 1; }

# ---------------------------------------------------------------------------
# 3. Provision what the fleet needs, over the server's own WS contract.
#    Prints `FIX_PORT=<n>` (empty when no acceptor exists) for the shell to read.
# ---------------------------------------------------------------------------
FIX_PORT="${CELNET_DEV_FIX_PORT:-}"
if [[ $RUN_SIMS -eq 1 ]]; then
    PROVISION_JS="$LOG_DIR/provision.mjs"
    cat > "$PROVISION_JS" <<'PROVISION'
import WebSocket from '../../gui/node_modules/ws/wrapper.mjs';
const [wsUrl, email, password, bookId, members] = process.argv.slice(2);
const ws = new WebSocket(wsUrl);
let id = 1; const pend = new Map();
const send = (type, body) => new Promise((res, rej) => {
  const c = id++; pend.set(c, { res, rej });
  ws.send(JSON.stringify({ type, correlation_id: c, ...body }));
  setTimeout(() => { if (pend.has(c)) { pend.delete(c); rej(new Error('timeout ' + type)); } }, 9000);
});
ws.on('message', (d) => {
  let m; try { m = JSON.parse(d.toString()); } catch { return; }
  if (m.correlation_id && pend.has(m.correlation_id)) {
    const p = pend.get(m.correlation_id); pend.delete(m.correlation_id); p.res(m);
  }
});
ws.on('error', (e) => { console.error('[dev] provision: ' + e.message); process.exit(0); });
ws.on('open', async () => {
  try {
    const login = await send('login', { email, password });
    const token = login.session_token;
    if (!token) { console.error('[dev] provision: login failed'); ws.close(); return; }

    // The FIX acceptor is operator-created, so its port is environment specific.
    // Prefer a fixed-income venue (what the RFQ/ESP legs speak); fall back to any
    // running acceptor rather than none.
    const conns = (await send('list_fix_connections', { session_token: token })).connections || [];
    const running = conns.filter((c) => c.running && c.bound_addr);
    const pick = running.find((c) => c.kind === 1 || c.kind === 2) || running[0];
    console.log('FIX_PORT=' + (pick ? String(pick.bound_addr).split(':').pop() : ''));

    // A provider only streams into a book that lists it as a member; with no such
    // book the feed connects and quotes nothing.
    const books = (await send('list_aggregated_books', { session_token: token })).books || [];
    if (!books.some((b) => b.id === bookId)) {
      const ids = Array.from({ length: Number(members) }, (_, i) => `LP-SIM-${String(i + 1).padStart(2, '0')}`);
      await send('create_aggregated_book', { session_token: token, spec: {
        name: bookId, member_connection_ids: ids,
        instrument_scope: { kind: 'all_members_quote' },
        params: { staleness_tau_ms: 30000, max_quote_age_ms: 60000,
                  divergence_gating: false, min_contributors: 1, depth_levels: 1 },
        enabled: true } });
      console.error(`[dev] created aggregated book '${bookId}' (${ids.join(', ')})`);
    } else {
      console.error(`[dev] aggregated book '${bookId}' already present — left as is`);
    }
  } catch (e) { console.error('[dev] provision: ' + e.message); }
  ws.close();
});
PROVISION
    if [[ -d "$REPO_ROOT/gui/node_modules/ws" ]]; then
        PROV_OUT="$(cd "$LOG_DIR" && node provision.mjs "ws://$WS_ADDR" "$SIM_USER" "$SIM_PASSWORD" "$DEV_BOOK" "$LPSIM_MEMBERS" 2>&1 || true)"
        echo "$PROV_OUT" | grep -v '^FIX_PORT=' || true
        DISCOVERED="$(echo "$PROV_OUT" | sed -n 's/^FIX_PORT=//p' | tail -1)"
        [[ -z "$FIX_PORT" ]] && FIX_PORT="$DISCOVERED"
    else
        echo "[dev] gui/node_modules absent — skipping auto-provision (run 'npm ci' in gui/)."
    fi
fi

# ---------------------------------------------------------------------------
# 4. Simulator fleet
# ---------------------------------------------------------------------------
if [[ $RUN_SIMS -eq 1 ]]; then
    echo "[dev] starting lp-sim ($LPSIM_MEMBERS providers → $GRPC_ADDR, book '$DEV_BOOK')"
    LPSIM_PASSWORD="$SIM_PASSWORD" \
        "$REPO_ROOT/target/debug/lp-sim" \
        --addr "http://$GRPC_ADDR" --members "$LPSIM_MEMBERS" \
        --interval "$LPSIM_INTERVAL" --user "$SIM_USER" \
        > >(tee "$LOG_DIR/lp-sim.log" | sed -u 's/^/[lp  ] /') 2>&1 &
    PIDS+=($!)

    if [[ -n "$FIX_PORT" ]]; then
        FIX_CLIENT="$REPO_ROOT/target/debug/examples/fix_rfq_client"
        echo "[dev] starting FIX RFQ leg → 127.0.0.1:$FIX_PORT"
        "$FIX_CLIENT" --addr "127.0.0.1:$FIX_PORT" --asset fi --intent rfq \
            --repeat 0 --interval 3 \
            > >(tee "$LOG_DIR/fix-rfq.log" | sed -u 's/^/[rfq ] /') 2>&1 &
        PIDS+=($!)
        echo "[dev] starting FIX ESP leg → 127.0.0.1:$FIX_PORT"
        "$FIX_CLIENT" --addr "127.0.0.1:$FIX_PORT" --asset esp \
            --grpc-addr "http://$GRPC_ADDR" --repeat 0 --interval 3 \
            > >(tee "$LOG_DIR/fix-esp.log" | sed -u 's/^/[esp ] /') 2>&1 &
        PIDS+=($!)
    else
        echo "[dev] no running FIX acceptor — SKIPPING the RFQ + ESP legs."
        echo "[dev]   Define one in Administration → Connections, then re-run."
    fi
fi

# ---------------------------------------------------------------------------
# 5. GUI
# ---------------------------------------------------------------------------
if [[ $RUN_GUI -eq 1 ]]; then
    echo "[dev] starting GUI  → http://localhost:$GUI_PORT"
    ( cd "$REPO_ROOT/gui" && exec npx vite --port "$GUI_PORT" --strictPort ) \
        > >(tee "$LOG_DIR/gui.log" | sed -u 's/^/[gui ] /') 2>&1 &
    PIDS+=($!)
fi

echo
echo "[dev] ── stack up ───────────────────────────────────────────────"
[[ $RUN_GUI -eq 1 ]] && echo "[dev]   GUI      http://localhost:$GUI_PORT   ($SIM_USER / $SIM_PASSWORD)"
echo "[dev]   WS       ws://$WS_ADDR        gRPC  $GRPC_ADDR"
if [[ $RUN_SIMS -eq 1 ]]; then
    echo "[dev]   sims     lp-sim x$LPSIM_MEMBERS → book '$DEV_BOOK'"
    [[ -n "$FIX_PORT" ]] && echo "[dev]            FIX RFQ + ESP legs → 127.0.0.1:$FIX_PORT"
    echo "[dev]   LP Panel Administration → LP Panel"
fi
echo "[dev]   logs     $LOG_DIR/{edge,lp-sim,fix-rfq,fix-esp,gui}.log"
echo "[dev] ─────────────────────────────────────────────── Ctrl-C to stop ──"
wait
