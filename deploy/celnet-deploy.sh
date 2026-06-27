#!/usr/bin/env bash
# ===========================================================================
# celnet-deploy.sh — interactive devops control for Celnet deployments.
#
# Wraps the Ansible playbooks (site/deploy/rollback) and on-box celnetctl with
# a menu. Pick a target from the inventory, then choose an action.
#
#   ./celnet-deploy.sh                 # interactive menu
#   ./celnet-deploy.sh -t uat full     # non-interactive: full setup & deploy to uat
#   ./celnet-deploy.sh -t uat release  # binary release
#   ./celnet-deploy.sh -t uat start|stop|restart|status|rollback|haproxy|releases
# ===========================================================================
set -euo pipefail
cd "$(dirname "$0")"

TARGET="${TARGET:-uat}"

c_bold=$'\033[1m'; c_grn=$'\033[32m'; c_ylw=$'\033[33m'; c_red=$'\033[31m'; c_cyn=$'\033[36m'; c_rst=$'\033[0m'
say()  { printf '%s\n' "$*"; }
hdr()  { printf '\n%s%s%s\n' "$c_bold$c_cyn" "$*" "$c_rst"; }
ok()   { printf '%s%s%s\n' "$c_grn" "$*" "$c_rst"; }
warn() { printf '%s%s%s\n' "$c_ylw" "$*" "$c_rst"; }
err()  { printf '%s%s%s\n' "$c_red" "$*" "$c_rst" >&2; }

require() { command -v "$1" >/dev/null 2>&1 || { err "Missing required command: $1"; exit 1; }; }
require ansible-playbook
require ansible

ensure_collections() {
  if ! ansible-galaxy collection list ansible.posix >/dev/null 2>&1; then
    warn "Installing Ansible collection requirements..."
    ansible-galaxy collection install -r requirements.yml
  fi
}

confirm() { read -r -p "$1 [y/N] " a; [[ "$a" =~ ^[Yy]$ ]]; }

cred_args() {  # set CRED_ARGS for the current target (bash 3.2 safe). $1=1 if play uses sudo.
  CRED_ARGS=()
  if [[ -f .vault_pass ]]; then
    # Vault passphrase on disk -> decrypt vault (which carries the sudo password).
    CRED_ARGS=(--vault-password-file .vault_pass)
  elif [[ -f "group_vars/${TARGET}/vault.yml" ]]; then
    # Encrypted vault present (loaded for every play against this env) -> prompt to unlock.
    CRED_ARGS=(--ask-vault-pass)
  elif [[ "${1:-0}" == "1" ]]; then
    # No vault and this play escalates -> prompt for the sudo password.
    CRED_ARGS=(--ask-become-pass)
  fi
}

playbook() {  # playbook <file> [needs_sudo]
  ensure_collections
  cred_args "${2:-0}"
  hdr ">> ansible-playbook $1 --limit $TARGET ${CRED_ARGS[*]:-}"
  ansible-playbook "$1" --limit "$TARGET" "${CRED_ARGS[@]+"${CRED_ARGS[@]}"}"
}

ctl() {       # ctl <celnetctl-action>
  hdr ">> celnetctl $1  (on $TARGET)"
  ansible "$TARGET" -m command -a "/opt/celnet/shared/bin/celnetctl $1" -o 2>/dev/null \
    || ansible "$TARGET" -m command -a "/opt/celnet/shared/bin/celnetctl $1"
}

show_releases() {
  hdr ">> releases on $TARGET"
  ansible "$TARGET" -m shell -a \
    "echo current: \$(readlink -f /opt/celnet/current 2>/dev/null || echo none); echo; ls -1t /opt/celnet/releases 2>/dev/null"
}

pick_target() {
  hdr "Inventory groups / hosts"
  ansible-inventory --list --yaml 2>/dev/null | grep -E '^( {2}[a-z].*:|    hosts:|      [a-z].*:)' || true
  read -r -p "Target host or group [${TARGET}]: " t
  [[ -n "${t:-}" ]] && TARGET="$t"
  ok "Target set to: $TARGET"
}

action() {
  case "$1" in
    full)     confirm "Full setup & deploy to '$TARGET' (installs packages, builds, HAProxy)?" && playbook site.yml 1 ;;
    release)  playbook deploy.yml ;;
    rollback) confirm "Roll back '$TARGET' to the previous release?" && playbook rollback.yml ;;
    haproxy)  playbook site.yml 1 ;;   # site re-applies haproxy idempotently; or use a tag
    start)    ctl start ;;
    stop)     confirm "Stop celnet-server on '$TARGET'?" && ctl stop ;;
    restart)  ctl restart ;;
    status)   ctl status ;;
    releases) show_releases ;;
    ping)     hdr ">> ping $TARGET"; ansible "$TARGET" -m ping ;;
    sshkey)   hdr ">> bootstrap SSH key"; read -r -p "user@host [celnet@136.115.32.199]: " uh; ./scripts/setup-ssh-key.sh "${uh:-celnet@136.115.32.199}" ;;
    vault)    hdr ">> edit sudo vault ($TARGET)"; ./scripts/vault-init.sh "$TARGET" ;;
    *) err "unknown action: $1"; return 2 ;;
  esac
}

menu() {
  while true; do
    hdr "================ Celnet Deploy — target: ${c_grn}${TARGET}${c_rst}${c_bold}${c_cyn} ================"
    cat <<MENU
  ${c_bold}Setup / Release${c_rst}
    1) Full setup & deploy        (provision + build + HAProxy + start)
    2) Binary release             (sync + incremental build + symlink swap + restart)
    3) Rollback to previous release
    9) (Re)apply HAProxy edge config

  ${c_bold}Service control${c_rst}
    4) Start          5) Stop          6) Restart          7) Status

  ${c_bold}Info / Access${c_rst}
    8) List releases / current      p) Ping target      t) Change target
    k) Bootstrap SSH key on target  v) Edit sudo vault (encrypted)

    0) Exit
MENU
    read -r -p "Select: " choice
    case "$choice" in
      1) action full ;;
      2) action release ;;
      3) action rollback ;;
      9) action haproxy ;;
      4) action start ;;
      5) action stop ;;
      6) action restart ;;
      7) action status ;;
      8) action releases ;;
      p|P) action ping ;;
      k|K) action sshkey ;;
      v|V) action vault ;;
      t|T) pick_target ;;
      0|q|Q) ok "bye"; exit 0 ;;
      *) warn "invalid choice" ;;
    esac
  done
}

# --- arg parsing ------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    -t|--target) TARGET="$2"; shift 2 ;;
    full|release|rollback|haproxy|start|stop|restart|status|releases|ping|sshkey|vault)
      action "$1"; exit $? ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
    *) err "unknown arg: $1"; exit 2 ;;
  esac
done

menu
