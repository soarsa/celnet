#!/usr/bin/env bash
# Celnet Sovereign Demonstration Launcher (September 2026)
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

ACT="${1:-all}"

echo "========================================================================"
echo "  CELNET DEMONSTRATION SUITE LAUNCHER (SEPTEMBER 2026)"
echo "  Target: Darwin aarch64 / Linux x86-64 Bare Metal"
echo "  Architecture: Cleanly separated from core project"
echo "========================================================================"

cd "${REPO_ROOT}"

if [ "${ACT}" == "--web" ] || [ "${ACT}" == "-w" ] || [ "${ACT}" == "web" ]; then
    echo "Opening SOTA Interactive Visual Studio in default browser..."
    TARGET_HTML="${REPO_ROOT}/docs/architecture/CELNET-INTERACTIVE-SOTA-VISUAL-DEMO.html"
    if command -v open >/dev/null 2>&1; then
        open "${TARGET_HTML}"
    elif command -v xdg-open >/dev/null 2>&1; then
        xdg-open "${TARGET_HTML}"
    else
        echo "Please open in browser: ${TARGET_HTML}"
    fi
    exit 0
fi

echo "Building and executing celnet-demo in release mode..."
if [ $# -eq 0 ]; then
    cargo run --release -p celnet-demo -- --act=all
else
    cargo run --release -p celnet-demo -- "$@"
fi
