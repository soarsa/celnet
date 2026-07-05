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
#   FIXSIM_ASSET (fi)        # fi = fixed income / OIS rates (default); fx = FX options
#   FIXSIM_HOST (127.0.0.1)  FIXSIM_PORT (fi:56002 / fx:56001)
#   FIXSIM_SENDER (fi:CELER_RATES / fx:CELER_FXO)  FIXSIM_TARGET (CELNET)
#   FIXSIM_CURVE (USD-OIS) FIXSIM_TENOR (5) FIXSIM_NOTIONAL (10000000)   # FI RFQ shape
#   FIXSIM_PERIOD (180) FIXSIM_JITTER (60)   # seconds between RFQs
#   FIXSIM_BIN (target/release/fix-sim)      # preferred once the full bot is built
#   FIXSIM_RFQ_BIN ()                        # prebuilt RFQ client (shipped by the release);
#                                            # used on the server where cargo is unavailable
#   FIXSIM_RUN_DIR (deploy/fix-sim-run) FIXSIM_LOG (<run>/log/fix-sim.log)
#   FIXSIM_ONESHOT (0) FIXSIM_DAEMON (0)
set -euo pipefail

# Asset class the simulator drives: "fi" (fixed income / OIS rates RFQs, the DEFAULT)
# or "fx" (FX-option RFQs). FI RFQs route to the rates desk (a human prices them in
# the GUI); FX RFQs auto-quote off a surface snapshot. Each asset dials its own
# managed acceptor and CompIDs (from the runtime registry /home/celnet/
# fix-connections.json). The sim is the CLIENT, so its SenderCompID is the venue's
# accepted counterparty (target_comp_id) and its TargetCompID is the venue's own
# CompID (sender_comp_id). Port/CompIDs default per asset below; override any of them.
FIXSIM_ASSET="${FIXSIM_ASSET:-fi}"
case "$FIXSIM_ASSET" in
  fi|rates|fixedincome|fixed_income) FIXSIM_ASSET="fi" ;;
  fx|fxo|options)                    FIXSIM_ASSET="fx" ;;
  *) echo "[fix-sim] ERROR: FIXSIM_ASSET must be fi|fx, got '$FIXSIM_ASSET'" >&2; exit 2 ;;
esac

FIXSIM_HOST="${FIXSIM_HOST:-127.0.0.1}"
if [ "$FIXSIM_ASSET" = "fi" ]; then
  # Rates venue "CELER_RATES_CELNET" (kind fixed_income_quote) @127.0.0.1:56002.
  FIXSIM_PORT="${FIXSIM_PORT:-56002}"
  FIXSIM_SENDER="${FIXSIM_SENDER:-CELER_RATES}"
  FIXSIM_TARGET="${FIXSIM_TARGET:-CELNET}"
else
  # FX-options venue "CELER_FXO_CELNET" (kind options) @127.0.0.1:56001.
  FIXSIM_PORT="${FIXSIM_PORT:-56001}"
  FIXSIM_SENDER="${FIXSIM_SENDER:-CELER_FXO}"
  FIXSIM_TARGET="${FIXSIM_TARGET:-CELNET}"
fi
# Fixed-income RFQ shape (whole-year OIS tenor, notional in ccy units).
FIXSIM_CURVE="${FIXSIM_CURVE:-USD-OIS}"
FIXSIM_TENOR="${FIXSIM_TENOR:-5}"
FIXSIM_NOTIONAL="${FIXSIM_NOTIONAL:-10000000}"
FIXSIM_PERIOD="${FIXSIM_PERIOD:-20}"
FIXSIM_JITTER="${FIXSIM_JITTER:-60}"
# Every Nth auto-quote is LIFTED (executed → booked deal) so the blotter fills with
# completed rates deals, not just shown quotes; the rest stay quoted-only. 0 = never lift.
FIXSIM_LIFT_EVERY="${FIXSIM_LIFT_EVERY:-3}"
# Optional RFS (streaming) leg: when a fixed-income STREAM acceptor is reachable at
# FIXSIM_STREAM_PORT, a SECOND supervised client subscribes and receives continuous
# re-priced quotes, lifting one per cycle (an executed streaming deal). Empty ⇒ disabled.
FIXSIM_STREAM_PORT="${FIXSIM_STREAM_PORT:-}"
FIXSIM_STREAM_SENDER="${FIXSIM_STREAM_SENDER:-CELER_RATES_STREAM}"
FIXSIM_STREAM_HOLD="${FIXSIM_STREAM_HOLD:-15}"
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
log "asset $FIXSIM_ASSET ; acceptor $FIXSIM_HOST:$FIXSIM_PORT ; identity $FIXSIM_SENDER -> $FIXSIM_TARGET"

# --- Resolve the runner up front so a build failure is LOUD, not hidden inside --
# each RFQ iteration (the old `cargo run --quiet` swallowed both build and
# connect errors, which is why a bare run looked like it did nothing).
RUNNER=()
if [ -x "$FIXSIM_BIN" ]; then
  log "using fix-sim binary $FIXSIM_BIN"
  RUNNER=("$FIXSIM_BIN")
elif [ -n "${FIXSIM_RFQ_BIN:-}" ] && [ -x "${FIXSIM_RFQ_BIN:-}" ]; then
  # A prebuilt RFQ-client binary shipped by the release (deploy/roles/celnet_release
  # installs it at <release>/bin/fix-rfq-client). This is the path used ON the server,
  # where the celnet user has no cargo to build the example at runtime.
  log "using prebuilt RFQ client $FIXSIM_RFQ_BIN"
  RUNNER=("$FIXSIM_RFQ_BIN")
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
SIDES=(observe buy sell)   # FX: observe = RFQ only; buy = lift the offer; sell = hit the bid
TYPES=(call put)
RATES_SIDES=(pay receive two-way)  # FI: pay fixed / receive fixed / two-way request
# Deterministic desk scenario: send several auto-priceable RFQs, then one that requires
# a human. The venue auto-quotes on-the-run tenors ({1,2,3,5,7,10}); any OTHER tenor
# ("one that doesn't exist" in the auto set) routes to the rates desk as a PENDING
# ticket. Notional is held small (always under the auto-quote cap) so the ONLY thing
# that forces a manual price is the tenor — exactly the trigger to demonstrate.
RATES_AUTO_TENORS=(2 3 5 7 10)                 # on-the-run ⇒ auto-quoted (35=S)
RATES_MANUAL_TENOR="${FIXSIM_MANUAL_TENOR:-15}"  # non-standard ⇒ routed to a human desk
FIXSIM_MANUAL_EVERY="${FIXSIM_MANUAL_EVERY:-4}"  # every Nth RFQ is a manual one

# Build the client argv. The client (fix-rfq-client) owns the request loop and, for
# fixed income, the tenor rotation (on-the-run auto-quoted; every Nth a `--manual-tenor`
# routed to a human desk), so ONE persistent invocation streams many RFQs over a SINGLE
# logon — no logon/logout churn per request.
build_client_args() {
  CLIENT_ARGS=("${RUNNER[@]}" --addr "$FIXSIM_HOST:$FIXSIM_PORT" \
    --sender "$FIXSIM_SENDER" --target "$FIXSIM_TARGET" --req-id "FIXSIM-$(date +%s)")
  if [ "$FIXSIM_ASSET" = "fi" ]; then
    CLIENT_ARGS+=(--asset fi --curve "$FIXSIM_CURVE" --tenor "$FIXSIM_TENOR" \
      --notional "$FIXSIM_NOTIONAL" --side pay \
      --manual-every "$FIXSIM_MANUAL_EVERY" --manual-tenor "$RATES_MANUAL_TENOR" \
      --lift-every "$FIXSIM_LIFT_EVERY")
  else
    CLIENT_ARGS+=(--asset fx --pair EURUSD --type call --side buy --strike 1.10)
  fi
}

if [ "$FIXSIM_ONESHOT" = "1" ]; then
  # A single RFQ (the client's default --repeat 1).
  build_client_args
  "${CLIENT_ARGS[@]}"
  exit 0
fi

# --- Optional RFS (streaming) leg -------------------------------------------------
# When a fixed-income STREAM acceptor is reachable at FIXSIM_STREAM_PORT, run a SECOND
# supervised client that SUBSCRIBES (RFS) and receives continuous re-priced quotes over
# ONE session, lifting one per cycle (an executed streaming deal). Backgrounded so the RFQ
# leg below stays in the foreground; reaped when this script exits. Skipped (with a loud,
# actionable log) when no stream acceptor is present, so the RFQ leg never breaks.
if [ -n "$FIXSIM_STREAM_PORT" ]; then
  stream_up=1
  if command -v nc >/dev/null 2>&1; then
    nc -z -w 3 "$FIXSIM_HOST" "$FIXSIM_STREAM_PORT" >/dev/null 2>&1 || stream_up=0
  else
    timeout 3 bash -c "exec 3<>/dev/tcp/$FIXSIM_HOST/$FIXSIM_STREAM_PORT" >/dev/null 2>&1 || stream_up=0
  fi
  if [ "$stream_up" = 1 ]; then
    STREAM_ARGS=("${RUNNER[@]}" --addr "$FIXSIM_HOST:$FIXSIM_STREAM_PORT" \
      --sender "$FIXSIM_STREAM_SENDER" --target "$FIXSIM_TARGET" \
      --req-id "FIXSIM-RFS-$(date +%s)" \
      --asset fi --intent rfs --curve "$FIXSIM_CURVE" --notional "$FIXSIM_NOTIONAL" \
      --side pay --manual-every 0 --lift-every "$FIXSIM_LIFT_EVERY" \
      --stream-hold "$FIXSIM_STREAM_HOLD" --repeat 0 --interval 2)
    log "RFS leg: streaming from $FIXSIM_HOST:$FIXSIM_STREAM_PORT ($FIXSIM_STREAM_SENDER); hold ${FIXSIM_STREAM_HOLD}s, lift every ${FIXSIM_LIFT_EVERY}."
    ( while true; do "${STREAM_ARGS[@]}" || log "RFS client exited ($?) — reconnecting in 5s"; sleep 5; done ) &
    STREAM_PID=$!
    trap 'kill "$STREAM_PID" 2>/dev/null || true' EXIT INT TERM
  else
    log "RFS leg: acceptor $FIXSIM_HOST:$FIXSIM_STREAM_PORT NOT reachable — skipping (RFQ leg continues)."
  fi
fi

# Persistent stream: log on ONCE and stream RFQs over the SAME session, one every
# FIXSIM_PERIOD seconds (the client sleeps between requests; it does not re-logon). A
# supervisor restarts the client if the session ever drops.
build_client_args
CLIENT_ARGS+=(--repeat 0 --interval "$FIXSIM_PERIOD")
log "streaming RFQs over ONE persistent session every ${FIXSIM_PERIOD}s (Ctrl-C to stop)."
while true; do
  "${CLIENT_ARGS[@]}" || log "sim client exited ($?) — reconnecting in 5s"
  sleep 5
done
