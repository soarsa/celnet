#!/usr/bin/env bash
# start-lp-sim.sh — run the celnet LP-SIM Treasury liquidity feed and log every
# consolidated composite it emits (per-bond best bid/offer + size + confidence +
# the contributing LP-SIM connections) to a file, so an operator can watch the feed
# with `tail -f`.
#
# LP-SIM is a synthetic liquidity provider: it loads the bundled US-Treasury
# reference universe, stands up a named `LP-SIM` member panel over it, and streams a
# stochastic two-way for every selected bond through the REAL celnet-aggregation
# consolidation engine — the same engine an operator-defined FI Aggregated Book uses.
# The lp-sim binary is self-contained (the universe is embedded) and loops on its own
# --interval, so this launcher just resolves the binary, daemonizes it with a PID
# file + log, and supervises a restart if it ever exits.
#
# Usage:
#   deploy/start-lp-sim.sh                # stream composites in the foreground (tees to log)
#   LPSIM_ONESHOT=1 deploy/start-lp-sim.sh   # emit ONE composite round and exit
#   LPSIM_DAEMON=1  deploy/start-lp-sim.sh   # background, write a PID file, log to file
#   tail -f deploy/lp-sim-run/log/lp-sim.log # watch the feed
#
# Env overrides (defaults match group_vars celnet_lpsim_* settings):
#   LPSIM_LP_NAME (LP-SIM)         # the LP connection name shown as the contributor
#   LPSIM_BOOK (ust-composite)     # the aggregated-book id label
#   LPSIM_MEMBERS (4)              # decorrelated LP member connections (>=3 for gating)
#   LPSIM_INTERVAL (2)             # seconds between composite emissions
#   LPSIM_INSTRUMENTS (all)        # all, or a CSV of ISINs/CUSIPs
#   LPSIM_MAX_INSTRUMENTS (12)     # cap on instruments streamed (log readability)
#   LPSIM_SEED (305419896)         # reproducibility seed
#   LPSIM_SETTLEMENT (2026-04-16)  # valuation date (YYYY-MM-DD)
#   LPSIM_INCLUDE_BILLS (0)        # 1 = also stream zero-coupon Bills
#   LPSIM_BIN (target/release/lp-sim)   # preferred prebuilt binary
#   LPSIM_RELEASE_BIN ()           # release-shipped binary (used ON the server, no cargo)
#   LPSIM_RUN_DIR (deploy/lp-sim-run)  LPSIM_LOG (<run>/log/lp-sim.log)
#   LPSIM_ONESHOT (0)  LPSIM_DAEMON (0)
set -euo pipefail

LPSIM_LP_NAME="${LPSIM_LP_NAME:-LP-SIM}"
LPSIM_BOOK="${LPSIM_BOOK:-ust-composite}"
LPSIM_MEMBERS="${LPSIM_MEMBERS:-4}"
LPSIM_INTERVAL="${LPSIM_INTERVAL:-2}"
LPSIM_INSTRUMENTS="${LPSIM_INSTRUMENTS:-all}"
LPSIM_MAX_INSTRUMENTS="${LPSIM_MAX_INSTRUMENTS:-12}"
LPSIM_SEED="${LPSIM_SEED:-305419896}"
LPSIM_SETTLEMENT="${LPSIM_SETTLEMENT:-2026-04-16}"
LPSIM_INCLUDE_BILLS="${LPSIM_INCLUDE_BILLS:-0}"
LPSIM_ONESHOT="${LPSIM_ONESHOT:-0}"
LPSIM_DAEMON="${LPSIM_DAEMON:-0}"
# Network feed mode: when set to a server gRPC endpoint (e.g.
# http://127.0.0.1:50051), lp-sim streams LpQuotes to the server's LpFeed ingest so
# the composite surfaces to GUI subscribers, instead of printing it locally. Absent
# ⇒ the local in-process composite print (the prior behaviour).
LPSIM_ADDR="${LPSIM_ADDR:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

RUN_DIR="${LPSIM_RUN_DIR:-$REPO_ROOT/deploy/lp-sim-run}"
mkdir -p "$RUN_DIR/log"
LOG="${LPSIM_LOG:-$RUN_DIR/log/lp-sim.log}"
PID_FILE="$RUN_DIR/lp-sim.pid"
LPSIM_BIN="${LPSIM_BIN:-$REPO_ROOT/target/release/lp-sim}"

# Source the Rust toolchain (shell does not persist env on this estate).
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

ts() { date +%Y-%m-%dT%H:%M:%S%z; }
log() { echo "[$(ts)] [lp-sim] $*"; }

# --- Daemon: re-exec self into the logfile, record the child PID, and return. --
if [ "$LPSIM_DAEMON" = "1" ] && [ "${LPSIM__CHILD:-0}" != "1" ]; then
  touch "$LOG"
  LPSIM__CHILD=1 nohup "$0" "$@" >>"$LOG" 2>&1 &
  echo $! >"$PID_FILE"
  echo "[lp-sim] daemonized pid $(cat "$PID_FILE"); logging to $LOG"
  echo "[lp-sim] follow with:  tail -f $LOG"
  exit 0
fi

# --- Foreground: mirror all output to the logfile so a watching terminal sees --
# exactly what lands in the log. (The daemon child already redirects to the log.)
if [ "${LPSIM__CHILD:-0}" != "1" ]; then
  exec > >(tee -a "$LOG") 2>&1
fi

log "run dir $RUN_DIR ; log $LOG"
log "feed $LPSIM_LP_NAME -> book $LPSIM_BOOK ; members $LPSIM_MEMBERS ; interval ${LPSIM_INTERVAL}s ; instruments $LPSIM_INSTRUMENTS"

# --- Resolve the runner up front so a build failure is LOUD, not hidden ---------
RUNNER=()
if [ -x "$LPSIM_BIN" ]; then
  log "using lp-sim binary $LPSIM_BIN"
  RUNNER=("$LPSIM_BIN")
elif [ -n "${LPSIM_RELEASE_BIN:-}" ] && [ -x "${LPSIM_RELEASE_BIN:-}" ]; then
  # A prebuilt binary shipped by the release (deploy/roles/celnet_release installs it
  # at <release>/bin/lp-sim). This is the path used ON the server, where the celnet
  # user has no cargo to build at runtime.
  log "using release-shipped lp-sim $LPSIM_RELEASE_BIN"
  RUNNER=("$LPSIM_RELEASE_BIN")
elif command -v cargo >/dev/null 2>&1; then
  log "no lp-sim binary yet — building it (cargo build --release -p celnet-lp-sim --bin lp-sim)..."
  if ! cargo build --release -p celnet-lp-sim --bin lp-sim; then
    log "ERROR: failed to build lp-sim — see the build output above."
    exit 1
  fi
  RUNNER=("$REPO_ROOT/target/release/lp-sim")
else
  log "ERROR: no lp-sim binary at $LPSIM_BIN and cargo is unavailable — nothing to run."
  exit 1
fi

# Build the client argv from the resolved env.
build_args() {
  ARGS=("${RUNNER[@]}" \
    --lp-name "$LPSIM_LP_NAME" --book "$LPSIM_BOOK" \
    --members "$LPSIM_MEMBERS" --interval "$LPSIM_INTERVAL" \
    --instruments "$LPSIM_INSTRUMENTS" --max-instruments "$LPSIM_MAX_INSTRUMENTS" \
    --seed "$LPSIM_SEED" --settlement "$LPSIM_SETTLEMENT")
  if [ "$LPSIM_INCLUDE_BILLS" = "1" ]; then ARGS+=(--include-bills); fi
  # Network feed: push LpQuotes to the server ingest so the composite surfaces to
  # GUI subscribers (mirrors how the FIX sim passes --addr to its acceptor).
  if [ -n "$LPSIM_ADDR" ]; then ARGS+=(--addr "$LPSIM_ADDR"); fi
}

if [ "$LPSIM_ONESHOT" = "1" ]; then
  build_args
  "${ARGS[@]}" --once
  exit 0
fi

# The lp-sim binary loops on its own --interval; supervise it so a crash restarts.
build_args
log "streaming composites every ${LPSIM_INTERVAL}s (Ctrl-C to stop)."
while true; do
  "${ARGS[@]}" || log "lp-sim exited ($?) — restarting in 5s"
  sleep 5
done
