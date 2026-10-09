#!/usr/bin/env sh
# common.sh — shared helpers for the lodestar host-side judge / execute-verify
# drivers. POSIX sh (dash/ash/bash/zsh; on Windows use Git-Bash or WSL).
#
# These drivers live OUTSIDE the engine. The engine stays pure-C, model-free,
# network-free. This library:
#   - loads config with precedence  env > lodestar.judge.toml > built-in default
#   - shells out to the engine seam (`lodestar cli knowledge_review ... --json`)
#   - detects host resources (RAM/cores/OS) for tier sizing
#   - maps a model id to a coarse family for the never-self pre-check
#
# It deliberately uses only POSIX utilities (sh, grep, sed, tr, awk, uname) so it
# runs on Linux, macOS, and Windows-Git-Bash without extra installs.
#
# shellcheck shell=sh

# ---- tiny logging (stderr; stdout is reserved for machine-readable output) ----
log()  { printf '%s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }
warn() { printf 'warning: %s\n' "$*" >&2; }

# ---- config loading ---------------------------------------------------------
# cfg_file_value <section> <key>  -> prints the value from the TOML file, or ""
# Minimal TOML reader: section headers + `key = "value"` / `key = value`.
# Good enough for this flat config; not a general TOML parser.
_LODESTAR_CFG=""
cfg_locate() {
    if [ -n "${LODESTAR_JUDGE_CONFIG:-}" ] && [ -f "${LODESTAR_JUDGE_CONFIG}" ]; then
        _LODESTAR_CFG="${LODESTAR_JUDGE_CONFIG}"
    elif [ -f "./lodestar.judge.toml" ]; then
        _LODESTAR_CFG="./lodestar.judge.toml"
    else
        _LODESTAR_CFG=""
    fi
}

cfg_file_value() {
    section="$1"; key="$2"
    [ -n "$_LODESTAR_CFG" ] || return 0
    awk -v sect="$section" -v k="$key" '
        /^[[:space:]]*#/ { next }
        /^[[:space:]]*\[/ {
            gsub(/[][[:space:]]/, ""); cur=$0; next
        }
        {
            line=$0
            sub(/#.*/, "", line)
            n=index(line, "=")
            if (n==0) next
            name=substr(line,1,n-1); val=substr(line,n+1)
            gsub(/[[:space:]]/, "", name)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", val)
            gsub(/^"|"$/, "", val)
            if (cur==sect && name==k) { print val; exit }
        }
    ' "$_LODESTAR_CFG"
}

# cfg <ENV_VAR> <section> <key> <default>  -> resolved value (env > file > default)
cfg() {
    envvar="$1"; section="$2"; key="$3"; def="$4"
    eval "v=\${$envvar:-}"
    if [ -n "$v" ]; then printf '%s' "$v"; return 0; fi
    fv=$(cfg_file_value "$section" "$key")
    if [ -n "$fv" ]; then printf '%s' "$fv"; return 0; fi
    printf '%s' "$def"
}

# ---- engine seam ------------------------------------------------------------
engine_bin() { cfg LODESTAR_BIN engine bin "lodestar"; }
engine_project() { cfg LODESTAR_PROJECT engine project "D-code-lodestar"; }

# engine_review_read <claim_id>  -> JSON: policy + rubric + evidence + author
# This is the READ-mode of the knowledge_review seam. lodestar builds this by graph
# projection — no model is invoked to produce it.
engine_review_read() {
    cid="$1"; proj=$(engine_project)
    "$(engine_bin)" cli knowledge_review \
        "{\"project\":\"$proj\",\"claim_id\":\"$cid\"}"
}

# engine_review_record <claim_id> <reviewer_model> <verdict> <concern> <digest>
# RECORD-mode: hand the engine the verdict. The engine enforces never-self,
# records, and adjudicates. lodestar calls no model here either.
engine_review_record() {
    cid="$1"; rm="$2"; verdict="$3"; concern="$4"; digest="$5"
    proj=$(engine_project)
    concern=$(json_escape "$concern"); digest=$(json_escape "$digest")
    rm=$(json_escape "$rm")
    "$(engine_bin)" cli knowledge_review \
        "{\"project\":\"$proj\",\"claim_id\":\"$cid\",\"reviewer_model\":\"$rm\",\"verdict\":\"$verdict\",\"concern\":\"$concern\",\"evidence_digest\":\"$digest\"}"
}

# json_escape <string>  -> minimally JSON-escaped (quotes, backslash, newlines)
json_escape() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g' | awk 'BEGIN{ORS=""} {if(NR>1)printf "\\n"; printf "%s",$0}'
}

# json_get <key>  reads stdin JSON, prints the string/number value for a top-level key.
# Deliberately dependency-free (no jq). Handles the flat shapes these drivers need.
json_get() {
    key="$1"
    sed -n "s/.*\"$key\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p; s/.*\"$key\"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p" | head -n1
}

# ---- host resource detection (RAM / cores / OS) -----------------------------
# The engine knows RAM/cores too, but a host driver must size the model itself
# (VRAM is the host's job per the architecture). These are portable best-effort.
host_os() {
    case "$(uname -s 2>/dev/null)" in
        Linux*)  printf 'linux' ;;
        Darwin*) printf 'macos' ;;
        MINGW*|MSYS*|CYGWIN*) printf 'windows' ;;
        *)       printf 'unknown' ;;
    esac
}

host_ram_gb() {
    os=$(host_os)
    case "$os" in
        linux)
            if [ -r /proc/meminfo ]; then
                awk '/MemTotal/ {printf "%d", $2/1024/1024}' /proc/meminfo
            else printf '0'; fi ;;
        macos)
            b=$(sysctl -n hw.memsize 2>/dev/null || echo 0)
            awk -v b="$b" 'BEGIN{printf "%d", b/1024/1024/1024}' ;;
        windows)
            # Git-Bash: ask WMIC/PowerShell; fall back to 0 if neither.
            kb=$(powershell.exe -NoProfile -Command \
                '(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory' 2>/dev/null | tr -d '\r')
            [ -n "$kb" ] && awk -v b="$kb" 'BEGIN{printf "%d", b/1024/1024/1024}' || printf '0' ;;
        *) printf '0' ;;
    esac
}

host_cores() {
    if command -v nproc >/dev/null 2>&1; then nproc; return; fi
    case "$(host_os)" in
        macos) sysctl -n hw.ncpu 2>/dev/null || echo 1 ;;
        windows) printf '%s' "${NUMBER_OF_PROCESSORS:-1}" ;;
        *) echo 1 ;;
    esac
}

# host_vram_gb  -> best-effort discrete-GPU VRAM in GB (0 if none/unknown).
# NVIDIA via nvidia-smi is the portable signal; everything else returns 0 and the
# caller falls back to RAM-based sizing.
host_vram_gb() {
    if command -v nvidia-smi >/dev/null 2>&1; then
        mb=$(nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits 2>/dev/null | head -n1 | tr -d ' ')
        [ -n "$mb" ] && awk -v m="$mb" 'BEGIN{printf "%d", m/1024}' || printf '0'
    else
        printf '0'
    fi
}

# resolve_tier  -> small | medium | large, from detected RAM/VRAM (or a pin).
# Mirrors judge-providers.md sizing: VRAM (discrete GPU, off the dev RAM) wins;
# otherwise size from system RAM headroom.
resolve_tier() {
    pin=$(cfg LODESTAR_JUDGE_TIER judge tier "auto")
    if [ "$pin" != "auto" ]; then printf '%s' "$pin"; return; fi
    vram=$(host_vram_gb); ram=$(host_ram_gb)
    if [ "$vram" -ge 8 ] 2>/dev/null; then printf 'medium'; return; fi
    if [ "$ram" -ge 32 ] 2>/dev/null; then printf 'medium'; return; fi
    if [ "$ram" -ge 16 ] 2>/dev/null; then printf 'small'; return; fi
    # Below 16 GB: still small, but the caller should warn about headroom.
    printf 'small'
}

# ---- family mapping (never-self pre-check, mirrors src/knowledge/review.c) ---
# Coarse family from a model id. The engine seam is the authority and will reject
# a same-family verdict; this pre-check lets a driver fail fast with a clear msg.
family_of() {
    id=$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')
    case "$id" in
        *agent*|*anthropic*) printf 'anthropic' ;;
        *gpt*|*openai*|*o1*|*o3*) printf 'openai' ;;
        *gemini*|*google*) printf 'google' ;;
        *llama*|*meta*) printf 'meta' ;;
        *mistral*|*mixtral*) printf 'mistral' ;;
        *ollama*|*local*) printf 'local' ;;
        *) printf 'unknown' ;;
    esac
}

cfg_locate
