#!/usr/bin/env sh
# adapters/plane.sh — Plane reference adapter (vendor-neutral contract, see
# ../lib.sh). HOST-SIDE ONLY; the engine never links this.
#
# Transport: Plane REST API via curl, key in the env var named by key_env.
# Config ([requirements.plane]):
#   base_url  = "https://api.plane.so"   env: LODESTAR_PLANE_BASE_URL
#   workspace = "<workspace-slug>"       env: LODESTAR_PLANE_WORKSPACE
#   project   = "<project-id>"           env: LODESTAR_PLANE_PROJECT
#   key_env   = "PLANE_API_KEY"          env: LODESTAR_PLANE_KEY_ENV
#
# Requirement mapping: the EPIC is a parent work item; its sub-issues are the
# requirements. Each sub-issue description (description_stripped) may carry the
# fenced lodestar block (anchors / target / deliverable) the bridge consumes.
#
# Status projection (state group names): done -> "completed" group,
# blocked/in-progress -> a state in the "started" group (name overridable).
#
# DEGRADES HONESTLY: no key -> exit 4; no curl -> exit 5; no jq -> exit 3.
# HONESTY: set-status done with an EMPTY receipt id is REFUSED (exit 1).
#
# shellcheck shell=sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/../../lib/common.sh"

BASE_URL=$(cfg LODESTAR_PLANE_BASE_URL requirements.plane base_url "https://api.plane.so")
WORKSPACE=$(cfg LODESTAR_PLANE_WORKSPACE requirements.plane workspace "")
PROJECT=$(cfg LODESTAR_PLANE_PROJECT requirements.plane project "")
KEY_ENV=$(cfg LODESTAR_PLANE_KEY_ENV requirements.plane key_env "PLANE_API_KEY")
TIMEOUT_S=$(cfg LODESTAR_REQ_TIMEOUT_S requirements timeout_s "30")

api_key() { eval "printf '%s' \"\${$KEY_ENV:-}\""; }
api_base() { printf '%s/api/v1/workspaces/%s/projects/%s' "$BASE_URL" "$WORKSPACE" "$PROJECT"; }

need_tools() {
    command -v curl >/dev/null 2>&1 || { log "[plane] no curl -> degrade."; exit 5; }
    [ -n "$(api_key)" ] || { log "[plane] no API key in \$$KEY_ENV -> degrade, nothing fetched/changed."; exit 4; }
    [ -n "$WORKSPACE" ] && [ -n "$PROJECT" ] || die "workspace/project not configured ([requirements.plane])"
    command -v jq >/dev/null 2>&1 || { log "[plane] jq not found (needed for Plane's JSON) -> unsupported."; exit 3; }
}

pcurl() {
    curl -sS --max-time "$TIMEOUT_S" -H "X-API-Key: $(api_key)" \
        -H "Content-Type: application/json" "$@"
}

capabilities() {
    cu=$(command -v curl >/dev/null 2>&1 && echo true || echo false)
    jr=$(command -v jq >/dev/null 2>&1 && echo true || echo false)
    ky=$([ -n "$(api_key)" ] && echo true || echo false)
    rd=false
    [ "$cu" = true ] && [ "$jr" = true ] && [ "$ky" = true ] && [ -n "$WORKSPACE" ] && [ -n "$PROJECT" ] && rd=true
    printf '{"tracker":"plane","ready":%s,"curl":%s,"jq":%s,"key":%s,"workspace":"%s"}\n' \
        "$rd" "$cu" "$jr" "$ky" "$WORKSPACE"
}

lodestar_block() {
    printf '%s\n' "$1" | sed -n '/^```lodestar$/,/^```$/p' \
        | sed -n "s/^$2:[[:space:]]*//p" | head -n1
}

fetch_epic() {
    fe_id="$1"; need_tools
    fe_out=$(pcurl "$(api_base)/issues/$fe_id/") || die "fetch of epic '$fe_id' failed"
    fe_title=$(printf '%s' "$fe_out" | jq -r '.name // ""')
    fe_desc=$(printf '%s' "$fe_out" | jq -r '.description_stripped // ""')
    fe_deliv=$(lodestar_block "$fe_desc" deliverable)
    printf '{"epic":"%s","deliverable":"%s","title":%s}\n' \
        "$fe_id" "$fe_deliv" "$(printf '%s' "$fe_title" | jq -Rs . | tr -d '\n')"
    fe_kids=$(pcurl "$(api_base)/issues/?parent=$fe_id") || die "child fetch failed"
    printf '%s' "$fe_kids" | jq -c '(.results // .)[] | {ticket:.id, title:(.name//""), text:(.description_stripped//"")}' \
        | while IFS= read -r kid; do
            k_ticket=$(printf '%s' "$kid" | jq -r '.ticket')
            k_title=$(printf '%s' "$kid" | jq -r '.title')
            k_text=$(printf '%s' "$kid" | jq -r '.text')
            k_anchors=$(lodestar_block "$k_text" anchors)
            k_target=$(lodestar_block "$k_text" target)
            k_flat=$(printf '%s\n' "$k_text" | sed '/^```lodestar$/,/^```$/d' | tr '\n' ' ' | sed 's/[[:space:]]*$//')
            printf '{"ticket":%s,"title":%s,"text":%s,"anchors":%s,"target":%s}\n' \
                "$(printf '%s' "$k_ticket" | jq -Rs . | tr -d '\n')" \
                "$(printf '%s' "$k_title" | jq -Rs . | tr -d '\n')" \
                "$(printf '%s' "$k_flat" | jq -Rs . | tr -d '\n')" \
                "$(printf '%s' "$k_anchors" | jq -Rs . | tr -d '\n')" \
                "$(printf '%s' "$k_target" | jq -Rs . | tr -d '\n')"
        done
}

get_status() {
    gs_id="$1"; need_tools
    gs_state_id=$(pcurl "$(api_base)/issues/$gs_id/" | jq -r '.state // ""')
    gs_name=""
    if [ -n "$gs_state_id" ]; then
        gs_name=$(pcurl "$(api_base)/states/" \
            | jq -r --arg id "$gs_state_id" '(.results // .)[] | select(.id==$id) | .name' | head -n1 \
            | tr '[:upper:]' '[:lower:]')
    fi
    printf '{"ticket":"%s","status":"%s"}\n' "$gs_id" "$gs_name"
}

state_group() {
    case "$1" in
        done)        printf 'completed' ;;
        blocked|in-progress) printf 'started' ;;
        *)           printf '%s' "$1" ;;
    esac
}

set_status() {
    ss_id="$1"; ss_status="$2"; ss_receipt="${3:-}"; ss_feed="${4:-}"
    if [ "$ss_status" = "done" ] && [ -z "$ss_receipt" ]; then
        log "[plane] REFUSED: status=done requires a roll-up receipt id (R5-a). Nothing changed."
        printf '{"ticket":"%s","status":"done","ok":false,"error":"receipt_required"}\n' "$ss_id"
        exit 1
    fi
    need_tools
    ss_group=$(state_group "$ss_status")
    ss_sid=$(pcurl "$(api_base)/states/" \
        | jq -r --arg g "$ss_group" '(.results // .)[] | select(.group==$g) | .id' | head -n1)
    [ -n "$ss_sid" ] || die "no workflow state in group '$ss_group' for project $PROJECT"
    pcurl -X PATCH -d "{\"state\":\"$ss_sid\"}" "$(api_base)/issues/$ss_id/" >/dev/null || die "state update failed"
    ss_note="lodestar verified roll-up receipt: $ss_receipt"
    [ -n "$ss_feed" ] && [ -f "$ss_feed" ] && ss_note="$ss_note (machine-readable feed: $(basename "$ss_feed"))"
    pcurl -X POST -d "{\"comment_html\":$(printf '%s' "$ss_note" | jq -Rs . | tr -d '\n')}" \
        "$(api_base)/issues/$ss_id/comments/" >/dev/null 2>&1 || true
    printf '{"ticket":"%s","status":"%s","ok":true,"receipt":"%s"}\n' "$ss_id" "$ss_status" "$ss_receipt"
}

cmd="${1:-}"
case "$cmd" in
    capabilities) capabilities ;;
    fetch-epic) [ $# -ge 2 ] || die "usage: fetch-epic <work_item_id>"; fetch_epic "$2" ;;
    get-status) [ $# -ge 2 ] || die "usage: get-status <work_item_id>"; get_status "$2" ;;
    set-status) [ $# -ge 3 ] || die "usage: set-status <work_item_id> <status> <receipt_id> [feed_file]"
                set_status "$2" "$3" "${4:-}" "${5:-}" ;;
    ""|-h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try capabilities, fetch-epic, get-status, set-status)" ;;
esac
