#!/usr/bin/env sh
# lib.sh — shared helpers for the lodestar host-side REQUIREMENTS BRIDGE (R5).
#
# The bridge lives ENTIRELY outside the engine (the tools/visual / tools/judge
# posture): the pure-C engine links no tracker code, makes no network call, and
# is byte-identical whether or not this directory exists. Everything here talks
# to the engine ONLY through the public MCP/CLI seam (`lodestar cli <tool>`),
# and to the tracker ONLY through a vendor-neutral adapter contract
# (adapters/*.sh). DEFAULT-OFF: with no config the tracker is `off`, both
# drivers print 'absent' and write nothing.
#
# ADAPTER CONTRACT (vendor-neutral; jira/linear/plane/github/stub all speak it):
#   adapter capabilities
#       -> one JSON line {"tracker":"<name>","ready":true|false,...}; exit 0.
#   adapter fetch-epic <epic_id>
#       -> the CANONICAL REQUIREMENT STREAM on stdout (line-oriented so POSIX
#          sh can parse it without jq):
#            line 1 : {"epic":"<id>","deliverable":"<slug>","title":"<text>"}
#            line 2+: {"ticket":"<id>","title":"<t>","text":"<requirement>",
#                      "anchors":"<qn> <qn> ...","target":"<repo-rel path or empty>"}
#          exit 0 ok · 1 error · 4 no API key · 5 no curl.
#   adapter get-status <ticket>
#       -> {"ticket":"<id>","status":"<status>"}
#   adapter set-status <ticket> <status> <receipt_id> [feed_file]
#       -> {"ticket":"<id>","status":"<status>","ok":true,"receipt":"<id>"}
#          MUST refuse status=done with an EMPTY receipt_id (exit 1): a
#          ticket's "Done" is a projection of the verified roll-up and needs a
#          roll-up receipt — "claimed verified but no receipt" must stay a
#          detectable absence (R5-a). feed_file, when given, is the
#          machine-readable knowledge_export feed the adapter may attach.
#
# shellcheck shell=sh

REQ_HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$REQ_HERE/../lib/common.sh"

# Allow a dedicated config file; it feeds the same single config surface
# common.sh reads (the visual-driver precedent).
if [ -n "${LODESTAR_REQUIREMENTS_CONFIG:-}" ] && [ -f "${LODESTAR_REQUIREMENTS_CONFIG}" ]; then
    LODESTAR_JUDGE_CONFIG="${LODESTAR_REQUIREMENTS_CONFIG}"; export LODESTAR_JUDGE_CONFIG
    cfg_locate
fi

# ── config (env > lodestar.judge.toml [requirements] > default) ──────────────
REQ_TRACKER=$(cfg LODESTAR_REQ_TRACKER requirements tracker "off")
REQ_ADAPTER_CMD=$(cfg LODESTAR_REQ_ADAPTER_CMD requirements adapter_cmd "")
# R5-b routing: "" -> imported claims stay draft testimony and the bridge prints
# the exact knowledge_review command a cross-family reviewer runs; a command ->
# it is invoked per authored claim id and is expected to drive/record a
# CROSS-FAMILY review through the engine seam (e.g. "sh tools/judge/judge-ollama.sh judge").
REQ_REVIEW_CMD=$(cfg LODESTAR_REQ_REVIEW_CMD requirements review_cmd "")
# VeriTrans round-trip confidence — DRAFT-ONLY AUTHORING HINT (§6.4 FLAG).
# The command receives <requirement_text> [target_path] and prints one line
#   CONFIDENCE: <0..1> [| NOTE: <text>]
# Its output is stored ONLY as the draft claim's free-text confidence
# annotation to help a human prioritize review. IT NEVER GATES: the bridge has
# no threshold branch, never skips/accepts/rejects a requirement on it, and the
# deterministic gate never sees it. The coverage-threshold acceptance use is
# PERMANENTLY REJECTED (plan §5, §6.4) — do not add one.
REQ_CONFIDENCE_CMD=$(cfg LODESTAR_REQ_CONFIDENCE_CMD requirements confidence_cmd "")
# The bridge's authoring identity. Claims it authors are TESTIMONY (R5-a):
# imported text is the tracker's word, not verified knowledge. Keeping the
# author the bridge (not a human) keeps provenance honest and never-self
# review meaningful.
REQ_AUTHOR=$(cfg LODESTAR_REQ_AUTHOR requirements author "requirements-bridge")
REQ_TOOL_VERSION=$(cfg LODESTAR_REQ_TOOL_VERSION requirements tool_version "requirements-bridge@0.5.0")
# Where the machine-readable status feeds (knowledge_export SARIF/conformance)
# are written before being handed to the adapter.
REQ_OUT_DIR=$(cfg LODESTAR_REQ_OUT_DIR requirements out_dir ".lodestar/requirements")

# ── engine seam (CLI; never linked) ──────────────────────────────────────────
# `lodestar cli <tool> '<json>'` prints the tool's TEXT payload on stdout and
# exits 1 with the message on stderr when the tool errored — exactly what a
# shell driver needs. No --json: we want the unescaped payload + the exit code.
engine_tool() {
    et_tool="$1"; et_json="$2"
    "$(engine_bin)" cli "$et_tool" "$et_json"
}

engine_present() {
    eb=$(engine_bin)
    command -v "$eb" >/dev/null 2>&1 || [ -x "$eb" ]
}

# native_path <path> -> the path as the (possibly native-Windows) engine binary
# must see it. Under Git-Bash/MSYS a /tmp/... path is meaningless to a native
# exe; cygpath -m renders C:/... which the engine normalizes itself.
native_path() {
    if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}

# ── adapter resolution + invocation ──────────────────────────────────────────
# resolve_adapter -> sets REQ_ADAPTER (a script path or a command token).
# Precedence: explicit adapter_cmd > adapters/<tracker>.sh. tracker=off never
# resolves (callers check first).
resolve_adapter() {
    if [ -n "$REQ_ADAPTER_CMD" ]; then
        REQ_ADAPTER="$REQ_ADAPTER_CMD"
        return 0
    fi
    REQ_ADAPTER="$REQ_HERE/adapters/$REQ_TRACKER.sh"
    [ -f "$REQ_ADAPTER" ] || return 1
    return 0
}

run_adapter() {
    if [ -f "$REQ_ADAPTER" ]; then
        sh "$REQ_ADAPTER" "$@"
    else
        # adapter_cmd is a single command token (no embedded spaces), the same
        # contract the judge/visual *_cmd knobs use.
        "$REQ_ADAPTER" "$@"
    fi
}

# ── canonical receipt JSON (R5-a, §5.3 of the lane contract) ─────────────────
# build_receipt_json <deliverable> <direction> <epistemic_source> <rollup_state>
#                    <target_hash> <ticket>
# -> ONE canonical line: keys in SORTED order, fixed shape, no wall clock —
# byte-identical for identical inputs (the event log's dedup/idempotency rule).
build_receipt_json() {
    br_deliv=$(json_escape "$1"); br_dir=$(json_escape "$2")
    br_src=$(json_escape "$3");   br_roll=$(json_escape "$4")
    br_hash=$(json_escape "$5");  br_ticket=$(json_escape "$6")
    br_tool=$(json_escape "$REQ_TOOL_VERSION"); br_tracker=$(json_escape "$REQ_TRACKER")
    printf '{"deliverable":"%s","direction":"%s","epistemic_source":"%s","receipt":"tracker-roundtrip","rollup_state":"%s","target_hash":"%s","ticket":"%s","tool_version":"%s","tracker":"%s"}' \
        "$br_deliv" "$br_dir" "$br_src" "$br_roll" "$br_hash" "$br_ticket" "$br_tool" "$br_tracker"
}

# events_dir <repo_root> -> where the engine's append-only event log lives
# (lode_kn_event_dir, event_log.c:439-448). The bridge only READS this to
# verify a receipt landed; it never writes here itself.
events_dir() { printf '%s/.lodestar/knowledge/events' "$1"; }

# ── canonical-stream field access ────────────────────────────────────────────
# line_get <key> <json_line> -> the string value of a top-level key on ONE flat
# JSON line ("" when absent). Rides common.sh json_get.
line_get() { printf '%s' "$2" | json_get "$1"; }
