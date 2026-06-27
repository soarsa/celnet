#!/usr/bin/env sh
# adapters/linear.sh — Linear reference adapter (vendor-neutral contract, see
# ../lib.sh). HOST-SIDE ONLY; the engine never links this.
#
# Transport: Linear GraphQL API (https://api.linear.app/graphql) via curl,
# key in the env var named by key_env. Config ([requirements.linear]):
#   key_env = "LINEAR_API_KEY"      env: LODESTAR_LINEAR_KEY_ENV
#
# Requirement mapping: the EPIC is a parent issue (Linear sub-issues are the
# requirements). Each sub-issue description may carry the fenced lodestar
# block (anchors / target / deliverable) the bridge consumes.
#
# Status projection (workflow state names, overridable):
#   done -> "Done" · blocked -> "Blocked" · in-progress -> "In Progress"
#
# DEGRADES HONESTLY: no key -> exit 4; no curl -> exit 5; no jq -> exit 3.
# HONESTY: set-status done with an EMPTY receipt id is REFUSED (exit 1).
#
# shellcheck shell=sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/../../lib/common.sh"

KEY_ENV=$(cfg LODESTAR_LINEAR_KEY_ENV requirements.linear key_env "LINEAR_API_KEY")
TIMEOUT_S=$(cfg LODESTAR_REQ_TIMEOUT_S requirements timeout_s "30")
GQL_URL="https://api.linear.app/graphql"

api_key() { eval "printf '%s' \"\${$KEY_ENV:-}\""; }

need_tools() {
    command -v curl >/dev/null 2>&1 || { log "[linear] no curl -> degrade."; exit 5; }
    [ -n "$(api_key)" ] || { log "[linear] no API key in \$$KEY_ENV -> degrade, nothing fetched/changed."; exit 4; }
    command -v jq >/dev/null 2>&1 || { log "[linear] jq not found (needed for GraphQL JSON) -> unsupported."; exit 3; }
}

gql() {
    # $1 = GraphQL document (variables inlined by the caller).
    curl -sS --max-time "$TIMEOUT_S" -X POST "$GQL_URL" \
        -H "Authorization: $(api_key)" -H "Content-Type: application/json" \
        -d "{\"query\":$(printf '%s' "$1" | jq -Rs . | tr -d '\n')}"
}

capabilities() {
    cu=$(command -v curl >/dev/null 2>&1 && echo true || echo false)
    jr=$(command -v jq >/dev/null 2>&1 && echo true || echo false)
    ky=$([ -n "$(api_key)" ] && echo true || echo false)
    rd=false
    [ "$cu" = true ] && [ "$jr" = true ] && [ "$ky" = true ] && rd=true
    printf '{"tracker":"linear","ready":%s,"curl":%s,"jq":%s,"key":%s}\n' "$rd" "$cu" "$jr" "$ky"
}

lodestar_block() {
    printf '%s\n' "$1" | sed -n '/^```lodestar$/,/^```$/p' \
        | sed -n "s/^$2:[[:space:]]*//p" | head -n1
}

fetch_epic() {
    fe_id="$1"; need_tools
    fe_out=$(gql "query { issue(id: \"$fe_id\") { identifier title description children { nodes { identifier title description } } } }") \
        || die "fetch of epic '$fe_id' failed"
    fe_err=$(printf '%s' "$fe_out" | jq -r '.errors[0].message // ""')
    [ -z "$fe_err" ] || die "linear: $fe_err"
    fe_title=$(printf '%s' "$fe_out" | jq -r '.data.issue.title // ""')
    fe_desc=$(printf '%s' "$fe_out" | jq -r '.data.issue.description // ""')
    fe_deliv=$(lodestar_block "$fe_desc" deliverable)
    printf '{"epic":"%s","deliverable":"%s","title":%s}\n' \
        "$fe_id" "$fe_deliv" "$(printf '%s' "$fe_title" | jq -Rs . | tr -d '\n')"
    printf '%s' "$fe_out" | jq -c '.data.issue.children.nodes[]' | while IFS= read -r kid; do
        k_ticket=$(printf '%s' "$kid" | jq -r '.identifier')
        k_title=$(printf '%s' "$kid" | jq -r '.title // ""')
        k_text=$(printf '%s' "$kid" | jq -r '.description // ""')
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
    gs_out=$(gql "query { issue(id: \"$gs_id\") { state { name } } }") || die "get-status failed"
    gs_name=$(printf '%s' "$gs_out" | jq -r '.data.issue.state.name // ""' | tr '[:upper:]' '[:lower:]')
    printf '{"ticket":"%s","status":"%s"}\n' "$gs_id" "$gs_name"
}

state_name() {
    case "$1" in
        done)        cfg LODESTAR_LINEAR_DONE_STATE requirements.linear done_state "Done" ;;
        blocked)     cfg LODESTAR_LINEAR_BLOCKED_STATE requirements.linear blocked_state "Blocked" ;;
        in-progress) cfg LODESTAR_LINEAR_INPROGRESS_STATE requirements.linear inprogress_state "In Progress" ;;
        *)           printf '%s' "$1" ;;
    esac
}

set_status() {
    ss_id="$1"; ss_status="$2"; ss_receipt="${3:-}"; ss_feed="${4:-}"
    if [ "$ss_status" = "done" ] && [ -z "$ss_receipt" ]; then
        log "[linear] REFUSED: status=done requires a roll-up receipt id (R5-a). Nothing changed."
        printf '{"ticket":"%s","status":"done","ok":false,"error":"receipt_required"}\n' "$ss_id"
        exit 1
    fi
    need_tools
    ss_name=$(state_name "$ss_status")
    # Resolve the workflow state id by name within the issue's team.
    ss_sid=$(gql "query { issue(id: \"$ss_id\") { team { states { nodes { id name } } } } }" \
        | jq -r --arg n "$ss_name" '.data.issue.team.states.nodes[] | select(.name==$n) | .id' | head -n1)
    [ -n "$ss_sid" ] || die "no workflow state named '$ss_name' on $ss_id's team (configure [requirements.linear] *_state)"
    ss_ok=$(gql "mutation { issueUpdate(id: \"$ss_id\", input: { stateId: \"$ss_sid\" }) { success } }" \
        | jq -r '.data.issueUpdate.success // false')
    [ "$ss_ok" = "true" ] || die "issueUpdate failed for $ss_id"
    ss_note="lodestar verified roll-up receipt: $ss_receipt"
    [ -n "$ss_feed" ] && [ -f "$ss_feed" ] && ss_note="$ss_note (machine-readable feed: $(basename "$ss_feed"))"
    gql "mutation { commentCreate(input: { issueId: \"$ss_id\", body: $(printf '%s' "$ss_note" | jq -Rs . | tr -d '\n') }) { success } }" >/dev/null || true
    printf '{"ticket":"%s","status":"%s","ok":true,"receipt":"%s"}\n' "$ss_id" "$ss_status" "$ss_receipt"
}

cmd="${1:-}"
case "$cmd" in
    capabilities) capabilities ;;
    fetch-epic) [ $# -ge 2 ] || die "usage: fetch-epic <epic_issue_id>"; fetch_epic "$2" ;;
    get-status) [ $# -ge 2 ] || die "usage: get-status <issue_id>"; get_status "$2" ;;
    set-status) [ $# -ge 3 ] || die "usage: set-status <issue_id> <status> <receipt_id> [feed_file]"
                set_status "$2" "$3" "${4:-}" "${5:-}" ;;
    ""|-h|--help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try capabilities, fetch-epic, get-status, set-status)" ;;
esac
