# Celnet Deployment (Ansible)

Build-on-target Ansible deployment for Celnet. Ships the two production binaries
(`celnet-server`, `celnet`) to a host, builds them natively, and publishes them
with a Capistrano-style versioned-release + atomic-symlink layout so binary
releases are incremental and instantly rollback-able. An HAProxy edge terminates
TLS for `https://app.uat.celnet.uk` and proxies to the local server (nginx serves
the GUI SPA; WebSocket-upgrade requests go to the celnet-server mirror).

> **Networking (required):** the host firewall is open, but the **cloud
> perimeter firewall must allow inbound `tcp:80,443`**. On GCP:
> `gcloud compute firewall-rules create celnet-allow-web --direction=INGRESS
> --action=ALLOW --rules=tcp:80,tcp:443 --source-ranges=0.0.0.0/0 --network=default`
> (AWS/Azure: open 80/443 in the security group / NSG). Also point DNS for the
> domain at the host IP. Symptom if missing: every on-box check is green
> (`https://127.0.0.1/` → 200) but browsers time out — `:443` ingress is dropped.

## Layout on the host (`/opt/celnet`)

```
/opt/celnet/
├── releases/<sha-timestamp>/bin/{celnet-server,celnet}   # immutable per release
├── current -> releases/<sha-timestamp>                   # atomic symlink swap
├── shared/
│   ├── config/celnet.env      # runtime env (CELNET_GRPC_ADDR, CELNET_WS_ADDR, …)
│   ├── bin/celnetctl          # start/stop/status/restart control script
│   ├── logs/                  # celnet-server stdout/err
│   ├── run/                   # pidfile
│   └── tls/app.uat.celnet.co.uk.pem
└── build/                     # synced source + cargo target/ (persists → incremental)
```

## Prerequisites

- **Controller (your mac):** `ansible` + `ansible-galaxy` (`brew install ansible`),
  `git`, `rsync`. **Key-based SSH** to `celnet@136.115.32.199` (see *SSH keys* below).
- **Target host:** Debian/Ubuntu (apt), reachable over SSH. The `celnet` account
  needs **passwordless sudo** for the one-time *full setup* (packages, user,
  HAProxy). Binary releases / rollbacks / start-stop run unprivileged.
- Collections (auto-installed by the menu on first run): `ansible.posix`.
- For *full setup*: the `celnet` sudo password in the vault (see *Sudo password* below).

## SSH keys

The deployment authenticates with an SSH **key** (no passwords). Bootstrap once:

```bash
cd deploy
./scripts/setup-ssh-key.sh celnet@136.115.32.199    # generates ~/.ssh/celnet_uat, installs it (password once)
```

After that, Ansible, the menu, and the start/stop scripts all use the key. The
key path is `~/.ssh/celnet_uat` by default — override with the `CELNET_SSH_KEY`
env var or `celnet_ssh_key` in `group_vars/uat.yml`. Connection options enforce
`IdentitiesOnly=yes` (offer only this key) and `BatchMode=yes` (fail fast rather
than fall back to a password prompt). From the menu, option **k** runs the same
bootstrap.

## Sudo password (Ansible Vault)

The `celnet` account's **sudo password** is stored encrypted, never in plaintext.
It lives as `ansible_become_password` in the encrypted `group_vars/uat/vault.yml`.
Only the *full setup* / HAProxy plays escalate (provision + HAProxy); releases,
rollbacks and start/stop do not use sudo.

**No vault yet?** The menu prompts for the sudo password (`--ask-become-pass`)
when `group_vars/<env>/vault.yml` is absent, so a first run works without it.

Create or edit the vault:

```bash
cd deploy
./scripts/vault-init.sh uat     # ansible-vault create/edit group_vars/uat/vault.yml
#   add:  ansible_become_password: "<the celnet sudo password>"
```

You choose a **vault passphrase** to unlock the file. To run unattended, put that
passphrase in `deploy/.vault_pass` (gitignored) — the menu and playbooks then
decrypt automatically. Otherwise the menu uses `--ask-vault-pass`. Note: once a
vault.yml exists it is loaded for *every* play against that env, so use
`.vault_pass` if you want unattended releases. With no vault and passwordless
sudo on the host, you can skip all of this.

## Quick start

```bash
cd deploy
./celnet-deploy.sh                 # interactive menu
# or non-interactive:
./celnet-deploy.sh -t uat full     # 1) full setup & deploy
./celnet-deploy.sh -t uat release  # 2) binary release (incremental)
./celnet-deploy.sh -t uat rollback # roll back to previous release
./scripts/start.sh uat             # start  (also stop.sh / status.sh)
```

## Menu actions

| # | Action | Playbook / cmd | Privilege |
|---|--------|----------------|-----------|
| 1 | Full setup & deploy | `site.yml` | sudo |
| 2 | Binary release | `deploy.yml` | unprivileged |
| 3 | Rollback | `rollback.yml` | unprivileged |
| 9 | (Re)apply HAProxy | `site.yml` | sudo |
| 4–7 | Start/Stop/Restart/Status | `celnetctl` over SSH | unprivileged |
| 8 | List releases / current | ad-hoc | unprivileged |

## How a binary release works (incremental)

1. `rsync` the working tree to `/opt/celnet/build` (excludes `target/`, so the
   remote cargo cache persists → only changed crates recompile).
2. `cargo build --release -p celnet-server -p celnet-cli` on the host
   (toolchain pinned by `rust-toolchain.toml`).
3. Copy the two binaries into `releases/<sha-timestamp>/bin/`.
4. `ln -sfn` swap of `current` → the new release (atomic).
5. Prune to the newest `celnet_keep_releases` (default 5).
6. `celnetctl restart` (graceful drain on SIGINT, then relaunch).

## Configuration

Edit `group_vars/all.yml` (shared) and `group_vars/uat.yml` (UAT). Key knobs:

- `celnet_env` — runtime env templated into `shared/config/celnet.env`.
  The WS mirror uses a **fixed** port (`CELNET_WS_ADDR`, default
  `127.0.0.1:50061`) so HAProxy can front it.
- `celnet_keep_releases` — release retention.
- `celnet_domain`, `celnet_http_port`, `celnet_https_port` — edge.
- `celnet_expose_grpc` — also front gRPC (HTTP/2) on `celnet_grpc_https_port`.

## TLS

UAT generates a **self-signed** cert for `app.uat.celnet.co.uk` on first setup
(`shared/tls/app.uat.celnet.co.uk.pem`). To use a real certificate, drop a
combined cert+key PEM at that path (set `celnet_tls_generate_self_signed: false`)
and re-run option 9. Point DNS / your hosts file for `app.uat.celnet.co.uk` at
`136.115.32.199`.

## On-box control (`celnetctl`)

```bash
ssh celnet@136.115.32.199 /opt/celnet/shared/bin/celnetctl status
#   start | stop | restart | status | tail [n]
```
