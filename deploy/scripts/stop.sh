#!/usr/bin/env bash
# Gracefully stop celnet-server on a target (default: uat). Thin wrapper over celnetctl.
#   ./scripts/stop.sh [target]
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET="${1:-uat}"
exec ansible "$TARGET" -m command -a "/opt/celnet/shared/bin/celnetctl stop"
