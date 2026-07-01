#!/usr/bin/env bash
# start-fix-sim.sh — launch the celnet FIX client simulator on the UAT box.
#
# Design: docs/FIX-SIM-DESIGN.md. This script is the UAT entrypoint. It:
#   * sources the Rust env and creates the store/ + log/ run dirs
#     (mirrors the QuickFIX FileStorePath / FileLogPath),
#   * TCP-preflights the acceptor host:port,
#   * PREFERS the full `fix-sim` binary when it has been built,
#   * otherwise FALLS BACK to looping the runnable celnet-fix example RFQ client
#     (`cargo run -p celnet-fix --example fix_rfq_client`) as a v0 simulator, so
#     UAT has a live FIX price-taker today — one randomized RFQ every period +/- jitter.
#
# It is deliberately honest: if neither the binary nor the example can run it exits
# non-zero with a clear message rather than pretending to have started.
#
# Usage:
#   deploy/start-fix-sim.sh              # loop RFQs against the UAT FXO acceptor
#   FIXSIM_ONESHOT=1 deploy/start-fix-sim.sh    # send one RFQ and exit
#   FIXSIM_DAEMON=1  deploy/start-fix-sim.sh    # background + write a PID file
#
# Env overrides (defaults = the UAT FXO session from the generated QuickFIX profile):
#   FIXSIM_HOST (127.0.0.1)  FIXSIM_PORT (56001)
#   FIXSIM_SENDER (CELER_FXO) FIXSIM_TARGET (CELNET) FIXSIM_DIALECT (fx)
#   FIXSIM_PERIOD (180) FIXSIM_JITTER (60)   # seconds between RFQs
#   FIXSIM_BIN (target/release/fix-sim) FIXSIM_CONFIG ()  # for the full bot when built
#   FIXSIM_RUN_DIR (deploy/fix-sim-run) FIXSIM_ONESHOT (0) FIXSIM_DAEMON (0)
set -euo pipefail

FIXSIM_HOST="${FIXSIM_HOST:-127.0.0.1}"
FIXSIM_PORT="${FIXSIM_PORT:-56001}"
FIXSIM_SENDER="${FIXSIM_SENDER:-CELER_FXO}"
FIXSIM_TARGET="${FIXSIM_TARGET:-CELNET}"
FIXSIM_DIALECT="${FIXSIM_DIALECT:-fx}"
FIXSIM_PERIOD="${FIXSIM_PERIOD:-180}"
FIXSIM_JITTER="${FIXSIM_JITTER:-60}"
FIXSIM_ONESHOT="${FIXSIM_ONESHOT:-0}"
FIXSIM_DAEMON="${FIXSIM_DAEMON:-0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

RUN_DIR="${FIXSIM_RUN_DIR:-$REPO_ROOT/deploy/fix-sim-run}"
mkdir -p "$RUN_DIR/store" "$RUN_DIR/log"
LOG="$RUN_DIR/log/fix-sim.$(date +%Y%m%d-%H%M%S).log"
PID_FILE="$RUN_DIR/fix-sim.pid"

FIXSIM_BIN="${FIXSIM_BIN:-$REPO_ROOT/target/release/fix-sim}"
FIXSIM_CONFIG="${FIXSIM_CONFIG:-}"

log() { echo "[fix-sim] $*"; }

# Source the Rust toolchain (shell does not persist env on this estate).
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

preflight() {
  if command -v nc >/dev/null 2>&1; then
    nc -z -w 3 "$FIXSIM_HOST" "$FIXSIM_PORT" >/dev/null 2>&1
  else
    timeout 3 bash -c "exec 3<>/dev/tcp/$FIXSIM_HOST/$FIXSIM_PORT" >/dev/null 2>&1
  fi
}
if preflight; then
  log "acceptor $FIXSIM_HOST:$FIXSIM_PORT reachable."
else
  log "WARN: acceptor $FIXSIM_HOST:$FIXSIM_PORT not reachable yet — the client will retry on its reconnect interval."
fi

# --- Preferred path: the full fix-sim binary (once step 2 of the design is built). ---
if [ -x "$FIXSIM_BIN" ]; then
  log "launching fix-sim binary: $FIXSIM_BIN (dialect=$FIXSIM_DIALECT)"
  set -- "$FIXSIM_BIN" --host "$FIXSIM_HOST" --port "$FIXSIM_PORT" \
    --sender "$FIXSIM_SENDER" --target "$FIXSIM_TARGET" --dialect "$FIXSIM_DIALECT"
  [ -n "$FIXSIM_CONFIG" ] && set -- "$@" --config "$FIXSIM_CONFIG"
  if [ "$FIXSIM_DAEMON" = "1" ]; then
    nohup "$@" >>"$LOG" 2>&1 & echo $! >"$PID_FILE"; log "daemonized pid $(cat "$PID_FILE"); log $LOG"
  else
    exec "$@"
  fi
  exit 0
fi

# --- Fallback: loop the runnable example RFQ price-taker (v0 simulator). ---
if ! command -v cargo >/dev/null 2>&1; then
  log "ERROR: no fix-sim binary at $FIXSIM_BIN and cargo is unavailable — nothing to start." >&2
  exit 1
fi
log "no fix-sim binary yet — looping celnet-fix example RFQ client as the v0 simulator."

PAIRS=(EURUSD GBPUSD USDJPY USDCHF AUDUSD EURGBP EURJPY)
SIDES=(observe buy sell)   # observe = watch; buy/sell = lift (accept) the quote
TYPES=(call put)

run_once() {
  local pair="${PAIRS[$((RANDOM % ${#PAIRS[@]}))]}"
  local side="${SIDES[$((RANDOM % ${#SIDES[@]}))]}"
  local otype="${TYPES[$((RANDOM % ${#TYPES[@]}))]}"
  local strike; strike="$(awk "BEGIN{printf \"%.4f\", 1.00 + (${RANDOM} % 40) / 100.0}")"
  local req="RFQ-$(date +%s)-$RANDOM"
  log "RFQ pair=$pair type=$otype side=$side strike=$strike req=$req -> $FIXSIM_HOST:$FIXSIM_PORT"
  cargo run --quiet -p celnet-fix --example fix_rfq_client -- \
    --addr "$FIXSIM_HOST:$FIXSIM_PORT" \
    --sender "$FIXSIM_SENDER" --target "$FIXSIM_TARGET" \
    --pair "$pair" --type "$otype" --side "$side" --strike "$strike" --req-id "$req"
}

loop() {
  while true; do
    run_once || log "RFQ run exited non-zero ($?), continuing"
    local span=$(( FIXSIM_PERIOD + (RANDOM % (2 * FIXSIM_JITTER + 1)) - FIXSIM_JITTER ))
    [ "$span" -lt 5 ] && span=5
    log "sleeping ${span}s"
    sleep "$span"
  done
}

if [ "$FIXSIM_ONESHOT" = "1" ]; then
  run_once
elif [ "$FIXSIM_DAEMON" = "1" ]; then
  nohup bash -c "$(declare -f log run_once loop); \
    FIXSIM_HOST=$FIXSIM_HOST FIXSIM_PORT=$FIXSIM_PORT FIXSIM_SENDER=$FIXSIM_SENDER \
    FIXSIM_TARGET=$FIXSIM_TARGET FIXSIM_PERIOD=$FIXSIM_PERIOD FIXSIM_JITTER=$FIXSIM_JITTER loop" \
    >>"$LOG" 2>&1 & echo $! >"$PID_FILE"
  log "daemonized pid $(cat "$PID_FILE"); log $LOG"
else
  loop
fi
