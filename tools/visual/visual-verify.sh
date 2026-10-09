#!/usr/bin/env sh
# visual-verify.sh — host-side reference driver for the ui:matches-mockup
# VISUAL-VERIFIER SEAM + the rendered-DOM a11y facts (a11y:rendered).
# DEFAULT-OFF. The engine stays pure-C, model-free, air-gapped: it emits the
# review REQUEST (claim id, kind, component spec, policy, target content-hash)
# and RECORDS the returned verdict. ALL render + diff + model work happens
# HERE, outside the engine — exactly like tools/judge/*.sh and
# tools/execute-verify/execute-verify.sh.
#
# THE TWO-STAGE PIPELINE (all OPEN tooling; NO commercial product, NO Figma):
#   1. render  — the anchored component's named Storybook story in ISOLATION via
#                @storybook/test-runner (MIT). Reproducible per-story state.
#   2. shot    — headless screenshot via Playwright (Apache-2.0) in the test-
#                runner's postVisit hook (page.screenshot()). No custom renderer.
#   3. STAGE 1 — DETERMINISTIC pixel/structural diff of the rendered story vs
#                the committed, content-addressed design TARGET (pixelmatch-
#                style; byte-identity via cmp always decides; an optional OCR
#                visual-block alignment refines the report, never decides).
#                A stage-1 match is a committed-baseline, fixed-renderer,
#                ZERO-MODEL verdict: the driver EARNS the DETERMINISTIC
#                reproducibility band — the engine never synthesizes it.
#   4. STAGE 2 — VLM, ONLY on a stage-1 FLAGGED DELTA, and only as a
#                COMPARATIVE judgment (screenshot vs target, pairwise — never
#                an absolute score: VLM judges rank reliably, absolute scores
#                are uninformative). CONFORMAL ABSTENTION: the vision command
#                reports an interval derived from its score-token
#                log-probabilities; a wide interval ⇒ the driver returns an
#                HONEST UNSUPPORTED/uncertain (the symbolic verdict stands).
#                Stage-2 verdicts are banded measured-with-caveats, NEVER
#                gate-grade.
#   5. a11y    — rendered-DOM accessibility facts via axe-core 4.12.x
#                (MPL-2.0, separate host process so its file-level copyleft
#                never touches the pure-C engine) run through Playwright
#                against the BUILT STATIC Storybook (file:// — no live
#                service). Every rendered fact carries the axe-core VERSION +
#                a deterministic RULESET HASH (rules churn every axe minor).
#                Gate baseline: WCAG 2.2 AA.
#   6. record  — return the verdict through the engine seam (knowledge_review /
#                lode_review_record -> lode_review_adjudicate): CROSS-FAMILY,
#                NEVER-SELF enforced by review.c (a same-family reviewer is
#                refused, LODE_REVIEW_SAME_FAMILY).
#
# RECOMMENDED OPEN-WEIGHT VISION JUDGES (host-installed, NEVER bundled —
# vendor-neutral; pick any cross-family model): Qwen3-VL, GLM-4.6V, Molmo
# (Apache-2.0), UI-TARS / UGround (pixel-level grounding without DOM
# metadata). Wire one via vision_cmd; the driver never ships a model.
#
# TRUST POSTURE (the design's load-bearing fence): the visual verdict NEVER
# widens trust beyond the deterministic design gate. A REFUTE can mark a claim
# contradicted; a single AFFIRM cannot certify beyond what the symbolic gate
# proved (pixels-match != conformance — the Figma2Code finding). Every verdict
# is reproducibility-banded; only a stage-1 zero-model match may claim the
# deterministic band.
#
# USAGE:
#   visual-verify.sh --self-test                       # capability matrix; no render, no model
#   visual-verify.sh capabilities                      # JSON: which open tools are present
#   visual-verify.sh stage1 <screenshot> <target_asset>
#       deterministic diff only; exit 0 match / 1 diff / 3 undecided
#   visual-verify.sh judge <claim_id> <screenshot> <target_asset>
#       two-stage verdict line on stdout; records NOTHING (no engine needed)
#   visual-verify.sh verify <claim_id> <story_id> <target_asset>
#       render story -> two-stage verdict -> record cross-family
#   visual-verify.sh a11y-judge <story_id> <axe_result_json>
#       rendered-a11y verdict line from an axe result; records NOTHING
#   visual-verify.sh a11y <claim_id> <story_id>
#       axe-core vs BUILT STATIC Storybook -> record (facts carry version+hash)
#
# DEGRADES WHEN ABSENT (honest seam, like judge/execute-verify — a machine
# without Playwright/Storybook/a pixel tool/a model gets an UNSUPPORTED or
# uncertain verdict, NEVER an error, NEVER a fabricated verdict):
#   - provider=off (default)        -> 'absent'; engine byte-identical; gate stands.
#   - test-runner/Playwright absent -> render 'unsupported'; the SEAM is exercised
#                                       with a RECORDED/STUB verdict (uncertain) —
#                                       stated honestly, NOT a live capability.
#   - no pixel-diff tool            -> stage-1 undecided on byte-differing files
#                                       (PNG encoders differ while pixels may not);
#                                       NO stage-2 without a flagged delta -> uncertain.
#   - no vision_cmd                 -> stage-2 absent: a flagged delta records an
#                                       uncertain stub verdict (no live model ran).
#   - wide conformal interval       -> honest abstention (uncertain); symbolic stands.
#   - axe/static-build/node absent  -> a11y 'unsupported' (exit 3); the STATIC
#                                       a11y:labeled/role facts stand alone, unchanged.
#   - engine binary missing         -> exit 2, records nothing.
#   - same-family reviewer          -> engine refuses (LODE_REVIEW_SAME_FAMILY).
#
# shellcheck shell=sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/../lib/common.sh"

if [ -n "${LODESTAR_VISUAL_CONFIG:-}" ] && [ -f "${LODESTAR_VISUAL_CONFIG}" ]; then
    LODESTAR_JUDGE_CONFIG="${LODESTAR_VISUAL_CONFIG}"; export LODESTAR_JUDGE_CONFIG
    cfg_locate
fi

PROVIDER=$(cfg LODESTAR_VISUAL_PROVIDER visual provider "off")
TEST_RUNNER=$(cfg LODESTAR_STORYBOOK_TEST_RUNNER visual test_runner "test-storybook")
PLAYWRIGHT_BIN=$(cfg LODESTAR_PLAYWRIGHT_BIN visual playwright_bin "playwright")
AXE_MODE=$(cfg LODESTAR_AXE_MODE visual axe_mode "addon")
SHOT_DIR=$(cfg LODESTAR_VISUAL_SHOTS visual screenshot_dir ".lodestar/visual/shots")
VISION_CMD=$(cfg LODESTAR_VISUAL_VISION_CMD visual vision_cmd "")
REVIEWER=$(cfg LODESTAR_VISUAL_REVIEWER visual reviewer_model "agent:visual-verifier")
# Stage-1 deterministic diff (P2). pixel_diff_cmd: explicit command wins;
# "none" disables pixel tooling explicitly (byte-identity via cmp still
# decides); "" -> feature-detect pixelmatch, then ImageMagick.
PIXELDIFF_CMD=$(cfg LODESTAR_VISUAL_PIXELDIFF_CMD visual pixel_diff_cmd "")
# Optional OCR visual-block alignment (Design2Code-lineage structural check).
# It REFINES the stage-1 report on a flagged delta; it never decides alone.
OCR_CMD=$(cfg LODESTAR_VISUAL_OCR_CMD visual ocr_cmd "")
# Conformal abstention policy (P2): a stage-2 interval wider than this is an
# uninformative judgment -> honest abstention. 0.40 = the span at which VLM
# score intervals are uninformative on natural images (arXiv 2604.25235).
CONFORMAL_MAX_WIDTH=$(cfg LODESTAR_VISUAL_CONFORMAL_MAX_WIDTH visual conformal_max_width "0.40")
# Rendered a11y (P3): the BUILT STATIC Storybook dir + the axe runner command.
# axe_runner contract: <static_dir> <story_id> <out_json> argv; writes JSON
# {"storyId":...,"axeVersion":"4.12.1","ruleIds":[...],"violationIds":[...]}.
# "" -> feature-detect node + the bundled reference runner; "none" disables.
STORYBOOK_STATIC=$(cfg LODESTAR_STORYBOOK_STATIC storybook static_dir "storybook-static")
AXE_RUNNER=$(cfg LODESTAR_AXE_RUNNER visual axe_runner "")
AXE_REF_RUNNER="$HERE/fixtures/axe-static.example.js"

test_runner_available() { command -v "$TEST_RUNNER" >/dev/null 2>&1 || command -v npx >/dev/null 2>&1; }
playwright_available()  { command -v "$PLAYWRIGHT_BIN" >/dev/null 2>&1 || command -v npx >/dev/null 2>&1; }
axe_available()         { [ "$AXE_MODE" != "off" ] && command -v npx >/dev/null 2>&1; }
vision_available()      { [ -n "$VISION_CMD" ]; }

# pixeldiff_tool -> the stage-1 pixel tool to use, or "" when none.
# Vendor-neutral feature detection: explicit pixel_diff_cmd > pixelmatch
# (pixelmatch-style CLI) > ImageMagick 7 (magick compare) > ImageMagick 6
# (compare). "none" disables explicitly (keeps self-tests deterministic).
pixeldiff_tool() {
    case "$PIXELDIFF_CMD" in
        none) printf ''; return 0 ;;
        ?*)   printf '%s' "$PIXELDIFF_CMD"; return 0 ;;
    esac
    if command -v pixelmatch >/dev/null 2>&1; then printf 'pixelmatch'; return 0; fi
    if command -v magick >/dev/null 2>&1; then printf 'magick'; return 0; fi
    if command -v compare >/dev/null 2>&1; then printf 'compare'; return 0; fi
    printf ''
}
pixeldiff_available() { [ -n "$(pixeldiff_tool)" ]; }

# ocr_tool -> the optional OCR tool for visual-block alignment, or "".
ocr_tool() {
    case "$OCR_CMD" in
        none) printf ''; return 0 ;;
        ?*)   printf '%s' "$OCR_CMD"; return 0 ;;
    esac
    command -v tesseract >/dev/null 2>&1 && printf 'tesseract' || printf ''
}
ocr_available() { [ -n "$(ocr_tool)" ]; }

# axe_ready -> 0 iff the rendered-a11y stack is fully present: axe mode on,
# the BUILT STATIC Storybook exists (no live service), and a runner resolves.
axe_ready() {
    axe_available || return 1
    [ -f "$STORYBOOK_STATIC/index.html" ] || return 1
    if [ -n "$AXE_RUNNER" ] && [ "$AXE_RUNNER" != "none" ]; then return 0; fi
    [ "$AXE_RUNNER" != "none" ] && command -v node >/dev/null 2>&1 && [ -f "$AXE_REF_RUNNER" ]
}

# effective_mode -> off | live | seam-stub | absent
#   off       : provider=off (default). Seam reports absent; engine byte-identical.
#   live      : render+shot tooling present AND a verdict path exists — a
#               stage-1 pixel tool (deterministic, zero-model) OR a vision_cmd.
#   seam-stub : provider on but a live tool absent -> exercise the seam with a
#               recorded/stub verdict (the honest CI pattern), NOT a live capability.
effective_mode() {
    case "$PROVIDER" in
        off) printf 'off'; return ;;
        reference) : ;;
        *) printf 'absent'; return ;;
    esac
    if test_runner_available && playwright_available && { pixeldiff_available || vision_available; }; then
        printf 'live'
    else
        printf 'seam-stub'
    fi
}

capabilities() {
    os=$(host_os); eff=$(effective_mode)
    tr=$(test_runner_available && echo true || echo false)
    pw=$(playwright_available && echo true || echo false)
    ax=$(axe_available && echo true || echo false)
    vc=$(vision_available && echo true || echo false)
    pd=$(pixeldiff_available && echo true || echo false)
    oc=$(ocr_available && echo true || echo false)
    ar=$(axe_ready && echo true || echo false)
    log "[VV] os=$os provider=$PROVIDER effective=$eff"
    log "[VV] open tools: storybook-test-runner(MIT)=$tr playwright(Apache-2.0)=$pw axe-core(MPL-2.0)=$ax"
    log "[VV] stage-1 pixel-diff=$pd ocr=$oc | stage-2 vision_cmd=$vc | rendered-a11y ready=$ar"
    log "[VV] design target = content-addressed IN-REPO asset (kn_hash). NO Figma / commercial dep."
    printf '{"os":"%s","provider":"%s","effective_mode":"%s","test_runner":%s,"playwright":%s,"axe":%s,"vision_cmd":%s,"pixel_diff":%s,"ocr":%s,"a11y_ready":%s}\n' \
        "$os" "$PROVIDER" "$eff" "$tr" "$pw" "$ax" "$vc" "$pd" "$oc" "$ar"
}

# render_story <story_id> -> screenshot path on stdout, or empty if unsupported.
# Real path: drive @storybook/test-runner whose postVisit hook calls Playwright
# page.screenshot(). We do not reimplement a renderer; we invoke the open tools.
render_story() {
    sid="$1"
    mkdir -p "$SHOT_DIR" 2>/dev/null || true
    shot="$SHOT_DIR/$(printf '%s' "$sid" | tr '/ :' '___').png"
    if ! test_runner_available || ! playwright_available; then
        log "[VV] render UNSUPPORTED: storybook test-runner / Playwright absent on this host."
        log "[VV]   install (host-side only): npm i -D @storybook/test-runner playwright && npx playwright install"
        return 3
    fi
    log "[VV] rendering story '$sid' in isolation via test-runner + Playwright -> $shot"
    # The reference invocation. The test-runner config's postVisit hook is where
    # page.screenshot({ path }) + (axe via axe-playwright) run; see
    # fixtures/test-runner.postVisit.example.js. We pass the story id filter and
    # the target shot path through env the hook reads, and scope the run to the
    # single story with a Jest name filter (-t). Requires a built/running
    # Storybook (LODESTAR_STORYBOOK_BUILD); absent ⇒ no screenshot ⇒ seam-stub.
    if command -v "$TEST_RUNNER" >/dev/null 2>&1; then set -- "$TEST_RUNNER";
    else set -- npx "$TEST_RUNNER"; fi
    LODESTAR_SHOT_PATH="$shot" LODESTAR_STORY_ID="$sid" \
        "$@" -t "$sid" >/dev/null 2>&1 || true
    if [ "$AXE_MODE" != "off" ]; then
        log "[VV] axe-core a11y scan ($AXE_MODE) runs in the same postVisit hook (separate host process)."
    fi
    [ -f "$shot" ] && printf '%s' "$shot" || { log "[VV] render produced no screenshot (story missing or build absent)."; return 3; }
}

# ── STAGE 1 — deterministic pixel/structural diff (P2) ──────────────────────

# stage1_metric <tool> <shot> <target> -> mismatched-pixel count on stdout, or
# empty when the tool produced no usable metric. Known tools are normalized;
# an explicit pixel_diff_cmd receives <shot> <target> and must print
# "PIXELDIFF: <n>".
stage1_metric() {
    smtool="$1"; sma="$2"; smb="$3"
    case "$smtool" in
        pixelmatch)
            # pixelmatch CLI prints "different pixels: N" (threshold default).
            pixelmatch "$sma" "$smb" 2>/dev/null \
                | sed -n 's/.*different pixels:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -n1 ;;
        magick)
            # ImageMagick 7: AE metric = absolute count of differing pixels (stderr).
            magick compare -metric AE "$sma" "$smb" null: 2>&1 >/dev/null \
                | sed -n 's/^\([0-9][0-9]*\).*/\1/p' | head -n1 ;;
        compare)
            compare -metric AE "$sma" "$smb" null: 2>&1 >/dev/null \
                | sed -n 's/^\([0-9][0-9]*\).*/\1/p' | head -n1 ;;
        *)
            "$smtool" "$sma" "$smb" 2>/dev/null \
                | sed -n 's/.*PIXELDIFF:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -n1 ;;
    esac
}

# ocr_extract <image> <tool> -> normalized extracted text ("" when unusable).
ocr_extract() {
    oimg="$1"; otool="$2"
    case "$otool" in
        tesseract) tesseract "$oimg" stdout 2>/dev/null | tr -s '[:space:]' ' ' ;;
        *)         "$otool" "$oimg" 2>/dev/null | tr -s '[:space:]' ' ' ;;
    esac
}

# ocr_note <shot> <target> -> text-aligned | text-delta | -
# Optional OCR visual-block alignment on a FLAGGED delta: equal extracted text
# means the delta is layout/style-only. Feature-detected; never decides alone.
ocr_note() {
    onotool=$(ocr_tool)
    [ -n "$onotool" ] || { printf '%s' '-'; return 0; }
    ota=$(ocr_extract "$1" "$onotool") || ota=""
    otb=$(ocr_extract "$2" "$onotool") || otb=""
    if [ -z "$ota" ] && [ -z "$otb" ]; then printf '%s' '-'; return 0; fi
    [ "$ota" = "$otb" ] && printf 'text-aligned' || printf 'text-delta'
}

# stage1_diff <shot> <target> -> one line on stdout:
#   STAGE1: match|diff|unsupported | METRIC: <mismatched_pixels|-> | TOOL: <tool|->
# exit 0 match · 1 diff · 3 undecided.
# Decidability split: byte-identity (cmp, always present) PROVES a match; a
# pixel tool quantifies a real pixel delta; bytes differ + no pixel tool ⇒
# UNDECIDED (PNG encoders differ while pixels may not — never guessed).
stage1_diff() {
    s1shot="$1"; s1tgt="$2"
    if [ ! -f "$s1shot" ] || [ ! -f "$s1tgt" ]; then
        printf 'STAGE1: unsupported | METRIC: - | TOOL: -\n'; return 3
    fi
    if cmp -s "$s1shot" "$s1tgt" 2>/dev/null; then
        printf 'STAGE1: match | METRIC: 0 | TOOL: cmp\n'; return 0
    fi
    s1tool=$(pixeldiff_tool)
    if [ -z "$s1tool" ]; then
        log "[VV] stage-1 UNDECIDED: bytes differ and no pixel-diff tool present (install pixelmatch or ImageMagick, or set pixel_diff_cmd)."
        printf 'STAGE1: unsupported | METRIC: - | TOOL: -\n'; return 3
    fi
    s1metric=$(stage1_metric "$s1tool" "$s1shot" "$s1tgt") || s1metric=""
    case "$s1metric" in
        ''|*[!0-9]*)
            log "[VV] stage-1 UNDECIDED: tool '$s1tool' produced no usable pixel metric."
            printf 'STAGE1: unsupported | METRIC: - | TOOL: %s\n' "$s1tool"; return 3 ;;
    esac
    if [ "$s1metric" -eq 0 ]; then
        printf 'STAGE1: match | METRIC: 0 | TOOL: %s\n' "$s1tool"; return 0
    fi
    printf 'STAGE1: diff | METRIC: %s | TOOL: %s\n' "$s1metric" "$s1tool"; return 1
}

# ── STAGE 2 — comparative VLM judgment with conformal abstention (P2) ───────

# run_vision <shot> <target> <claim_id> <stage1_summary>
#   -> one line: VERDICT: <affirm|refute|uncertain> | CONCERN: <c> [| INTERVAL: <lo> <hi>]
# COMPARATIVE only: the request hands the model BOTH images plus the stage-1
# delta context; the model judges screenshot-vs-target PAIRWISE — never an
# absolute score. The optional INTERVAL is the vision command's conformal
# interval over its score-token log-probabilities (distribution-free
# calibration); the DRIVER enforces the abstention policy on it.
run_vision() {
    shot="$1"; target="$2"; cid="$3"; s1sum="${4:-}"
    req=$(printf '{"claim_id":"%s","kind":"ui:matches-mockup","mode":"comparative","screenshot":"%s","target":"%s","stage1":"%s"}' \
        "$cid" "$shot" "$target" "$s1sum")
    if vision_available; then
        log "[VV] stage-2: handing screenshot + in-repo target to vision_cmd (comparative, cross-family)."
        # The vision command owns ALL model work; it must print the verdict line.
        # shellcheck disable=SC2086
        "$VISION_CMD" "$shot" "$target" "$req" 2>/dev/null || printf 'VERDICT: uncertain | CONCERN: vision_cmd failed to run'
    else
        # Honest degradation: NO live vision model here. Exercise the seam with a
        # recorded/stub verdict — the same pattern the judge/execute-verify seams
        # use in CI — and SAY SO. Never a fabricated affirm.
        log "[VV] no vision_cmd configured -> RECORDED/STUB verdict (seam exercised, NOT a live judgment)."
        printf 'VERDICT: uncertain | CONCERN: visual provider absent; seam recorded a stub verdict (no live model ran)'
    fi
}

# interval_of <vline> -> "lo hi" or "" when the line carries no interval.
interval_of() {
    printf '%s' "$1" | sed -n 's/.*INTERVAL:[[:space:]]*\([0-9.][0-9.]*\)[[:space:]]\{1,\}\([0-9.][0-9.]*\).*/\1 \2/p' | head -n1
}

# conformal_abstains <vline> -> 0 (abstain: interval present AND wider than
# CONFORMAL_MAX_WIDTH) | 1 (narrow interval, or no interval reported).
# A missing interval is an UNCALIBRATED verdict: it stands (back-compat with
# vision commands that predate calibration) and is recorded as uncalibrated in
# the evidence digest — never silently promoted to calibrated.
conformal_abstains() {
    caiv=$(interval_of "$1")
    [ -n "$caiv" ] || return 1
    awk -v w="$CONFORMAL_MAX_WIDTH" -v iv="$caiv" 'BEGIN {
        n = split(iv, a, " ");
        if (n < 2) exit 1;
        if (a[2] - a[1] > w + 0) exit 0;
        exit 1;
    }'
}

# stage2_judge <shot> <target> <claim_id> <stage1_summary> -> final verdict line.
stage2_judge() {
    s2line=$(run_vision "$1" "$2" "$3" "$4")
    if conformal_abstains "$s2line"; then
        s2iv=$(interval_of "$s2line" | tr ' ' '-')
        log "[VV] stage-2 conformal interval ($s2iv) wider than $CONFORMAL_MAX_WIDTH -> HONEST ABSTENTION (symbolic verdict stands)."
        printf 'VERDICT: uncertain | CONCERN: conformal abstention: score-token interval %s wider than %s — comparative judgment uninformative; symbolic verdict stands | INTERVAL: %s | BAND: measured-with-caveats | STAGE1: %s\n' \
            "$s2iv" "$CONFORMAL_MAX_WIDTH" "$(interval_of "$s2line")" "$4"
        return 0
    fi
    # A stage-2/model verdict is ALWAYS measured-with-caveats — never gate-grade.
    printf '%s | BAND: measured-with-caveats | STAGE1: %s\n' "$s2line" "$4"
}

# do_judge <claim_id> <shot> <target> -> the two-stage verdict line on stdout.
# Records NOTHING (no engine binary needed) — the testable two-stage core.
do_judge() {
    cid="$1"; djshot="$2"; djtarget="$3"
    s1line=$(stage1_diff "$djshot" "$djtarget") && s1rc=0 || s1rc=$?
    s1=$(printf '%s' "$s1line" | sed -n 's/.*STAGE1:[[:space:]]*\([a-z-]*\).*/\1/p')
    s1metric=$(printf '%s' "$s1line" | sed -n 's/.*METRIC:[[:space:]]*\([0-9][0-9]*\).*/\1/p')
    s1tool=$(printf '%s' "$s1line" | sed -n 's/.*TOOL:[[:space:]]*\([^ |][^ |]*\).*/\1/p')
    log "[VV] stage-1: $s1line (rc=$s1rc)"
    case "$s1" in
        match)
            # Committed baseline + fixed renderer + ZERO model: the driver (the
            # backend side of the seam) EARNS the deterministic band here — the
            # engine never synthesizes it. No stage-2, no model invoked.
            printf 'VERDICT: affirm | CONCERN: stage-1 deterministic diff: rendered story matches the committed design target (tool=%s, mismatched-pixels=0) | BAND: deterministic | STAGE1: match\n' \
                "$s1tool"
            ;;
        diff)
            s1ocr=$(ocr_note "$djshot" "$djtarget")
            s1sum="diff:${s1metric:-?}px tool=$s1tool ocr=$s1ocr"
            log "[VV] stage-1 flagged a delta ($s1sum) -> stage-2 comparative judgment."
            stage2_judge "$djshot" "$djtarget" "$cid" "$s1sum"
            ;;
        *)
            # Stage-1 undecided and stage-2 runs ONLY on a flagged delta -> an
            # honest uncertain; the deterministic symbolic gate stands alone.
            printf 'VERDICT: uncertain | CONCERN: stage-1 deterministic diff undecided (bytes differ, no pixel tool) and stage-2 runs only on a flagged delta; symbolic verdict stands | BAND: measured-with-caveats | STAGE1: unsupported\n'
            ;;
    esac
}

# record_verdict <claim_id> <verdict_line>
# Maps affirm/refute/uncertain and records cross-family via the engine seam. The
# engine enforces never-self; a same-family reviewer is refused at the seam.
# The evidence digest carries the reproducibility band, the stage-1 outcome,
# the conformal interval (or calibration=absent for an uncalibrated stage-2
# verdict), and — for rendered a11y — the axe version + ruleset hash.
record_verdict() {
    cid="$1"; vline="$2"
    verdict=$(printf '%s' "$vline" | sed -n 's/.*VERDICT:[[:space:]]*\([a-z]*\).*/\1/p')
    concern=$(printf '%s' "$vline" | sed -n 's/.*CONCERN:[[:space:]]*//p')
    concern=$(printf '%s' "$concern" \
        | sed 's/ | INTERVAL:.*//; s/ | BAND:.*//; s/ | STAGE1:.*//; s/ | AXE:.*//; s/ | RULESET:.*//')
    case "$verdict" in affirm|refute|uncertain) : ;; *) verdict="uncertain"; concern="unparseable verdict line: $vline" ;; esac
    band=$(printf '%s' "$vline" | sed -n 's/.*BAND:[[:space:]]*\([a-z-]*\).*/\1/p')
    [ -n "$band" ] || band="measured-with-caveats"
    # Only the zero-model stage-1 match may claim the deterministic band; any
    # other claim of it is clamped here exactly like the engine clamps a host
    # backend that over-claims reproducibility.
    s1f=$(printf '%s' "$vline" | sed -n 's/.*STAGE1:[[:space:]]*\([a-z-]*\).*/\1/p')
    if [ "$band" = "deterministic" ] && [ "$s1f" != "match" ]; then
        band="measured-with-caveats"
    fi
    digest="visual-verify:band=$band"
    [ -n "$s1f" ] && digest="$digest;stage1=$s1f"
    iv=$(interval_of "$vline" | tr ' ' '-')
    if [ -n "$iv" ]; then
        digest="$digest;interval=$iv"
    elif [ "$s1f" = "diff" ]; then
        digest="$digest;calibration=absent"
    fi
    axev=$(printf '%s' "$vline" | sed -n 's/.*AXE:[[:space:]]*\([^ |][^ |]*\).*/\1/p')
    rsh=$(printf '%s' "$vline" | sed -n 's/.*RULESET:[[:space:]]*\([0-9a-f][0-9a-f]*\).*/\1/p')
    if [ -n "$axev" ] && [ "$axev" != "-" ]; then
        digest="$digest;axe=$axev;ruleset=${rsh:--}"
    fi
    # Family pre-check for a clear local message; the engine is the authority.
    af=$(family_of "$REVIEWER")
    log "[VV] recording verdict via engine seam: verdict=$verdict reviewer=$REVIEWER (family=$af) digest=$digest"
    log "[VV] trust fence: a REFUTE marks contradicted; an AFFIRM never overrides a symbolic refute; banded ($band), never a hard gate."
    out=$(engine_review_record "$cid" "$REVIEWER" "$verdict" "$concern" "$digest")
    case "$out" in
        *'same_family'*|*'SAME_FAMILY'*)
            warn "[VV] engine refused same-family reviewer (LODE_REVIEW_SAME_FAMILY). Pick a cross-family reviewer_model." ;;
    esac
    printf '%s\n' "$out"
}

do_verify() {
    cid="$1"; sid="$2"; target="$3"
    bin=$(engine_bin)
    if ! command -v "$bin" >/dev/null 2>&1 && [ ! -x "$bin" ]; then
        log "[VV] engine binary '$bin' not found; cannot record a verdict (set LODESTAR_BIN)."; return 2
    fi
    [ -f "$target" ] || die "design target '$target' not found (must be an IN-REPO, content-addressed asset)"
    eff=$(effective_mode)
    if [ "$eff" = "off" ] || [ "$eff" = "absent" ]; then
        log "[VV] provider=$PROVIDER (effective=$eff): visual seam OFF. Engine byte-identical; deterministic design gate stands alone."
        printf 'absent\n'; return 0
    fi
    shot=$(render_story "$sid") || shot=""
    if [ -z "$shot" ]; then
        log "[VV] render unsupported -> seam-stub: a recorded/stub UNCERTAIN verdict exercises the seam (honest; never a fabricated verdict)."
        vline='VERDICT: uncertain | CONCERN: render stack absent (storybook test-runner / Playwright); seam recorded a stub verdict — no live render or model ran | BAND: measured-with-caveats | STAGE1: unsupported'
    else
        vline=$(do_judge "$cid" "$shot" "$target")
    fi
    log "[VV] two-stage result: $vline"
    record_verdict "$cid" "$vline"
}

# ── P3 — rendered-DOM a11y facts (axe-core, BUILT STATIC Storybook) ─────────

# ruleset_hash: stdin = newline-separated rule ids -> deterministic 16-hex id
# of the SORTED rule-id set (POSIX cksum: crc + length, each 8 hex). Same
# axe-core version + same active rules ⇒ same hash; ANY rule churn flips it —
# that is the staleness signal the per-fact provenance exists for.
ruleset_hash() {
    LC_ALL=C sort | cksum | awk '{printf "%08x%08x", $1, $2}'
}

# json_str_array <key> <file> -> the elements of a flat JSON string array,
# one per line (dependency-free; the runner contract keeps arrays flat).
json_str_array() {
    jsakey="$1"; jsafile="$2"
    tr -d '\n\r' < "$jsafile" \
        | sed -n "s/.*\"$jsakey\":[[:space:]]*\\[\\([^]]*\\)\\].*/\\1/p" \
        | tr ',' '\n' | sed 's/[[:space:]"]//g' | sed '/^$/d'
}

# a11y_judge <story_id> <axe_result_json> -> rendered-a11y verdict line:
#   VERDICT: <v> | CONCERN: <c> | BAND: measured-with-caveats | AXE: axe-core@<ver> | RULESET: <16-hex>
# exit 0 verdict printed · 3 DEFER (provenance undecidable -> the fact is
# OMITTED, never recorded without version+hash, never guessed).
a11y_judge() {
    ajsid="$1"; ajjson="$2"
    [ -f "$ajjson" ] || { log "[VV] a11y DEFER: axe result '$ajjson' not found."; return 3; }
    ajver=$(tr -d '\n\r' < "$ajjson" | sed -n 's/.*"axeVersion":[[:space:]]*"\([^"]*\)".*/\1/p')
    if [ -z "$ajver" ]; then
        log "[VV] a11y DEFER: axe result carries no axeVersion — provenance undecidable; rendered fact OMITTED (static a11y facts stand alone)."
        return 3
    fi
    ajrules=$(json_str_array ruleIds "$ajjson")
    if [ -z "$ajrules" ]; then
        log "[VV] a11y DEFER: axe result carries no ruleIds — ruleset hash undecidable; rendered fact OMITTED."
        return 3
    fi
    ajhash=$(printf '%s\n' "$ajrules" | ruleset_hash)
    ajviols=$(json_str_array violationIds "$ajjson")
    if [ -z "$ajviols" ]; then ajnv=0; else ajnv=$(printf '%s\n' "$ajviols" | sed -n '$='); fi
    if [ "$ajnv" -eq 0 ]; then
        printf 'VERDICT: affirm | CONCERN: axe-core@%s found 0 rendered violations for story %s (WCAG 2.2 AA baseline) | BAND: measured-with-caveats | AXE: axe-core@%s | RULESET: %s\n' \
            "$ajver" "$ajsid" "$ajver" "$ajhash"
    else
        ajlist=$(printf '%s' "$ajviols" | tr '\n' ',' | sed 's/,$//')
        printf 'VERDICT: refute | CONCERN: axe-core@%s flagged %s rendered a11y violation(s) for story %s: %s | BAND: measured-with-caveats | AXE: axe-core@%s | RULESET: %s\n' \
            "$ajver" "$ajnv" "$ajsid" "$ajlist" "$ajver" "$ajhash"
    fi
}

# run_axe <story_id> <out_json> -> 0 iff the runner produced a result file.
# The runner targets the BUILT STATIC Storybook via file:// (no live service).
run_axe() {
    rasid="$1"; raout="$2"
    mkdir -p "$(dirname "$raout")" 2>/dev/null || true
    rm -f "$raout" 2>/dev/null || true
    if [ -n "$AXE_RUNNER" ] && [ "$AXE_RUNNER" != "none" ]; then
        "$AXE_RUNNER" "$STORYBOOK_STATIC" "$rasid" "$raout" >/dev/null 2>&1 || true
    else
        node "$AXE_REF_RUNNER" "$STORYBOOK_STATIC" "$rasid" "$raout" >/dev/null 2>&1 || true
    fi
    [ -f "$raout" ]
}

# do_a11y <claim_id> <story_id> — rendered a11y facts -> record cross-family.
# Trust posture mirrors the static kinds: a rendered refute tightens; with the
# stack absent the STATIC a11y:labeled/role facts stand alone, UNCHANGED —
# rendered facts are additive evidence, never a precondition.
do_a11y() {
    cid="$1"; sid="$2"
    bin=$(engine_bin)
    if ! command -v "$bin" >/dev/null 2>&1 && [ ! -x "$bin" ]; then
        log "[VV] engine binary '$bin' not found; cannot record a verdict (set LODESTAR_BIN)."; return 2
    fi
    eff=$(effective_mode)
    if [ "$eff" = "off" ] || [ "$eff" = "absent" ]; then
        log "[VV] provider=$PROVIDER (effective=$eff): visual seam OFF. Engine byte-identical; static a11y facts stand alone."
        printf 'absent\n'; return 0
    fi
    if ! axe_ready; then
        log "[VV] rendered a11y UNSUPPORTED: need axe mode on, a BUILT STATIC Storybook at '$STORYBOOK_STATIC' (npx storybook build), and an axe runner (node + @axe-core/playwright, or set axe_runner)."
        log "[VV] static a11y:labeled / a11y:role facts stand alone, unchanged (rendered facts are additive, never a precondition)."
        printf 'unsupported\n'; return 3
    fi
    outj="$SHOT_DIR/$(printf '%s' "$sid" | tr '/ :' '___').axe.json"
    mkdir -p "$SHOT_DIR" 2>/dev/null || true
    if ! run_axe "$sid" "$outj"; then
        log "[VV] rendered a11y UNSUPPORTED: axe runner produced no result for story '$sid' (story missing from the static build?)."
        printf 'unsupported\n'; return 3
    fi
    vline=$(a11y_judge "$sid" "$outj") || { printf 'unsupported\n'; return 3; }
    log "[VV] rendered a11y result: $vline"
    record_verdict "$cid" "$vline"
}

self_test() {
    log "[VV] self-test: capability matrix dry-run (NO render, NO diff, NO model, NO engine write)."
    capabilities >/dev/null
    eff=$(effective_mode); os=$(host_os); rc=0
    case "$eff" in
        off)       log "[VV] provider OFF (default). Seam absent; engine byte-identical; gate stands. exit 0."; rc=0 ;;
        absent)    log "[VV] provider '$PROVIDER' unknown -> treated absent. exit 0 (gate stands)."; rc=0 ;;
        live)      log "[VV] LIVE: render + a verdict path present. Two-stage: deterministic diff first; VLM (comparative, conformal abstention) only on flagged deltas."; rc=0 ;;
        seam-stub) log "[VV] SEAM-STUB: provider on but a live tool absent -> seam exercised with a RECORDED/STUB verdict (honest, not a live capability). exit 3."; rc=3 ;;
    esac
    log "[VV] stage-1 pixel tool: '$(pixeldiff_tool)' ocr: '$(ocr_tool)' | conformal max width: $CONFORMAL_MAX_WIDTH"
    log "[VV] rendered a11y (axe-core 4.12.x, BUILT STATIC Storybook, WCAG 2.2 AA): $(axe_ready && echo ready || echo unsupported) — absent ⇒ static a11y facts stand alone."
    log "[VV] cross-family check: reviewer '$REVIEWER' -> family $(family_of "$REVIEWER") (engine enforces never-self)."
    log "[VV] open licenses: Storybook/test-runner MIT, Playwright Apache-2.0, axe-core MPL-2.0. NO Figma/commercial dep."
    log "[VV] vendor-neutral open-weight judges (recommended, never bundled): Qwen3-VL, GLM-4.6V, Molmo, UI-TARS/UGround."
    log "[VV] deterministic two-stage logic self-test: sh tools/visual/visual-verify-selftest.sh (recorded fixtures; no model)."
    return "$rc"
}

cmd="${1:-}"
rc=0
case "$cmd" in
    --self-test|self-test) self_test || rc=$? ;;
    capabilities|caps) capabilities || rc=$? ;;
    stage1) [ $# -ge 3 ] || die "usage: stage1 <screenshot> <target_asset>"
            stage1_diff "$2" "$3" || rc=$? ;;
    judge)  [ $# -ge 4 ] || die "usage: judge <claim_id> <screenshot> <target_asset>"
            do_judge "$2" "$3" "$4" || rc=$? ;;
    verify) [ $# -ge 4 ] || die "usage: verify <claim_id> <story_id> <target_asset>"
            do_verify "$2" "$3" "$4" || rc=$? ;;
    a11y-judge) [ $# -ge 3 ] || die "usage: a11y-judge <story_id> <axe_result_json>"
            a11y_judge "$2" "$3" || rc=$? ;;
    a11y)   [ $# -ge 3 ] || die "usage: a11y <claim_id> <story_id>"
            do_a11y "$2" "$3" || rc=$? ;;
    ""|-h|--help) sed -n '2,86p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try --self-test, capabilities, stage1, judge, verify, a11y-judge, a11y)" ;;
esac
exit "$rc"
