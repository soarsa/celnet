#!/usr/bin/env bash
# tools/fix-test.sh — interactive driver for the celnet FIX RFQ test client.
#
# A menu over the `celnet-fix` `fix_rfq_client` example (a real FIX 4.4 price-taker).
# It can stand up a FIX-enabled demo edge for you, then send RFQs from sensible
# defaults or a custom ticket and print the returned two-way quote.
#
#   1) Send RFQ with the current ticket     2) Edit the ticket
#   3) Quick presets                          4) Start / stop a local FIX edge
#   5) Settings (addresses, CompIDs)          q) Quit
#
# If a FIX acceptor is already listening (e.g. you ran
#   CELNET_FIX_ADDR=127.0.0.1:9099 ./run_dev.sh
# yourself) the script uses it as-is and never starts/stops its own.
#
# Usage:
#   tools/fix-test.sh            # interactive menu
#   tools/fix-test.sh --send     # build edge if needed, send one default RFQ, exit
#   tools/fix-test.sh -h|--help  # this help

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# GUIDE.md sources $HOME/.cargo/env for rustup-curl installs; Homebrew rustup puts
# cargo on PATH directly, so skip silently when the file is absent.
if [[ -f "$HOME/.cargo/env" ]]; then
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "fix-test.sh: cargo not on PATH. Install rustup or fix your shell." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Defaults — the ticket + wiring. Editable from the menu.
# ---------------------------------------------------------------------------
FIX_ADDR="${CELNET_FIX_ADDR:-127.0.0.1:9099}"
# A self-started edge only needs its FIX acceptor; its gRPC/WS ports are incidental.
# Default them OFF the standard 50551/8081 that `run_dev.sh` uses, so this script
# can stand up a FIX edge alongside a running run_dev.sh without an AddrInUse collision.
GRPC_ADDR="${CELNET_GRPC_ADDR:-127.0.0.1:50599}"
WS_ADDR="${CELNET_WS_ADDR:-127.0.0.1:8099}"
SENDER="${CELNET_FIX_TARGET:-CELNET-CPTY}"   # our SenderCompID == the venue's expected counterparty
TARGET="${CELNET_FIX_SENDER:-CELNET}"        # the venue's SenderCompID

PAIR="EURUSD"
OTYPE="call"
STRIKE="1.10"
EXPIRY="1.0"
SIDE="observe"
SETTLE="deliverable"
EXERCISE="european"

LOG_DIR="$REPO_ROOT/target/dev"
mkdir -p "$LOG_DIR"
EDGE_LOG="$LOG_DIR/fix-edge.log"
EDGE_PID=""   # set only if WE start the edge (then we own teardown)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
port_open() {
    local hostport="$1" host port
    host="${hostport%:*}"
    port="${hostport##*:}"
    (exec 3<>"/dev/tcp/$host/$port") 2>/dev/null && { exec 3<&- 3>&- || true; return 0; }
    return 1
}

cleanup() {
    trap - EXIT INT TERM
    if [[ -n "$EDGE_PID" ]] && kill -0 "$EDGE_PID" 2>/dev/null; then
        echo
        echo "[fix-test] stopping the FIX edge we started (pid $EDGE_PID)…"
        kill -TERM "$EDGE_PID" 2>/dev/null || true
        for _ in 1 2 3 4 5; do
            kill -0 "$EDGE_PID" 2>/dev/null || break
            sleep 0.2
        done
        kill -KILL "$EDGE_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

start_edge() {
    if port_open "$FIX_ADDR"; then
        echo "[fix-test] a FIX acceptor is already listening on $FIX_ADDR — using it."
        return 0
    fi
    echo "[fix-test] building + starting a FIX-enabled demo edge on $FIX_ADDR …"
    if ! cargo build --release -p celnet-server --example demo_edge; then
        echo "[fix-test] edge build failed." >&2
        return 1
    fi
    CELNET_FIX_ADDR="$FIX_ADDR" CELNET_GRPC_ADDR="$GRPC_ADDR" CELNET_WS_ADDR="$WS_ADDR" \
        CELNET_FIX_SENDER="$TARGET" CELNET_FIX_TARGET="$SENDER" \
        "$REPO_ROOT/target/release/examples/demo_edge" >"$EDGE_LOG" 2>&1 &
    EDGE_PID=$!
    echo -n "[fix-test] waiting for FIX $FIX_ADDR "
    for _ in $(seq 1 150); do
        if port_open "$FIX_ADDR"; then echo " ready (pid $EDGE_PID, log $EDGE_LOG)."; return 0; fi
        if ! kill -0 "$EDGE_PID" 2>/dev/null; then
            echo; echo "[fix-test] edge exited early — see $EDGE_LOG" >&2; EDGE_PID=""; return 1
        fi
        sleep 0.2; echo -n "."
    done
    echo; echo "[fix-test] timed out waiting for $FIX_ADDR — see $EDGE_LOG" >&2
    return 1
}

stop_edge() {
    if [[ -z "$EDGE_PID" ]]; then
        echo "[fix-test] no edge owned by this script (nothing to stop)."
        return 0
    fi
    kill -TERM "$EDGE_PID" 2>/dev/null || true
    sleep 0.4
    kill -KILL "$EDGE_PID" 2>/dev/null || true
    echo "[fix-test] stopped edge pid $EDGE_PID."
    EDGE_PID=""
}

send_rfq() {
    if ! port_open "$FIX_ADDR"; then
        echo "[fix-test] nothing listening on $FIX_ADDR."
        read -rp "  start a local FIX edge now? [Y/n] " ans
        case "${ans:-Y}" in
            [nN]*) echo "  aborted — start an edge first (menu option 4)."; return 1 ;;
            *) start_edge || return 1 ;;
        esac
    fi
    echo
    cargo run -q -p celnet-fix --example fix_rfq_client -- \
        --addr "$FIX_ADDR" --pair "$PAIR" --type "$OTYPE" --strike "$STRIKE" \
        --expiry-years "$EXPIRY" --side "$SIDE" --settlement "$SETTLE" \
        --exercise "$EXERCISE" --sender "$SENDER" --target "$TARGET"
    echo
}

show_ticket() {
    cat <<EOF

  current ticket
    pair        $PAIR
    type        $OTYPE
    strike      $STRIKE
    expiry      ${EXPIRY}y
    side        $SIDE
    settlement  $SETTLE
    exercise    $EXERCISE
    FIX edge    $FIX_ADDR   ($SENDER → $TARGET)
EOF
}

edit_ticket() {
    local v
    read -rp "  pair        [$PAIR] " v;     PAIR="${v:-$PAIR}"; PAIR="${PAIR^^}"
    read -rp "  type        [$OTYPE] (call/put) " v; OTYPE="${v:-$OTYPE}"
    read -rp "  strike      [$STRIKE] " v;   STRIKE="${v:-$STRIKE}"
    read -rp "  expiry yrs  [$EXPIRY] " v;   EXPIRY="${v:-$EXPIRY}"
    read -rp "  side        [$SIDE] (observe/buy/sell) " v; SIDE="${v:-$SIDE}"
    read -rp "  settlement  [$SETTLE] (deliverable/ndf) " v; SETTLE="${v:-$SETTLE}"
    read -rp "  exercise    [$EXERCISE] (european/american) " v; EXERCISE="${v:-$EXERCISE}"
}

choose_preset() {
    cat <<EOF

  presets
    1) EURUSD 1Y ATM call · observe        (the default)
    2) EURUSD 1Y 1.05 put · observe
    3) GBPUSD 6M 1.30 call · observe
    4) USDJPY 3M 150 call · buy (lift)
    5) EURUSD 1Y ATM call · sell (hit)
EOF
    local p; read -rp "  preset> " p
    case "$p" in
        1) PAIR=EURUSD OTYPE=call STRIKE=1.10 EXPIRY=1.0  SIDE=observe ;;
        2) PAIR=EURUSD OTYPE=put  STRIKE=1.05 EXPIRY=1.0  SIDE=observe ;;
        3) PAIR=GBPUSD OTYPE=call STRIKE=1.30 EXPIRY=0.5  SIDE=observe ;;
        4) PAIR=USDJPY OTYPE=call STRIKE=150  EXPIRY=0.25 SIDE=buy ;;
        5) PAIR=EURUSD OTYPE=call STRIKE=1.10 EXPIRY=1.0  SIDE=sell ;;
        *) echo "  (unchanged)"; return ;;
    esac
    SETTLE=deliverable EXERCISE=european
    echo "  ticket set."
}

edit_settings() {
    local v
    read -rp "  FIX addr    [$FIX_ADDR] " v; FIX_ADDR="${v:-$FIX_ADDR}"
    read -rp "  our SenderCompID  [$SENDER] " v; SENDER="${v:-$SENDER}"
    read -rp "  venue TargetCompID [$TARGET] " v; TARGET="${v:-$TARGET}"
}

edge_menu() {
    if [[ -n "$EDGE_PID" ]] && kill -0 "$EDGE_PID" 2>/dev/null; then
        echo "  edge: running (pid $EDGE_PID) on $FIX_ADDR"
        read -rp "  [s]top it, or anything else to leave running: " a
        [[ "${a:-}" == s* ]] && stop_edge
    elif port_open "$FIX_ADDR"; then
        echo "  edge: an external acceptor is already on $FIX_ADDR (not ours)."
    else
        read -rp "  no edge on $FIX_ADDR — start one? [Y/n] " a
        case "${a:-Y}" in [nN]*) : ;; *) start_edge ;; esac
    fi
}

# ---------------------------------------------------------------------------
# Non-interactive fast path: `--send` fires one RFQ with the defaults.
# ---------------------------------------------------------------------------
case "${1:-}" in
    -h|--help) awk 'NR==1{next} /^#/{sub(/^# ?/,""); print; next} {exit}' "$0"; exit 0 ;;
    --send) send_rfq; exit 0 ;;
    "") : ;;
    *) echo "fix-test.sh: unknown argument: $1" >&2; exit 2 ;;
esac

# ---------------------------------------------------------------------------
# Interactive menu
# ---------------------------------------------------------------------------
echo "celnet FIX RFQ test driver — edge $FIX_ADDR"
while :; do
    show_ticket
    cat <<'EOF'

  menu
    1) Send RFQ (current ticket)
    2) Edit ticket
    3) Quick presets
    4) Start / stop local FIX edge
    5) Settings (addresses, CompIDs)
    q) Quit
EOF
    read -rp "select> " choice || break
    case "$choice" in
        1) send_rfq ;;
        2) edit_ticket ;;
        3) choose_preset ;;
        4) edge_menu ;;
        5) edit_settings ;;
        q|Q|quit|exit) break ;;
        "") : ;;
        *) echo "  ? unknown choice: $choice" ;;
    esac
done

echo "bye."
