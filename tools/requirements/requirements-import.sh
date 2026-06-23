#!/usr/bin/env sh
# requirements-import.sh — IMPORT direction of the host-side PM bridge (R5).
#
# WHAT IT DOES: reads an epic and its child requirements FROM the tracker (via
# the vendor-neutral adapter contract, ../requirements/lib.sh) and authors each
# requirement as a DRAFT `spec:satisfies` claim through the engine's public
# knowledge_put seam. DEFAULT-OFF; the engine never links tracker code and is
# byte-identical with this tool absent.
#
# EPISTEMIC POSTURE (R5-a, NabaOS tool-receipt provenance):
#   - Every imported claim is TESTIMONY: the tracker's word, not verified
#     knowledge. The claim text carries a machine-readable
#     [epistemic_source=testimony ...] tag, the author is the bridge identity
#     ("requirements-bridge"), and the engine's own ASSERT event — emitted by
#     knowledge_put through lode_kn_event_append into the append-only event log
#     (.lodestar/knowledge/events/) — IS the import receipt. The receipt is the
#     event already emitted; the bridge only tags its epistemic source.
#   - Claims enter as draft. spec:satisfies decomposition can ACTIVATE only
#     through the deterministic gate (R1) or the cross-family review — never
#     because a tracker said so.
#
# CROSS-FAMILY REVIEW ROUTING (R5-b, GSAR typed grounding): every authored
# claim is routed through the shipped knowledge_review seam BEFORE its
# decomposition is trusted. With review_cmd configured, the bridge invokes it
# per claim (it must drive/record a CROSS-FAMILY verdict through the engine —
# e.g. a tools/judge driver); without it, the claims stay draft testimony and
# the exact review command is printed. The engine enforces never-self.
#
# VERITRANS AUTHORING-CONFIDENCE HINT (§6.4 FLAG — read this twice):
#   confidence_cmd, when configured, annotates each DRAFT claim with a
#   round-trip NL<->PL confidence value — a review-PRIORITIZATION hint for the
#   human queue, stored only in the claim's free-text confidence field. IT
#   NEVER GATES: there is deliberately NO threshold branch anywhere in this
#   file — a 0.01-confidence requirement is authored exactly like a 0.99 one.
#   The coverage-threshold acceptance use is PERMANENTLY REJECTED (plan §5).
#
# USAGE:
#   requirements-import.sh --self-test          # capability dry-run; no fetch, no write
#   requirements-import.sh capabilities         # JSON: tracker/adapter/engine readiness
#   requirements-import.sh import <epic_id>     # tracker epic -> draft claims -> review route
#
# DEGRADES WHEN ABSENT (honest seam, the visual/judge pattern):
#   - tracker=off (default)  -> 'absent'; engine byte-identical; nothing written.
#   - engine binary missing  -> exit 2, records nothing.
#   - adapter unavailable    -> exit 3 (or the adapter's 4 no-key / 5 no-curl).
#   - requirement w/o anchors-> reported SKIP (a claim must anchor to the
#                                graph; never guessed), import continues.
#
# shellcheck shell=sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/lib.sh"

capabilities() {
    ad=false; adname="-"
    if [ "$REQ_TRACKER" != "off" ] && resolve_adapter; then ad=true; adname="$REQ_ADAPTER"; fi
    en=$(engine_present && echo true || echo false)
    rv=$([ -n "$REQ_REVIEW_CMD" ] && echo true || echo false)
    cf=$([ -n "$REQ_CONFIDENCE_CMD" ] && echo true || echo false)
    log "[RQ] tracker=$REQ_TRACKER adapter=$adname engine=$en review_cmd=$rv confidence_cmd=$cf"
    printf '{"tracker":"%s","adapter":%s,"engine":%s,"review_cmd":%s,"confidence_cmd":%s,"author":"%s"}\n' \
        "$REQ_TRACKER" "$ad" "$en" "$rv" "$cf" "$REQ_AUTHOR"
}

# confidence_hint <text> <target> -> free-text annotation for the claim's
# confidence field ("testimony" when no confidence_cmd is configured).
# DRAFT-ONLY HINT: the return value is stored verbatim and NEVER branched on.
confidence_hint() {
    if [ -z "$REQ_CONFIDENCE_CMD" ]; then
        printf 'testimony'
        return 0
    fi
    ch_line=$("$REQ_CONFIDENCE_CMD" "$1" "$2" 2>/dev/null) || ch_line=""
    ch_val=$(printf '%s' "$ch_line" | sed -n 's/.*CONFIDENCE:[[:space:]]*\([0-9.][0-9.]*\).*/\1/p' | head -n1)
    if [ -n "$ch_val" ]; then
        # The hint rides along; the epistemic source stays testimony. No
        # comparison against any threshold happens here or anywhere (§6.4).
        printf 'testimony; authoring-hint:roundtrip=%s' "$ch_val"
    else
        log "[RQ] confidence_cmd produced no parseable CONFIDENCE line -> claim annotated testimony only (hint omitted, never guessed)."
        printf 'testimony'
    fi
}

# route_review <claim_id> — R5-b: the imported claim meets the devil's-advocate
# pass before its decomposition is trusted.
route_review() {
    rr_id="$1"
    if [ -n "$REQ_REVIEW_CMD" ]; then
        log "[RQ] routing claim $rr_id through cross-family review via review_cmd."
        if ! "$REQ_REVIEW_CMD" "$rr_id"; then
            log "[RQ] review_cmd failed for $rr_id -> claim stays draft testimony (honest; never auto-trusted)."
            return 1
        fi
        return 0
    fi
    # No reviewer configured: verify the claim is review-ready (the engine
    # serves its rubric) and print the exact command. The claim stays draft.
    proj=$(engine_project)
    rr_brief=$(engine_tool knowledge_review "{\"project\":\"$proj\",\"claim_id\":\"$rr_id\"}") || rr_brief=""
    case "$rr_brief" in
        *'"rubric"'*) log "[RQ] claim $rr_id is review-ready (rubric served). Run a cross-family reviewer, e.g.:" ;;
        *)            log "[RQ] claim $rr_id: review brief unavailable (claim may need re-author)." ;;
    esac
    log "[RQ]   sh tools/judge/judge-ollama.sh judge $rr_id"
    return 2
}

do_import() {
    epic="$1"
    if [ "$REQ_TRACKER" = "off" ]; then
        log "[RQ] tracker=off (default): requirements bridge OFF. Engine byte-identical; nothing imported."
        printf 'absent\n'; return 0
    fi
    if ! engine_present; then
        log "[RQ] engine binary '$(engine_bin)' not found; cannot author claims (set LODESTAR_BIN)."
        return 2
    fi
    if ! resolve_adapter; then
        log "[RQ] no adapter for tracker '$REQ_TRACKER' (adapters/$REQ_TRACKER.sh missing and no adapter_cmd) -> unsupported."
        return 3
    fi
    proj=$(engine_project)

    stream=$(run_adapter fetch-epic "$epic") || return $?
    header=$(printf '%s\n' "$stream" | head -n1)
    deliv=$(line_get deliverable "$header")
    etitle=$(line_get title "$header")
    log "[RQ] epic=$epic deliverable=${deliv:--} title=${etitle:--} (tracker=$REQ_TRACKER)"

    authored=0; skipped=0; failed=0; reviewed=0; pending=0
    ids=""
    # Body lines: one flat requirement object per line (the adapter contract).
    body=$(printf '%s\n' "$stream" | sed -n '2,$p')
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        ticket=$(line_get ticket "$line")
        rtitle=$(line_get title "$line")
        rtext=$(line_get text "$line")
        anchors=$(line_get anchors "$line")
        target=$(line_get target "$line")
        [ -n "$ticket" ] || continue

        if [ -z "$anchors" ]; then
            log "[RQ] SKIP $ticket: no anchors in the tracker item — a claim must anchor to live graph symbols (never guessed)."
            skipped=$((skipped+1))
            continue
        fi

        conf=$(confidence_hint "$rtext" "$target")

        # Anchor set: the tracker-named graph symbols, plus the committed
        # acceptance target as the design-target sentinel when present (the
        # R1 spec:satisfies carrier — content-addressed, swept for staleness
        # by the existing resolver; no new invalidation path).
        ajson=""
        for qn in $anchors; do
            ajson="$ajson{\"qualified_name\":\"$(json_escape "$qn")\"},"
        done
        if [ -n "$target" ]; then
            ajson="$ajson{\"qualified_name\":\"design-target:$(json_escape "$target")\"},"
        fi
        ajson="[${ajson%,}]"

        # The testimony tag — machine-readable provenance ON the claim text;
        # the engine's ASSERT event (author=$REQ_AUTHOR) is the import receipt.
        text="$rtitle: $rtext [epistemic_source=testimony tracker=$REQ_TRACKER epic=$epic ticket=$ticket deliverable=${deliv:--} via=$REQ_TOOL_VERSION]"

        put="{\"project\":\"$proj\",\"kind\":\"spec:satisfies\",\"text\":\"$(json_escape "$text")\",\"confidence\":\"$(json_escape "$conf")\",\"author\":\"$(json_escape "$REQ_AUTHOR")\",\"anchors\":$ajson}"
        if out=$(engine_tool knowledge_put "$put"); then
            cid=$(printf '%s' "$out" | json_get id)
            state=$(printf '%s' "$out" | json_get state)
            log "[RQ] authored $ticket -> claim $cid (state=$state, testimony)"
            authored=$((authored+1)); ids="$ids $cid"
            if route_review "$cid"; then reviewed=$((reviewed+1)); else pending=$((pending+1)); fi
        else
            log "[RQ] FAILED $ticket: knowledge_put rejected the claim (see engine message above)."
            failed=$((failed+1))
        fi
    done <<EOF
$body
EOF

    printf 'IMPORT: epic=%s deliverable=%s authored=%s reviewed=%s pending-review=%s skipped=%s failed=%s claims:%s\n' \
        "$epic" "${deliv:--}" "$authored" "$reviewed" "$pending" "$skipped" "$failed" "${ids:- -}"
    [ "$failed" -eq 0 ] || return 1
    return 0
}

self_test() {
    log "[RQ] self-test: capability dry-run (NO fetch, NO model, NO engine write)."
    capabilities >/dev/null
    if [ "$REQ_TRACKER" = "off" ]; then
        log "[RQ] tracker OFF (default). Bridge absent; engine byte-identical. exit 0."
        return 0
    fi
    engine_present || { log "[RQ] engine '$(engine_bin)' not found (set LODESTAR_BIN). exit 2."; return 2; }
    resolve_adapter || { log "[RQ] adapter for '$REQ_TRACKER' unavailable. exit 3."; return 3; }
    run_adapter capabilities >/dev/null 2>&1 || log "[RQ] note: adapter reported not-ready (key/tooling); fetch will degrade honestly."
    log "[RQ] ready: tracker=$REQ_TRACKER adapter=$REQ_ADAPTER project=$(engine_project)"
    log "[RQ] honesty: imports are draft TESTIMONY until the cross-family review clears them; confidence hints never gate (§6.4)."
    return 0
}

cmd="${1:-}"
rc=0
case "$cmd" in
    --self-test|self-test) self_test || rc=$? ;;
    capabilities|caps) capabilities || rc=$? ;;
    import) [ $# -ge 2 ] || die "usage: import <epic_id>"; do_import "$2" || rc=$? ;;
    ""|-h|--help) sed -n '2,50p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try --self-test, capabilities, import)" ;;
esac
exit "$rc"
