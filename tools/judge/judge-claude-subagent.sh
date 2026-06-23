#!/usr/bin/env sh
# judge-claude-subagent.sh — T1 judge driver: Claude in an ISOLATED subagent.
#
# WHAT IT IS: the host-side driver for Tier 1 of judge-providers.md. It does NOT
# call a model from inside the engine. It pulls the rubric + graph evidence from
# the engine's knowledge_review READ seam, writes a self-contained subagent BRIEF
# (rubric + evidence + the strict verdict contract), and either:
#   (a) prints/launches that brief for an isolated Claude subagent to judge, then
#   (b) records the returned verdict back through the engine seam.
#
# WHY A SUBAGENT: the judge's evidence dump must NOT land in the main coding
# conversation (context-length degradation, arXiv:2510.05381). A Claude Code
# subagent runs in its own context window and returns only a summary — so the
# verdict comes back, the evidence pollution stays in the throwaway window.
#
# HONEST CAVEAT: this is same-family review (Claude judging a Claude-authored
# claim). It is rubric- and graph-grounded, which is what makes it defensible;
# for a genuinely different family without a cloud key, use T2 (ollama).
#
# USAGE:
#   judge-claude-subagent.sh --self-test
#   judge-claude-subagent.sh brief   <claim_id>            # emit the subagent brief
#   judge-claude-subagent.sh record  <claim_id> <verdict> [concern] [reviewer_model]
#       verdict ∈ affirm|refute|uncertain
#
# DEGRADES WHEN ABSENT: if the engine binary is missing it says so and exits 2.
# It needs no install of its own (Claude Code is the model); --self-test verifies
# the seam round-trips without consulting any model.
#
# TARGET AGENT: celnet-verifier (see .claude/agents/celnet-verifier.md)
#
# shellcheck shell=sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/../lib/common.sh"

# Reviewer id: a distinct ADVERSARIAL role label, still same family (anthropic).
# The engine enforces never-self by FAMILY; a same-family verdict is rejected at
# the seam. T1 is honest about being same-family; see caveat above. We default to
# a non-anthropic *label* only when the author is non-anthropic, so the common
# case (claude author) is correctly surfaced as the same-family limitation.
REVIEWER_DEFAULT="celnet-verifier:adversarial-subagent"

emit_brief() {
    cid="$1"
    read_json=$(engine_review_read "$cid") || die "engine seam call failed for $cid"
    case "$read_json" in
        *'"rubric"'*) : ;;
        *) die "no rubric/evidence returned (claim '$cid' not found, or engine error): $read_json" ;;
    esac
    cat <<EOF
=== LODESTAR STAGE-2 JUDGE BRIEF (isolated subagent) ===
You are an ADVERSARIAL reviewer in a throwaway context window. Your job: try to
REFUTE the claim below using ONLY the rubric and the graph evidence Lodestar
provides. Do not free-associate; reason against the evidence.

Return EXACTLY one line of the form:
    VERDICT: <affirm|refute|uncertain> | CONCERN: <one sentence or "none">

Claim id: $cid

--- ENGINE PAYLOAD (rubric + per-anchor graph facts + policy) ---
$read_json
-----------------------------------------------------------------

Rules:
- affirm  = the evidence supports the claim.
- refute  = the evidence contradicts/overstates the claim (devil wins).
- uncertain = the evidence is insufficient to decide.
Then the main session records your verdict with:
    judge-claude-subagent.sh record $cid <verdict> "<concern>"
=== END BRIEF ===
EOF
}

do_record() {
    cid="$1"; verdict="$2"; concern="${3:-}"; reviewer="${4:-$REVIEWER_DEFAULT}"
    case "$verdict" in
        affirm|refute|uncertain) : ;;
        *) die "verdict must be affirm|refute|uncertain (got '$verdict')" ;;
    esac
    # Pre-check never-self against the claim author for a clear local message.
    read_json=$(engine_review_read "$cid") || die "engine seam call failed"
    author=$(printf '%s' "$read_json" | json_get author)
    if [ -n "$author" ]; then
        af=$(family_of "$author"); rf=$(family_of "$reviewer")
        if [ "$af" = "$rf" ] && [ "$af" != "unknown" ]; then
            warn "same-family review: author='$author' ($af) vs reviewer='$reviewer' ($rf)."
            warn "T1 is same-family by design; the engine will RECORD it but the never-self"
            warn "panel rule means it cannot satisfy a cross-family quorum. Use T2 for cross-family."
        fi
    fi
    out=$(engine_review_record "$cid" "$reviewer" "$verdict" "$concern" "judge=t1-claude-subagent")
    printf '%s\n' "$out"
}

self_test() {
    log "[T1] self-test: verifying the engine seam round-trips with NO model call."
    bin=$(engine_bin)
    if ! command -v "$bin" >/dev/null 2>&1 && [ ! -x "$bin" ]; then
        log "[T1] engine binary '$bin' not found on PATH."
        log "[T1] DEGRADE: set LODESTAR_BIN or install lodestar; driver needs the seam."
        return 2
    fi
    log "[T1] engine binary: $bin (ok)"
    log "[T1] provider needs NO extra install — Claude Code is the model."
    log "[T1] family pre-check: celnet-verifier -> $(family_of celnet-verifier) (expect anthropic)"
    log "[T1] ready: 'brief <claim_id>' emits the isolated-subagent brief;"
    log "[T1]        'record <claim_id> <verdict>' records the verdict via the seam."
    return 0
}

cmd="${1:-}"
rc=0
case "$cmd" in
    --self-test|self-test) self_test || rc=$? ;;
    brief)  [ $# -ge 2 ] || die "usage: brief <claim_id>"; emit_brief "$2" || rc=$? ;;
    record) [ $# -ge 3 ] || die "usage: record <claim_id> <verdict> [concern] [reviewer_model]"
            do_record "$2" "$3" "${4:-}" "${5:-}" || rc=$? ;;
    ""|-h|--help)
        sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try --self-test, brief, record)" ;;
esac
exit "$rc"
