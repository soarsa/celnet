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
#   FIXSIM_ASSET=fx  deploy/start-fix-sim.sh   # drive the FX-options (RFS) leg instead
#   FIXSIM_ASSET=both FIXSIM_DAEMON=1 deploy/start-fix-sim.sh  # BOTH legs, two daemons
#   tail -f deploy/fix-sim-run/log/fix-sim.log      # watch a single-asset run
#   tail -f deploy/fix-sim-run/{fi,fx}/log/fix-sim.log  # watch each leg of a both run
#
# Env overrides (defaults match group_vars/all.yml celnet_env FIX settings):
#   FIXSIM_ASSET (fi)        # fi = fixed income / OIS rates (default); fx = FX options;
#                            # both = drive fi AND fx concurrently (two daemons)
#   FIXSIM_HOST (127.0.0.1)  FIXSIM_PORT (fi:56002 / fx:56001)
#   FIXSIM_SENDER (fi:CELER_RATES / fx:CELER_FXO)  FIXSIM_TARGET (CELNET)
#   FIXSIM_CURVE (USD-OIS) FIXSIM_TENOR (5) FIXSIM_NOTIONAL (10000000)   # FI RFQ shape
#   FIXSIM_PERIOD (120) FIXSIM_JITTER (60)   # seconds between RFQs (120s heartbeat)
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
  # "both" drives BOTH the fixed-income (rates/OIS) leg and the FX-options leg
  # concurrently, each as its own supervised daemon with a DISJOINT run dir / PID /
  # log so they never collide. See the dispatch below.
  both|all|fi+fx|fifx)               FIXSIM_ASSET="both" ;;
  *) echo "[fix-sim] ERROR: FIXSIM_ASSET must be fi|fx|both, got '$FIXSIM_ASSET'" >&2; exit 2 ;;
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
# Cadence: the whole-second fallback (the operator-standard heartbeat). Named +
# env-overridable, no bare magic number; flows to the client's `--interval`.
FIXSIM_PERIOD="${FIXSIM_PERIOD:-120}"
FIXSIM_JITTER="${FIXSIM_JITTER:-60}"
# FAST sub-second cadence (ms). When non-empty it OVERRIDES FIXSIM_PERIOD via the client's
# `--interval-ms`, so a redeploy builds up deals/positions/flow/risk quickly. Default 750ms
# — clearly livelier than the 120s heartbeat, still bounded so it never floods the venue.
# Set FIXSIM_PERIOD_MS="" to fall back to the whole-second FIXSIM_PERIOD.
FIXSIM_PERIOD_MS="${FIXSIM_PERIOD_MS:-750}"
# Every Nth auto-quote is LIFTED (executed → booked deal) so the blotter fills with
# completed rates deals, not just shown quotes; the rest stay quoted-only. 0 = never lift.
FIXSIM_LIFT_EVERY="${FIXSIM_LIFT_EVERY:-3}"
# Optional RFS (streaming) leg: when a fixed-income STREAM acceptor is reachable at
# FIXSIM_STREAM_PORT, a SECOND supervised client subscribes and receives continuous
# re-priced quotes, lifting one per cycle (an executed streaming deal). Empty ⇒ disabled.
FIXSIM_STREAM_PORT="${FIXSIM_STREAM_PORT:-}"
FIXSIM_STREAM_SENDER="${FIXSIM_STREAM_SENDER:-CELER_RATES_STREAM}"
FIXSIM_STREAM_HOLD="${FIXSIM_STREAM_HOLD:-15}"
# FAST RFS hold per cycle (ms). Non-empty ⇒ the RFS/ESP legs use `--stream-hold-ms`;
# default 2000ms so the fast cadence turns over many stream cycles.
FIXSIM_STREAM_HOLD_MS="${FIXSIM_STREAM_HOLD_MS:-2000}"
# Optional ESP leg: when FIXSIM_ESP=1 AND a fixed-income STREAM acceptor is reachable at
# FIXSIM_ESP_PORT (defaults to FIXSIM_STREAM_PORT), a supervised client runs `--asset esp`
# — it downloads the top-N reference-data instruments over gRPC (FIXSIM_GRPC_ADDR) and
# streams bond RFS on them (priced off the aggregated-book composite, tiered by the
# connection's pricing group), randomly lifting some to book live streaming deals.
FIXSIM_ESP="${FIXSIM_ESP:-0}"
FIXSIM_ESP_PORT="${FIXSIM_ESP_PORT:-${FIXSIM_STREAM_PORT:-}}"
FIXSIM_ESP_SENDER="${FIXSIM_ESP_SENDER:-${FIXSIM_STREAM_SENDER:-CELER_RATES_STREAM}}"
FIXSIM_GRPC_ADDR="${FIXSIM_GRPC_ADDR:-http://127.0.0.1:50051}"
FIXSIM_ESP_INSTRUMENTS="${FIXSIM_ESP_INSTRUMENTS:-15}"
FIXSIM_ESP_SEED="${FIXSIM_ESP_SEED:-0x5EED1234}"
FIXSIM_USER="${FIXSIM_USER:-admin@celnet.com}"
FIXSIM_PASSWORD="${FIXSIM_PASSWORD:-password}"
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

# Auto-detect the release-shipped RFQ-client binary so ON THE SERVER (no cargo) the sim
# "just runs" without the operator hand-setting FIXSIM_RFQ_BIN. The release role
# (deploy/roles/celnet_release) installs it at <release>/bin/fix-rfq-client, and `current`
# symlinks the live release — so from REPO_ROOT=<...>/shared the sibling `../current/bin`
# is the live binary. Probe the standard locations in order; first hit wins. An explicit
# FIXSIM_RFQ_BIN (or a full FIXSIM_BIN) still overrides — this only fills the empty default.
if [ -z "${FIXSIM_RFQ_BIN:-}" ]; then
  for __cand in \
    "$REPO_ROOT/../current/bin/fix-rfq-client" \
    "$REPO_ROOT/bin/fix-rfq-client" \
    "$SCRIPT_DIR/fix-rfq-client" \
    "/opt/celnet/current/bin/fix-rfq-client"; do
    if [ -x "$__cand" ]; then
      FIXSIM_RFQ_BIN="$(cd "$(dirname "$__cand")" && pwd)/$(basename "$__cand")"
      break
    fi
  done
fi

# Source the Rust toolchain (shell does not persist env on this estate).
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

ts() { date +%Y-%m-%dT%H:%M:%S%z; }
log() { echo "[$(ts)] [fix-sim] $*"; }

# --- RESTART, don't ACCUMULATE (same rationale as start-lp-sim.sh): stop every prior
# fix-sim (supervisor script + fix_rfq_client binary) before starting fresh, so each
# release RESTARTS the incoming-client RFS/ESP sims instead of leaking a new one on top.
# Runs ONCE at the top level: the re-exec'd child (FIXSIM__CHILD) and the asset=both
# fan-out legs (FIXSIM__RESTARTED) skip it, so a fresh sim never kills itself/its sibling.
if [ "${FIXSIM__CHILD:-0}" != "1" ] && [ "${FIXSIM__RESTARTED:-0}" != "1" ]; then
  __self=$$; __parent=${PPID:-0}; __uid="$(id -u)"
  # Match BOTH binary spellings: the cargo example target is `fix_rfq_client` (underscore),
  # but the RELEASE ships it as `fix-rfq-client` (hyphen). The `fix[-_]rfq[-_]client` class
  # sweeps either — an underscore-only pattern silently leaks the hyphenated release binary.
  for __pid in $(pgrep -u "$__uid" -f 'fix[-_]rfq[-_]client|start-fix-sim.sh' 2>/dev/null || true); do
    case "$__pid" in "$__self"|"$__parent") continue ;; esac
    kill "$__pid" 2>/dev/null || true
  done
  sleep 1
  # Belt-and-braces: SIGKILL any fix-rfq-client / fix_rfq_client BINARY still up (matches
  # the binary, never this shell script).
  pkill -9 -u "$__uid" -f 'fix[-_]rfq[-_]client' 2>/dev/null || true
fi

# --- asset=both: fan out into two independent supervised daemons ------------------
# Drive the fixed-income (rates/OIS) leg AND the FX-options leg concurrently, each as
# its own supervised daemon under a DISJOINT run dir (⇒ its own PID file + log), so the
# two never collide. Each child re-execs this same script with a single asset and
# FIXSIM_DAEMON=1; all other env (period, jitter, manual/lift cadence, the prebuilt
# FIXSIM_RFQ_BIN path) is inherited, and each child re-derives its own port/CompIDs for
# its asset. Follow either leg with:
#   tail -f <run>/fi/log/fix-sim.log   # rates/OIS leg  (CELER_RATES @ :56002)
#   tail -f <run>/fx/log/fix-sim.log   # FX-options leg (CELER_FXO   @ :56001)
if [ "$FIXSIM_ASSET" = "both" ]; then
  BASE_RUN="${FIXSIM_RUN_DIR:-$REPO_ROOT/deploy/fix-sim-run}"
  log "asset=both — launching supervised FI and FX legs as separate daemons under $BASE_RUN"
  FIXSIM__RESTARTED=1 FIXSIM_ASSET=fi FIXSIM_DAEMON=1 FIXSIM_RUN_DIR="$BASE_RUN/fi" "$0"
  FIXSIM__RESTARTED=1 FIXSIM_ASSET=fx FIXSIM_DAEMON=1 FIXSIM_RUN_DIR="$BASE_RUN/fx" "$0"
  log "both legs launched; PID files at $BASE_RUN/fi/fix-sim.pid and $BASE_RUN/fx/fix-sim.pid"
  exit 0
fi

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
# Deterministic desk scenario: a deliberate MIX of auto-priced and manual RFQs. The venue
# auto-quotes on-the-run tenors ({2,3,5,7,10}) on the known curve — those get lifted and
# BOOKED. ~1 in 3 (--manual-every 3 ⇒ ~2/3 auto : ~1/3 manual) is a MANUAL one the venue
# cannot auto-price and routes to the rates desk as an ALERT-worthy manual intervention.
# The client alternates the two manual variants deterministically by iteration index:
#   * unconfigured tenor — the valid curve on RATES_MANUAL_TENOR (off the auto set), and
#   * unknown security  — a bogus curve symbol (FIXSIM_MANUAL_SECURITY) on a valid tenor.
# Notional is held small (under the auto-quote cap) so the ONLY thing forcing a manual
# price is the tenor/security — exactly the exception-only-notification triggers.
RATES_AUTO_TENORS=(2 3 5 7 10)                 # on-the-run ⇒ auto-quoted (35=S)
RATES_MANUAL_TENOR="${FIXSIM_MANUAL_TENOR:-15}"  # non-standard ⇒ UNCONFIGURED_TENOR
FIXSIM_MANUAL_SECURITY="${FIXSIM_MANUAL_SECURITY:-XXX-UNKNOWN}"  # bogus ⇒ UNKNOWN_SECURITY
FIXSIM_MANUAL_EVERY="${FIXSIM_MANUAL_EVERY:-3}"  # every Nth RFQ is manual (~1/3 manual)

# Build the client argv. The client (fix-rfq-client) owns the request loop and, for
# fixed income, the tenor/security rotation (on-the-run auto-quoted; every Nth a manual
# one — a `--manual-tenor` or a `--manual-security` — routed to a human desk), so ONE
# persistent invocation streams many RFQs over a SINGLE logon — no logon/logout per request.
build_client_args() {
  CLIENT_ARGS=("${RUNNER[@]}" --addr "$FIXSIM_HOST:$FIXSIM_PORT" \
    --sender "$FIXSIM_SENDER" --target "$FIXSIM_TARGET" --req-id "FIXSIM-$(date +%s)")
  if [ "$FIXSIM_ASSET" = "fi" ]; then
    CLIENT_ARGS+=(--asset fi --curve "$FIXSIM_CURVE" --tenor "$FIXSIM_TENOR" \
      --notional "$FIXSIM_NOTIONAL" --side pay \
      --manual-every "$FIXSIM_MANUAL_EVERY" --manual-tenor "$RATES_MANUAL_TENOR" \
      --manual-security "$FIXSIM_MANUAL_SECURITY" \
      --lift-every "$FIXSIM_LIFT_EVERY")
  else
    # FX: the client OWNS the rotation (major deliverable pairs, near-the-money strikes,
    # short-dated expiries, call/put) via its deterministic, seed-free `sim::fx_leg`
    # grid, so ONE persistent invocation streams many varied vanilla-option RFQs over a
    # SINGLE logon. Most auto-quote off the surface; every --manual-every-th is a
    # deliberately UNPRICEABLE leg (American exercise, or an NDF request on a deliverable
    # major) the venue routes to the FX desk (no Quote(S)); every --lift-every-th
    # auto-quote is LIFTED → an executed & booked FX deal. No --side ⇒ observe by
    # default, so the per-cycle lift cadence (not a blanket buy) drives the booking mix.
    CLIENT_ARGS+=(--asset fx \
      --manual-every "$FIXSIM_MANUAL_EVERY" \
      --lift-every "$FIXSIM_LIFT_EVERY")
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
    # Cadence flags: prefer the fast ms knobs (sub-second) over the whole-second fallback.
    if [ -n "$FIXSIM_PERIOD_MS" ]; then
      STREAM_CADENCE=(--stream-hold-ms "$FIXSIM_STREAM_HOLD_MS" --interval-ms "$FIXSIM_PERIOD_MS")
    else
      STREAM_CADENCE=(--stream-hold "$FIXSIM_STREAM_HOLD" --interval 2)
    fi
    STREAM_ARGS=("${RUNNER[@]}" --addr "$FIXSIM_HOST:$FIXSIM_STREAM_PORT" \
      --sender "$FIXSIM_STREAM_SENDER" --target "$FIXSIM_TARGET" \
      --req-id "FIXSIM-RFS-$(date +%s)" \
      --asset fi --intent rfs --curve "$FIXSIM_CURVE" --notional "$FIXSIM_NOTIONAL" \
      --side pay --manual-every 0 --lift-every "$FIXSIM_LIFT_EVERY" \
      --repeat 0 "${STREAM_CADENCE[@]}")
    log "RFS leg: streaming from $FIXSIM_HOST:$FIXSIM_STREAM_PORT ($FIXSIM_STREAM_SENDER); lift every ${FIXSIM_LIFT_EVERY}."
    ( while true; do "${STREAM_ARGS[@]}" || log "RFS client exited ($?) — reconnecting in 5s"; sleep 5; done ) &
    STREAM_PID=$!
    trap 'kill "$STREAM_PID" 2>/dev/null || true' EXIT INT TERM
  else
    log "RFS leg: acceptor $FIXSIM_HOST:$FIXSIM_STREAM_PORT NOT reachable — skipping (RFQ leg continues)."
  fi
fi

# --- Optional ESP (streaming) leg -------------------------------------------------
# When FIXSIM_ESP=1 and a fixed-income STREAM acceptor is reachable at FIXSIM_ESP_PORT,
# run a supervised `--asset esp` client: it downloads the top-N reference-data instruments
# over gRPC (FIXSIM_GRPC_ADDR) and streams bond RFS on them — priced off the aggregated-book
# composite, tiered by the connection's pricing group — randomly lifting some to book live
# streaming deals. Backgrounded like the RFS leg; skipped with a loud log if unreachable.
if [ "$FIXSIM_ESP" = "1" ] && [ -n "$FIXSIM_ESP_PORT" ]; then
  esp_up=1
  if command -v nc >/dev/null 2>&1; then
    nc -z -w 3 "$FIXSIM_HOST" "$FIXSIM_ESP_PORT" >/dev/null 2>&1 || esp_up=0
  else
    timeout 3 bash -c "exec 3<>/dev/tcp/$FIXSIM_HOST/$FIXSIM_ESP_PORT" >/dev/null 2>&1 || esp_up=0
  fi
  if [ "$esp_up" = 1 ]; then
    if [ -n "$FIXSIM_PERIOD_MS" ]; then
      ESP_CADENCE=(--stream-hold-ms "$FIXSIM_STREAM_HOLD_MS" --interval-ms "$FIXSIM_PERIOD_MS")
    else
      ESP_CADENCE=(--stream-hold "$FIXSIM_STREAM_HOLD" --interval 1)
    fi
    ESP_ARGS=("${RUNNER[@]}" --addr "$FIXSIM_HOST:$FIXSIM_ESP_PORT" \
      --sender "$FIXSIM_ESP_SENDER" --target "$FIXSIM_TARGET" \
      --req-id "FIXSIM-ESP-$(date +%s)" \
      --asset esp --grpc-addr "$FIXSIM_GRPC_ADDR" \
      --esp-instruments "$FIXSIM_ESP_INSTRUMENTS" --seed "$FIXSIM_ESP_SEED" \
      --user "$FIXSIM_USER" --password "$FIXSIM_PASSWORD" \
      --notional "$FIXSIM_NOTIONAL" --repeat 0 "${ESP_CADENCE[@]}")
    log "ESP leg: streaming top-$FIXSIM_ESP_INSTRUMENTS refdata bonds from $FIXSIM_HOST:$FIXSIM_ESP_PORT (refdata $FIXSIM_GRPC_ADDR)."
    ( while true; do "${ESP_ARGS[@]}" || log "ESP client exited ($?) — reconnecting in 5s"; sleep 5; done ) &
    ESP_PID=$!
    trap 'kill "${STREAM_PID:-}" "$ESP_PID" 2>/dev/null || true' EXIT INT TERM
  else
    log "ESP leg: acceptor $FIXSIM_HOST:$FIXSIM_ESP_PORT NOT reachable — skipping (RFQ leg continues)."
  fi
fi

# Persistent stream: log on ONCE and stream RFQs over the SAME session (the client sleeps
# between requests; it does not re-logon). A supervisor restarts the client if the session
# ever drops. The FAST sub-second cadence (FIXSIM_PERIOD_MS via --interval-ms) is preferred;
# an empty FIXSIM_PERIOD_MS falls back to the whole-second FIXSIM_PERIOD.
build_client_args
if [ -n "$FIXSIM_PERIOD_MS" ]; then
  CLIENT_ARGS+=(--repeat 0 --interval-ms "$FIXSIM_PERIOD_MS")
  log "streaming RFQs over ONE persistent session every ${FIXSIM_PERIOD_MS}ms (Ctrl-C to stop)."
else
  CLIENT_ARGS+=(--repeat 0 --interval "$FIXSIM_PERIOD")
  log "streaming RFQs over ONE persistent session every ${FIXSIM_PERIOD}s (Ctrl-C to stop)."
fi
while true; do
  "${CLIENT_ARGS[@]}" || log "sim client exited ($?) — reconnecting in 5s"
  sleep 5
done
