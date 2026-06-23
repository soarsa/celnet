#!/usr/bin/env sh
# tdad-rank.sh — closed-form TDAD blast-radius scoring (R3-a, arXiv:2603.17973).
#
# stdin : one line per (symbol, reaching-edge):   qn<TAB>via<TAB>hop
#         via is one of the closed edge classes below; hop is the BFS hop
#         count from the seed ("-" when the strategy is hop-less: TESTS,
#         IMPORTS).
# stdout: tier<TAB>score<TAB>qn<TAB>via<TAB>hop — deduped to ONE line per qn
#         (max score wins; ties broken by via, then hop, deterministically),
#         sorted high -> medium -> low -> unscored, score desc, qn asc.
#         Byte-stable for identical input (LC_ALL=C throughout).
#
# THE CLOSED FORM (never extended silently):
#   Direct       hop==1 over the CALLS family or a CROSS_* edge      0.95
#   Transitive   2 <= hop <= horizon (default 3), same families      0.70
#   Coverage     via TESTS                                           0.80
#   Imports      via IMPORTS                                         0.50
#
# Anything OUTSIDE the closed form (an unknown edge class, a hop beyond the
# horizon) lands in the 'unscored' bucket with score 0.00 — REPORTED, never
# guessed into a tier. Tiers: high >= 0.80, medium >= 0.60, low > 0, unscored.
#
# HONESTY: this is a RANKING SIGNAL over a deterministic edge set, NEVER a
# gate. It orders the draft stubs a human/agent curates; it cannot flip a
# claim, mint a verdict, or block anything (plan §3 R3-a).
#
# USAGE:  tdad-rank.sh [--horizon N]    (reads stdin, writes stdout)
#
# shellcheck shell=sh
set -u
LC_ALL=C
export LC_ALL

HORIZON=3
case "${1:-}" in
    --horizon)
        HORIZON="${2:?--horizon needs a value}"
        ;;
    -h|--help)
        sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    "") ;;
    *)
        printf 'error: unknown option %s (try --help)\n' "$1" >&2
        exit 1
        ;;
esac

TAB=$(printf '\t')

awk -F"$TAB" -v horizon="$HORIZON" '
BEGIN { OFS = "\t" }
function family(via) {
    return (via == "CALLS" || via == "HTTP_CALLS" || via == "ASYNC_CALLS" || \
            via == "DATA_FLOWS" || via == "CONSUMES_TOKEN" || \
            via == "CROSS_HTTP_CALLS" || via == "CROSS_ASYNC_CALLS" || \
            via == "CROSS_CHANNEL" || via == "CROSS_CONSUMES_TOKEN")
}
# The closed form. Returns -1 for anything it does not measure: that is the
# unscored bucket, never a guessed weight.
function score(via, hop,    h) {
    if (via == "TESTS")   return 0.80
    if (via == "IMPORTS") return 0.50
    if (family(via)) {
        if (hop == "-") return -1
        h = hop + 0
        if (h == 1) return 0.95
        if (h >= 2 && h <= horizon) return 0.70
        return -1   # beyond the horizon: reachable but not in the closed form
    }
    return -1       # unknown edge class: never guessed into a tier
}
NF >= 2 {
    qn = $1; via = $2; hop = (NF >= 3 && $3 != "" ? $3 : "-")
    s = score(via, hop)
    if (!(qn in best) || s > best[qn] || \
        (s == best[qn] && (via < bvia[qn] || (via == bvia[qn] && hop < bhop[qn])))) {
        best[qn] = s; bvia[qn] = via; bhop[qn] = hop
    }
}
END {
    for (qn in best) {
        s = best[qn]
        if (s < 0)          { tier = "unscored"; rank = 3; s = 0 }
        else if (s >= 0.80) { tier = "high";     rank = 0 }
        else if (s >= 0.60) { tier = "medium";   rank = 1 }
        else                { tier = "low";      rank = 2 }
        # sortable prefix: tier rank, inverted milliscore, then qn (unique key
        # => total order => byte-stable output regardless of awk array order).
        printf "%d\t%04d\t%s\t%s\t%.2f\t%s\t%s\n", \
               rank, 1000 - int(s * 1000 + 0.5), qn, tier, s, bvia[qn], bhop[qn]
    }
}
' | sort -t "$TAB" -k1,1n -k2,2n -k3,3 \
  | awk -F"$TAB" 'BEGIN { OFS = "\t" } { print $4, $5, $3, $6, $7 }'
