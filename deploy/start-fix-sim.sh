#!/usr/bin/env bash
# start-fix-sim.sh — run the celnet FIX quote simulator against the live acceptor
# and log EVERY RFQ it sends plus every Quote / fill it receives to a file, so an
# operator can watch the request/response flow with `tail -f`.
#
# Design: docs/FIX-SIM-DESIGN.md. Until the full `fix-sim` bot binary is built
# (design step 2), this drives the runnable celnet-fix example RFQ client
# (`--example fix_rfq_client`) — a real FIX 4.4 price-taker, not a stub — one
# randomized RFQ every period ± jitter.
#
# What it needs to work (the reason a bare run used to "do nothing"):
#   * a FIX acceptor must be listening at FIXSIM_HOST:FIXSIM_PORT. The server binds
#     its acceptors from the MANAGED registry (fix-connections.json, GUI FIX admin) —
#     on the UAT host that is "CELER_FXO_CELNET" at 127.0.0.1:56001 (the default here).
#     Do NOT set CELNET_FIX_ADDR to add a legacy acceptor: it collides with this
#     registry acceptor on the same port and aborts the server at boot (EADDRINUSE).
#     If nothing is listening, each attempt now logs a LOUD, actionable message.
#   * the CompIDs must line up: the simulator's FIXSIM_SENDER must equal the venue's
#     accepted counterparty (target_comp_id, "CELER_FXO"), and FIXSIM_TARGET must
#     equal the venue's own CompID (sender_comp_id, "CELNET").
#
# Usage:
#   deploy/start-fix-sim.sh              # loop RFQs in the foreground (tees to log)
#   FIXSIM_ONESHOT=1 deploy/start-fix-sim.sh   # send one RFQ and exit
#   FIXSIM_DAEMON=1  deploy/start-fix-sim.sh   # background, write a PID file, log to file
#   tail -f deploy/fix-sim-run/log/fix-sim.log # watch requests in/out
#
# Env overrides (defaults match group_vars/all.yml celnet_env FIX settings):
#   FIXSIM_HOST (127.0.0.1)  FIXSIM_PORT (56001)
#   FIXSIM_SENDER (CELER_FXO)  FIXSIM_TARGET (CELNET)
#   FIXSIM_PERIOD (180) FIXSIM_JITTER (60)   # seconds between RFQs
#   FIXSIM_BIN (target/release/fix-sim)      # preferred once the full bot is built
#   FIXSIM_RUN_DIR (deploy/fix-sim-run) FIXSIM_LOG (<run>/log/fix-sim.log)
#   FIXSIM_ONESHOT (0) FIXSIM_DAEMON (0)
set -euo pipefail

# Defaults match the live MANAGED FIX acceptor on the UAT host, from the runtime
# registry /home/celnet/fix-connections.json: connection "CELER_FXO_CELNET", kind
# options, bind_addr 127.0.0.1:56001, sender_comp_id=CELNET, target_comp_id=CELER_FXO.
# The simulator is the CLIENT, so it presents the venue's counterparty CompID as its
# SenderCompID (= venue target_comp_id = CELER_FXO) and addresses the venue's own
# CompID as its TargetCompID (= venue sender_comp_id = CELNET). If the acceptor's
# bind_addr/CompIDs change in the GUI FIX admin, update these to match.
FIXSIM_HOST="${FIXSIM_HOST:-127.0.0.1}"
FIXSIM_PORT="${FIXSIM_PORT:-56001}"
FIXSIM_SENDER="${FIXSIM_SENDER:-CELER_FXO}"
FIXSIM_TARGET="${FIXSIM_TARGET:-CELNET}"
FIXSIM_PERIOD="${FIXSIM_PERIOD:-180}"
FIXSIM_JITTER="${FIXSIM_JITTER:-60}"
FIXSIM_ONESHOT="${FIXSIM_ONESHOT:-0}"
FIXSIM_DAEMON="${FIXSIM_DAEMON:-0}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

RUN_DIR="${FIXSIM_RUN_DIR:-$REPO_ROOT/deploy/fix-sim-run}"
mkdir -p "$RUN_DIR/log"
LOG="${FIXSIM_LOG:-$RUN_DIR/log/fix-sim.log}"
PID_FILE="$RUN_DIR/fix-sim.pid"
FIXSIM_BIN="${FIXSIM_BIN:-$REPO_ROOT/target/release/fix-sim}"

# Source the Rust toolchain (shell does not persist env on this estate).
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

ts() { date +%Y-%m-%dT%H:%M:%S%z; }
log() { echo "[$(ts)] [fix-sim] $*"; }

# --- Daemon: re-exec self into the logfile, record the child PID, and return. --
# Serializing shell functions through `bash -c` (the old approach) was brittle;
# re-exec keeps ONE code path and sends all child output straight to the log.
if [ "$FIXSIM_DAEMON" = "1" ] && [ "${FIXSIM__CHILD:-0}" != "1" ]; then
  touch "$LOG"
  FIXSIM__CHILD=1 nohup "$0" "$@" >>"$LOG" 2>&1 &
  echo $! >"$PID_FILE"
  echo "[fix-sim] daemonized pid $(cat "$PID_FILE"); logging to $LOG"
  echo "[fix-sim] follow with:  tail -f $LOG"
  exit 0
fi

# --- Foreground: mirror all output to the logfile so a watching terminal sees --
# exactly what lands in the log. (The daemon child already writes to the log via
# the nohup redirect above, so it must NOT tee — that would double every line.)
if [ "${FIXSIM__CHILD:-0}" != "1" ]; then
  exec > >(tee -a "$LOG") 2>&1
fi

log "run dir $RUN_DIR ; log $LOG"
log "acceptor $FIXSIM_HOST:$FIXSIM_PORT ; identity $FIXSIM_SENDER -> $FIXSIM_TARGET"

# --- Resolve the runner up front so a build failure is LOUD, not hidden inside --
# each RFQ iteration (the old `cargo run --quiet` swallowed both build and
# connect errors, which is why a bare run looked like it did nothing).
RUNNER=()
if [ -x "$FIXSIM_BIN" ]; then
  log "using fix-sim binary $FIXSIM_BIN"
  RUNNER=("$FIXSIM_BIN")
elif command -v cargo >/dev/null 2>&1; then
  log "no fix-sim binary yet — building the celnet-fix example RFQ client (v0 simulator)..."
  if ! cargo build -p celnet-fix --example fix_rfq_client; then
    log "ERROR: failed to build the example RFQ client — see the build output above."
    exit 1
  fi
  EX_BIN="$REPO_ROOT/target/debug/examples/fix_rfq_client"
  if [ ! -x "$EX_BIN" ]; then
    EX_BIN="$(find "$REPO_ROOT/target" -name fix_rfq_client -type f -perm -u+x 2>/dev/null | head -1)"
  fi
  if [ ! -x "${EX_BIN:-}" ]; then
    log "ERROR: built the example but cannot locate its binary under target/."
    exit 1
  fi
  log "example built: $EX_BIN"
  RUNNER=("$EX_BIN")
else
  log "ERROR: no fix-sim binary at $FIXSIM_BIN and cargo is unavailable — nothing to run."
  exit 1
fi

preflight() {
  if command -v nc >/dev/null 2>&1; then
    nc -z -w 3 "$FIXSIM_HOST" "$FIXSIM_PORT" >/dev/null 2>&1
  else
    timeout 3 bash -c "exec 3<>/dev/tcp/$FIXSIM_HOST/$FIXSIM_PORT" >/dev/null 2>&1
  fi
}
if preflight; then
  log "acceptor reachable."
else
  log "WARN: acceptor $FIXSIM_HOST:$FIXSIM_PORT is NOT reachable."
  log "      -> confirm the managed FIX acceptor is enabled (GUI FIX admin / fix-connections.json)"
  log "         and its bind_addr matches FIXSIM_PORT (CELER_FXO_CELNET=56001), then restart the sim."
  log "      Continuing anyway — each RFQ attempt is logged below so failures are visible."
fi

PAIRS=(EURUSD GBPUSD USDJPY USDCHF AUDUSD EURGBP EURJPY)
SIDES=(observe buy sell)   # observe = RFQ only; buy = lift the offer; sell = hit the bid
TYPES=(call put)

run_once() {
  local pair="${PAIRS[$((RANDOM % ${#PAIRS[@]}))]}"
  local side="${SIDES[$((RANDOM % ${#SIDES[@]}))]}"
  local otype="${TYPES[$((RANDOM % ${#TYPES[@]}))]}"
  local strike
  strike="$(awk "BEGIN{printf \"%.4f\", 1.00 + (${RANDOM} % 40) / 100.0}")"
  local req="RFQ-$(date +%s)-$RANDOM"
  log ">>> RFQ pair=$pair type=$otype side=$side strike=$strike req=$req"
  if "${RUNNER[@]}" --addr "$FIXSIM_HOST:$FIXSIM_PORT" \
      --sender "$FIXSIM_SENDER" --target "$FIXSIM_TARGET" \
      --pair "$pair" --type "$otype" --side "$side" --strike "$strike" --req-id "$req"; then
    log "<<< RFQ $req complete"
  else
    log "<<< RFQ $req exited non-zero ($?) — see the client output above"
  fi
}

if [ "$FIXSIM_ONESHOT" = "1" ]; then
  run_once
  exit 0
fi

log "looping RFQs every ${FIXSIM_PERIOD}s +/-${FIXSIM_JITTER}s (Ctrl-C to stop)."
while true; do
  run_once || true
  span=$(( FIXSIM_PERIOD + (RANDOM % (2 * FIXSIM_JITTER + 1)) - FIXSIM_JITTER ))
  [ "$span" -lt 5 ] && span=5
  log "sleeping ${span}s"
  sleep "$span"
done
