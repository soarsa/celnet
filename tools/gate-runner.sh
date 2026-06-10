#!/usr/bin/env bash
# tools/gate-runner.sh — resumable serial gate runner (the `just t1` / `just t2` engine).
#
# WHY (docs/PARALLEL-SESSIONS.md §4.2): long gates (~60-90 min) on this machine die at
# session-spend walls and restarts used to lose the whole run. This runner executes an
# ordered list of gate steps SERIALLY and journals each step's PASS/FAIL, REAL exit code,
# and the literal last output line to a gitignored JSONL ledger. On re-invocation it SKIPS
# any step already journaled PASS for the IDENTICAL tree (HEAD sha + a dirty-working-tree
# content hash) AND the identical command — so a killed gate resumes from the last green
# step instead of restarting. Any edit to any tracked/untracked (non-ignored) file changes
# the fingerprint and re-arms every step: a skip can never mask an unverified change.
#
# HARD LESSONS ENCODED:
#   * No pipe-masking: the step's exit code is taken from PIPESTATUS[0] across the
#     output tee and is propagated verbatim as this runner's exit code on failure.
#   * The literal output line (e.g. "All gates passed.") is journaled so the operator
#     can verify the actual gate evidence, not a wrapper's exit status.
#
# USAGE
#   tools/gate-runner.sh [--ledger FILE] [--fresh] <gate> <step>::<command> [<step>::<command> ...]
#   tools/gate-runner.sh status                 # fingerprint + ledger entries for this tree
#   tools/gate-runner.sh last-green <gate>      # HEAD sha of <gate>'s last full PASS (exit 1 if none)
#
#   <gate> and <step> names: [A-Za-z0-9._-]+ . <command> runs via `bash -c` from the repo
#   root and must be self-contained (e.g. `source "$HOME/.cargo/env" && cargo ...`).
#   --fresh ignores prior PASS entries (forces a full re-run; still journals).
#
# LEDGER (.gate-ledger.jsonl at the repo/worktree root; gitignored — MUST stay ignored so
# appends don't perturb the tree fingerprint; per-worktree, machine-local, append-only):
#   {"ts":"...","head":"<sha>","tree":"<sha>+<dirty16>","gate":"...","step":"...",
#    "cmd_fp":"<sha256/12 of command>","status":"PASS|FAIL","exit":N,"line":"...","cmd":"..."}
# A full-gate success appends a summary entry with step="__gate__" (used by `last-green`,
# which `just t1` uses to derive "crates changed since the last green T1").
set -euo pipefail

REPO_ROOT=$(git rev-parse --show-toplevel)
LEDGER="${GATE_LEDGER:-$REPO_ROOT/.gate-ledger.jsonl}"
FRESH=0
GR_TMP=""
trap '[ -z "$GR_TMP" ] || rm -f "$GR_TMP"' EXIT

usage() {
    sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
}

validate_name() {
    case $1 in
        '' | *[!A-Za-z0-9._-]*)
            echo "gate-runner: invalid gate/step name '$1' (allowed: A-Za-z0-9._-)" >&2
            exit 2
            ;;
    esac
}

json_escape() {
    local s=$1
    s=${s//\\/\\\\}
    s=${s//\"/\\\"}
    s=${s//$'\t'/\\t}
    printf '%s' "$s"
}

# Tree fingerprint = HEAD sha + sha256 of (tracked diff vs HEAD ++ every untracked
# non-ignored file's path+blob hash). Gitignored files (target/, the ledger itself,
# lcov.info, ...) are excluded, so build artifacts never invalidate a green step.
tree_fingerprint() {
    local head dirty
    head=$(git -C "$REPO_ROOT" rev-parse HEAD)
    dirty=$(
        {
            git -C "$REPO_ROOT" diff HEAD --
            git -C "$REPO_ROOT" ls-files --others --exclude-standard | LC_ALL=C sort |
                while IFS= read -r f; do
                    printf 'untracked %s ' "$f"
                    git -C "$REPO_ROOT" hash-object -- "$f" 2>/dev/null || echo vanished
                done
        } | shasum -a 256 | cut -d' ' -f1
    )
    printf '%s+%s' "$head" "${dirty:0:16}"
}

append_entry() {
    local head=$1 tree=$2 gate=$3 step=$4 fp=$5 status=$6 rc=$7 line=$8 cmd=$9 ts
    ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    printf '{"ts":"%s","head":"%s","tree":"%s","gate":"%s","step":"%s","cmd_fp":"%s","status":"%s","exit":%d,"line":"%s","cmd":"%s"}\n' \
        "$ts" "$head" "$tree" "$gate" "$step" "$fp" "$status" "$rc" \
        "$(json_escape "$line")" "$(json_escape "$cmd")" >>"$LEDGER"
}

cmd_status() {
    local tree
    tree=$(tree_fingerprint)
    echo "tree:   $tree"
    echo "ledger: $LEDGER"
    if [ -f "$LEDGER" ]; then
        grep -F "\"tree\":\"$tree\"" "$LEDGER" || echo "(no entries for this tree)"
    else
        echo "(no ledger yet)"
    fi
}

cmd_last_green() {
    local gate=$1 entry sha
    validate_name "$gate"
    [ -f "$LEDGER" ] || return 1
    entry=$(grep -F "\"gate\":\"$gate\",\"step\":\"__gate__\",\"cmd_fp\":\"-\",\"status\":\"PASS\"" "$LEDGER" | tail -n 1 || true)
    [ -n "$entry" ] || return 1
    sha=$(printf '%s\n' "$entry" | sed -n 's/.*"head":"\([0-9a-f]\{40\}\)".*/\1/p')
    [ -n "$sha" ] || return 1
    printf '%s\n' "$sha"
}

run_gate() {
    local gate=$1
    shift
    validate_name "$gate"
    local head tree total=$# idx=0
    head=$(git -C "$REPO_ROOT" rev-parse HEAD)
    tree=$(tree_fingerprint)
    echo "gate-runner: gate=$gate steps=$total tree=$tree"
    echo "gate-runner: ledger=$LEDGER (resume: re-run the same command after a kill/fix)"

    local spec name cmd fp key prev rc line
    for spec in "$@"; do
        idx=$((idx + 1))
        name=${spec%%::*}
        cmd=${spec#*::}
        if [ "$name" = "$spec" ] || [ -z "$name" ] || [ -z "$cmd" ]; then
            echo "gate-runner: bad step spec (want NAME::COMMAND): $spec" >&2
            exit 2
        fi
        validate_name "$name"
        fp=$(printf '%s' "$cmd" | shasum -a 256 | cut -c1-12)
        key="\"tree\":\"$tree\",\"gate\":\"$gate\",\"step\":\"$name\",\"cmd_fp\":\"$fp\",\"status\":\"PASS\""

        if [ "$FRESH" -eq 0 ] && [ -f "$LEDGER" ] && grep -qF "$key" "$LEDGER"; then
            prev=$(grep -F "$key" "$LEDGER" | tail -n 1 | sed -n 's/.*"line":"\(.*\)","cmd":".*/\1/p')
            echo "[$idx/$total] SKIP $gate/$name — PASS journaled for this exact tree+command (${prev:-no line})"
            continue
        fi

        echo "[$idx/$total] RUN  $gate/$name :: $cmd"
        GR_TMP=$(mktemp "${TMPDIR:-/tmp}/gate-runner.XXXXXX")
        set +e
        (cd "$REPO_ROOT" && bash -c "$cmd") 2>&1 | tee "$GR_TMP"
        rc=${PIPESTATUS[0]} # REAL exit code of the step, never the tee's
        set -e
        line=$(awk 'NF { l = $0 } END { print l }' "$GR_TMP" | tr -d '\000-\010\013\014\016-\037' | cut -c1-400)
        rm -f "$GR_TMP"
        GR_TMP=""

        if [ "$rc" -ne 0 ]; then
            append_entry "$head" "$tree" "$gate" "$name" "$fp" FAIL "$rc" "$line" "$cmd"
            echo "[$idx/$total] FAIL $gate/$name — exit=$rc (real, unmasked) — $line" >&2
            echo "gate-runner: fix, then re-run the same gate — the green steps above resume as SKIP." >&2
            exit "$rc"
        fi
        append_entry "$head" "$tree" "$gate" "$name" "$fp" PASS 0 "$line" "$cmd"
        echo "[$idx/$total] PASS $gate/$name — exit=0 — $line"
    done

    append_entry "$head" "$tree" "$gate" "__gate__" "-" PASS 0 "all $total steps green" "-"
    echo "gate-runner: GATE PASS — $gate ($total/$total steps green for tree $tree)"
}

while [ $# -gt 0 ]; do
    case $1 in
        --ledger)
            [ $# -ge 2 ] || usage
            LEDGER=$2
            shift 2
            ;;
        --fresh)
            FRESH=1
            shift
            ;;
        -h | --help) usage ;;
        status)
            cmd_status
            exit 0
            ;;
        last-green)
            [ $# -ge 2 ] || usage
            if cmd_last_green "$2"; then exit 0; else exit 1; fi
            ;;
        *) break ;;
    esac
done

[ $# -ge 2 ] || usage
run_gate "$@"
