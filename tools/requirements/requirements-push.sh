#!/usr/bin/env sh
# requirements-push.sh — PUSH-BACK direction of the host-side PM bridge (R5).
#
# WHAT IT DOES: projects lodestar's VERIFIED deliverable roll-up (R2) onto the
# tracker. A ticket's "Done" is a projection of the verified roll-up — never a
# human button, never the tracker's own word. DEFAULT-OFF; host-side only.
#
# THE ROLL-UP (read, never computed by trust-widening):
#   1. Preferred: the R2 `deliverable=` filter on knowledge_get — the engine's
#      own weakest-child-state projection {deliverable, state, children}.
#   2. Fallback (engine predates the R2 filter): the weakest state across the
#      anchor's spec:satisfies claims, computed ONLY from engine-reported
#      states — any contradicted -> contradicted; else any stale -> stale;
#      else any draft/retired -> draft; ALL active -> active; none -> UNKNOWN
#      (and an unknown roll-up pushes NOTHING — never guessed).
#
# RECEIPT-FIRST ORDERING (R5-a, NabaOS): the round-trip receipt is recorded in
# the engine's append-only event log BEFORE the tracker is touched, so "Done
# without a receipt" is structurally impossible from this bridge, and every
# adapter additionally REFUSES set-status done with an empty receipt id. The
# receipt rides as a `requirements:receipt` claim through knowledge_put: the
# engine emits the ASSERT event (lode_kn_event_append) with the canonical
# sorted-key receipt JSON in its payload and author=requirements-bridge. The
# receipt claim itself is an unrecognized kind at the deterministic gate, so it
# DEFERS and stays draft forever — it is a ledger entry, never trusted
# knowledge, and can never mint a PASS.
#
# STATUS PROJECTION:  active -> done · contradicted -> blocked ·
#                     stale/draft -> in-progress · unknown -> NO PUSH (exit 3).
#
# MACHINE-READABLE FEED: every push exports the knowledge_export SARIF +
# conformance projections into out_dir and hands the SARIF path to the adapter
# (attachable). The tracker gets the same deterministic feed CI gets.
#
# USAGE:
#   requirements-push.sh --self-test
#   requirements-push.sh push <ticket> --anchor "<qn> [qn...]" [--deliverable <slug>]
#   requirements-push.sh rollup --anchor "<qn> [qn...]" [--deliverable <slug>]
#   requirements-push.sh receipt-json <ticket> <deliverable> <rollup_state> <target_hash>
#       (debug: print the canonical receipt JSON — byte-stable for equal inputs)
#
# DEGRADES WHEN ABSENT: tracker=off -> 'absent' (exit 0, engine byte-identical);
# engine missing -> exit 2; adapter unavailable -> exit 3; unknown roll-up ->
# exit 3, nothing recorded, nothing pushed.
#
# shellcheck shell=sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/lib.sh"

ALL_STATES='["draft","active","stale","contradicted","retired"]'

# claims_for <anchor_qn> -> the raw knowledge_get claims JSON (all states).
claims_for() {
    proj=$(engine_project)
    engine_tool knowledge_get \
        "{\"project\":\"$proj\",\"qualified_name\":\"$(json_escape "$1")\",\"states\":$ALL_STATES}"
}

# claim_rows <claims_json> -> one "id<TAB>kind:constraint<TAB>state" line per
# claim. Splits records on the claim-object prefix {"id":" (kn_claim_to_json
# key order: id, kind[, constraint], state, text, ... — the keys we match all
# precede the free-text field, so a claim text can never spoof them).
claim_rows() {
    printf '%s' "$1" | awk '
    {
        n = split($0, parts, /\{"id":"/)
        for (i = 2; i <= n; i++) {
            p = parts[i]; id = ""; k = ""; c = ""; s = ""
            if (match(p, /^[^"]*/))              { id = substr(p, RSTART, RLENGTH) }
            if (match(p, /"kind":"[^"]*"/))      { k = substr(p, RSTART + 8, RLENGTH - 9) }
            if (match(p, /"constraint":"[^"]*"/)) { c = substr(p, RSTART + 14, RLENGTH - 15) }
            if (match(p, /"state":"[^"]*"/))     { s = substr(p, RSTART + 9, RLENGTH - 10) }
            if (id != "" && s != "") { printf "%s\t%s:%s\t%s\n", id, k, c, s }
        }
    }'
}

# rollup_from_rows <rows> -> the weakest-child-state projection over the
# spec:satisfies rows ONLY (receipts and other kinds never count).
rollup_from_rows() {
    rf_rows=$(printf '%s\n' "$1" | awk -F '\t' '$2 == "spec:satisfies" { print $3 }')
    rf_n=$(printf '%s' "$rf_rows" | grep -c . || true)
    if [ "$rf_n" -eq 0 ]; then printf 'unknown'; return 0; fi
    if printf '%s\n' "$rf_rows" | grep -q '^contradicted$'; then printf 'contradicted'; return 0; fi
    if printf '%s\n' "$rf_rows" | grep -q '^stale$'; then printf 'stale'; return 0; fi
    if printf '%s\n' "$rf_rows" | grep -qE '^(draft|retired)$'; then printf 'draft'; return 0; fi
    printf 'active'
}

# read_rollup <anchors> <deliverable> — sets ROLLUP_STATE + ROLLUP_SOURCE +
# TARGET_HASH (the engine-captured design-target content id, "" when absent;
# the bridge NEVER re-derives a hash — one hasher, the engine's).
read_rollup() {
    rr_anchors="$1"; rr_deliv="$2"
    ROLLUP_STATE="unknown"; ROLLUP_SOURCE="none"; TARGET_HASH=""
    rr_first=${rr_anchors%% *}
    proj=$(engine_project)

    # Preferred: the engine's own R2 deliverable projection. The response key
    # is `deliverable_rollup` (mcp.c handle_knowledge_get); its object opens
    # {"deliverable":"<slug>","state":"<state>",...} so the state is reachable
    # before any nested brace. The engine projection is authoritative whenever
    # it actually attributed children to the slug (children_count > 0) OR an
    # unreadable committed target floored it (unreadable_targets > 0); a
    # zero-knowledge roll-up (0 children, 0 unreadable — e.g. claims authored
    # without committed targets) defers to the anchor-fallback below, which
    # only ever projects engine-reported claim states.
    if [ -n "$rr_deliv" ]; then
        rr_out=$(engine_tool knowledge_get \
            "{\"project\":\"$proj\",\"qualified_name\":\"$(json_escape "$rr_first")\",\"deliverable\":\"$(json_escape "$rr_deliv")\",\"states\":$ALL_STATES}" 2>/dev/null) || rr_out=""
        case "$rr_out" in
            *'"deliverable_rollup"'*)
                rr_state=$(printf '%s' "$rr_out" | sed -n 's/.*"deliverable_rollup":{[^}]*"state":"\([a-z]*\)".*/\1/p' | head -n1)
                rr_kids=$(printf '%s' "$rr_out" | sed -n 's/.*"children_count":\([0-9]*\).*/\1/p' | head -n1)
                rr_unread=$(printf '%s' "$rr_out" | sed -n 's/.*"unreadable_targets":\([0-9]*\).*/\1/p' | head -n1)
                if [ -n "$rr_state" ] && { [ "${rr_kids:-0}" -gt 0 ] || [ "${rr_unread:-0}" -gt 0 ]; }; then
                    ROLLUP_STATE="$rr_state"; ROLLUP_SOURCE="deliverable-filter"
                fi ;;
        esac
    fi

    # Fallback: weakest state over the anchors' spec:satisfies claims.
    rr_all=""
    for qn in $rr_anchors; do
        rr_json=$(claims_for "$qn") || continue
        rr_all="$rr_all
$(claim_rows "$rr_json")"
        if [ -z "$TARGET_HASH" ]; then
            TARGET_HASH=$(printf '%s' "$rr_json" \
                | sed -n 's/.*"qualified_name":"design-target:[^"]*","node_content_hash":"\([0-9a-f]\{16\}\)".*/\1/p' | head -n1)
        fi
    done
    rr_all=$(printf '%s\n' "$rr_all" | grep -v '^$' | sort -u || true)
    if [ "$ROLLUP_SOURCE" = "none" ]; then
        ROLLUP_STATE=$(rollup_from_rows "$rr_all")
        ROLLUP_SOURCE="anchor-fallback"
    fi
    ROLLUP_ROWS="$rr_all"
}

status_for_state() {
    case "$1" in
        active)       printf 'done' ;;
        contradicted) printf 'blocked' ;;
        stale|draft)  printf 'in-progress' ;;
        *)            printf '' ;;
    esac
}

# export_feed <deliverable_or_ticket> -> writes the SARIF + conformance
# projections; prints the SARIF path (the adapter attachment).
export_feed() {
    proj=$(engine_project)
    mkdir -p "$REQ_OUT_DIR" 2>/dev/null || true
    ef_base="$REQ_OUT_DIR/$(printf '%s' "$1" | tr '/ :' '___')"
    if engine_tool knowledge_export "{\"project\":\"$proj\",\"format\":\"sarif\",\"filter\":\"all\"}" > "$ef_base.sarif.json" 2>/dev/null; then
        log "[RQ] machine-readable feed: $ef_base.sarif.json (SARIF 2.1.0; active->pass, never a non-active pass)"
    else
        rm -f "$ef_base.sarif.json"; log "[RQ] SARIF export unavailable."
    fi
    if engine_tool knowledge_export "{\"project\":\"$proj\",\"format\":\"conformance\",\"filter\":\"all\"}" > "$ef_base.conformance.md" 2>/dev/null; then
        log "[RQ] conformance sheet: $ef_base.conformance.md"
    else
        rm -f "$ef_base.conformance.md"
    fi
    [ -f "$ef_base.sarif.json" ] && printf '%s' "$ef_base.sarif.json" || printf ''
}

do_rollup() {
    ra=""; rd=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --anchor) ra="${2:-}"; shift 2 ;;
            --deliverable) rd="${2:-}"; shift 2 ;;
            *) die "unknown option '$1'" ;;
        esac
    done
    [ -n "$ra" ] || die "usage: rollup --anchor \"<qn> [qn...]\" [--deliverable <slug>]"
    engine_present || { log "[RQ] engine '$(engine_bin)' not found."; return 2; }
    read_rollup "$ra" "$rd"
    log "[RQ] children:"
    printf '%s\n' "$ROLLUP_ROWS" | sed 's/^/[RQ]   /' >&2
    printf 'ROLLUP: state=%s source=%s target_hash=%s\n' "$ROLLUP_STATE" "$ROLLUP_SOURCE" "${TARGET_HASH:--}"
}

do_push() {
    ticket="$1"; shift
    pa=""; pd=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --anchor) pa="${2:-}"; shift 2 ;;
            --deliverable) pd="${2:-}"; shift 2 ;;
            *) die "unknown option '$1'" ;;
        esac
    done
    [ -n "$pa" ] || die "usage: push <ticket> --anchor \"<qn> [qn...]\" [--deliverable <slug>]"

    if [ "$REQ_TRACKER" = "off" ]; then
        log "[RQ] tracker=off (default): requirements bridge OFF. Engine byte-identical; nothing pushed."
        printf 'absent\n'; return 0
    fi
    engine_present || { log "[RQ] engine '$(engine_bin)' not found; cannot read the roll-up (set LODESTAR_BIN)."; return 2; }
    resolve_adapter || { log "[RQ] no adapter for tracker '$REQ_TRACKER' -> unsupported."; return 3; }
    proj=$(engine_project)

    read_rollup "$pa" "$pd"
    log "[RQ] verified roll-up: state=$ROLLUP_STATE (source=$ROLLUP_SOURCE, target_hash=${TARGET_HASH:--})"
    status=$(status_for_state "$ROLLUP_STATE")
    if [ -z "$status" ]; then
        log "[RQ] roll-up UNKNOWN (no spec:satisfies claims on the anchors) -> NOTHING pushed, NOTHING recorded (never guessed)."
        return 3
    fi

    # ── RECEIPT FIRST (R5-a). The push receipt is tool-output: it reports the
    # engine's own verified roll-up, not anyone's testimony.
    receipt=$(build_receipt_json "${pd:-$ticket}" "status-push" "tool-output" "$ROLLUP_STATE" "$TARGET_HASH" "$ticket")
    first_anchor=${pa%% *}
    rput="{\"project\":\"$proj\",\"kind\":\"requirements:receipt\",\"text\":\"$(json_escape "$receipt")\",\"confidence\":\"tool-output\",\"author\":\"$(json_escape "$REQ_AUTHOR")\",\"anchors\":[{\"qualified_name\":\"$(json_escape "$first_anchor")\"}]}"
    rout=$(engine_tool knowledge_put "$rput") \
        || { log "[RQ] receipt write FAILED -> tracker untouched (Done requires a receipt; never push without one)."; return 1; }
    receipt_id=$(printf '%s' "$rout" | json_get id)
    log "[RQ] round-trip receipt recorded: claim=$receipt_id (ASSERT event in .lodestar/knowledge/events/, author=$REQ_AUTHOR)"
    if [ -d ".lodestar/knowledge/events" ]; then
        if grep -l "tracker-roundtrip" .lodestar/knowledge/events/*.json >/dev/null 2>&1; then
            log "[RQ] receipt event verified present in the event log."
        fi
    fi

    # ── the machine-readable feed rides with the status.
    feed=$(export_feed "${pd:-$ticket}")

    # ── project the status onto the tracker (the adapter refuses done w/o receipt).
    if ack=$(run_adapter set-status "$ticket" "$status" "$receipt_id" "$feed"); then
        log "[RQ] tracker ack: $ack"
    else
        log "[RQ] tracker update FAILED. The receipt exists ($receipt_id) but the ticket did not flip — the safe failure direction (a receipt without Done is honest; Done without a receipt never happens)."
        printf 'PUSH: ticket=%s status=FAILED rollup=%s receipt=%s source=%s\n' \
            "$ticket" "$ROLLUP_STATE" "$receipt_id" "$ROLLUP_SOURCE"
        return 1
    fi

    printf 'PUSH: ticket=%s status=%s rollup=%s receipt=%s source=%s feed=%s\n' \
        "$ticket" "$status" "$ROLLUP_STATE" "$receipt_id" "$ROLLUP_SOURCE" "${feed:--}"
}

self_test() {
    log "[RQ] self-test: capability dry-run (NO roll-up read, NO write, NO push)."
    if [ "$REQ_TRACKER" = "off" ]; then
        log "[RQ] tracker OFF (default). Bridge absent; engine byte-identical. exit 0."
        return 0
    fi
    engine_present || { log "[RQ] engine '$(engine_bin)' not found (set LODESTAR_BIN). exit 2."; return 2; }
    resolve_adapter || { log "[RQ] adapter for '$REQ_TRACKER' unavailable. exit 3."; return 3; }
    log "[RQ] ready: tracker=$REQ_TRACKER adapter=$REQ_ADAPTER project=$(engine_project) out_dir=$REQ_OUT_DIR"
    log "[RQ] honesty: Done is a projection of the verified roll-up WITH a receipt; an unknown roll-up pushes nothing."
    return 0
}

cmd="${1:-}"
rc=0
case "$cmd" in
    --self-test|self-test) self_test || rc=$? ;;
    push)   [ $# -ge 2 ] || die "usage: push <ticket> --anchor \"<qn> [qn...]\" [--deliverable <slug>]"
            t="$2"; shift 2; do_push "$t" "$@" || rc=$? ;;
    rollup) shift; do_rollup "$@" || rc=$? ;;
    receipt-json) [ $# -ge 5 ] || die "usage: receipt-json <ticket> <deliverable> <rollup_state> <target_hash>"
            build_receipt_json "$3" "status-push" "tool-output" "$4" "$5" "$2"; printf '\n' ;;
    ""|-h|--help) sed -n '2,49p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try --self-test, push, rollup, receipt-json)" ;;
esac
exit "$rc"
