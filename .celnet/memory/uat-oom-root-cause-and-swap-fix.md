---
name: uat-oom-root-cause-and-swap-fix
description: "UAT OOM DURABLY FIXED 2026-08-10 — box is a STANDALONE GCE VM (no MIG), resized e2-medium 4 GiB → e2-standard-2 8 GiB + 4 GiB swap installed via instance-metadata startup-script (runs as root every boot, no box-sudo). Server starts via celnetctl (NOT systemd) so it needs a manual start after any reboot."
metadata: 
  node_type: memory
  type: project
  originSessionId: bd2a0685-2d52-4bdc-95d4-4e7872001f3e
---

## RESOLUTION (2026-08-10) — durable fix applied via gcloud, box healthy.
The box is a **standalone GCE instance** `celnet-app-3` (zone `us-east4-c`, project
`project-88e14d51-c027-4795-99d`, visible to gcloud acct `bencuthbert@tbarindustries.com`).
**There is NO managed instance group and NO instance template** — `automaticRestart:true`
restarts the SAME VM (same disk), it does NOT recreate from an image. So the earlier fear
below ("recreation wipes ansible swap → needs privileged site.yml") was WRONG for this setup:
disk-persisted swap survives restarts, and the sudo-less-celnet problem is sidestepped entirely.
Fix applied directly (needs the VM `TERMINATED` for the resize):
1. `gcloud compute instances set-machine-type celnet-app-3 --zone us-east4-c --machine-type e2-custom-2-8192`
   → normalizes to **e2-standard-2 (2 vCPU, 8 GiB)**. 8 GiB fits build(~2.5G)+runtime(~1.5G)+agents(~1G).
2. `gcloud compute instances add-metadata … --metadata-from-file startup-script=<swap script>`
   → boot-time root script: dd (hole-free) /swapfile 4 GiB, mkswap, swapon, fstab persist,
   vm.swappiness=10. Idempotent, re-applies every boot. (script saved in session scratchpad
   `celnet-swap-startup.sh`.) Serial console confirms it ran ("Adding 4194300k swap on /swapfile").
3. `gcloud compute instances start …` → new external IP each start (this time **136.107.165.2**;
   IP churns → update `deploy/inventory/hosts.ini` ansible_host every time).
Verified live: `free -m` = 7945 MB RAM + 4095 MB swap; app HTTPS 200.
**⚠ Server does NOT auto-start on reboot** — no systemd unit; it runs via
`/opt/celnet/shared/bin/celnetctl start` (nginx:8080 + HAProxy:443 ARE systemd and auto-start,
so the SPA serves but the backend 50051/50061 is down until celnetctl start). After ANY box
reboot/stop-start: `ssh celnet@<ip> /opt/celnet/shared/bin/celnetctl start`.
The `8819c440` deploy-tree swap task + jobs=1 remain valid as belt-and-braces but are now
redundant with the startup-script; jobs=1 still helps the on-box build. Today's code
(f1c2671f/abf51e2b/213d2721) is still NOT deployed — UAT runs b71767bb; a deploy will now
succeed without OOM on the 8 GiB box.

## ORIGINAL DIAGNOSIS (pre-resolution) — kept for context:

**Root cause of "the process keeps OOMing and killing the machine" (diagnosed 2026-08-10).**
The UAT box is ~4 GiB RAM with **`SwapTotal: 0`**, and every deploy builds the release
**ON the box**. `celnet-server` compiles at `opt-level=3` + `linker-plugin-lto` +
`codegen-units=1` → a single `rustc` peaks **~1.7 GiB**; the release role used
`CARGO_BUILD_JOBS=2` (two `rustc` ≈ 2.5 GiB) on top of ~1 GiB of GCP/otel/haproxy agents,
then `npm ci` + Vite spike again. On a swapless 4 GiB box that exceeds physical RAM → kernel
OOM-killer → box dies → cloud health-check **recreates the instance** (hostname churned
`celnet-app-1..3`, IP changed 3× in a day; `journalctl --list-boots` showed 6+ boots, one
only 13 min). The OOM kernel trace never survived because the instance is recreated, not
rebooted — that's why it was never found in logs before.

**How it was diagnosed (reusable):** the celnet user has **NO passwordless sudo** (`sudo -n`
fails) so kernel logs weren't readable and swap can't be added live. Instead an EXTERNAL
sampler was run from the Mac (`scratchpad/oom-sampler.sh` — SSHes every 3s, appends
`free`/top-RSS/`MemAvailable`/loadavg to a LOCAL log that survives the box dying). The
process list itself was the smoking gun: `ps -eo pid,rss,args` showed the celnet-server
`rustc` at 1.68 GiB RSS mid-build.

**Fix landed in the deploy tree (local, un-pushed) 2026-08-10:**
- `deploy/roles/celnet_provision/` — NEW swap-file task block + defaults
  (`celnet_swap_enabled=true`, `/swapfile`, `celnet_swap_size_mb=4096`, `celnet_swappiness=10`):
  dd-allocates (hole-free ⇒ swapon-safe), mkswap, `/etc/fstab` persist, `swapon`, sysctl.
  Idempotent. Runs under `celnet_provision` (become: true) in `site.yml`.
- `deploy/roles/celnet_release/tasks/main.yml` — `CARGO_BUILD_JOBS` default **2 → 1**
  (one heavy rustc at a time). Affects all three build tasks (server/cli, fix example, lp-sim).

**⚠ Applying the swap needs privilege the celnet account lacks from a normal session.**
`deploy.yml` (become: false) gets jobs=1 automatically but does NOT create swap. Swap is only
created by **`site.yml`** (runs `celnet_provision` become: true, BEFORE the build, so swap is
live before the heavy compile). site.yml "requires passwordless sudo for the celnet user" —
but `sudo -n` currently fails, so the user must run site.yml with a privileged path
(e.g. `--ask-become-pass`, or an admin ansible_user). **If the box is recreated from an image
without re-running provisioning, ansible-applied swap won't persist** — then bake swap into the
GCP image / instance startup-script instead. Command:
`cd deploy && env -u GITHUB_TOKEN ansible-playbook site.yml --limit uat` (add `--ask-become-pass`
if celnet needs a sudo password). Don't launch it while another build is already running on the
box (two concurrent builds guarantee OOM). Supersedes the "just re-run release" advice in
[[deploy-ssh-drop-on-silent-build]] / [[uat-deploy-recovery-and-hazards]] — the real fix is swap.
