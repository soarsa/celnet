#!/usr/bin/env bash
# run_dev.sh — THE single-command local bring-up of Celnet.
#
# Replaces the previous split between `tools/dev.sh` (demo_edge + GUI, no
# simulators) and a separate full-stack script: one entry point, flags to vary it.
#
# Starts, in dependency order:
#
#   1. celnet-server        gRPC 127.0.0.1:50551 · ws://127.0.0.1:8081
#   2. lp-sim               N liquidity providers -> the LpFeed gRPC ingest.
#                           Each streams the WHOLE quotable universe: cash bonds,
#                           the listed Treasury FUTURES complex, the swap/OIS curve
#                           points and the SOFR STIR strip (`lpsim::quotable_lines`),
#                           filtered to what is still listed at the settlement date.
#   3. fix_rfq_client (RFQ) one-shot QuoteRequest(R) -> the FIXED_INCOME_QUOTE venue
#   4. fix_rfq_client (RFS) MarketDataRequest(V) stream -> the FIXED_INCOME_STREAM venue
#   5. gui                  Vite dev server on http://localhost:5173
#
# Those inbound connections are exactly what Administration -> LP Panel reports on.
#
# ## The two FI venues are SEPARATE acceptors, and that is not optional
#
# The server gates each dialect on the acceptor's own kind (`rates_intent_for_kind`):
# a `MarketDataRequest(V)` arriving at the RFQ acceptor is dropped on the floor, and a
# `QuoteRequest(R)` arriving at the stream acceptor likewise — no reject, no log line.
# This script used to discover ONE acceptor and point both legs at it, so whichever
# leg mismatched was silently dead and the stack looked healthy while producing half
# the flow. It now ensures one venue of EACH kind and gives each leg its own port.
#
# "RFS" here is the venue that used to be labelled ESP. It is request-for-stream:
# the client names an instrument AND its own clip size, and the stream is priced for
# that clip. An executable streaming price is dealer-published and clip-independent,
# so this venue cannot serve one.
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
# ## It clears the decks first
#
# A previous stack — crashed, backgrounded, or left in another terminal — keeps
# its ports bound, and the server then dies on startup with
# `Os { code: 48, AddrInUse }`. Teardown-on-exit cannot help: the whole problem is
# a process that did NOT exit cleanly. So the script sweeps on ENTRY too.
#
# It kills by PORT OWNERSHIP (whatever holds gRPC / WS / the GUI port, found via
# `lsof`) rather than by name, so it reclaims the ports it actually needs whoever
# owns them, and by process IMAGE for the simulators, which bind nothing and so
# cannot be found by port. Then it WAITS for the ports to be released: SIGKILL is
# asynchronous, and returning before the kernel has torn the socket down is
# exactly what re-creates the AddrInUse it was trying to prevent.
#
# `--no-sweep` opts out — use it when deliberately running a second stack on
# different ports, since the simulator sweep is image-matched and would take the
# other stack's sims with it.
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
#   ./run_dev.sh --no-sweep       # do NOT kill a stack already running (see below)
#
# Env overrides: CELNET_GRPC_ADDR, CELNET_WS_ADDR, CELNET_GUI_PORT,
#   CELNET_DEV_FIX_RFQ_PORT / CELNET_DEV_FIX_RFS_PORT (skip discovery for that venue),
#   CELNET_DEV_RFQ_PORT / CELNET_DEV_RFS_PORT (bind ports used when CREATING a venue),
#   CELNET_DEV_SIM_USER/_PASSWORD, CELNET_DEV_LPSIM_MEMBERS,
#   CELNET_DEV_LPSIM_INTERVAL, CELNET_DEV_BOOK.

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
# The two fixed-income FIX venues the fleet drives. They MUST be separate
# acceptors: the server gates each dialect on the acceptor's own kind
# (`rates_intent_for_kind`), so a MarketDataRequest sent to the RFQ acceptor —
# or a QuoteRequest sent to the stream acceptor — is silently ignored. Pointing
# both legs at one port, which this script used to do, therefore left one of the
# two flows dead with no error anywhere.
DEV_RFQ_PORT="${CELNET_DEV_RFQ_PORT:-9101}"
DEV_RFS_PORT="${CELNET_DEV_RFS_PORT:-9102}"

SKIP_BUILD=0
RUN_GUI=1
RUN_SIMS=1
DEMO_EDGE=0
SWEEP=1
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=1 ;;
        --no-gui)     RUN_GUI=0 ;;
        --no-sims)    RUN_SIMS=0 ;;
        --demo-edge)  DEMO_EDGE=1 ;;
        --no-sweep)   SWEEP=0 ;;
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
# 0. Clear the decks — see the header note on why entry-sweep is not optional.
# ---------------------------------------------------------------------------
port_owners() {  # port_owners <port…> -> pids holding a LISTEN socket
    command -v lsof >/dev/null 2>&1 || return 0
    local args=() p
    for p in "$@"; do args+=(-iTCP:"$p"); done
    lsof -nP "${args[@]}" -sTCP:LISTEN -t 2>/dev/null | sort -u || true
}

port_free() {  # port_free <host> <port>
    ! (exec 3<>"/dev/tcp/$1/$2") 2>/dev/null
}

preflight_sweep() {
    local grpc_port="${GRPC_ADDR##*:}" ws_port="${WS_ADDR##*:}"
    local victims
    # Whoever holds the ports we need, plus the simulators (which bind nothing,
    # so they can only be matched by image).
    victims="$(port_owners "$grpc_port" "$ws_port" "$GUI_PORT")"
    victims="$victims
$(pgrep -u "$(id -u)" -f 'target/debug/lp-sim|examples/fix_rfq_client' 2>/dev/null || true)"
    victims="$(echo "$victims" | tr ' ' '\n' | grep -E '^[0-9]+$' | sort -u || true)"
    [[ -z "$victims" ]] && return 0

    echo "[dev] a stack is already running — reclaiming ports $grpc_port/$ws_port/$GUI_PORT"
    # shellcheck disable=SC2086
    kill -TERM $victims 2>/dev/null || true
    sleep 1
    # shellcheck disable=SC2086
    kill -KILL $victims 2>/dev/null || true

    # SIGKILL is asynchronous: wait for the sockets to actually be released, or the
    # bind below races a corpse that still holds them.
    local waited=0
    while (( waited < 20 )); do
        if port_free "${GRPC_ADDR%:*}" "$grpc_port" && port_free "${WS_ADDR%:*}" "$ws_port"; then
            echo "[dev] ports released."
            return 0
        fi
        sleep 0.5
        waited=$(( waited + 1 ))
    done
    echo "[dev] WARNING: a port is still bound after 10s — the bind below may fail." >&2
}

# NOT `[[ … ]] && preflight_sweep` — with --no-sweep that whole statement evaluates
# false, and under `set -e` a false statement at top level exits the script.
if [[ $SWEEP -eq 1 ]]; then
    preflight_sweep
fi

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
FIX_RFQ_PORT="${CELNET_DEV_FIX_RFQ_PORT:-}"
FIX_RFS_PORT="${CELNET_DEV_FIX_RFS_PORT:-}"
if [[ $RUN_SIMS -eq 1 ]]; then
    PROVISION_JS="$LOG_DIR/provision.mjs"
    cat > "$PROVISION_JS" <<'PROVISION'
import WebSocket from '../../gui/node_modules/ws/wrapper.mjs';
const [wsUrl, email, password, bookId, members] = process.argv.slice(2);
// argv[7]/argv[8] are the RFQ / RFS bind ports (read inside ensureVenue).
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

    // The two fixed-income venues are SEPARATE acceptors, because the server gates
    // each dialect on the acceptor's own kind: kind 1 (FIXED_INCOME_QUOTE) serves the
    // one-shot QuoteRequest(R), kind 2 (FIXED_INCOME_STREAM) serves the
    // MarketDataRequest(V) stream. A leg pointed at the wrong kind is IGNORED, with no
    // reject and no log — so each leg gets its own port, and a missing venue is
    // created rather than silently skipped.
    //
    // Every connection belongs to a desk (the server rejects a blank one) — but the
    // desk also decides who SEES the venue's traffic: the server drops RFQ/deal
    // notifications for a venue whose desk has no user on it, and says so at boot.
    // So prefer a desk the sim user actually belongs to over just any desk.
    const me = login.user || {};
    let desks = (await send('list_desks', { session_token: token })).desks || [];
    const mine = me.all_desks ? desks.map((d) => d.id) : (me.desk_ids || []);
    let deskId = mine[0] || '';
    if (!deskId) {
      const made = await send('create_desk', { session_token: token, name: 'Dev Desk' });
      deskId = (made.desk && made.desk.id) || '';
      if (deskId) {
        console.error('[dev] created desk `' + deskId + '`');
        console.error('[dev]   NOTE: ' + (me.email || 'the sim user') + ' is not a member of it, so '
          + 'desk-routed notifications for the dev venues will be DROPPED until you add them '
          + '(Administration → Users).');
      }
    }

    // Reuse a RUNNING acceptor of each kind; otherwise define one. Idempotent — an
    // existing venue is never replaced, only adopted.
    const ensureVenue = async (kind, id, name, port) => {
      const conns = (await send('list_fix_connections', { session_token: token })).connections || [];
      const live = conns.find((c) => c.kind === kind && c.running && c.bound_addr);
      if (live) return String(live.bound_addr).split(':').pop();
      if (!deskId) { console.error('[dev] no desk — cannot define the ' + name + ' venue'); return ''; }
      // A defined-but-stopped venue of this kind is left alone rather than fought over.
      if (conns.some((c) => c.id === id)) {
        console.error('[dev] ' + name + ' venue `' + id + '` exists but is not running — leaving it as is');
        return '';
      }
      const spec = { id, name, kind, bind_addr: '127.0.0.1:' + port,
                     sender_comp_id: 'CELNET', target_comp_id: 'CELNET-CPTY',
                     enabled: true, desk: deskId };
      const made = await send('create_fix_connection', { session_token: token, spec });
      if (made.error) { console.error('[dev] could not create ' + name + ': ' + made.error); return ''; }
      console.error('[dev] created ' + name + ' venue `' + id + '` on 127.0.0.1:' + port);
      return String(port);
    };

    const rfqPort = await ensureVenue(1, 'dev-fi-rfq', 'FI RFQ', process.argv[7]);
    const rfsPort = await ensureVenue(2, 'dev-fi-rfs', 'FI RFS (stream)', process.argv[8]);
    console.log('FIX_RFQ_PORT=' + rfqPort);
    console.log('FIX_RFS_PORT=' + rfsPort);

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
        PROV_OUT="$(cd "$LOG_DIR" && node provision.mjs "ws://$WS_ADDR" "$SIM_USER" "$SIM_PASSWORD" "$DEV_BOOK" "$LPSIM_MEMBERS" "$DEV_RFQ_PORT" "$DEV_RFS_PORT" 2>&1 || true)"
        echo "$PROV_OUT" | grep -v '^FIX_RFQ_PORT=\|^FIX_RFS_PORT=' || true
        [[ -z "$FIX_RFQ_PORT" ]] && FIX_RFQ_PORT="$(echo "$PROV_OUT" | sed -n 's/^FIX_RFQ_PORT=//p' | tail -1)"
        [[ -z "$FIX_RFS_PORT" ]] && FIX_RFS_PORT="$(echo "$PROV_OUT" | sed -n 's/^FIX_RFS_PORT=//p' | tail -1)"
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

    FIX_CLIENT="$REPO_ROOT/target/debug/examples/fix_rfq_client"

    # RFQ flow — one-shot QuoteRequest(R) with SubscriptionRequestType(263)=0 against
    # the FIXED_INCOME_QUOTE venue. Only that acceptor kind answers it.
    if [[ -n "$FIX_RFQ_PORT" ]]; then
        echo "[dev] starting FIX RFQ leg → 127.0.0.1:$FIX_RFQ_PORT"
        "$FIX_CLIENT" --addr "127.0.0.1:$FIX_RFQ_PORT" --asset fi --intent rfq \
            --repeat 0 --interval 3 \
            > >(tee "$LOG_DIR/fix-rfq.log" | sed -u 's/^/[rfq ] /') 2>&1 &
        PIDS+=($!)
    else
        echo "[dev] no FI RFQ venue — SKIPPING the RFQ leg (see the provision output above)."
    fi

    # The RFS leg downloads its instrument list over gRPC first, so it needs a service
    # credential. The client REFUSES a password on argv (argv is world-readable) and
    # prefers a 0600 file over the environment — so hand it exactly that, written under
    # target/dev/ with the dev credential this script already holds. Without this the
    # leg exits at startup with "no RFS service credential configured", which is why the
    # streaming flow has never actually run from this script.
    FIXSIM_PW_FILE="$LOG_DIR/.fixsim_pw"
    ( umask 177; printf '%s' "$SIM_PASSWORD" > "$FIXSIM_PW_FILE" )
    chmod 600 "$FIXSIM_PW_FILE"

    # RFS flow — MarketDataRequest(V) subscribe against the FIXED_INCOME_STREAM venue,
    # which streams a two-way priced for the CLIENT'S OWN clip and books an RFS deal on
    # a lift. This is the venue that used to be called ESP; it is request-for-stream,
    # not an executable streaming price, because the client supplies the notional.
    if [[ -n "$FIX_RFS_PORT" ]]; then
        echo "[dev] starting FIX RFS (streaming) leg → 127.0.0.1:$FIX_RFS_PORT"
        FIXSIM_USER="$SIM_USER" FIXSIM_PASSWORD_FILE="$FIXSIM_PW_FILE" \
        "$FIX_CLIENT" --addr "127.0.0.1:$FIX_RFS_PORT" --asset rfs \
            --grpc-addr "http://$GRPC_ADDR" --repeat 0 --interval 3 \
            > >(tee "$LOG_DIR/fix-rfs.log" | sed -u 's/^/[rfs ] /') 2>&1 &
        PIDS+=($!)
    else
        echo "[dev] no FI STREAM venue — SKIPPING the RFS leg (see the provision output above)."
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
    [[ -n "$FIX_RFQ_PORT" ]] && echo "[dev]            FIX RFQ leg          → 127.0.0.1:$FIX_RFQ_PORT"
    [[ -n "$FIX_RFS_PORT" ]] && echo "[dev]            FIX RFS stream leg   → 127.0.0.1:$FIX_RFS_PORT"
    echo "[dev]   LP Panel Administration → LP Panel"
fi
echo "[dev]   logs     $LOG_DIR/{edge,lp-sim,fix-rfq,fix-rfs,gui}.log"
echo "[dev] ─────────────────────────────────────────────── Ctrl-C to stop ──"
wait
