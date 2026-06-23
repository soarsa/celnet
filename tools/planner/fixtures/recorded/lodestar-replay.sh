#!/usr/bin/env sh
# lodestar-replay.sh — RECORDED engine replay for planner-selftest.sh (the
# tools/visual/fixtures/recorded discipline): real `lodestar cli` output
# shapes captured once and replayed byte-identically. NO engine binary, NO
# graph, NO network, NO write to any store.
#
# argv mirrors the engine CLI seam exactly:   <this> cli <tool> '<args-json>'
#   - stdout: the tool's inner JSON (what `lodestar cli` prints on success)
#   - stderr + exit 1: the error path (what the CLI does for a tool error)
#
# knowledge_put is the ONE write-shaped tool: the replay appends the raw args
# to $LODESTAR_REPLAY_PUT_LOG (when set) so the selftest can assert exactly
# what the planner authored, and answers with a draft-state receipt. Setting
# LODESTAR_REPLAY_PUT_STATE=active simulates a (forbidden) auto-activation so
# the selftest can prove the planner ABORTS instead of proceeding.
#
# The recorded estate (fx-options, the contract §1.5 vocabulary):
#   capture   submit_trade(seed) / build_payload / FxOptionTrade / submit_handler
#   pricing   post_v1_fx_price / price_fx_option / garman_kohlhagen /
#             publish_priced_trade / test_garman_kohlhagen / docs_helper
#   risk      on_priced_trade / on_priced_alert (the latter reachable ONLY via
#             the recorded CROSS_ASYNC_CALLS row — exercises the cross-merge)
#   settlement / frontend  present in the recordings but NOT reachable from
#             the seed — the reachable-only gate must exclude them.
#
# shellcheck shell=sh
set -u
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

[ "${1:-}" = "cli" ] || { printf 'error: replay expects: cli <tool> <json>\n' >&2; exit 1; }
tool="${2:-}"
args="${3:-}"

has() { case "$args" in *"$1"*) return 0 ;; *) return 1 ;; esac; }

case "$tool" in
    trace_path)
        if has '"function_name":"capture.client.submit_trade"'; then
            cat "$HERE/trace-cross-service.json"
        else
            printf '{"error":"function not found","hint":"Use search_graph to find the exact name."}\n' >&2
            exit 1
        fi
        ;;
    query_graph)
        if   has 'CROSS_HTTP_CALLS';     then cat "$HERE/cross-http.json"
        elif has 'CROSS_ASYNC_CALLS';    then cat "$HERE/cross-async.json"
        elif has 'CROSS_CHANNEL';        then cat "$HERE/cross-channel.json"
        elif has 'CROSS_CONSUMES_TOKEN'; then cat "$HERE/cross-token.json"
        elif has ':TESTS';               then cat "$HERE/tests-edges.json"
        elif has ':IMPORTS';             then cat "$HERE/imports-edges.json"
        else printf '{"columns":[],"rows":[],"total":0}\n'
        fi
        ;;
    detect_changes)
        cat "$HERE/detect-changes.json"
        ;;
    knowledge_put)
        n=1
        if [ -n "${LODESTAR_REPLAY_PUT_LOG:-}" ]; then
            printf '%s\n' "$args" >> "$LODESTAR_REPLAY_PUT_LOG"
            n=$(wc -l < "$LODESTAR_REPLAY_PUT_LOG" | tr -d '[:space:]')
        fi
        st="${LODESTAR_REPLAY_PUT_STATE:-draft}"
        printf '{"id":"kn-fixture-%s","state":"%s","gate":{"constraint":"","passed":false,"writes":[]},"anchors":[]}\n' "$n" "$st"
        ;;
    *)
        printf 'error: replay has no recording for tool %s\n' "$tool" >&2
        exit 1
        ;;
esac
