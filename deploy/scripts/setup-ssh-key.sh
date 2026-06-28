#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# setup-ssh-key.sh — one-time SSH key bootstrap for a Celnet host.
#
# Generates an ed25519 keypair (if absent) and installs the public key on the
# target's authorized_keys. The host password is prompted exactly ONCE here;
# after this, all Ansible/menu/start-stop operations use the key.
#
#   ./scripts/setup-ssh-key.sh [user@host]      (default: celnet@136.115.32.199)
#   CELNET_SSH_KEY=~/.ssh/other ./scripts/setup-ssh-key.sh celnet@1.2.3.4
# ---------------------------------------------------------------------------
set -euo pipefail
KEY="${CELNET_SSH_KEY:-$HOME/.ssh/celnet_uat}"
TARGET="${1:-celnet@136.115.32.199}"

if [[ ! -f "$KEY" ]]; then
  echo "Generating SSH key: $KEY"
  mkdir -p "$(dirname "$KEY")"
  ssh-keygen -t ed25519 -f "$KEY" -N "" -C "celnet-deploy"
else
  echo "Reusing existing key: $KEY"
fi

echo "Installing public key on $TARGET (you will be asked for the password once)..."
if command -v ssh-copy-id >/dev/null 2>&1; then
  ssh-copy-id -i "${KEY}.pub" "$TARGET"
else
  # Portable fallback when ssh-copy-id is unavailable.
  ssh "$TARGET" 'umask 077; mkdir -p ~/.ssh && cat >> ~/.ssh/authorized_keys' < "${KEY}.pub"
fi

echo "Verifying key-only login..."
ssh -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes "$TARGET" 'echo "key auth OK on $(hostname)"'
echo "Done. The deployment is now key-authenticated with $KEY"
