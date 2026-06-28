#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# vault-init.sh — create or edit the encrypted sudo-password vault for an env.
#
#   ./scripts/vault-init.sh [env]        (default env: uat)
#
# Stores the become (sudo) password as `ansible_become_password` in
# group_vars/<env>/vault.yml, encrypted with Ansible Vault. You choose the
# VAULT password (the passphrase that unlocks the file) when prompted; record
# it in a password manager, or drop it in deploy/.vault_pass (gitignored) so
# the menu/playbooks can decrypt non-interactively.
# ---------------------------------------------------------------------------
set -euo pipefail
cd "$(dirname "$0")/.."
ENV_NAME="${1:-uat}"
VAULT="group_vars/${ENV_NAME}/vault.yml"
mkdir -p "group_vars/${ENV_NAME}"

PW_ARGS=()
[[ -f .vault_pass ]] && PW_ARGS=(--vault-password-file .vault_pass)

if [[ -f "$VAULT" ]]; then
  echo "Editing existing vault: $VAULT"
  exec ansible-vault edit "${PW_ARGS[@]+"${PW_ARGS[@]}"}" "$VAULT"
fi

echo "Creating new vault: $VAULT"
echo "Add a line:  ansible_become_password: \"<the celnet sudo password>\""
exec ansible-vault create "${PW_ARGS[@]+"${PW_ARGS[@]}"}" "$VAULT"
