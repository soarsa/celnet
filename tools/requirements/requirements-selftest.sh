#!/usr/bin/env sh
# requirements-selftest.sh — the R5 gate harness: a FULL ROUND-TRIP on a
# sandbox tracker, receipt presence, engine purity (nm/strings gate), and the
# default-build byte-identity preconditions.
#
# WHAT IS STUBBED AND WHAT IS NOT (the load-bearing line):
#   - The EXTERNAL TRACKER is the stub (adapters/stub.sh, a real file-based
#     adapter speaking the full vendor contract) — stubbing the third-party
#     service is correct.
#   - The ENGINE IS NEVER STUBBED: every claim, review verdict, roll-up state
#     and receipt event in the live section goes through the real `lodestar`
#     binary (knowledge_put / knowledge_review / knowledge_get /
#     knowledge_export) against a real indexed sandbox repo, with
#     LODESTAR_CACHE_DIR pinned to a private dir.
#   - The cross-family REVIEWER is a recorded/stub verdict driven through the
#     real engine seam (the honest CI pattern of the judge/visual drivers);
#     the engine still enforces never-self on it.
#
# CHECK LADDER:
#   pure   1. default-off -> 'absent', exit 0, nothing written
#          2. stub adapter contract (seed / fetch / status)
#          3. "Done requires a receipt" at the adapter boundary
#          4. canonical receipt JSON: sorted keys, byte-identical re-runs
#          5. engine purity: no tracker symbols/strings in the built binary
#          6. default build byte-identical: zero build-input references to
#             tools/requirements (the bridge is never compiled or linked)
#   live   7. sandbox indexed; anchors discovered FROM the graph
#          8. import -> draft claims, testimony-tagged, ASSERT receipts in the
#             event log; a 0.01 confidence hint still authors (NEVER gates,
#             the §6.4 FLAG)
#          9. cross-family review routes -> claims active
#         10. push: verified roll-up -> receipt event -> ticket done WITH
#             receipt id; ticket was NOT done before the receipt existed
#         11. negative: unreviewed deliverable -> roll-up draft -> ticket
#             stays off-done; the receipt records the honest non-active state
#
# EXIT: 0 all green (live included) · 1 any check failed · 2 pure checks green
# but engine binary absent (live section skipped — run again after a build).
#
# shellcheck shell=sh
set -u
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/../lib/common.sh"

IMPORT="$HERE/requirements-import.sh"
PUSH="$HERE/requirements-push.sh"
STUB="$HERE/adapters/stub.sh"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lodereq-selftest.XXXXXX") || exit 1
trap 'rm -rf "$TMP"' EXIT INT TERM

fails=0; checks=0
ok()  { checks=$((checks+1)); printf 'ok   %s\n' "$1"; }
bad() { checks=$((checks+1)); fails=$((fails+1)); printf 'FAIL %s\n' "$1"; }
has() { case "$1" in *"$2"*) return 0 ;; *) return 1 ;; esac; }

native_path() {
    if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

# ── 1. default-off: the bridge is absent and writes NOTHING ─────────────────
# Run from a scratch cwd so any accidental write (state dir, out dir, event
# log) would be visible as a new entry.
mkdir -p "$TMP/scratch"
out=$(cd "$TMP/scratch" && LODESTAR_REQ_TRACKER=off sh "$IMPORT" import EPIC-1 2>/dev/null); rc=$?
if [ "$rc" -eq 0 ] && [ "$out" = "absent" ]; then
    ok 'default-off: import reports absent (exit 0), engine byte-identical'
else bad "default-off import: rc=$rc out=$out"; fi

out=$(cd "$TMP/scratch" && LODESTAR_REQ_TRACKER=off sh "$PUSH" push T-1 --anchor x 2>/dev/null); rc=$?
if [ "$rc" -eq 0 ] && [ "$out" = "absent" ]; then
    ok 'default-off: push reports absent (exit 0), nothing recorded'
else bad "default-off push: rc=$rc out=$out"; fi

leftovers=$(find "$TMP/scratch" -mindepth 1 2>/dev/null || true)
if [ -z "$leftovers" ]; then
    ok 'default-off: no file or state dir was created anywhere'
else bad "default-off: unexpected writes: $leftovers"; fi

# ── 2. stub adapter speaks the vendor-neutral contract ──────────────────────
export LODESTAR_REQ_STUB_DIR="$TMP/stub"
sh "$STUB" seed-epic FXO-1 2>/dev/null <<'EOF'
{"epic":"FXO-1","deliverable":"fx-options","title":"FX Options"}
{"ticket":"FXO-101","title":"req one","text":"capture submits trades","anchors":"QN1","target":""}
{"ticket":"FXO-102","title":"req two","text":"pricing prices trades","anchors":"QN2","target":""}
EOF
out=$(sh "$STUB" fetch-epic FXO-1 2>/dev/null)
n=$(printf '%s\n' "$out" | grep -c '"ticket"')
if has "$out" '"deliverable":"fx-options"' && [ "$n" -eq 2 ]; then
    ok 'stub: fetch-epic returns the canonical header + one flat line per requirement'
else bad "stub fetch-epic: out=$out"; fi

out=$(sh "$STUB" get-status FXO-1 2>/dev/null)
if has "$out" '"status":"todo"'; then
    ok 'stub: an untouched ticket reads back todo'
else bad "stub get-status: out=$out"; fi

# ── 3. Done requires a roll-up receipt — enforced at the adapter boundary ───
out=$(sh "$STUB" set-status FXO-1 done "" 2>/dev/null); rc=$?
st=$(sh "$STUB" get-status FXO-1 2>/dev/null)
if [ "$rc" -ne 0 ] && has "$out" 'receipt_required' && has "$st" '"status":"todo"'; then
    ok 'receipt gate: set-status done with NO receipt is refused; status untouched'
else bad "receipt gate: rc=$rc out=$out st=$st"; fi

out=$(sh "$STUB" set-status FXO-1 in-progress "" 2>/dev/null); rc=$?
if [ "$rc" -eq 0 ] && has "$out" '"ok":true'; then
    ok 'receipt gate: non-done statuses need no receipt (projection, not verification)'
else bad "stub in-progress: rc=$rc out=$out"; fi

out=$(sh "$STUB" set-status FXO-1 done "r-test-1" 2>/dev/null); rc=$?
st=$(cat "$TMP/stub/tickets/FXO-1" 2>/dev/null)
if [ "$rc" -eq 0 ] && [ "$st" = "done r-test-1" ]; then
    ok 'receipt gate: done WITH a receipt id flips the ticket and stores the receipt'
else bad "stub done-with-receipt: rc=$rc st=$st"; fi
sh "$STUB" set-status FXO-1 todo "" >/dev/null 2>&1   # reset for the live round-trip

# ── 4. canonical receipt JSON: sorted keys, deterministic bytes ─────────────
r1=$(sh "$PUSH" receipt-json FXO-1 fx-options active 0123456789abcdef 2>/dev/null)
r2=$(sh "$PUSH" receipt-json FXO-1 fx-options active 0123456789abcdef 2>/dev/null)
want='{"deliverable":"fx-options","direction":"status-push","epistemic_source":"tool-output","receipt":"tracker-roundtrip","rollup_state":"active","target_hash":"0123456789abcdef","ticket":"FXO-1","tool_version":"requirements-bridge@0.5.0","tracker":"off"}'
if [ "$r1" = "$r2" ] && [ "$r1" = "$want" ]; then
    ok 'receipt JSON: canonical sorted-key shape, byte-identical across runs'
else bad "receipt JSON: r1=$r1"; fi

# ── 5. engine purity — the nm/strings gate ───────────────────────────────────
# The engine must contain no tracker symbols/strings: the bridge is host-side
# shell, never compiled in. Symbol names also live in the binary's bytes, so a
# LC_ALL=C grep -a over the binary covers both the strings and symtab surfaces;
# nm (when present) is run additionally for a symbol-level report.
BIN=$(engine_bin)
BIN_PATH=$(command -v "$BIN" 2>/dev/null || true)
[ -z "$BIN_PATH" ] && [ -x "$BIN" ] && BIN_PATH="$BIN"
FORBIDDEN='atlassian api\.linear\.app api\.plane\.so tracker-roundtrip requirements-bridge epistemic_source'
if [ -n "$BIN_PATH" ] && [ -f "$BIN_PATH" ]; then
    hits=""
    for tok in $FORBIDDEN; do
        if LC_ALL=C grep -aq "$tok" "$BIN_PATH" 2>/dev/null; then hits="$hits $tok"; fi
    done
    if command -v nm >/dev/null 2>&1; then
        for tok in $FORBIDDEN; do
            if nm "$BIN_PATH" 2>/dev/null | LC_ALL=C grep -q "$tok"; then hits="$hits nm:$tok"; fi
        done
    fi
    if [ -z "$hits" ]; then
        ok "nm gate: engine binary carries no tracker symbols/strings ($BIN_PATH)"
    else bad "nm gate: forbidden tokens in engine binary:$hits"; fi
else
    printf 'note %s\n' 'nm gate: engine binary not found here; the gate runs post-build (see exit 2)'
fi

# ── 6. default build byte-identical: the bridge is never a build input ──────
ROOT=$(CDPATH= cd -- "$HERE/../.." && pwd)
refs=$(grep -rn 'tools/requirements' "$ROOT/Makefile" "$ROOT/scripts" 2>/dev/null | grep -v Binary || true)
csrc=$(find "$HERE" -name '*.c' -o -name '*.h' 2>/dev/null || true)
if [ -z "$refs" ] && [ -z "$csrc" ]; then
    ok 'byte-identity: no Makefile/scripts reference and no C sources under tools/requirements (build inputs unchanged)'
else bad "byte-identity: refs=[$refs] csrc=[$csrc]"; fi

# ── live round-trip (REAL engine, sandbox repo, stub tracker) ────────────────
engine_missing=0
if [ -z "$BIN_PATH" ] || [ ! -f "$BIN_PATH" ]; then
    engine_missing=1
    printf 'note %s\n' 'live round-trip SKIPPED: engine binary absent (exit 2; re-run after a build)'
elif [ "${LODESTAR_REQ_SELFTEST_LIVE:-1}" = "0" ]; then
    printf 'note %s\n' 'live round-trip SKIPPED: LODESTAR_REQ_SELFTEST_LIVE=0'
else
    # Private cache: the real engine writes only inside $TMP.
    export LODESTAR_CACHE_DIR="$TMP/cache"
    mkdir -p "$TMP/cache"
    SANDBOX="$TMP/repo"
    mkdir -p "$SANDBOX/pricing" "$SANDBOX/capture"
    cat > "$SANDBOX/pricing/core.py" <<'EOF'
def garman_kohlhagen(spot, strike, vol, t, rd, rf):
    return max(spot - strike, 0.0)


def price_fx_option(spot, strike, vol, t, rd, rf):
    premium = garman_kohlhagen(spot, strike, vol, t, rd, rf)
    return premium
EOF
    cat > "$SANDBOX/capture/client.py" <<'EOF'
def submit_trade(trade):
    return {"status": "submitted", "trade": trade}
EOF

    NPATH=$(native_path "$SANDBOX")
    idx=$("$BIN_PATH" cli index_repository "{\"repo_path\":\"$NPATH\",\"persistence\":true}" 2>/dev/null)
    PROJECT=$(printf '%s' "$idx" | json_get project)
    export LODESTAR_PROJECT="$PROJECT"
    if [ -n "$PROJECT" ] && has "$idx" '"status":"indexed"'; then
        ok "live: sandbox repo indexed as project '$PROJECT' (real pipeline, pinned cache)"
    else bad "live: sandbox index failed: $idx"; fi

    # 7. anchors come FROM the graph (never guessed).
    QN1=$("$BIN_PATH" cli search_graph "{\"project\":\"$PROJECT\",\"name_pattern\":\"price_fx_option\"}" 2>/dev/null \
        | sed -n 's/.*"qualified_name":"\([^"]*\)".*/\1/p' | head -n1)
    QN2=$("$BIN_PATH" cli search_graph "{\"project\":\"$PROJECT\",\"name_pattern\":\"submit_trade\"}" 2>/dev/null \
        | sed -n 's/.*"qualified_name":"\([^"]*\)".*/\1/p' | head -n1)
    if [ -n "$QN1" ] && [ -n "$QN2" ]; then
        ok "live: anchors resolved from the graph ($QN1, $QN2)"
    else bad "live: anchor discovery failed (QN1=$QN1 QN2=$QN2)"; fi

    # Recorded/stub helpers: a LOW round-trip confidence (must never gate) and
    # a cross-family reviewer verdict recorded through the REAL engine seam.
    cat > "$TMP/conf-low.sh" <<'EOF'
#!/bin/sh
printf 'CONFIDENCE: 0.01 | NOTE: low round-trip agreement (recorded selftest hint)\n'
EOF
    chmod +x "$TMP/conf-low.sh"
    cat > "$TMP/review-affirm.sh" <<EOF
#!/bin/sh
# Recorded cross-family verdict driven through the REAL engine (never-self is
# enforced by the engine; the reviewer family differs from the bridge author).
"$BIN_PATH" cli knowledge_review "{\"project\":\"$PROJECT\",\"claim_id\":\"\$1\",\"reviewer_model\":\"selftest:gpt-cross-reviewer\",\"verdict\":\"affirm\",\"concern\":\"recorded selftest verdict\",\"evidence_digest\":\"requirements-selftest:recorded\"}" >/dev/null
EOF
    chmod +x "$TMP/review-affirm.sh"

    # Seed the tracker epic with the graph-resolved anchors.
    sh "$STUB" seed-epic FXO-1 2>/dev/null <<EOF
{"epic":"FXO-1","deliverable":"fx-options","title":"FX Options"}
{"ticket":"FXO-101","title":"capture submits trades","text":"the capture service submits FX option trades","anchors":"$QN2","target":""}
{"ticket":"FXO-102","title":"pricing prices trades","text":"the pricing service prices FX option trades","anchors":"$QN1","target":""}
EOF

    # 8. import: epic -> draft testimony claims + review route (R5-a + R5-b).
    out=$(LODESTAR_REQ_TRACKER=stub LODESTAR_REQ_CONFIDENCE_CMD="$TMP/conf-low.sh" \
          LODESTAR_REQ_REVIEW_CMD="$TMP/review-affirm.sh" \
          sh "$IMPORT" import FXO-1 2>"$TMP/import.log"); rc=$?
    sum=$(printf '%s\n' "$out" | grep '^IMPORT:' || true)
    if [ "$rc" -eq 0 ] && has "$sum" 'authored=2' && has "$sum" 'reviewed=2' && has "$sum" 'failed=0'; then
        ok 'live: import authored 2 claims and routed both through cross-family review'
    else bad "live import: rc=$rc sum=$sum (log: $(tail -n3 "$TMP/import.log" 2>/dev/null | tr '\n' ' '))"; fi
    if has "$(cat "$TMP/import.log")" 'state=draft'; then
        ok 'live: imported claims entered as DRAFT (testimony, never auto-trusted)'
    else bad 'live: imported claims did not enter as draft'; fi

    EV="$SANDBOX/.lodestar/knowledge/events"
    if [ -d "$EV" ] && grep -l 'requirements-bridge' "$EV"/*.json >/dev/null 2>&1 \
       && grep -l 'epistemic_source=testimony' "$EV"/*.json >/dev/null 2>&1; then
        ok 'live: import receipts present — ASSERT events authored by requirements-bridge, tagged testimony (R5-a)'
    else bad "live: no testimony-tagged bridge events under $EV"; fi

    # The §6.4 FLAG, machine-checked: the 0.01 hint annotated the claims AND
    # both were still authored — the hint NEVER gates.
    claims=$("$BIN_PATH" cli knowledge_get "{\"project\":\"$PROJECT\",\"qualified_name\":\"$QN1\",\"states\":[\"draft\",\"active\",\"stale\",\"contradicted\",\"retired\"]}" 2>/dev/null)
    if has "$claims" 'authoring-hint:roundtrip=0.01' && has "$claims" 'epistemic_source=testimony'; then
        ok 'live: VeriTrans hint rides as draft-only annotation; a 0.01 confidence still authored (never gates, §6.4)'
    else bad "live: confidence annotation missing from stored claim: $claims"; fi

    # 9. the recorded cross-family affirms activated the claims (real engine
    # adjudication: single-tier kind, one distinct-family affirm).
    if has "$claims" '"state":"active"'; then
        ok 'live: cross-family review cleared the testimony -> claims active (R5-b)'
    else bad "live: claims not active after review: $claims"; fi

    # 10. push: ticket must be off-done BEFORE, done WITH receipt AFTER.
    pre=$(sh "$STUB" get-status FXO-1 2>/dev/null)
    out=$(LODESTAR_REQ_TRACKER=stub LODESTAR_REQ_OUT_DIR="$TMP/out" \
          sh "$PUSH" push FXO-1 --anchor "$QN1 $QN2" --deliverable fx-options 2>"$TMP/push.log"); rc=$?
    line=$(printf '%s\n' "$out" | grep '^PUSH:' || true)
    rid=$(printf '%s' "$line" | sed -n 's/.*receipt=\([^ ]*\).*/\1/p')
    post=$(cat "$TMP/stub/tickets/FXO-1" 2>/dev/null)
    if [ "$rc" -eq 0 ] && ! has "$pre" '"status":"done"' && has "$line" 'status=done' \
       && has "$line" 'rollup=active' && [ -n "$rid" ] && [ "$post" = "done $rid" ]; then
        ok "live: verified roll-up projected to the tracker — done ONLY with receipt $rid"
    else bad "live push: rc=$rc pre=$pre line=$line post=$post"; fi
    if grep -l 'tracker-roundtrip' "$EV"/*.json >/dev/null 2>&1 \
       && grep -l 'status-push' "$EV"/*.json >/dev/null 2>&1; then
        ok 'live: round-trip RECEIPT event present in the append-only event log'
    else bad 'live: no tracker-roundtrip receipt event found'; fi
    if [ -f "$TMP/out/fx-options.sarif.json" ]; then
        ok 'live: knowledge_export SARIF feed produced as the machine-readable status'
    else printf 'note %s\n' 'live: SARIF feed not produced (engine export unavailable?)'; fi

    # 11. negative: an UNREVIEWED deliverable must never flip done.
    QN3=$("$BIN_PATH" cli search_graph "{\"project\":\"$PROJECT\",\"name_pattern\":\"garman_kohlhagen\"}" 2>/dev/null \
        | sed -n 's/.*"qualified_name":"\([^"]*\)".*/\1/p' | head -n1)
    sh "$STUB" seed-epic NEG-1 2>/dev/null <<EOF
{"epic":"NEG-1","deliverable":"neg-demo","title":"negative case"}
{"ticket":"NEG-201","title":"unreviewed requirement","text":"this testimony is never reviewed","anchors":"$QN3","target":""}
EOF
    LODESTAR_REQ_TRACKER=stub sh "$IMPORT" import NEG-1 >/dev/null 2>&1
    out=$(LODESTAR_REQ_TRACKER=stub LODESTAR_REQ_OUT_DIR="$TMP/out" \
          sh "$PUSH" push NEG-1 --anchor "$QN3" --deliverable neg-demo 2>/dev/null); rc=$?
    line=$(printf '%s\n' "$out" | grep '^PUSH:' || true)
    post=$(cat "$TMP/stub/tickets/NEG-1" 2>/dev/null)
    if [ "$rc" -eq 0 ] && has "$line" 'rollup=draft' && has "$line" 'status=in-progress' \
       && ! has "$post" 'done'; then
        ok 'live: unreviewed testimony rolls up draft -> ticket stays off-done (receipt records the honest state)'
    else bad "live negative: rc=$rc line=$line post=$post"; fi
fi

# ── summary ──────────────────────────────────────────────────────────────────
printf '%s\n' "── requirements-bridge selftest: $((checks-fails))/$checks ok ──"
[ "$fails" -eq 0 ] || exit 1
[ "$engine_missing" -eq 1 ] && exit 2
exit 0
