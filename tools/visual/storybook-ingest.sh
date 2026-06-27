#!/usr/bin/env sh
# storybook-ingest.sh — host-side ingest of a built Storybook index.json into
# component-anchored design knowledge. SYMBOLIC, DETERMINISTIC, NO MODEL.
#
# WHAT IT IS: the optional Storybook-manifest ingest step from the visual-
# alignment design (docs/design/visual-alignment-neurosymbolic.md §7.2 item 3).
# Storybook (MIT) renders each component in ISOLATION as named "stories" and
# emits a generated index.json — schema "v":3, a flat `stories` map of
# story-id -> {type,title,name,importPath,componentPath,tags}. That manifest is
# PURE SYMBOLIC DATA: components, stories, the render-state each story names. We
# parse it (no model, no commercial dep — Storybook is open-source, and this
# reads its OWN generated artifact) and:
#   (a) map each story's component to the REAL component qualified-name Lodestar
#       already holds in the graph, and
#   (b) anchor the deterministic design kinds (design:token / ui:component /
#       a11y:*) to that qn as DRAFT claims for the constraint gate to verify,
#       carrying the story id as the named render-state.
#
# It invokes NO model and writes NO claim the engine has not validated: every
# anchor goes through `knowledge_put`, which REJECTS an unresolved qualified-name
# (we then DEFER that story, never guess a node). The engine stays pure-C.
#
# WHY OPTIONAL: Storybook is OPTIONAL input here, not a runtime dep of the gate.
# Absent manifest ⇒ this step is a clean no-op; the design constraint kinds still
# anchor by hand and the deterministic gate still runs.
#
# USAGE:
#   storybook-ingest.sh --self-test                  # dry-run; no engine writes
#   storybook-ingest.sh list   [index.json]          # parse manifest -> stories (stdout JSON lines)
#   storybook-ingest.sh anchor [index.json] [kinds]  # put DRAFT design claims per story-component
#       kinds default: "design:token,a11y:labeled"   (comma list; ui:component:<tag> allowed)
#
# DEGRADES WHEN ABSENT: no index.json -> 'no manifest' note, exit 3, nothing
# written (the gate stands on hand-anchored claims). Engine binary missing ->
# exit 2. An unresolvable component qn -> that story DEFERS (warned, skipped),
# the rest proceed. NEVER a fabricated claim, NEVER a guessed anchor.
#
# shellcheck shell=sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$HERE/../lib/common.sh"

# Allow a visual-specific config override that still reuses common.sh's reader.
if [ -n "${LODESTAR_VISUAL_CONFIG:-}" ] && [ -f "${LODESTAR_VISUAL_CONFIG}" ]; then
    LODESTAR_JUDGE_CONFIG="${LODESTAR_VISUAL_CONFIG}"; export LODESTAR_JUDGE_CONFIG
    cfg_locate
fi

INDEX_DEFAULT=$(cfg LODESTAR_STORYBOOK_INDEX storybook index_json "storybook-static/index.json")

# resolve_index [arg] -> the manifest path to use (arg > config default)
resolve_index() { [ -n "${1:-}" ] && printf '%s' "$1" || printf '%s' "$INDEX_DEFAULT"; }

# component_qn_for <importPath|componentPath> -> a best-effort Lodestar qualified
# name. Storybook stories carry an importPath like "./src/ui/Button.tsx" and a
# title like "UI/Button". We DO NOT fabricate a qn: we hand the engine the path-
# derived candidate and let `knowledge_put` resolve it against the live graph.
# Absent/ambiguous -> empty (the caller DEFERS the story).
component_qn_for() {
    ip="$1"
    # strip leading ./ and a .tsx/.jsx/.ts/.js extension; keep the path as the
    # candidate the engine maps to a node qn (it owns qn resolution, not us).
    printf '%s' "$ip" | sed 's#^\./##; s#\.\(tsx\|jsx\|ts\|js\)$##'
}

# parse_stories <index.json> -> emit one JSON line per story:
#   {"id":..,"title":..,"name":..,"importPath":..,"component_candidate":..}
# Dependency-free (no jq): a small awk state machine over the flat "stories" map.
parse_stories() {
    idx="$1"
    [ -f "$idx" ] || return 1
    # The manifest is JSON; we walk the `stories` object members. Each member is
    # an object with string fields we need (id,title,name,importPath). We do not
    # need a full JSON parser for the flat v:3 shape — extract per-story records.
    awk '
        # collapse to one token stream: split on } to get per-story object chunks
        { buf = buf $0 }
        END {
            # isolate the stories map body
            s = index(buf, "\"stories\"")
            if (s == 0) { exit 3 }
            rest = substr(buf, s)
            n = split(rest, parts, "}")
            for (i = 1; i <= n; i++) {
                chunk = parts[i]
                if (chunk !~ /"importPath"/) continue
                id = field(chunk, "id")
                title = field(chunk, "title")
                name = field(chunk, "name")
                ip = field(chunk, "importPath")
                if (id == "" || ip == "") continue
                printf "{\"id\":\"%s\",\"title\":\"%s\",\"name\":\"%s\",\"importPath\":\"%s\"}\n", id, title, name, ip
            }
        }
        function field(c, key,   re, m, v) {
            re = "\"" key "\"[[:space:]]*:[[:space:]]*\""
            if (match(c, re)) {
                v = substr(c, RSTART + RLENGTH)
                sub(/".*/, "", v)
                return v
            }
            return ""
        }
    ' "$idx"
}

do_list() {
    idx=$(resolve_index "${1:-}")
    if [ ! -f "$idx" ]; then
        log "[VIZ] no Storybook manifest at '$idx' — ingest is a no-op (optional input)."
        log "[VIZ]   build one with: npx storybook build   (writes storybook-static/index.json)"
        return 3
    fi
    log "[VIZ] parsing Storybook manifest: $idx"
    out=$(parse_stories "$idx") || die "manifest has no \"stories\" map (schema v:3 expected): $idx"
    [ -n "$out" ] || { log "[VIZ] manifest parsed but contains zero stories with importPath."; return 0; }
    printf '%s\n' "$out"
}

do_anchor() {
    idx=$(resolve_index "${1:-}")
    kinds="${2:-design:token,a11y:labeled}"
    bin=$(engine_bin)
    if ! command -v "$bin" >/dev/null 2>&1 && [ ! -x "$bin" ]; then
        log "[VIZ] engine binary '$bin' not found; cannot anchor (set LODESTAR_BIN)."
        return 2
    fi
    if [ ! -f "$idx" ]; then
        log "[VIZ] no manifest at '$idx'; nothing to anchor (no-op). Stage-1 gate stands."
        return 3
    fi
    proj=$(engine_project)
    stories=$(parse_stories "$idx") || die "could not parse stories from $idx"
    [ -n "$stories" ] || { log "[VIZ] no stories to anchor."; return 0; }

    # Tally across the pipe subshell via a temp file (POSIX-safe; a `while` in a
    # pipe runs in a subshell, so plain shell vars would not survive the loop).
    tally="$HERE/.viz_tally"; printf '0 0\n' > "$tally"
    printf '%s\n' "$stories" | while IFS= read -r line; do
        [ -n "$line" ] || continue
        ip=$(printf '%s' "$line" | json_get importPath)
        sid=$(printf '%s' "$line" | json_get id)
        title=$(printf '%s' "$line" | json_get title)
        cand=$(component_qn_for "$ip")
        read -r a d < "$tally"
        if [ -z "$cand" ]; then
            warn "[VIZ] story '$sid': no component path -> DEFER (not guessed)."
            printf '%s %s\n' "$a" "$((d + 1))" > "$tally"; continue
        fi
        # Anchor each requested design kind as a DRAFT claim. The engine resolves
        # the qn against the live graph and REJECTS an unresolved anchor — we
        # treat that rejection as a DEFER, never an invented node.
        oldifs=$IFS; IFS=','
        for kind in $kinds; do
            IFS=$oldifs
            constraint=$(printf '%s' "$kind" | sed 's/^[^:]*://')
            text="Design adherence ($kind) for Storybook story '$sid' (render-state '$title'); verified by the deterministic design gate."
            req=$(printf '{"project":"%s","kind":"%s","constraint":"%s","text":"%s","confidence":"medium","anchors":[{"qualified_name":"%s"}],"story":"%s"}' \
                "$proj" "${kind%%:*}" "$constraint" "$(json_escape "$text")" "$cand" "$sid")
            resp=$("$bin" cli knowledge_put "$req" --json 2>/dev/null || true)
            read -r a d < "$tally"
            case "$resp" in
                *unresolved_anchor*)
                    warn "[VIZ] story '$sid' -> qn candidate '$cand' did not resolve -> DEFER."
                    printf '%s %s\n' "$a" "$((d + 1))" > "$tally" ;;
                *'"id"'*)
                    log "[VIZ] anchored $kind -> $cand  (story=$sid)"
                    printf '%s %s\n' "$((a + 1))" "$d" > "$tally" ;;
                *)
                    warn "[VIZ] story '$sid' kind '$kind': unexpected engine response -> DEFER: $resp"
                    printf '%s %s\n' "$a" "$((d + 1))" > "$tally" ;;
            esac
            IFS=','
        done
        IFS=$oldifs
    done
    read -r anchored deferred < "$tally"; rm -f "$tally"
    log "[VIZ] ingest complete: $anchored anchored, $deferred deferred (deferred = skipped, never guessed)."
    return 0
}

self_test() {
    log "[VIZ] self-test: Storybook ingest dry-run (NO model, NO engine writes)."
    fix="$HERE/fixtures/index.json"
    if [ -f "$fix" ]; then
        log "[VIZ] parsing bundled fixture manifest: $fix"
        n=$(parse_stories "$fix" | grep -c . || true)
        log "[VIZ] fixture stories parsed: $n (expected > 0)"
        [ "$n" -gt 0 ] || { log "[VIZ] FIXTURE PARSE FAILED"; return 1; }
    else
        log "[VIZ] no bundled fixture (fixtures/index.json) — parser smoke skipped."
    fi
    idx=$(resolve_index "")
    if [ -f "$idx" ]; then
        log "[VIZ] configured manifest present: $idx (ingest would anchor against the live graph)."
    else
        log "[VIZ] configured manifest '$idx' ABSENT -> ingest degrades to a clean no-op (exit 3)."
    fi
    log "[VIZ] Storybook is OPTIONAL symbolic input; the deterministic design gate never depends on it."
    return 0
}

cmd="${1:-}"
rc=0
case "$cmd" in
    --self-test|self-test) self_test || rc=$? ;;
    list)   do_list "${2:-}" || rc=$? ;;
    anchor) do_anchor "${2:-}" "${3:-}" || rc=$? ;;
    ""|-h|--help) sed -n '2,42p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) die "unknown command '$cmd' (try --self-test, list, anchor)" ;;
esac
exit "$rc"
