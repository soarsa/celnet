#!/usr/bin/env bash
# tools/dev.sh — single-command local bring-up of the Celnet backend edge + GUI.
#
# Starts:
#   1. celnet-server `demo_edge` example (the same edge the GUI/Excel e2e suites
#      drive) on gRPC 127.0.0.1:50551 + WS 127.0.0.1:8081.
#   2. gui Vite dev server on http://localhost:5173.
#
# Streams both logs prefixed (`[edge]` / `[gui ]`) AND tees full output to
# target/dev/{edge,gui}.log so a panic stack survives the scrollback. Ctrl-C
# tears both down cleanly.
#
# Env overrides honored by demo_edge (see crates/celnet-server/examples/demo_edge.rs):
#   CELNET_GRPC_ADDR, CELNET_WS_ADDR, CELNET_FIX_ADDR, CELNET_DEMO_LPS,
#   CELNET_ACCESS_MODE.
#
# Usage:
#   tools/dev.sh             # build + run edge + GUI
#   tools/dev.sh --skip-build # skip release rebuild (use the cached binary)
#   tools/dev.sh --no-gui     # backend only

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# CLAUDE.md sources $HOME/.cargo/env because rustup-curl installs put cargo there
# and the shell doesn't persist env between Claude tool calls. Skip silently when
# it doesn't exist (e.g. Homebrew rustup, which puts cargo on PATH directly).
if [[ -f "$HOME/.cargo/env" ]]; then
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "dev.sh: cargo not on PATH. Install rustup or fix your shell." >&2
    exit 1
fi

LOG_DIR="$REPO_ROOT/target/dev"
mkdir -p "$LOG_DIR"
EDGE_LOG="$LOG_DIR/edge.log"
GUI_LOG="$LOG_DIR/gui.log"

GRPC_ADDR="${CELNET_GRPC_ADDR:-127.0.0.1:50551}"
WS_ADDR="${CELNET_WS_ADDR:-127.0.0.1:8081}"
GUI_PORT="${CELNET_GUI_PORT:-5173}"

SKIP_BUILD=0
RUN_GUI=1
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=1 ;;
        --no-gui)     RUN_GUI=0 ;;
        -h|--help)
            # Print the leading comment block (every line starting with `# `).
            awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"
            exit 0
            ;;
        *)
            echo "dev.sh: unknown flag: $arg" >&2
            exit 2
            ;;
    esac
done

EDGE_PID=""
GUI_PID=""

# EDGE_PID / GUI_PID point at the real server processes (see the spawn blocks —
# process substitution + `exec vite` keep $! off the subshell/npm wrapper), so a
# plain TERM-then-KILL reaps them and frees the ports. The tee/sed log relays are
# fed by process substitution and exit on their own once the server's fd closes.
signal() {
    local pid="$1" sig="$2"
    [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null && kill "-$sig" "$pid" 2>/dev/null || true
}

cleanup() {
    local code=$?
    trap - EXIT INT TERM
    echo
    echo "[dev] tearing down…"
    signal "$GUI_PID"  TERM
    signal "$EDGE_PID" TERM
    for _ in 1 2 3 4 5; do
        local alive=0
        [[ -n "$GUI_PID"  ]] && kill -0 "$GUI_PID"  2>/dev/null && alive=1
        [[ -n "$EDGE_PID" ]] && kill -0 "$EDGE_PID" 2>/dev/null && alive=1
        [[ $alive -eq 0 ]] && break
        sleep 0.2
    done
    signal "$GUI_PID"  KILL
    signal "$EDGE_PID" KILL
    exit "$code"
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------------
# 1. Build the edge in release first (cold cargo build under the GUI's wait
#    silently inflates startup; the env-lesson in justfile §t2 calls this out).
# ---------------------------------------------------------------------------
if [[ $SKIP_BUILD -eq 0 ]]; then
    echo "[dev] building celnet-server demo_edge (release)…"
    cargo build --release -p celnet-server --example demo_edge
fi

EDGE_BIN="$REPO_ROOT/target/release/examples/demo_edge"
if [[ ! -x "$EDGE_BIN" ]]; then
    echo "[dev] $EDGE_BIN missing — re-run without --skip-build." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# 2. Spawn the edge. Process substitution (`> >(tee | sed)`) keeps the tee→log
#    + `[edge]` prefix, while leaving $! pointing at the *real* demo_edge process
#    (a `( … | tee )` pipeline would make $! the subshell and orphan the server
#    on teardown — that orphan then squats on the WS port and the next run dies
#    with AddrInUse).
# ---------------------------------------------------------------------------
echo "[dev] starting edge   → gRPC $GRPC_ADDR  ws://$WS_ADDR  (log: $EDGE_LOG)"
CELNET_GRPC_ADDR="$GRPC_ADDR" \
CELNET_WS_ADDR="$WS_ADDR" \
    "$EDGE_BIN" > >(tee "$EDGE_LOG" | sed -u 's/^/[edge] /') 2>&1 &
EDGE_PID=$!

# ---------------------------------------------------------------------------
# 3. Wait for the WS port to be listening before launching the GUI. Pure-bash
#    /dev/tcp probe — no nc/curl dependency. ~30 s ceiling.
# ---------------------------------------------------------------------------
WS_HOST="${WS_ADDR%:*}"
WS_PORT="${WS_ADDR##*:}"
echo -n "[dev] waiting for ws://$WS_ADDR "
ready=0
for _ in $(seq 1 150); do
    if (exec 3<>"/dev/tcp/$WS_HOST/$WS_PORT") 2>/dev/null; then
        exec 3<&- 3>&- || true
        ready=1
        break
    fi
    if ! kill -0 "$EDGE_PID" 2>/dev/null; then
        echo
        echo "[dev] edge exited before binding $WS_ADDR — see $EDGE_LOG" >&2
        exit 1
    fi
    sleep 0.2
    echo -n "."
done
if [[ $ready -ne 1 ]]; then
    echo
    echo "[dev] timed out waiting for $WS_ADDR — see $EDGE_LOG" >&2
    exit 1
fi
echo " ready."

# ---------------------------------------------------------------------------
# 4. (Optional) GUI Vite dev server. npm install on first run if node_modules
#    isn't there; otherwise straight to `npm run dev`.
# ---------------------------------------------------------------------------
if [[ $RUN_GUI -eq 1 ]]; then
    if ! command -v npm >/dev/null 2>&1; then
        echo "[dev] npm not found on PATH — install Node ≥ 22 or rerun with --no-gui." >&2
        exit 1
    fi
    # Guard on the actual `vite` binary, not just node_modules/: a partial or
    # interrupted install leaves the directory present but the dev entrypoint
    # missing, which used to slip past a bare `-d node_modules` check and fail
    # later with `vite: command not found`. `npm ci` needs a lockfile; fall back
    # to `npm install` (which writes one) when package-lock.json is absent.
    if [[ ! -x "$REPO_ROOT/gui/node_modules/.bin/vite" ]]; then
        if [[ -f "$REPO_ROOT/gui/package-lock.json" ]]; then
            echo "[dev] installing GUI deps (npm ci)…"
            npm --prefix "$REPO_ROOT/gui" ci
        else
            echo "[dev] installing GUI deps (npm install — no lockfile)…"
            npm --prefix "$REPO_ROOT/gui" install
        fi
    fi
    echo "[dev] starting GUI    → http://localhost:$GUI_PORT  (log: $GUI_LOG)"
    # Run the vite binary directly (the guard above guarantees it exists) instead
    # of `npm run dev`: npm spawns vite as a *child* and does not forward signals,
    # so killing npm orphans vite onto the port. `( cd … && exec vite )` replaces
    # the subshell with vite in place, so $! is vite's real PID and a plain kill
    # reaps it. `package.json`'s `dev` script is exactly `vite`, so behavior is
    # identical (vite reads gui/vite.config from the cd'd cwd).
    (
        cd "$REPO_ROOT/gui" && exec node_modules/.bin/vite \
            --port "$GUI_PORT" --strictPort
    ) > >(tee "$GUI_LOG" | sed -u 's/^/[gui ] /') 2>&1 &
    GUI_PID=$!
fi

echo "[dev] up. Ctrl-C to stop."

# ---------------------------------------------------------------------------
# 5. Wait. If either child dies, tear the other down and exit with its code.
# ---------------------------------------------------------------------------
while :; do
    if [[ -n "$EDGE_PID" ]] && ! kill -0 "$EDGE_PID" 2>/dev/null; then
        wait "$EDGE_PID" 2>/dev/null || rc=$?
        echo "[dev] edge exited (rc=${rc:-0})." >&2
        exit "${rc:-1}"
    fi
    if [[ -n "$GUI_PID" ]] && ! kill -0 "$GUI_PID" 2>/dev/null; then
        wait "$GUI_PID" 2>/dev/null || rc=$?
        echo "[dev] gui exited (rc=${rc:-0})." >&2
        exit "${rc:-1}"
    fi
    sleep 0.5
done
