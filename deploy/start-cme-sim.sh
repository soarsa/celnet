#!/usr/bin/env bash
# start-cme-sim.sh — run the celnet listed Treasury-futures venue simulator and log
# everything it emits to a file, so an operator can watch the venue with `tail -f`.
#
# cme-sim is a SINGLE simulated exchange (connection id `cme-sim`). It quotes the
# Treasury futures complex (ZT/ZF/ZN/TN/ZB/UB) from the reference registry's REAL
# contract terms — face value, minimum price increment, tick value, notional coupon,
# delivery dates, derived DV01 per contract — anchored to the real cash Treasury
# curve, publishes whole-contract tick-aligned two-ways into the server's LpFeed
# ingest, and ACCEPTS ORDERS against them over FIX (answering each with a real
# ExecutionReport(8)).
#
# It is a first-class peer of lp-sim: same flag style, self-contained (the universe
# is embedded), loops on its own --interval, so this launcher just resolves the
# binary, daemonizes it with a PID file + log, and supervises a restart.
#
# !! OPERATOR NOTE — BREAKING CHANGE TO AN AGGREGATED BOOK !!
#    Treasury futures are no longer quoted by the OTC panel (lp-sim). An aggregated
#    book that carries futures MUST list `cme-sim` in its member_connection_ids, or
#    those lines will have no contributor and every futures hedge routed at them will
#    backstop to the synthetic COMPOSITE venue.
#
# Usage:
#   deploy/start-cme-sim.sh                    # foreground (tees to log)
#   CMESIM_ONESHOT=1 deploy/start-cme-sim.sh   # publish ONE round and exit
#   CMESIM_DAEMON=1  deploy/start-cme-sim.sh   # background, PID file, log to file
#   tail -f deploy/cme-sim-run/log/cme-sim.log # watch the venue
#
# Env overrides (defaults match group_vars celnet_cmesim_* settings):
#   CMESIM_INTERVAL (2)            # seconds between quote rounds
#   CMESIM_INTERVAL_MS ()          # sub-second override (ms); empty ⇒ whole seconds
#   CMESIM_SEED (305419896)        # reproducibility seed
#   CMESIM_SETTLEMENT (2026-04-16) # valuation date; also the front-month roll date
#   CMESIM_ADDR ()                 # server gRPC endpoint; empty ⇒ local print mode
#   CMESIM_ORDER_PORT (0.0.0.0:5710)  # where the FIX order acceptor binds
#   CMESIM_BIN (target/release/cme-sim)   # preferred prebuilt binary
#   CMESIM_RELEASE_BIN ()          # release-shipped binary (used ON the server)
#   CMESIM_RUN_DIR (deploy/cme-sim-run)  CMESIM_LOG (<run>/log/cme-sim.log)
#   CMESIM_ONESHOT (0)  CMESIM_DAEMON (0)
set -euo pipefail

CMESIM_INTERVAL="${CMESIM_INTERVAL:-2}"
CMESIM_INTERVAL_MS="${CMESIM_INTERVAL_MS:-}"
CMESIM_SEED="${CMESIM_SEED:-305419896}"
CMESIM_SETTLEMENT="${CMESIM_SETTLEMENT:-2026-04-16}"
CMESIM_ADDR="${CMESIM_ADDR:-}"
# The FIX order acceptor. A venue that quotes but will not trade is a price display,
# so this is set by default; pass an empty string to publish prices only.
CMESIM_ORDER_PORT="${CMESIM_ORDER_PORT:-0.0.0.0:5710}"
CMESIM_ONESHOT="${CMESIM_ONESHOT:-0}"
CMESIM_DAEMON="${CMESIM_DAEMON:-0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

RUN_DIR="${CMESIM_RUN_DIR:-$REPO_ROOT/deploy/cme-sim-run}"
mkdir -p "$RUN_DIR/log"
LOG="${CMESIM_LOG:-$RUN_DIR/log/cme-sim.log}"
PID_FILE="$RUN_DIR/cme-sim.pid"
CMESIM_BIN="${CMESIM_BIN:-$REPO_ROOT/target/release/cme-sim}"

# Source the Rust toolchain (shell does not persist env on this estate).
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

ts() { date +%Y-%m-%dT%H:%M:%S%z; }
log() { echo "[$(ts)] [cme-sim] $*"; }

# --- RESTART, don't ACCUMULATE ------------------------------------------------
# Every simulator is a long-lived CLIENT of celnet-server sitting in a reconnect
# loop; a relaunch that does not stop the previous generation leaks a new one on top
# (this is exactly how 10+ orphaned lp-sim daemons piled up on the UAT box). Runs
# ONLY in the parent invocation, so a fresh venue never kills itself or its sibling.
if [ "${CMESIM__CHILD:-0}" != "1" ]; then
  __self=$$; __parent=${PPID:-0}; __uid="$(id -u)"
  for __pid in $(pgrep -u "$__uid" -f 'cme-sim' 2>/dev/null || true); do
    case "$__pid" in "$__self"|"$__parent") continue ;; esac
    kill "$__pid" 2>/dev/null || true
  done
  sleep 1
  # Belt-and-braces: SIGKILL any cme-sim BINARY still up. The daemon invocation
  # always carries `--settlement`; this pattern matches the binary, not this script.
  pkill -9 -u "$__uid" -f 'cme-sim --settlement' 2>/dev/null || true
  rm -f "$PID_FILE" 2>/dev/null || true
fi

# --- Daemon: re-exec self into the logfile, record the child PID, and return. --
if [ "$CMESIM_DAEMON" = "1" ] && [ "${CMESIM__CHILD:-0}" != "1" ]; then
  touch "$LOG"
  CMESIM__CHILD=1 nohup "$0" "$@" >>"$LOG" 2>&1 &
  echo $! >"$PID_FILE"
  echo "[cme-sim] daemonized pid $(cat "$PID_FILE"); logging to $LOG"
  echo "[cme-sim] follow with:  tail -f $LOG"
  exit 0
fi

# --- Foreground: mirror all output to the logfile ------------------------------
if [ "${CMESIM__CHILD:-0}" != "1" ]; then
  exec > >(tee -a "$LOG") 2>&1
fi

log "run dir $RUN_DIR ; log $LOG"
log "venue cme-sim ; interval ${CMESIM_INTERVAL}s ; settlement $CMESIM_SETTLEMENT ; orders ${CMESIM_ORDER_PORT:-DISABLED}"

# --- Resolve the runner up front so a build failure is LOUD, not hidden --------
RUNNER=()
if [ -x "$CMESIM_BIN" ]; then
  log "using cme-sim binary $CMESIM_BIN"
  RUNNER=("$CMESIM_BIN")
elif [ -n "${CMESIM_RELEASE_BIN:-}" ] && [ -x "${CMESIM_RELEASE_BIN:-}" ]; then
  # A prebuilt binary shipped by the release (deploy/roles/celnet_release installs it
  # at <release>/bin/cme-sim). This is the path used ON the server, where the celnet
  # user has no cargo to build at runtime.
  log "using release-shipped cme-sim $CMESIM_RELEASE_BIN"
  RUNNER=("$CMESIM_RELEASE_BIN")
elif command -v cargo >/dev/null 2>&1; then
  log "no cme-sim binary yet — building it (cargo build --release -p celnet-cme-sim --bin cme-sim)..."
  if ! cargo build --release -p celnet-cme-sim --bin cme-sim; then
    log "ERROR: failed to build cme-sim — see the build output above."
    exit 1
  fi
  RUNNER=("$REPO_ROOT/target/release/cme-sim")
else
  log "ERROR: no cme-sim binary at $CMESIM_BIN and cargo is unavailable — nothing to run."
  exit 1
fi

# Build the venue argv from the resolved env.
build_args() {
  ARGS=("${RUNNER[@]}" \
    --settlement "$CMESIM_SETTLEMENT" \
    --interval "$CMESIM_INTERVAL" \
    --seed "$CMESIM_SEED")
  [ -n "$CMESIM_INTERVAL_MS" ] && ARGS+=(--interval-ms "$CMESIM_INTERVAL_MS")
  [ -n "$CMESIM_ADDR" ] && ARGS+=(--addr "$CMESIM_ADDR")
  [ -n "$CMESIM_ORDER_PORT" ] && ARGS+=(--order-port "$CMESIM_ORDER_PORT")
  # IMPORTANT: end with an explicit success. The last expression above is a
  # short-circuit `[ … ] && …` that returns 1 whenever the variable is empty; as the
  # function's final command that would make build_args return 1, and under `set -e`
  # the caller would abort BEFORE ever launching cme-sim — the daemon then logs its
  # banner and dies silently. (This exact bug bit start-lp-sim.sh.)
  return 0
}

if [ "$CMESIM_ONESHOT" = "1" ]; then
  build_args
  "${ARGS[@]}" --once
  exit 0
fi

# The cme-sim binary loops on its own --interval; supervise it so a crash restarts.
build_args
log "publishing every ${CMESIM_INTERVAL}s (Ctrl-C to stop)."
while true; do
  "${ARGS[@]}" || log "cme-sim exited ($?) — restarting in 5s"
  sleep 5
done
