#!/usr/bin/env bash
# tools/dev-sims.sh — local bring-up of the FULL stack: the real celnet-server,
# the GUI, and the whole simulator fleet.
#
# `tools/dev.sh` runs the `demo_edge` example, which is the right target for the
# e2e suites but is NOT the binary a box deploys and carries no inbound simulator
# fleet. This script brings up what a deployed environment actually runs, so a
# feature can be exercised against real inbound liquidity:
#
#   1. celnet-server        gRPC 127.0.0.1:50551 · ws://127.0.0.1:8081
#   2. lp-sim               N liquidity providers -> the LpFeed gRPC ingest
#   3. fix_rfq_client (RFQ) one-shot quote requests against the FIX acceptor
#   4. fix_rfq_client (ESP) Market-Data streaming off reference data
#   5. gui                  Vite dev server on http://localhost:5173
#
# Those are the four inbound connections the Administration -> LP Panel reports
# on. Legs 3 and 4 need a FIX acceptor to dial, so they start only when one is
# reachable — a missing acceptor SKIPS the leg with a message rather than leaving
# a client retrying into a closed port forever.
#
# Ctrl-C tears the fleet down, and the EXIT trap also sweeps by process image: a
# simulator is a long-lived client of the server, and one left running keeps
# authenticating and pushing quotes into the next run's books (the same orphan
# hazard `simsctl` closes on a box).
#
# Usage:
#   tools/dev-sims.sh                 # build + run everything
#   tools/dev-sims.sh --skip-build    # use the cached debug binaries
#   tools/dev-sims.sh --no-gui        # backend + sims only
#   tools/dev-sims.sh --no-sims       # server + GUI only (real binary, no fleet)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if [[ -f "$HOME/.cargo/env" ]]; then
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi
command -v cargo >/dev/null 2>&1 || { echo "dev-sims.sh: cargo not on PATH." >&2; exit 1; }

LOG_DIR="$REPO_ROOT/target/dev"
mkdir -p "$LOG_DIR"

GRPC_ADDR="${CELNET_GRPC_ADDR:-127.0.0.1:50551}"
WS_ADDR="${CELNET_WS_ADDR:-127.0.0.1:8081}"
GUI_PORT="${CELNET_GUI_PORT:-5173}"
# The FIX acceptor the client legs dial. Defined in the GUI (Administration ->
# Connections) and persisted, so it exists only once an operator has made one.
FIX_HOST="${CELNET_DEV_FIX_HOST:-127.0.0.1}"
FIX_PORT="${CELNET_DEV_FIX_PORT:-56001}"
# The service credential the sims authenticate with. The local dev seed is the
# admin account; a box uses per-simulator least-privilege identities instead
# (docs/SIMULATOR-SERVICE-IDENTITIES.md).
SIM_USER="${CELNET_DEV_SIM_USER:-admin@celnet.com}"
SIM_PASSWORD="${CELNET_DEV_SIM_PASSWORD:-password}"
LPSIM_MEMBERS="${CELNET_DEV_LPSIM_MEMBERS:-4}"
LPSIM_INTERVAL="${CELNET_DEV_LPSIM_INTERVAL:-2}"

SKIP_BUILD=0
RUN_GUI=1
RUN_SIMS=1
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=1 ;;
        --no-gui)     RUN_GUI=0 ;;
        --no-sims)    RUN_SIMS=0 ;;
        -h|--help)
            awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"
            exit 0
            ;;
        *) echo "dev-sims.sh: unknown flag: $arg" >&2; exit 2 ;;
    esac
done

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
    # Sweep by process image as well as by pid. A simulator that outlived its
    # recorded pid keeps pushing quotes into the next run's books, which shows up
    # as a phantom provider on the LP Panel matching nothing configured.
    pkill -9 -u "$(id -u)" -f 'target/debug/lp-sim|examples/fix_rfq_client' 2>/dev/null || true
    exit "$code"
}
trap cleanup EXIT INT TERM

# --- 1. build -------------------------------------------------------------
if [[ $SKIP_BUILD -eq 0 ]]; then
    echo "[dev] building server + simulators (debug — fast iteration)…"
    cargo build -p celnet-server --bin celnet-server
    if [[ $RUN_SIMS -eq 1 ]]; then
        cargo build -p celnet-lp-sim --bin lp-sim
        cargo build -p celnet-fix --example fix_rfq_client
    fi
fi

SERVER_BIN="$REPO_ROOT/target/debug/celnet-server"
[[ -x "$SERVER_BIN" ]] || { echo "[dev] $SERVER_BIN missing — re-run without --skip-build." >&2; exit 1; }

# --- 2. server ------------------------------------------------------------
# Process substitution keeps $! pointing at the REAL process (a `( … | tee )`
# pipeline would make $! the subshell and orphan the server on teardown — that
# orphan then squats on the WS port and the next run dies with AddrInUse).
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

# --- 3. simulator fleet ---------------------------------------------------
if [[ $RUN_SIMS -eq 1 ]]; then
    echo "[dev] starting lp-sim ($LPSIM_MEMBERS providers → $GRPC_ADDR)"
    LPSIM_PASSWORD="$SIM_PASSWORD" \
        "$REPO_ROOT/target/debug/lp-sim" \
        --addr "http://$GRPC_ADDR" --members "$LPSIM_MEMBERS" \
        --interval "$LPSIM_INTERVAL" --user "$SIM_USER" \
        > >(tee "$LOG_DIR/lp-sim.log" | sed -u 's/^/[lp  ] /') 2>&1 &
    PIDS+=($!)

    # The FIX legs need an acceptor. Probe rather than assume: an unreachable
    # port means no acceptor is defined yet (Administration → Connections), and a
    # client left dialing a closed port just fills the log with reconnects.
    if (exec 3<>"/dev/tcp/$FIX_HOST/$FIX_PORT") 2>/dev/null; then
        exec 3<&- 3>&- || true
        FIX_CLIENT="$REPO_ROOT/target/debug/examples/fix_rfq_client"
        echo "[dev] starting FIX RFQ leg → $FIX_HOST:$FIX_PORT"
        "$FIX_CLIENT" --addr "$FIX_HOST:$FIX_PORT" --asset fi --intent rfq \
            --repeat 0 --interval 3 \
            > >(tee "$LOG_DIR/fix-rfq.log" | sed -u 's/^/[rfq ] /') 2>&1 &
        PIDS+=($!)
        echo "[dev] starting FIX ESP leg → $FIX_HOST:$FIX_PORT"
        "$FIX_CLIENT" --addr "$FIX_HOST:$FIX_PORT" --asset esp \
            --grpc-addr "http://$GRPC_ADDR" --repeat 0 --interval 3 \
            > >(tee "$LOG_DIR/fix-esp.log" | sed -u 's/^/[esp ] /') 2>&1 &
        PIDS+=($!)
    else
        echo "[dev] no FIX acceptor on $FIX_HOST:$FIX_PORT — SKIPPING the RFQ + ESP legs."
        echo "[dev]   Define one in Administration → Connections, then re-run"
        echo "[dev]   (or set CELNET_DEV_FIX_PORT to an existing acceptor)."
    fi
fi

# --- 4. GUI ---------------------------------------------------------------
if [[ $RUN_GUI -eq 1 ]]; then
    echo "[dev] starting GUI  → http://localhost:$GUI_PORT"
    ( cd "$REPO_ROOT/gui" && exec npx vite --port "$GUI_PORT" --strictPort ) \
        > >(tee "$LOG_DIR/gui.log" | sed -u 's/^/[gui ] /') 2>&1 &
    PIDS+=($!)
fi

echo
echo "[dev] ── stack up ───────────────────────────────────────────────"
echo "[dev]   GUI      http://localhost:$GUI_PORT"
echo "[dev]   WS       ws://$WS_ADDR"
echo "[dev]   gRPC     $GRPC_ADDR"
echo "[dev]   logs     $LOG_DIR/{edge,lp-sim,fix-rfq,fix-esp,gui}.log"
echo "[dev]   sign in  $SIM_USER / $SIM_PASSWORD"
echo "[dev]   LP Panel Administration → LP Panel (needs an aggregated book whose"
echo "[dev]            members are LP-SIM-01..0$LPSIM_MEMBERS — Administration → Aggregation)"
echo "[dev] ─────────────────────────────────────────────── Ctrl-C to stop ──"
wait
