---
name: deploy-ssh-drop-on-silent-build
description: "UAT release can fail UNREACHABLE mid cargo-build (swapless 3.8Gi box); pre-warm build on box under nohup with capped jobs, then re-run deploy."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: fc815517-2b4d-4d65-96c9-8795d695a3f6
---

Context: operator said "continue then deploy" while landing the celnet-refdata Phase-1
seed; the `release` deploy then failed at the on-box build. No code importers/API/schema —
this is a devops runbook note. Affected surface: `deploy/celnet-deploy.sh` + the
`celnet_release` ansible role build task.

`./celnet-deploy.sh -t uat release` runs `cargo build --release` on the UAT box
(136.64.157.182, celnet@) as one long **silent** ansible task. On this **swapless
3.8 GiB** box a parallel link of `celnet-server` spikes memory and makes sshd briefly
unresponsive past the keepalive window → ansible reports **`UNREACHABLE!` "Data could
not be sent to remote host"** at the build step. This is a transport drop, NOT an OOM
task-failure, and NOT a downtime: the atomic symlink swap never runs, so the *previous*
release keeps serving (`/opt/celnet/current` still points at the old release dir).

**Why:** `ansible.cfg` already sets `ServerAliveInterval=30` + `ControlPersist=120s`, but
the memory spike stalls sshd longer than the keepalive tolerance.

**How to apply — pre-warm then deploy:**
1. The failed deploy already rsynced source to `/opt/celnet/build` (that task succeeds
   before the build). So build there directly, detached + memory-capped:
   `cd /opt/celnet/build && source /opt/celnet/.cargo/env` with
   `CARGO_HOME=/opt/celnet/.cargo RUSTUP_HOME=/opt/celnet/.rustup RUSTC_WRAPPER= CARGO_BUILD_JOBS=2`,
   then `setsid nohup bash -c "cargo build --release -p celnet-server -p celnet-cli -p celnet-lp-sim >> prewarm.log 2>&1; echo EXIT=\$? >> prewarm.log" &`.
   `CARGO_BUILD_JOBS=2` caps peak memory; `nohup`+`setsid` survive an SSH drop.
2. Poll `grep -q "^EXIT=" /opt/celnet/build/prewarm.log` over ssh until done (5-15 min).
3. Re-run `./celnet-deploy.sh -t uat release` — its build task now finds target/ warm and
   returns in seconds (no long silent hold), then does the atomic swap + restart.

Box facts: no `timeout` binary on macOS (use `ssh -o ConnectTimeout`). Release layout is
Capistrano: `/opt/celnet/{build,releases,current}`. `celnet_build_packages` =
celnet-server + celnet-cli; lp-sim/fix built via role flags. See [[gui-formatting-settings-shipped]].
