#!/usr/bin/env sh
# planner-selftest.sh — deterministic self-test for the R3 planner (the §3 R3
# machine-checkable gate as a runnable harness, the visual-verify-selftest
# discipline): RECORDED engine outputs only — NO engine binary, NO graph, NO
# model, NO network, NO store write. The replay stub mirrors the
# `lodestar cli <tool> <json>` seam byte-for-byte.
#
# What it proves (plan §3 R3 gate + the lane's negative/defer cases):
#   reachable-only   stubs/services come ONLY from graph reachability: the
#                    recorded settlement/frontend symbols (present in the
#                    whole-graph recordings) never appear; a TESTS/IMPORTS row
#                    on an unreachable symbol is never folded in.
#   tier ordering    TDAD closed form: Direct 0.95 / Transitive 2..3 0.70 /
#                    TESTS 0.80 / IMPORTS 0.50; high -> medium -> low order;
#                    out-of-horizon and unknown edge classes land in
#                    'unscored' — reported, never guessed into a tier.
#   determinism      two identical runs are byte-identical (no timestamps).
#   draft-only       every authored stub is spec:satisfies + draft; a stub
#                    coming back 'active' ABORTS the planner (exit 4).
#   narrow-only      --services can only narrow the graph-fixed candidate
#                    set; asking for an unreachable service is REFUSED.
#   honest failure   an unresolvable seed exits non-zero with NO plan emitted.
#   overlay honesty  the detect_changes overlay is display-only: the ranked
#                    impacted set is byte-identical with and without it.
#
# shellcheck shell=sh
set -u
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PLANNER="$HERE/planner.sh"
RANK="$HERE/tdad-rank.sh"
FIX="$HERE/fixtures/recorded"
TMP="${TMPDIR:-/tmp}/planner-selftest.$$"
mkdir -p "$TMP"
trap 'rm -rf "$TMP"' EXIT INT TERM

fails=0; checks=0
ok()  { checks=$((checks+1)); printf 'ok   %s\n' "$1"; }
bad() { checks=$((checks+1)); fails=$((fails+1)); printf 'FAIL %s\n' "$1"; }
has() { case "$1" in *"$2"*) return 0 ;; *) return 1 ;; esac; }

# Deterministic environment: replay engine only; no user config may leak in.
chmod +x "$FIX/lodestar-replay.sh" 2>/dev/null || true
LODESTAR_BIN="$FIX/lodestar-replay.sh"
export LODESTAR_BIN
unset LODESTAR_PROJECT LODESTAR_JUDGE_CONFIG LODESTAR_PLANNER_CONFIG \
      LODESTAR_PLANNER_DEPTH LODESTAR_PLANNER_HORIZON \
      LODESTAR_PLANNER_SERVICE_SEGS LODESTAR_PLANNER_MAX_ANCHORS \
      LODESTAR_PLANNER_AUTHOR LODESTAR_PLANNER_PUT_PROJECT \
      LODESTAR_PLANNER_CONFIDENCE LODESTAR_REPLAY_PUT_STATE \
      LODESTAR_REPLAY_PUT_LOG 2>/dev/null || true
cd "$TMP" || exit 1

SEED=capture.client.submit_trade
TAB=$(printf '\t')

# ── tdad-rank: the closed form itself ───────────────────────────────────────

out=$(printf 'a.x.direct\tCALLS\t1\nb.y.trans\tCALLS\t2\nc.z.test\tTESTS\t-\nd.w.imp\tIMPORTS\t-\n' | sh "$RANK")
expect=$(printf 'high\t0.95\ta.x.direct\tCALLS\t1\nhigh\t0.80\tc.z.test\tTESTS\t-\nmedium\t0.70\tb.y.trans\tCALLS\t2\nlow\t0.50\td.w.imp\tIMPORTS\t-')
if [ "$out" = "$expect" ]; then
    ok 'rank: closed form Direct 0.95 / TESTS 0.80 / Transitive 0.70 / IMPORTS 0.50, tier-ordered'
else bad "rank closed form: got [$out]"; fi

out=$(printf 'far.away\tCALLS\t5\nweird.cls\tFUNKY_EDGE\t1\n' | sh "$RANK")
if has "$out" "unscored${TAB}0.00${TAB}far.away" && has "$out" "unscored${TAB}0.00${TAB}weird.cls" \
   && ! has "$out" 'high' && ! has "$out" 'medium' && ! has "$out" 'low'; then
    ok 'rank: out-of-horizon + unknown edge class -> unscored (reported, never tiered)'
else bad "rank unscored: got [$out]"; fi

out=$(printf 'q.one\tCALLS\t2\nq.one\tTESTS\t-\n' | sh "$RANK")
if [ "$out" = "$(printf 'high\t0.80\tq.one\tTESTS\t-')" ]; then
    ok 'rank: per-symbol dedup takes the MAX strategy score (0.80 TESTS over 0.70 transitive)'
else bad "rank dedup: got [$out]"; fi

in=$(printf 'm.a\tCALLS\t1\nm.b\tIMPORTS\t-\nm.c\tCALLS\t3\nm.d\tTESTS\t-\n')
r1=$(printf '%s\n' "$in" | sh "$RANK"); r2=$(printf '%s\n' "$in" | sh "$RANK")
if [ "$r1" = "$r2" ] && [ -n "$r1" ]; then
    ok 'rank: identical input -> byte-identical output (deterministic)'
else bad 'rank determinism'; fi

out=$(printf 'h.two\tCALLS\t4\n' | sh "$RANK" --horizon 4)
if [ "$out" = "$(printf 'medium\t0.70\th.two\tCALLS\t4')" ]; then
    ok 'rank: --horizon extends the transitive band explicitly (never silently)'
else bad "rank horizon: got [$out]"; fi

# ── planner --self-test (replay engine present) ─────────────────────────────

sh "$PLANNER" --self-test >/dev/null 2>&1; rc=$?
if [ "$rc" -eq 0 ]; then
    ok 'planner: --self-test ready (rank sane, engine seam reachable)'
else bad "planner self-test rc=$rc"; fi

# ── plan: the fixture estate (reachable-only, tiers, determinism) ───────────

plan1=$(sh "$PLANNER" plan "$SEED" --deliverable fx-options 2>"$TMP/err1"); rc=$?
if [ "$rc" -eq 0 ] && has "$plan1" '"plan":"lode.planner.v1"'; then
    ok 'plan: emits a lode.planner.v1 plan from the recorded estate'
else bad "plan basic: rc=$rc err=$(cat "$TMP/err1")"; fi

if ! has "$plan1" 'settlement' && ! has "$plan1" 'frontend'; then
    ok 'plan: REACHABLE-ONLY — recorded settlement/frontend symbols never proposed'
else bad 'plan reachable-only: unreachable service leaked into the plan'; fi

if has "$plan1" '"services":["capture","pricing","risk"]'; then
    ok 'plan: services = exactly the cross-service-reachable set (sorted)'
else bad "plan services: $plan1"; fi

tiers=$(printf '%s\n' "$plan1" | awk '{
    while (match($0, /"tier":"[a-z]+"/)) {
        printf "%s%s", (n++ ? " " : ""), substr($0, RSTART + 8, RLENGTH - 9)
        $0 = substr($0, RSTART + RLENGTH)
    } }')
if [ "$tiers" = "high high high high high medium medium medium medium medium low" ]; then
    ok 'plan: tier ordering high(5) -> medium(5) -> low(1), exactly as the closed form scores'
else bad "plan tier order: [$tiers]"; fi

if has "$plan1" '"impacted":[{"qualified_name":"capture.api.submit_handler","service":"capture","tier":"high","score":0.95,"via":"CALLS","hop":1}'; then
    ok 'plan: within-tier order is score desc then qualified_name asc (first entry exact)'
else bad 'plan first impacted entry'; fi

if has "$plan1" '{"qualified_name":"risk.alerts.on_priced_alert","service":"risk","tier":"medium","score":0.70,"via":"CROSS_ASYNC_CALLS","hop":3}'; then
    ok 'plan: cross-edge merge pulls the query_graph-only CROSS_ASYNC_CALLS far side in at hop+1'
else bad 'plan cross-merge symbol missing/mis-scored'; fi

if has "$plan1" '{"qualified_name":"pricing.tests.test_garman_kohlhagen","service":"pricing","tier":"high","score":0.80,"via":"TESTS","hop":null}' \
   && has "$plan1" '{"qualified_name":"pricing.api.docs_helper","service":"pricing","tier":"low","score":0.50,"via":"IMPORTS","hop":null}'; then
    ok 'plan: TESTS coverage 0.80 (high) and IMPORTS 0.50 (low) overlays applied with null hop'
else bad 'plan tests/imports overlays'; fi

if has "$plan1" '{"from":"capture.client.submit_trade","type":"CROSS_HTTP_CALLS","to":"pricing.routes.post_v1_fx_price"}' \
   && has "$plan1" '{"from":"pricing.events.publish_priced_trade","type":"CROSS_ASYNC_CALLS","to":"risk.consumers.on_priced_trade"}' \
   && has "$plan1" '{"from":"pricing.events.publish_priced_trade","type":"CROSS_ASYNC_CALLS","to":"risk.alerts.on_priced_alert"}'; then
    ok 'plan: contract_edges carry the typed CROSS_* spine with exact qualified_names'
else bad 'plan contract edges'; fi

if has "$plan1" '"unscored":[]'; then
    ok 'plan: nothing outside the closed form on this estate -> unscored is empty, not omitted'
else bad 'plan unscored section'; fi

plan2=$(sh "$PLANNER" plan "$SEED" --deliverable fx-options 2>/dev/null)
if [ "$plan1" = "$plan2" ]; then
    ok 'plan: DETERMINISM — two identical runs are byte-identical'
else bad 'plan determinism'; fi

# ── proposals + draft-stub authoring ────────────────────────────────────────

if has "$plan1" '{"service":"capture","anchors":["capture.client.submit_trade","capture.api.submit_handler","capture.client.build_payload","capture.models.FxOptionTrade"]}'; then
    ok 'plan: seed service proposal anchors the seed first, then ranked order'
else bad 'plan capture proposal'; fi

pricing_anchors=$(printf '%s\n' "$plan1" | sed 's/.*{"service":"pricing","anchors":\[\([^]]*\)\].*/\1/')
n_anchors=$(printf '%s\n' "$pricing_anchors" | awk -F'","' '{ print NF }')
if [ "$n_anchors" = "5" ] && ! has "$pricing_anchors" 'docs_helper'; then
    ok 'plan: anchors capped at max_anchors=5 by rank (the 0.50 IMPORTS symbol drops first)'
else bad "plan pricing anchors: n=$n_anchors [$pricing_anchors]"; fi

put1="$TMP/put1.log"
plan_s=$(LODESTAR_REPLAY_PUT_LOG="$put1" sh "$PLANNER" plan "$SEED" --deliverable fx-options --emit-stubs 2>/dev/null); rc=$?
nputs=$(wc -l < "$put1" | tr -d '[:space:]')
if [ "$rc" -eq 0 ] && [ "$nputs" = "3" ]; then
    ok 'stubs: --emit-stubs authors exactly one stub per reachable service (3), none beyond'
else bad "stubs count: rc=$rc nputs=$nputs"; fi

if [ "$(awk '/"kind":"spec:satisfies"/ { n++ } END { print n + 0 }' "$put1")" = "3" ] \
   && has "$(sed -n '1p' "$put1")" '"project":"capture"' \
   && has "$(sed -n '2p' "$put1")" '"project":"pricing"' \
   && has "$(sed -n '3p' "$put1")" '"project":"risk"'; then
    ok 'stubs: every put is kind spec:satisfies, routed to its service project'
else bad 'stubs payload kinds/projects'; fi

if has "$(sed -n '1p' "$put1")" '"anchors":[{"qualified_name":"capture.client.submit_trade"}' \
   && [ "$(awk 'NR==2 { print gsub(/"qualified_name"/, "&") }' "$put1")" = "5" ]; then
    ok 'stubs: anchors are the ranked qualified_names (seed first; pricing capped at 5)'
else bad 'stubs anchors'; fi

if has "$plan_s" '"stubs":[{"service":"capture","claim_id":"kn-fixture-1","state":"draft"},{"service":"pricing","claim_id":"kn-fixture-2","state":"draft"},{"service":"risk","claim_id":"kn-fixture-3","state":"draft"}]'; then
    ok 'stubs: every authored stub is recorded DRAFT — nothing auto-activates'
else bad "stubs states: $plan_s"; fi

# ── negative / defer cases ──────────────────────────────────────────────────

outp=$(LODESTAR_REPLAY_PUT_STATE=active LODESTAR_REPLAY_PUT_LOG="$TMP/put-h.log" \
       sh "$PLANNER" plan "$SEED" --emit-stubs 2>"$TMP/err-h"); rc=$?
if [ "$rc" -eq 4 ] && [ -z "$outp" ] && has "$(cat "$TMP/err-h")" 'HONESTY VIOLATION'; then
    ok 'honesty: a stub coming back ACTIVE aborts the planner (exit 4, no plan emitted)'
else bad "honesty abort: rc=$rc out=[$outp]"; fi

outp=$(sh "$PLANNER" plan nosuch.service.symbol 2>"$TMP/err-nf"); rc=$?
if [ "$rc" -eq 3 ] && [ -z "$outp" ]; then
    ok 'honest failure: unresolvable seed -> exit 3, NO plan fabricated'
else bad "seed-not-found: rc=$rc out=[$outp]"; fi

put_n="$TMP/put-narrow.log"
outp=$(LODESTAR_REPLAY_PUT_LOG="$put_n" sh "$PLANNER" plan "$SEED" --services pricing --emit-stubs 2>/dev/null); rc=$?
if [ "$rc" -eq 0 ] && [ "$(wc -l < "$put_n" | tr -d '[:space:]')" = "1" ] \
   && has "$outp" '"proposals":[{"service":"pricing"' \
   && has "$outp" '"services":["capture","pricing","risk"]'; then
    ok 'narrow-only: --services pricing narrows the stubs; the graph-fixed set stays visible'
else bad "narrow: rc=$rc"; fi

outp=$(sh "$PLANNER" plan "$SEED" --services settlement 2>"$TMP/err-n2"); rc=$?
if [ "$rc" -eq 4 ] && [ -z "$outp" ] && has "$(cat "$TMP/err-n2")" 'can only narrow'; then
    ok 'narrow-only: --services settlement (unreachable) is REFUSED (exit 4), never granted'
else bad "narrow refuse: rc=$rc out=[$outp]"; fi

planc=$(sh "$PLANNER" plan "$SEED" --deliverable fx-options --changes main 2>/dev/null); rc=$?
imp_a=$(printf '%s\n' "$plan1" | sed 's/.*"impacted":\[\(.*\)\],"unscored".*/\1/')
imp_c=$(printf '%s\n' "$planc" | sed 's/.*"impacted":\[\(.*\)\],"unscored".*/\1/')
if [ "$rc" -eq 0 ] && has "$planc" '"changed_overlay":{"changed_files":["capture/client.py"]' \
   && has "$planc" 'display only - never folded into the ranking' \
   && [ "$imp_a" = "$imp_c" ]; then
    ok 'overlay: detect_changes is display-only — the ranked impacted set is byte-identical'
else bad "overlay: rc=$rc"; fi

# ── summary ─────────────────────────────────────────────────────────────────
printf '%s\n' "── planner selftest: $((checks-fails))/$checks ok ──"
[ "$fails" -eq 0 ] || exit 1
exit 0
