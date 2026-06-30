#!/usr/bin/env bash
# celnet-dev-setup.sh — one-command collaborator bootstrap for the Celnet dev environment.
#
# Brings a fresh `git clone` (or a `git pull`) to a fully-working state in one run:
#   1. installs/updates the lodestar MCP binary (deterministic code graph + verified "why")
#   2. installs the Claude Code plugins this repo uses (LSPs, proto, GUI e2e, plugin-dev)
#   3. rebuilds the structural graph and rehydrates the committed verified-knowledge mirror
#   4. leaves the committed settings/hooks in place so it is all live on the next Claude
#      Code session start / `/mcp` reconnect.
#
# Reproducible-from-the-repo: every input is either committed here (`.mcp.json`,
# `.claude/`, `.lodestar/project-id`, the knowledge mirror, this script) or fetched from a
# pinned public source (the lodestar release, the github plugin marketplaces).
#
# Idempotent and re-runnable: run it again any time to pick up a newer lodestar release or
# to refresh the knowledge projection after a `git pull`. Safe to run alongside a live
# Claude session — it only writes the shared lodestar cache (cross-process-locked), the
# user-scope plugin config, and ~/.local/bin; it never touches this repo's git tree.
#
# Multi-session / parallel work: plugins + the lodestar binary are machine-global, and the
# project key (`.lodestar/project-id`, committed) is shared, so every `git worktree` you add
# off a current branch inherits this setup and shares one knowledge store automatically.
#
# Usage:
#   tools/celnet-dev-setup.sh                  # full setup (recommended first run)
#   tools/celnet-dev-setup.sh --update         # force a lodestar upgrade to the latest release
#   tools/celnet-dev-setup.sh --with-syncd     # also install the opt-in LAN knowledge-sync daemon
#   tools/celnet-dev-setup.sh --no-plugins     # skip the Claude Code plugin install
#   tools/celnet-dev-setup.sh --no-knowledge   # skip the index + knowledge rehydration
#   tools/celnet-dev-setup.sh --help
set -euo pipefail

# ── Constants (the one current contract — no version negotiation) ───────────────
LODESTAR_REPO="soarsa/lodestar"
LODESTAR_MIN_VERSION="0.9.0"          # floor compatible with the committed knowledge mirror
PROJECT_KEY="github.com-soarsa-celnet"
INSTALL_ONELINER="https://raw.githubusercontent.com/${LODESTAR_REPO}/main/install.sh"

# Plugins this repo actually uses, by marketplace. Deliberately EXCLUDED:
#   • claude-codewiki — its marketplace is a local directory, not a shareable git source.
#   • expo — disabled estate-wide (no React Native here).
#   • atlassian / gitlab — left to each developer's own preference (celer-estate optional).
PLUGINS_OFFICIAL=(plugin-dev rust-analyzer-lsp typescript-lsp pyright-lsp playwright chrome-devtools-mcp frontend-design)
PLUGINS_BUF=(protobuf)
BUF_MARKETPLACE_REPO="bufbuild/claude-plugins"

# ── Args ────────────────────────────────────────────────────────────────────────
DO_PLUGINS=1; DO_KNOWLEDGE=1; FORCE_UPDATE=0; WITH_SYNCD=0
for a in "$@"; do case "$a" in
  --no-plugins)   DO_PLUGINS=0 ;;
  --no-knowledge) DO_KNOWLEDGE=0 ;;
  --update)       FORCE_UPDATE=1 ;;
  --with-syncd)   WITH_SYNCD=1 ;;
  -h|--help)      sed -n '2,40p' "$0"; exit 0 ;;
  *) echo "warning: ignoring unknown argument: $a" >&2 ;;
esac; done

# ── Output helpers ────────────────────────────────────────────────────────────────
c_g=$'\033[32m'; c_y=$'\033[33m'; c_r=$'\033[31m'; c_b=$'\033[1m'; c_0=$'\033[0m'
[ -t 1 ] || { c_g=; c_y=; c_r=; c_b=; c_0=; }
step() { printf '\n%s==>%s %s%s%s\n' "$c_b" "$c_0" "$c_b" "$*" "$c_0"; }
ok()   { printf '   %s✔%s %s\n' "$c_g" "$c_0" "$*"; }
warn() { printf '   %s!%s %s\n' "$c_y" "$c_0" "$*" >&2; }
die()  { printf '\n%serror:%s %s\n' "$c_r" "$c_0" "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# semver "$1 >= $2" using sort -V (portable, no external semver tool)
ver_ge() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]; }

# ── 0. Preflight ─────────────────────────────────────────────────────────────────
step "Preflight"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
[ -f "$REPO/.lodestar/project-id" ] || die "run this from inside the celnet checkout (no .lodestar/project-id at $REPO)."
PIN="$(tr -d '[:space:]' < "$REPO/.lodestar/project-id")"
[ "$PIN" = "$PROJECT_KEY" ] || warn "project-id is '$PIN' (expected '$PROJECT_KEY')"
for t in git curl python3; do have "$t" || die "missing required tool: $t"; done
have tar || die "missing required tool: tar"
have sqlite3 || warn "sqlite3 not found — the knowledge-projection guard will fall back (non-fatal)."
CLAUDE_OK=1; have claude || { CLAUDE_OK=0; warn "the 'claude' CLI is not on PATH — plugin install will be skipped (install Claude Code first)."; }
ok "repo: $REPO"
ok "project key: $PIN"

# Ensure ~/.local/bin (lodestar's install target) is usable + on PATH for this run.
mkdir -p "$HOME/.local/bin"
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) export PATH="$HOME/.local/bin:$PATH"; warn "added ~/.local/bin to PATH for this run — add it permanently to your shell profile." ;; esac

# ── 1. lodestar binary: install / update ──────────────────────────────────────────
step "lodestar MCP binary"
latest_tag() { curl -fsSL "https://api.github.com/repos/${LODESTAR_REPO}/releases/latest" 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin).get("tag_name","").lstrip("v"))' 2>/dev/null || true; }
run_installer() {
  curl -fsSL "$INSTALL_ONELINER" | bash || die "lodestar install failed — see ${INSTALL_ONELINER%/install.sh}/releases"
}
if ! have lodestar; then
  ok "lodestar not found — installing latest…"; run_installer
else
  cur="$(lodestar --version 2>/dev/null | awk '{print $NF}')"
  ok "installed: lodestar $cur"
  want="$(latest_tag)"
  if [ "$FORCE_UPDATE" = 1 ]; then
    ok "forcing upgrade check…"; run_installer
  elif ! ver_ge "$cur" "$LODESTAR_MIN_VERSION"; then
    warn "below required floor $LODESTAR_MIN_VERSION — upgrading…"; run_installer
  elif [ -n "$want" ] && ! ver_ge "$cur" "$want"; then
    ok "newer release available ($want) — upgrading…"; run_installer
  else
    ok "up to date (latest: ${want:-unknown})"
  fi
fi
have lodestar || die "lodestar still not on PATH after install."
# Health probe (CLAUDE.md guardrail #3)
if lodestar doctor --json >/dev/null 2>&1; then
  lodestar doctor --json | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if d.get("ok") else 1)' \
    && ok "doctor: ok (problems: 0), $(lodestar --version)" || die "lodestar doctor reported problems — run: lodestar doctor --json"
else
  warn "lodestar doctor unavailable on this build; continuing."
fi

# ── 1b. Optional: the LAN knowledge-sync daemon (opt-in, off by default) ───────────
if [ "$WITH_SYNCD" = 1 ]; then
  step "lodestar-syncd (opt-in LAN knowledge sync)"
  if have lodestar-syncd; then ok "already installed: $(lodestar-syncd --version 2>/dev/null)"; else
    os=$(uname -s); arch=$(uname -m)
    case "$os" in Darwin) os=darwin;; Linux) os=linux;; *) die "syncd: unsupported OS $os";; esac
    case "$arch" in arm64|aarch64) arch=arm64;; x86_64|amd64) arch=amd64;; *) die "syncd: unsupported arch $arch";; esac
    tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
    asset="lodestar-${os}-${arch}.tar.gz"
    ok "downloading $asset…"
    curl -fsSL "https://github.com/${LODESTAR_REPO}/releases/latest/download/${asset}" -o "$tmp/$asset" || die "syncd download failed"
    tar -xzf "$tmp/$asset" -C "$tmp"
    [ -f "$tmp/lodestar-syncd" ] || die "lodestar-syncd not present in the release archive"
    [ "$os" = darwin ] && { xattr -d com.apple.quarantine "$tmp/lodestar-syncd" 2>/dev/null || true; codesign --sign - --force "$tmp/lodestar-syncd" 2>/dev/null || true; }
    install -m 0755 "$tmp/lodestar-syncd" "$HOME/.local/bin/lodestar-syncd"
    ok "installed: $(lodestar-syncd --version 2>/dev/null)"
  fi
  warn "syncd is installed but NOT started — it is opt-in and only useful across machines on a LAN."
  warn "to enable real-time team sync later, run lodestar-syncd with a shared PSK (see the lodestar sync guide)."
fi

# ── 2. Claude Code plugins ─────────────────────────────────────────────────────────
if [ "$DO_PLUGINS" = 1 ] && [ "$CLAUDE_OK" = 1 ]; then
  step "Claude Code plugins"
  installed="$(claude plugin list 2>/dev/null || true)"
  ensure_plugin() { # $1=name  $2=marketplace
    if printf '%s' "$installed" | grep -q "$1@$2"; then ok "$1 (already installed)"; return; fi
    if claude plugin install "$1@$2" --scope user >/dev/null 2>&1; then ok "installed $1@$2"; else warn "could not install $1@$2 (continue)"; fi
  }
  # buf marketplace (github-sourced ⇒ reproducible); add if absent.
  if ! claude plugin marketplace list 2>/dev/null | grep -q "buf-plugins"; then
    claude plugin marketplace add "$BUF_MARKETPLACE_REPO" >/dev/null 2>&1 && ok "added marketplace buf-plugins" || warn "could not add buf-plugins marketplace"
  fi
  for p in "${PLUGINS_OFFICIAL[@]}"; do ensure_plugin "$p" "claude-plugins-official"; done
  for p in "${PLUGINS_BUF[@]}";      do ensure_plugin "$p" "buf-plugins"; done
  ok "plugins load on your next Claude Code session."
elif [ "$DO_PLUGINS" = 1 ]; then
  step "Claude Code plugins"; warn "skipped — 'claude' CLI not found."
else
  step "Claude Code plugins"; ok "skipped (--no-plugins)"
fi

# ── 3. Structural index + verified-knowledge rehydration ───────────────────────────
if [ "$DO_KNOWLEDGE" = 1 ]; then
  step "Structural graph + verified knowledge"
  ok "indexing (absolute path — never '.')…"
  lodestar index "$REPO" >/dev/null 2>&1 || warn "index returned non-zero (continue)"
  if [ -f "$REPO/tools/lodestar/replay-knowledge.py" ] && [ -f "$REPO/.lodestar/knowledge/claims-mirror.json" ]; then
    mirror_n="$(python3 -c "import json;print(len(json.load(open('$REPO/.lodestar/knowledge/claims-mirror.json'))))" 2>/dev/null || echo '?')"
    ok "rehydrating verified knowledge from the committed mirror ($mirror_n claims)…"
    python3 "$REPO/tools/lodestar/replay-knowledge.py" 2>&1 | tail -1 | sed 's/^/   /'
  else
    warn "knowledge mirror or replay script missing — skipping rehydration."
  fi
else
  step "Structural graph + verified knowledge"; ok "skipped (--no-knowledge)"
fi

# ── 4. Done ────────────────────────────────────────────────────────────────────────
step "Setup complete"
cat <<EOF
   $(lodestar --version 2>/dev/null)  •  project=$PIN
   The committed settings are already in place:
     • .mcp.json            — registers the lodestar MCP for this project
     • .claude/settings.json — SessionStart/Stop hooks (auto index + knowledge replay; non-blocking)
     • .lodestar/project-id  — pins the shared knowledge store
   ${c_b}Restart Claude Code (or run /mcp) so the lodestar server + plugins load.${c_0}
   Solo is fine: nothing here requires a second session or the network — collaboration is
   additive (a shared store + the committed mirror) and every hook is non-blocking.
EOF
