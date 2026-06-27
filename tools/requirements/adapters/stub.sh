#!/usr/bin/env sh
# adapters/stub.sh — LOCAL, FILE-BASED tracker adapter (the sandbox tracker).
#
# A deterministic, network-free tracker that speaks the full vendor-neutral
# adapter contract (see ../lib.sh). It exists so (a) the R5 round-trip gate can
# run against a REAL adapter without any external service — stubbing the
# EXTERNAL tracker is correct here, the engine is never stubbed — and (b)
# air-gapped machines get a working local requirements surface.
#
# State layout (LODESTAR_REQ_STUB_DIR, default .lodestar/requirements/stub):
#   epics/<epic_id>.stream   the canonical requirement stream, verbatim
#   tickets/<ticket_id>      one line: "<status> <receipt_id>"
#
# Extra (stub-only) command:
#   stub.sh seed-epic <epic_id>   reads a canonical stream from stdin and
#                                 stores it (how a "tracker-side" epic exists).
#
# HONESTY: set-status done with an EMPTY receipt id is REFUSED (exit 1) and the
# stored status is left untouched — "Done requires a roll-up receipt" is
# enforced at the tracker boundary too, not only in the bridge (R5-a).
#
# shellcheck shell=sh
set -eu

STUB_DIR="${LODESTAR_REQ_STUB_DIR:-.lodestar/requirements/stub}"

log() { printf '%s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

capabilities() {
    printf '{"tracker":"stub","ready":true,"network":false,"state_dir":"%s"}\n' "$STUB_DIR"
}

seed_epic() {
    se_id="$1"
    mkdir -p "$STUB_DIR/epics"
    cat > "$STUB_DIR/epics/$se_id.stream"
    log "[stub] epic '$se_id' seeded ($STUB_DIR/epics/$se_id.stream)"
}

fetch_epic() {
    fe_id="$1"
    fe_f="$STUB_DIR/epics/$fe_id.stream"
    [ -f "$fe_f" ] || die "stub tracker has no epic '$fe_id' (seed it: stub.sh seed-epic $fe_id < stream)"
    cat "$fe_f"
}

get_status() {
    gs_id="$1"
    gs_f="$STUB_DIR/tickets/$gs_id"
    if [ -f "$gs_f" ]; then
        gs_line=$(cat "$gs_f")
        gs_status=${gs_line%% *}
        printf '{"ticket":"%s","status":"%s"}\n' "$gs_id" "$gs_status"
    else
        printf '{"ticket":"%s","status":"todo"}\n' "$gs_id"
    fi
}

set_status() {
    ss_id="$1"; ss_status="$2"; ss_receipt="${3:-}"
    if [ "$ss_status" = "done" ] && [ -z "$ss_receipt" ]; then
        log "[stub] REFUSED: status=done requires a roll-up receipt id (none supplied). Status unchanged."
        printf '{"ticket":"%s","status":"done","ok":false,"error":"receipt_required"}\n' "$ss_id"
        exit 1
    fi
    mkdir -p "$STUB_DIR/tickets"
    printf '%s %s\n' "$ss_status" "$ss_receipt" > "$STUB_DIR/tickets/$ss_id"
    printf '{"ticket":"%s","status":"%s","ok":true,"receipt":"%s"}\n' "$ss_id" "$ss_status" "$ss_receipt"
}

cmd="${1:-}"
case "$cmd" in
    capabilities) capabilities ;;
    seed-epic)  [ $# -ge 2 ] || die "usage: seed-epic <epic_id> < stream"; seed_epic "$2" ;;
    fetch-epic) [ $# -ge 2 ] || die "usage: fetch-epic <epic_id>"; fetch_epic "$2" ;;
    get-status) [ $# -ge 2 ] || die "usage: get-status <ticket>"; get_status "$2" ;;
    set-status) [ $# -ge 3 ] || die "usage: set-status <ticket> <status> <receipt_id> [feed_file]"
                set_status "$2" "$3" "${4:-}" ;;
    ""|-h|--help) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try capabilities, seed-epic, fetch-epic, get-status, set-status)" ;;
esac
