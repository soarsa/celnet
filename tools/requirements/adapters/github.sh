#!/usr/bin/env sh
# adapters/github.sh — GitHub Issues reference adapter (vendor-neutral
# contract, see ../lib.sh). HOST-SIDE ONLY; the engine never links this.
#
# Transport: the `gh` CLI (preferred — auth + JSON built in). Config
# ([requirements.github]; env twin wins):
#   repo = "owner/name"              env: LODESTAR_GITHUB_REPO
#
# Requirement mapping: the EPIC is an issue whose body lists child issues as
# task items ("- [ ] #123"). Each child issue's body may carry the fenced
# lodestar block (anchors / target / deliverable) the bridge consumes:
#     ```lodestar
#     anchors: pricing.core.price_fx_option
#     target: deliverables/fx-options.acceptance.json
#     ```
#
# Status projection: GitHub issues have open/closed, not workflow states —
#   done        -> close the issue (+ receipt comment)
#   blocked     -> label 'lodestar:blocked' + receipt comment (stays open)
#   in-progress -> label 'lodestar:in-progress' (stays open)
#
# DEGRADES HONESTLY: no `gh` -> exit 5 (install https://cli.github.com), not
# authenticated -> exit 4. Nothing fabricated on any degrade path.
# HONESTY: set-status done with an EMPTY receipt id is REFUSED (exit 1).
#
# shellcheck shell=sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/../../lib/common.sh"

REPO=$(cfg LODESTAR_GITHUB_REPO requirements.github repo "")

need_tools() {
    command -v gh >/dev/null 2>&1 || { log "[github] 'gh' CLI not found -> degrade."; exit 5; }
    gh auth status >/dev/null 2>&1 || { log "[github] gh not authenticated (gh auth login) -> degrade."; exit 4; }
    [ -n "$REPO" ] || die "repo not configured ([requirements.github] repo = \"owner/name\")"
}

capabilities() {
    gh_ok=$(command -v gh >/dev/null 2>&1 && echo true || echo false)
    au=false
    [ "$gh_ok" = true ] && gh auth status >/dev/null 2>&1 && au=true
    rd=false
    [ "$au" = true ] && [ -n "$REPO" ] && rd=true
    printf '{"tracker":"github","ready":%s,"gh":%s,"auth":%s,"repo":"%s"}\n' "$rd" "$gh_ok" "$au" "$REPO"
}

lodestar_block() {
    printf '%s\n' "$1" | sed -n '/^```lodestar$/,/^```$/p' \
        | sed -n "s/^$2:[[:space:]]*//p" | head -n1
}

fetch_epic() {
    fe_id="$1"; need_tools
    fe_num=${fe_id#\#}
    fe_title=$(gh issue view "$fe_num" -R "$REPO" --json title --jq .title) || die "epic #$fe_num not found in $REPO"
    fe_body=$(gh issue view "$fe_num" -R "$REPO" --json body --jq .body)
    fe_deliv=$(lodestar_block "$fe_body" deliverable)
    printf '{"epic":"%s","deliverable":"%s","title":"%s"}\n' \
        "$fe_num" "$fe_deliv" "$(printf '%s' "$fe_title" | sed 's/\\/\\\\/g; s/"/\\"/g')"
    # Child issues: every "- [ ] #N" / "- [x] #N" task item in the epic body.
    printf '%s\n' "$fe_body" | sed -n 's/^- \[[ xX]\] #\([0-9][0-9]*\).*/\1/p' \
        | while IFS= read -r kid; do
            [ -n "$kid" ] || continue
            k_title=$(gh issue view "$kid" -R "$REPO" --json title --jq .title) || continue
            k_text=$(gh issue view "$kid" -R "$REPO" --json body --jq .body | tr -d '\r')
            k_anchors=$(lodestar_block "$k_text" anchors)
            k_target=$(lodestar_block "$k_text" target)
            k_flat=$(printf '%s\n' "$k_text" | sed '/^```lodestar$/,/^```$/d' | tr '\n' ' ' | sed 's/[[:space:]]*$//; s/"/\\"/g')
            printf '{"ticket":"%s","title":"%s","text":"%s","anchors":"%s","target":"%s"}\n' \
                "$kid" "$(printf '%s' "$k_title" | sed 's/"/\\"/g')" "$k_flat" "$k_anchors" "$k_target"
        done
}

get_status() {
    gs_id="$1"; need_tools
    gs_num=${gs_id#\#}
    gs_state=$(gh issue view "$gs_num" -R "$REPO" --json state --jq .state | tr '[:upper:]' '[:lower:]')
    [ "$gs_state" = "closed" ] && gs_state="done"
    printf '{"ticket":"%s","status":"%s"}\n' "$gs_num" "$gs_state"
}

set_status() {
    ss_id="$1"; ss_status="$2"; ss_receipt="${3:-}"; ss_feed="${4:-}"
    ss_num=${ss_id#\#}
    if [ "$ss_status" = "done" ] && [ -z "$ss_receipt" ]; then
        log "[github] REFUSED: status=done requires a roll-up receipt id (R5-a). Nothing changed."
        printf '{"ticket":"%s","status":"done","ok":false,"error":"receipt_required"}\n' "$ss_num"
        exit 1
    fi
    need_tools
    ss_note="lodestar verified roll-up receipt: $ss_receipt"
    [ -n "$ss_feed" ] && [ -f "$ss_feed" ] && ss_note="$ss_note (machine-readable feed: $(basename "$ss_feed"))"
    case "$ss_status" in
        done)
            gh issue comment "$ss_num" -R "$REPO" -b "$ss_note" >/dev/null || true
            gh issue close "$ss_num" -R "$REPO" >/dev/null || die "close failed" ;;
        blocked)
            gh issue edit "$ss_num" -R "$REPO" --add-label "lodestar:blocked" >/dev/null 2>&1 || true
            gh issue comment "$ss_num" -R "$REPO" -b "$ss_note" >/dev/null || true ;;
        *)
            gh issue edit "$ss_num" -R "$REPO" --add-label "lodestar:in-progress" >/dev/null 2>&1 || true ;;
    esac
    printf '{"ticket":"%s","status":"%s","ok":true,"receipt":"%s"}\n' "$ss_num" "$ss_status" "$ss_receipt"
}

cmd="${1:-}"
case "$cmd" in
    capabilities) capabilities ;;
    fetch-epic) [ $# -ge 2 ] || die "usage: fetch-epic <epic_issue_number>"; fetch_epic "$2" ;;
    get-status) [ $# -ge 2 ] || die "usage: get-status <issue_number>"; get_status "$2" ;;
    set-status) [ $# -ge 3 ] || die "usage: set-status <issue_number> <status> <receipt_id> [feed_file]"
                set_status "$2" "$3" "${4:-}" "${5:-}" ;;
    ""|-h|--help) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try capabilities, fetch-epic, get-status, set-status)" ;;
esac
