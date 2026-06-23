#!/usr/bin/env sh
# planner.sh — R3 graph-derived decomposition + ranked blast radius (HOST-SIDE).
#
# WHAT IT IS: the host-side planning helper of the v0.5.0 Verified Requirements
# layer (docs/design/v0.5.0-requirements-plan.md §3 R3). From a SEED
# qualified_name it orchestrates the SHIPPED engine tools — zero engine edits,
# zero new engine surface — through the same `lodestar cli <tool> <json>` seam
# the judge drivers use:
#
#   trace_path   mode=cross_service with EXPLICIT edge_types (the CALLS family
#                plus the pass_cross_repo CROSS_* contract edges): reachability
#                from the seed, with hop counts.
#   query_graph  the typed CROSS_HTTP_CALLS / CROSS_ASYNC_CALLS / CROSS_CHANNEL
#                / CROSS_CONSUMES_TOKEN edge lists (exact qualified_names; also
#                the safety net when the BFS view lags), plus the TESTS and
#                IMPORTS overlays of the TDAD closed form.
#   detect_changes (--changes) a working-tree change overlay: DISPLAY ONLY,
#                never folded into the ranking (its symbols carry no
#                qualified_name, so scoring them would be a guess).
#   knowledge_put (--emit-stubs) DRAFT `spec:satisfies` stubs anchored to each
#                impacted qualified_name.
#
# THE HONESTY CONTRACT (R3-a / R3-b — the breaker attacks these):
#   - The GRAPH decides WHAT: the candidate set is graph reachability only.
#     `--services` can only NARROW the graph-fixed candidate set; asking for a
#     service the graph does not reach is REFUSED (exit 4), never granted.
#   - The TDAD score (Direct 0.95 / Transitive 2..3-hop 0.70 / TESTS 0.80 /
#     IMPORTS 0.50) is a RANKING SIGNAL over a deterministic edge set. It
#     never gates anything; it orders the stubs a human/agent curates.
#   - Stubs are DRAFT-ONLY by construction: a spec:satisfies claim with no
#     committed acceptance target defers at the Stage-1 gate and stays draft.
#     If a stub ever comes back non-draft, the planner ABORTS (exit 4) — it
#     never proceeds past an auto-activation.
#   - Anything outside the closed scoring form (unknown edge class, beyond
#     the hop horizon) is emitted under "unscored" with a reason — reported,
#     never guessed into a tier.
#   - The plan output carries no timestamps and is fully sorted: identical
#     graph state => byte-identical plan (the determinism gate).
#
# USAGE:
#   planner.sh --self-test
#   planner.sh plan <seed_qualified_name> [options]
#       --deliverable <slug>   deliverable slug used in the stub texts
#       --depth N              trace depth (default 3)
#       --horizon N            TDAD transitive horizon (default 3)
#       --services a,b         NARROW stub proposals to these reachable services
#       --changes [<base>]     add the detect_changes overlay (display only)
#       --emit-stubs           author the draft stubs via knowledge_put
#   planner.sh rank [--horizon N]      stdin/stdout passthrough to tdad-rank.sh
#
# EXIT CODES: 0 ok · 1 usage · 2 engine binary missing · 3 engine/tool failure
# (incl. seed not found) · 4 refused (honesty violation / non-narrowing
# --services).
#
# shellcheck shell=sh
set -u
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/../lib/common.sh"

# Planner config file overlay: env > lodestar.planner.toml > judge-toml default.
if [ -n "${LODESTAR_PLANNER_CONFIG:-}" ] && [ -f "${LODESTAR_PLANNER_CONFIG}" ]; then
    _LODESTAR_CFG="${LODESTAR_PLANNER_CONFIG}"
elif [ -f "./lodestar.planner.toml" ]; then
    _LODESTAR_CFG="./lodestar.planner.toml"
elif [ -f "$HERE/lodestar.planner.toml" ]; then
    _LODESTAR_CFG="$HERE/lodestar.planner.toml"
fi

TOOL_VERSION="lodestar-planner@0.5.0"
BIN=$(cfg LODESTAR_BIN engine bin "lodestar")
CFG_PROJECT=$(cfg LODESTAR_PROJECT engine project "")
DEPTH=$(cfg LODESTAR_PLANNER_DEPTH planner depth "3")
HORIZON=$(cfg LODESTAR_PLANNER_HORIZON planner horizon "3")
SERVICE_SEGS=$(cfg LODESTAR_PLANNER_SERVICE_SEGS planner service_segs "1")
MAX_ANCHORS=$(cfg LODESTAR_PLANNER_MAX_ANCHORS planner max_anchors "5")
AUTHOR=$(cfg LODESTAR_PLANNER_AUTHOR planner author "lodestar-planner@0.5.0")
PUT_PROJECT=$(cfg LODESTAR_PLANNER_PUT_PROJECT planner put_project "service")
CONFIDENCE=$(cfg LODESTAR_PLANNER_CONFIDENCE planner confidence "low")
RANK="$HERE/tdad-rank.sh"

TAB=$(printf '\t')
LC_ALL=C
export LC_ALL

TMP="${TMPDIR:-/tmp}/lode-planner.$$"
mkdir -p "$TMP"
trap 'rm -rf "$TMP"' EXIT INT TERM

refuse() { log "[PLAN] REFUSED: $*"; exit 4; }

# ---- engine seam ------------------------------------------------------------
# `lodestar cli <tool> <json>` prints the tool's inner JSON on stdout and the
# error text on stderr (exit 1) — the planner NEVER fabricates a result for a
# failed tool call; it surfaces the error and stops (exit 3).
engine_tool() {
    _t="$1"; _j="$2"
    if ! "$BIN" cli "$_t" "$_j" 2>"$TMP/engine.err"; then
        log "[PLAN] engine tool '$_t' failed:"
        sed 's/^/[PLAN]   /' "$TMP/engine.err" >&2 || true
        return 3
    fi
}

# ---- tiny JSON readers (the common.sh json_get discipline: the flat, compact
# single-line shapes the engine emits; values carry no escaped quotes) --------

# json_array_body <key>  — stdin: compact JSON; stdout: the bracket-balanced
# body of "key":[ ... ] (empty when the key is absent or the array is empty).
json_array_body() {
    awk -v key="$1" '
    {
        pat = "\"" key "\":["
        i = index($0, pat)
        if (i == 0) next
        rest = substr($0, i + length(pat))
        d = 1; n = length(rest); out = ""; instr = 0
        for (j = 1; j <= n; j++) {
            c = substr(rest, j, 1)
            if (c == "\"") instr = !instr
            if (!instr) {
                if (c == "[") d++
                else if (c == "]") { d--; if (d == 0) break }
            }
            out = out c
        }
        print out
        exit
    }'
}

# trace_visited <callees|callers> <trace.json>  -> qn \t hop \t bare \t dir
trace_visited() {
    json_array_body "$1" < "$2" | awk -v dir="$1" '
    BEGIN { OFS = "\t" }
    !/^[[:space:]]*$/ {
        gsub(/\},[[:space:]]*\{/, "}\n{")
        n = split($0, items, "\n")
        for (i = 1; i <= n; i++) {
            o = items[i]; qn = ""; nm = ""; hop = ""
            if (match(o, /"qualified_name":"[^"]*"/)) qn = substr(o, RSTART + 18, RLENGTH - 19)
            if (match(o, /"name":"[^"]*"/))           nm = substr(o, RSTART + 8, RLENGTH - 9)
            if (match(o, /"hop":[0-9]+/))             hop = substr(o, RSTART + 6, RLENGTH - 6)
            if (qn != "" && hop != "") print qn, hop, nm, dir
        }
    }'
}

# trace_edges <trace.json>  -> from \t to \t type   (bare names, engine shape)
trace_edges() {
    json_array_body "edges" < "$1" | awk '
    BEGIN { OFS = "\t" }
    !/^[[:space:]]*$/ {
        gsub(/\},[[:space:]]*\{/, "}\n{")
        n = split($0, items, "\n")
        for (i = 1; i <= n; i++) {
            o = items[i]; f = ""; t = ""; ty = ""
            if (match(o, /"from":"[^"]*"/)) f  = substr(o, RSTART + 8, RLENGTH - 9)
            if (match(o, /"to":"[^"]*"/))   t  = substr(o, RSTART + 6, RLENGTH - 7)
            if (match(o, /"type":"[^"]*"/)) ty = substr(o, RSTART + 8, RLENGTH - 9)
            if (f != "" && t != "" && ty != "") print f, t, ty
        }
    }'
}

# qg_rows — stdin: a query_graph result; stdout: one row per line, tab-joined.
qg_rows() {
    json_array_body "rows" | awk '
    BEGIN { OFS = "\t" }
    !/^[[:space:]]*$/ {
        gsub(/\],[[:space:]]*\[/, "]\n[")
        n = split($0, rows, "\n")
        for (i = 1; i <= n; i++) {
            r = rows[i]
            sub(/^\[/, "", r); sub(/\]$/, "", r)
            m = split(r, parts, /","/)
            line = ""
            for (j = 1; j <= m; j++) {
                v = parts[j]
                sub(/^"/, "", v); sub(/"$/, "", v)
                line = line (j > 1 ? OFS : "") v
            }
            if (line != "") print line
        }
    }'
}

# str_list <file>  -> "a","b","c"   (JSON string list from lines)
str_list() {
    awk 'BEGIN { first = 1 } { printf "%s\"%s\"", (first ? "" : ","), $0; first = 0 }' "$1"
}

# service_of_qn <qn>  -> first SERVICE_SEGS dot-segments
service_of_qn() {
    printf '%s' "$1" | awk -F. -v segs="$SERVICE_SEGS" \
        '{ s = $1; for (i = 2; i <= segs && i <= NF; i++) s = s "." $i; print s }'
}

# ---- self-test (dry-run: no engine call, no model, no network) --------------
self_test() {
    log "[PLAN] self-test: dry-run (no engine call, no model, no network)."
    for t in awk sed sort tr; do
        command -v "$t" >/dev/null 2>&1 || { log "[PLAN] missing POSIX tool '$t'"; return 3; }
    done
    [ -f "$RANK" ] || { log "[PLAN] tdad-rank.sh missing next to planner.sh"; return 3; }
    out=$(printf 'a.b.c\tCALLS\t1\n' | sh "$RANK") || return 3
    case "$out" in
        high*) log "[PLAN] tdad-rank closed form sane (Direct hop-1 -> high 0.95)." ;;
        *)     log "[PLAN] tdad-rank sanity failed: $out"; return 3 ;;
    esac
    if command -v "$BIN" >/dev/null 2>&1; then
        log "[PLAN] engine '$BIN' found; ready."
        return 0
    fi
    log "[PLAN] engine binary '$BIN' not found (set LODESTAR_BIN); planning needs it."
    return 2
}

# ---- the draft-stub put (honesty-guarded) -----------------------------------
# put_stub <service> <anchors-file>  -> stdout: service \t claim_id \t state
put_stub() {
    _svc="$1"; _anchf="$2"
    _pj="$PUT_PROJECT"
    [ "$_pj" = "service" ] && _pj="$_svc"
    _aj=$(awk 'BEGIN { first = 1 } { printf "%s{\"qualified_name\":\"%s\"}", (first ? "" : ","), $0; first = 0 }' "$_anchf")
    if [ -z "$_aj" ]; then
        log "[PLAN] service '$_svc': no scored anchors; no stub authored."
        return 0
    fi
    _txt="[draft][planner] ${DELIV:-deliverable}/$_svc: spec:satisfies stub proposed from seed $SEED by graph reachability (TDAD-ranked). Commit an acceptance target (*.acceptance.json) and anchor it via design-target:<ref>, then re-verify. This stub never auto-activates."
    _pl="{\"project\":\"$(json_escape "$_pj")\",\"kind\":\"spec:satisfies\",\"text\":\"$(json_escape "$_txt")\",\"author\":\"$(json_escape "$AUTHOR")\",\"confidence\":\"$(json_escape "$CONFIDENCE")\",\"anchors\":[$_aj]}"
    _resp=$(engine_tool knowledge_put "$_pl") || { log "[PLAN] knowledge_put failed for '$_svc'"; exit 3; }
    _cid=$(printf '%s' "$_resp" | json_get id)
    _st=$(printf '%s' "$_resp" | json_get state)
    [ -n "$_cid" ] || refuse "knowledge_put returned no claim id for service '$_svc'"
    if [ "$_st" != "draft" ]; then
        refuse "HONESTY VIOLATION: stub for '$_svc' came back state='$_st' — planner stubs must stay draft (nothing auto-activates; plan §3 R3-b)"
    fi
    log "[PLAN] service '$_svc': draft stub $_cid authored ($(wc -l < "$_anchf" | tr -d '[:space:]') anchors)."
    printf '%s\t%s\t%s\n' "$_svc" "$_cid" "$_st"
}

# ---- plan -------------------------------------------------------------------
do_plan() {
    SEED=""; DELIV=""; EMIT=0; WANT_CHANGES=0; CHANGES_BASE=""; ONLY_SERVICES=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --deliverable) shift; DELIV="${1:?--deliverable needs a value}" ;;
            --depth)       shift; DEPTH="${1:?--depth needs a value}" ;;
            --horizon)     shift; HORIZON="${1:?--horizon needs a value}" ;;
            --services)    shift; ONLY_SERVICES="${1:?--services needs a value}" ;;
            --changes)
                WANT_CHANGES=1
                case "${2:-}" in ""|-*) ;; *) shift; CHANGES_BASE="$1" ;; esac ;;
            --emit-stubs)  EMIT=1 ;;
            -*) die "unknown option '$1' (try --help)" ;;
            *)
                if [ -z "$SEED" ]; then SEED="$1"; else die "unexpected argument '$1'"; fi ;;
        esac
        shift
    done
    [ -n "$SEED" ] || die "usage: planner.sh plan <seed_qualified_name> [options]"
    command -v "$BIN" >/dev/null 2>&1 || {
        log "[PLAN] engine binary '$BIN' not found (set LODESTAR_BIN)."
        exit 2
    }

    PROJECT="$CFG_PROJECT"
    [ -n "$PROJECT" ] || PROJECT=${SEED%%.*}
    seed_service=$(service_of_qn "$SEED")

    # 1) Reachability: ONE trace from the seed over the closed edge family —
    # the cross_service mode defaults PLUS the pass_cross_repo CROSS_* edges,
    # passed explicitly (explicit edge_types > mode defaults in the engine).
    et='"CALLS","HTTP_CALLS","ASYNC_CALLS","DATA_FLOWS","CONSUMES_TOKEN","CROSS_HTTP_CALLS","CROSS_ASYNC_CALLS","CROSS_CHANNEL","CROSS_CONSUMES_TOKEN"'
    targs="{\"project\":\"$(json_escape "$PROJECT")\",\"function_name\":\"$(json_escape "$SEED")\",\"mode\":\"cross_service\",\"direction\":\"both\",\"depth\":$DEPTH,\"include_tests\":true,\"edge_types\":[$et]}"
    engine_tool trace_path "$targs" > "$TMP/trace.json" || exit 3

    { trace_visited callees "$TMP/trace.json"; trace_visited callers "$TMP/trace.json"; } > "$TMP/visited.raw"
    trace_edges "$TMP/trace.json" > "$TMP/tedges.tsv"

    # 2) Typed CROSS_* contract edges with exact qualified_names (sparse —
    # whole-graph per type, filtered to incidence below). Also the TESTS and
    # IMPORTS overlays of the closed form.
    : > "$TMP/cross.tsv"
    for ct in CROSS_HTTP_CALLS CROSS_ASYNC_CALLS CROSS_CHANNEL CROSS_CONSUMES_TOKEN; do
        q="MATCH (a)-[r:$ct]->(b) RETURN a.qualified_name, b.qualified_name"
        engine_tool query_graph "{\"project\":\"$(json_escape "$PROJECT")\",\"query\":\"$q\"}" > "$TMP/qg.json" || exit 3
        qg_rows < "$TMP/qg.json" | awk -F"$TAB" -v ty="$ct" 'BEGIN { OFS = "\t" } NF >= 2 { print $1, ty, $2 }' >> "$TMP/cross.tsv"
    done
    q="MATCH (a)-[r:TESTS]->(b) RETURN a.qualified_name, b.qualified_name"
    engine_tool query_graph "{\"project\":\"$(json_escape "$PROJECT")\",\"query\":\"$q\"}" > "$TMP/qg.json" || exit 3
    qg_rows < "$TMP/qg.json" > "$TMP/tests.tsv"
    q="MATCH (a)-[r:IMPORTS]->(b) RETURN a.qualified_name, b.qualified_name"
    engine_tool query_graph "{\"project\":\"$(json_escape "$PROJECT")\",\"query\":\"$q\"}" > "$TMP/qg.json" || exit 3
    qg_rows < "$TMP/qg.json" > "$TMP/imports.tsv"

    # 3) Annotate each visited symbol with its arrival edge type (joined on
    # the trace's bare-name edge list; ambiguous joins fall back to the BFS
    # family label CALLS — a DISPLAY refinement, never a new edge), dedup to
    # the minimum hop per qualified_name.
    awk -F"$TAB" '
    BEGIN { OFS = "\t" }
    FILENAME == ARGV[1] {
        if (to_t[$2] == "") to_t[$2] = $3; else if (to_t[$2] != $3) to_t[$2] = "MIXED"
        if (fr_t[$1] == "") fr_t[$1] = $3; else if (fr_t[$1] != $3) fr_t[$1] = "MIXED"
        next
    }
    {
        via = "CALLS"
        t = ($4 == "callees") ? to_t[$3] : fr_t[$3]
        if (t != "" && t != "MIXED") via = t
        print $1, $2, via
    }' "$TMP/tedges.tsv" "$TMP/visited.raw" \
        | sort -t "$TAB" -k1,1 -k2,2n -k3,3 | awk -F"$TAB" '!seen[$1]++' > "$TMP/visited.tsv"

    # 4) Cross-edge merge (the safety net): a CROSS_* row whose near side is
    # reachable pulls its far side in at hop+1 — bounded fixed-point, depth
    # capped by the horizon. Then emit the rank input (qn \t via \t hop).
    awk -F"$TAB" -v seed="$SEED" -v horizon="$HORIZON" '
    BEGIN { OFS = "\t" }
    FILENAME == ARGV[1] {
        if (!($1 in hop)) { hop[$1] = $2 + 0; via[$1] = $3; ord[++n] = $1 }
        next
    }
    { cf[++m] = $1; cty[m] = $2; ct2[m] = $3 }
    END {
        if (!(seed in hop)) { hop[seed] = 0; via[seed] = "SEED" }
        for (pass = 1; pass <= horizon; pass++)
            for (i = 1; i <= m; i++)
                if ((cf[i] in hop) && !(ct2[i] in hop)) {
                    hop[ct2[i]] = hop[cf[i]] + 1; via[ct2[i]] = cty[i]; ord[++n] = ct2[i]
                }
        for (i = 1; i <= n; i++)
            if (ord[i] != seed) print ord[i], via[ord[i]], hop[ord[i]]
    }' "$TMP/visited.tsv" "$TMP/cross.tsv" > "$TMP/rank-in.tsv"

    # 5) Coverage + imports overlays — applied to the SCORED set only (hop
    # within the horizon); overlays never extend the call frontier.
    awk -F"$TAB" -v seed="$SEED" -v horizon="$HORIZON" '
    BEGIN { OFS = "\t" }
    FILENAME == ARGV[1] { if ($3 != "-" && $3 + 0 <= horizon) set[$1] = 1; next }
    ($2 == seed) || ($2 in set) { print $1, "TESTS", "-" }
    ' "$TMP/rank-in.tsv" "$TMP/tests.tsv" > "$TMP/tests-overlay.tsv"
    awk -F"$TAB" -v seed="$SEED" -v horizon="$HORIZON" '
    BEGIN { OFS = "\t" }
    FILENAME == ARGV[1] { if ($3 != "-" && $3 + 0 <= horizon) set[$1] = 1; next }
    ($2 == seed) || ($2 in set) { print $1, "IMPORTS", "-" }
    ' "$TMP/rank-in.tsv" "$TMP/imports.tsv" > "$TMP/imports-overlay.tsv"

    cat "$TMP/rank-in.tsv" "$TMP/tests-overlay.tsv" "$TMP/imports-overlay.tsv" > "$TMP/rank-all.tsv"

    # 6) TDAD closed-form ranking (a signal, never a gate).
    sh "$RANK" --horizon "$HORIZON" < "$TMP/rank-all.tsv" > "$TMP/ranked.tsv" || exit 3

    # 7) Services = the graph-fixed candidate set (seed + scored symbols).
    {
        printf '%s\n' "$SEED"
        awk -F"$TAB" '$1 != "unscored" { print $3 }' "$TMP/ranked.tsv"
    } | awk -F. -v segs="$SERVICE_SEGS" \
        '{ s = $1; for (i = 2; i <= segs && i <= NF; i++) s = s "." $i; print s }' \
      | sort -u > "$TMP/services.txt"

    # 8) --services may only NARROW the candidate set (R3-b: any IF-judgement
    # is host-side OVER the graph-fixed set; it never adds to it).
    if [ -n "$ONLY_SERVICES" ]; then
        printf '%s\n' "$ONLY_SERVICES" | tr ',' '\n' | sed '/^$/d' | sort -u > "$TMP/only.txt"
        while IFS= read -r s; do
            grep -Fxq "$s" "$TMP/services.txt" || \
                refuse "service '$s' is not in the graph-fixed candidate set [$(tr '\n' ' ' < "$TMP/services.txt")] — --services can only narrow, never add"
        done < "$TMP/only.txt"
        cp "$TMP/only.txt" "$TMP/stub-services.txt"
    else
        cp "$TMP/services.txt" "$TMP/stub-services.txt"
    fi

    # 9) Per-service anchor selection: ranked order (tier, score desc, qn asc),
    # seed first for its own service, capped at MAX_ANCHORS. Deterministic.
    while IFS= read -r svc; do
        {
            [ "$svc" = "$seed_service" ] && printf '%s\n' "$SEED"
            awk -F"$TAB" -v svc="$svc" -v segs="$SERVICE_SEGS" '
            $1 == "unscored" { next }
            {
                n = split($3, p, "."); s = p[1]
                for (i = 2; i <= segs && i <= n; i++) s = s "." p[i]
                if (s == svc) print $3
            }' "$TMP/ranked.tsv"
        } | awk '!seen[$0]++' | head -n "$MAX_ANCHORS" > "$TMP/anchors.$svc"
    done < "$TMP/stub-services.txt"

    # 10) Optional detect_changes overlay (display only — NEVER ranked).
    if [ "$WANT_CHANGES" -eq 1 ]; then
        dargs="{\"project\":\"$(json_escape "$PROJECT")\"${CHANGES_BASE:+,\"base_branch\":\"$(json_escape "$CHANGES_BASE")\"}}"
        engine_tool detect_changes "$dargs" > "$TMP/changes.json" || exit 3
    fi

    # 11) Author the draft stubs (only with --emit-stubs).
    : > "$TMP/stubs.tsv"
    if [ "$EMIT" -eq 1 ]; then
        while IFS= read -r svc; do
            put_stub "$svc" "$TMP/anchors.$svc" >> "$TMP/stubs.tsv"
        done < "$TMP/stub-services.txt"
    fi

    # 12) Contract edges: CROSS_* rows with BOTH endpoints in the reachable set.
    awk -F"$TAB" -v seed="$SEED" '
    BEGIN { OFS = "\t" }
    FILENAME == ARGV[1] { set[$1] = 1; next }
    (($1 in set) || ($1 == seed)) && (($3 in set) || ($3 == seed)) { print }
    ' "$TMP/rank-in.tsv" "$TMP/cross.tsv" | sort -u > "$TMP/contract.tsv"

    # 13) Emit the plan (single line, fully sorted, no timestamps).
    sv_json=$(str_list "$TMP/services.txt")
    impacted_json=$(awk -F"$TAB" -v segs="$SERVICE_SEGS" '
    BEGIN { first = 1 }
    $1 != "unscored" {
        n = split($3, p, "."); s = p[1]
        for (i = 2; i <= segs && i <= n; i++) s = s "." p[i]
        printf "%s{\"qualified_name\":\"%s\",\"service\":\"%s\",\"tier\":\"%s\",\"score\":%s,\"via\":\"%s\",\"hop\":%s}", \
            (first ? "" : ","), $3, s, $1, $2, $4, ($5 == "-" ? "null" : $5)
        first = 0
    }' "$TMP/ranked.tsv")
    unscored_json=$(awk -F"$TAB" -v horizon="$HORIZON" '
    BEGIN { first = 1 }
    $1 == "unscored" {
        reason = "unknown-edge-class"
        if (($4 ~ /^(CALLS|HTTP_CALLS|ASYNC_CALLS|DATA_FLOWS|CONSUMES_TOKEN|CROSS_HTTP_CALLS|CROSS_ASYNC_CALLS|CROSS_CHANNEL|CROSS_CONSUMES_TOKEN)$/) && $5 != "-" && ($5 + 0) > horizon)
            reason = "out-of-horizon"
        printf "%s{\"qualified_name\":\"%s\",\"via\":\"%s\",\"hop\":%s,\"reason\":\"%s\"}", \
            (first ? "" : ","), $3, $4, ($5 == "-" ? "null" : $5), reason
        first = 0
    }' "$TMP/ranked.tsv")
    ce_json=$(awk -F"$TAB" 'BEGIN { first = 1 }
        { printf "%s{\"from\":\"%s\",\"type\":\"%s\",\"to\":\"%s\"}", (first ? "" : ","), $1, $2, $3; first = 0 }' \
        "$TMP/contract.tsv")
    props=""
    while IFS= read -r svc; do
        aj=$(str_list "$TMP/anchors.$svc")
        props="$props${props:+,}{\"service\":\"$svc\",\"anchors\":[$aj]}"
    done < "$TMP/stub-services.txt"
    stubs_json=$(awk -F"$TAB" 'BEGIN { first = 1 }
        { printf "%s{\"service\":\"%s\",\"claim_id\":\"%s\",\"state\":\"%s\"}", (first ? "" : ","), $1, $2, $3; first = 0 }' \
        "$TMP/stubs.tsv")
    ov_json=null
    if [ "$WANT_CHANGES" -eq 1 ]; then
        cfj=$(json_array_body changed_files < "$TMP/changes.json" | awk '
        BEGIN { first = 1 }
        !/^[[:space:]]*$/ {
            n = split($0, a, /","/)
            for (i = 1; i <= n; i++) {
                v = a[i]; sub(/^"/, "", v); sub(/"$/, "", v)
                if (v != "") { printf "%s\"%s\"", (first ? "" : ","), v; first = 0 }
            }
        }')
        isj=$(json_array_body impacted_symbols < "$TMP/changes.json" | awk '
        BEGIN { first = 1 }
        !/^[[:space:]]*$/ {
            gsub(/\},[[:space:]]*\{/, "}\n{")
            n = split($0, items, "\n")
            for (i = 1; i <= n; i++) {
                o = items[i]; nm = ""; lb = ""; fl = ""
                if (match(o, /"name":"[^"]*"/))  nm = substr(o, RSTART + 8, RLENGTH - 9)
                if (match(o, /"label":"[^"]*"/)) lb = substr(o, RSTART + 9, RLENGTH - 10)
                if (match(o, /"file":"[^"]*"/))  fl = substr(o, RSTART + 8, RLENGTH - 9)
                if (nm != "") {
                    printf "%s{\"name\":\"%s\",\"label\":\"%s\",\"file\":\"%s\"}", (first ? "" : ","), nm, lb, fl
                    first = 0
                }
            }
        }')
        ov_json="{\"changed_files\":[$cfj],\"impacted_symbols\":[$isj],\"note\":\"display only - never folded into the ranking\"}"
    fi
    dlv_json=null
    [ -n "$DELIV" ] && dlv_json="\"$(json_escape "$DELIV")\""

    printf '{"plan":"lode.planner.v1","tool_version":"%s","seed":"%s","seed_service":"%s","project":"%s","deliverable":%s,"depth":%s,"horizon":%s,"services":[%s],"impacted":[%s],"unscored":[%s],"contract_edges":[%s],"changed_overlay":%s,"proposals":[%s],"stubs":[%s]}\n' \
        "$TOOL_VERSION" "$SEED" "$seed_service" "$PROJECT" "$dlv_json" "$DEPTH" "$HORIZON" \
        "$sv_json" "$impacted_json" "$unscored_json" "$ce_json" "$ov_json" "$props" "$stubs_json"
}

# ---- dispatch ---------------------------------------------------------------
cmd="${1:-}"
rc=0
case "$cmd" in
    --self-test|self-test) self_test || rc=$? ;;
    plan) shift; do_plan "$@" || rc=$? ;;
    rank) shift; sh "$RANK" "$@" || rc=$? ;;
    ""|-h|--help) sed -n '2,58p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try --self-test, plan, rank)" ;;
esac
exit "$rc"
