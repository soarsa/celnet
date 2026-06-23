#!/usr/bin/env sh
# adapters/jira.sh — JIRA Cloud reference adapter (vendor-neutral contract,
# see ../lib.sh). HOST-SIDE ONLY; the engine never links or sees this.
#
# Transport: JIRA Cloud REST v2 via curl, basic auth (email + API token).
# Config ([requirements.jira] in lodestar.judge.toml; env twin wins):
#   base_url  = "https://your-org.atlassian.net"   env: LODESTAR_JIRA_BASE_URL
#   user_env  = "JIRA_USER"                        env: LODESTAR_JIRA_USER_ENV
#   key_env   = "JIRA_API_TOKEN"                   env: LODESTAR_JIRA_KEY_ENV
#
# Requirement mapping: an EPIC's child issues are the requirements. Each child
# issue's description may carry a fenced lodestar block the adapter extracts:
#     ```lodestar
#     anchors: pricing.core.price_fx_option capture.client.submit_trade
#     target: deliverables/fx-options.acceptance.json
#     ```
# A child without an anchors line is emitted with anchors="" — the bridge
# reports it SKIPPED (a claim must anchor to the graph; never guessed).
#
# DEGRADES HONESTLY (the judge-api legend): no API token -> exit 4; no curl ->
# exit 5; no jq (needed to parse JIRA's nested JSON) -> exit 3 'unsupported'.
# Nothing is fabricated on any degrade path.
#
# HONESTY: set-status done with an EMPTY receipt id is REFUSED (exit 1) — a
# ticket's Done is a projection of the verified roll-up + receipt (R5-a).
#
# shellcheck shell=sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/../../lib/common.sh"

BASE_URL=$(cfg LODESTAR_JIRA_BASE_URL requirements.jira base_url "")
USER_ENV=$(cfg LODESTAR_JIRA_USER_ENV requirements.jira user_env "JIRA_USER")
KEY_ENV=$(cfg LODESTAR_JIRA_KEY_ENV requirements.jira key_env "JIRA_API_TOKEN")
TIMEOUT_S=$(cfg LODESTAR_REQ_TIMEOUT_S requirements timeout_s "30")

api_user() { eval "printf '%s' \"\${$USER_ENV:-}\""; }
api_key()  { eval "printf '%s' \"\${$KEY_ENV:-}\""; }

need_tools() {
    command -v curl >/dev/null 2>&1 || { log "[jira] no curl -> degrade."; exit 5; }
    [ -n "$(api_key)" ] || { log "[jira] no API token in \$$KEY_ENV -> degrade, nothing fetched/changed."; exit 4; }
    [ -n "$BASE_URL" ] || die "base_url not configured ([requirements.jira] base_url)"
    command -v jq >/dev/null 2>&1 || { log "[jira] jq not found (needed for JIRA's nested JSON) -> unsupported."; exit 3; }
}

jcurl() {
    curl -sS --max-time "$TIMEOUT_S" -u "$(api_user):$(api_key)" \
        -H "Content-Type: application/json" "$@"
}

capabilities() {
    cu=$(command -v curl >/dev/null 2>&1 && echo true || echo false)
    jr=$(command -v jq >/dev/null 2>&1 && echo true || echo false)
    ky=$([ -n "$(api_key)" ] && echo true || echo false)
    rd=false
    [ "$cu" = true ] && [ "$jr" = true ] && [ "$ky" = true ] && [ -n "$BASE_URL" ] && rd=true
    printf '{"tracker":"jira","ready":%s,"curl":%s,"jq":%s,"token":%s,"base_url":"%s"}\n' \
        "$rd" "$cu" "$jr" "$ky" "$BASE_URL"
}

# lodestar_block <description_text> <field> -> the field value from the fenced
# lodestar block ("" when absent).
lodestar_block() {
    printf '%s\n' "$1" | sed -n '/^```lodestar$/,/^```$/p' \
        | sed -n "s/^$2:[[:space:]]*//p" | head -n1
}

fetch_epic() {
    fe_id="$1"; need_tools
    fe_epic=$(jcurl "$BASE_URL/rest/api/2/issue/$fe_id?fields=summary,description") \
        || die "fetch of epic '$fe_id' failed"
    fe_title=$(printf '%s' "$fe_epic" | jq -r '.fields.summary // ""')
    fe_desc=$(printf '%s' "$fe_epic" | jq -r '.fields.description // ""')
    fe_deliv=$(lodestar_block "$fe_desc" deliverable)
    printf '{"epic":"%s","deliverable":"%s","title":%s}\n' \
        "$fe_id" "$fe_deliv" "$(printf '%s' "$fe_title" | jq -Rs .)"
    # Children: company-managed projects use "Epic Link", team-managed use parent.
    fe_jql="parent=$fe_id OR \"Epic Link\"=$fe_id"
    fe_kids=$(jcurl -G "$BASE_URL/rest/api/2/search" \
        --data-urlencode "jql=$fe_jql" \
        --data-urlencode "fields=summary,description") || die "child search failed"
    printf '%s' "$fe_kids" | jq -c '.issues[] | {ticket:.key, title:(.fields.summary//""), text:(.fields.description//"")}' \
        | while IFS= read -r kid; do
            k_ticket=$(printf '%s' "$kid" | jq -r '.ticket')
            k_title=$(printf '%s' "$kid" | jq -r '.title')
            k_text=$(printf '%s' "$kid" | jq -r '.text')
            k_anchors=$(lodestar_block "$k_text" anchors)
            k_target=$(lodestar_block "$k_text" target)
            k_body=$(printf '%s\n' "$k_text" | sed '/^```lodestar$/,/^```$/d' | tr '\n' ' ' | sed 's/[[:space:]]*$//')
            printf '{"ticket":%s,"title":%s,"text":%s,"anchors":%s,"target":%s}\n' \
                "$(printf '%s' "$k_ticket" | jq -Rs . | tr -d '\n')" \
                "$(printf '%s' "$k_title" | jq -Rs . | tr -d '\n')" \
                "$(printf '%s' "$k_body" | jq -Rs . | tr -d '\n')" \
                "$(printf '%s' "$k_anchors" | jq -Rs . | tr -d '\n')" \
                "$(printf '%s' "$k_target" | jq -Rs . | tr -d '\n')"
        done
}

get_status() {
    gs_id="$1"; need_tools
    gs_out=$(jcurl "$BASE_URL/rest/api/2/issue/$gs_id?fields=status") || die "get-status failed"
    gs_name=$(printf '%s' "$gs_out" | jq -r '.fields.status.name // ""' | tr '[:upper:]' '[:lower:]')
    printf '{"ticket":"%s","status":"%s"}\n' "$gs_id" "$gs_name"
}

# Map the bridge's canonical status to a JIRA transition NAME; workflow ids
# vary per instance, so the adapter looks the id up by name at call time.
transition_name() {
    case "$1" in
        done)        cfg LODESTAR_JIRA_DONE_TRANSITION requirements.jira done_transition "Done" ;;
        blocked)     cfg LODESTAR_JIRA_BLOCKED_TRANSITION requirements.jira blocked_transition "Blocked" ;;
        in-progress) cfg LODESTAR_JIRA_INPROGRESS_TRANSITION requirements.jira inprogress_transition "In Progress" ;;
        *)           printf '%s' "$1" ;;
    esac
}

set_status() {
    ss_id="$1"; ss_status="$2"; ss_receipt="${3:-}"; ss_feed="${4:-}"
    if [ "$ss_status" = "done" ] && [ -z "$ss_receipt" ]; then
        log "[jira] REFUSED: status=done requires a roll-up receipt id (R5-a). Nothing changed."
        printf '{"ticket":"%s","status":"done","ok":false,"error":"receipt_required"}\n' "$ss_id"
        exit 1
    fi
    need_tools
    ss_tname=$(transition_name "$ss_status")
    ss_tid=$(jcurl "$BASE_URL/rest/api/2/issue/$ss_id/transitions" \
        | jq -r --arg n "$ss_tname" '.transitions[] | select(.name==$n) | .id' | head -n1)
    [ -n "$ss_tid" ] || die "no transition named '$ss_tname' on $ss_id (configure [requirements.jira] *_transition)"
    jcurl -X POST -d "{\"transition\":{\"id\":\"$ss_tid\"}}" \
        "$BASE_URL/rest/api/2/issue/$ss_id/transitions" >/dev/null || die "transition failed"
    ss_note="lodestar verified roll-up receipt: $ss_receipt"
    [ -n "$ss_feed" ] && [ -f "$ss_feed" ] && ss_note="$ss_note (machine-readable feed: $(basename "$ss_feed"))"
    jcurl -X POST -d "{\"body\":$(printf '%s' "$ss_note" | jq -Rs . | tr -d '\n')}" \
        "$BASE_URL/rest/api/2/issue/$ss_id/comment" >/dev/null || true
    printf '{"ticket":"%s","status":"%s","ok":true,"receipt":"%s"}\n' "$ss_id" "$ss_status" "$ss_receipt"
}

cmd="${1:-}"
case "$cmd" in
    capabilities) capabilities ;;
    fetch-epic) [ $# -ge 2 ] || die "usage: fetch-epic <epic_id>"; fetch_epic "$2" ;;
    get-status) [ $# -ge 2 ] || die "usage: get-status <ticket>"; get_status "$2" ;;
    set-status) [ $# -ge 3 ] || die "usage: set-status <ticket> <status> <receipt_id> [feed_file]"
                set_status "$2" "$3" "${4:-}" "${5:-}" ;;
    ""|-h|--help) sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try capabilities, fetch-epic, get-status, set-status)" ;;
esac
