#!/usr/bin/env bash
# Show celnet-server status on a target (default: uat).
#   ./scripts/status.sh [target]
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET="${1:-uat}"
exec ansible "$TARGET" -m command -a "/opt/celnet/shared/bin/celnetctl status"
