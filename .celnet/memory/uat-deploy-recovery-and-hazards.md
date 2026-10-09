---
name: uat-deploy-recovery-and-hazards
description: "UAT binary release failed at npm ci (disk 100% full) then the release dirs got deleted mid-deploy by a concurrent actor; how to reclaim disk + manually atomic-publish from build artifacts, plus two live hazards (phantom release-deleter, leaked lp-sim procs)."
metadata: 
  node_type: memory
  type: project
  originSessionId: b69ceb7d-66d6-4bc7-ac17-96e24adc4293
---

2026-07-28: A `deploy.yml --limit uat` binary release failed twice, then was recovered manually. Durable lessons for the undersized UAT box (136.64.157.182, `/dev/sda1` only 9.7G, celnet user, NO sudo).

**Symptom 1 — `npm ci` ENOSPC:** disk was 100% full (0 bytes avail). Root-owned `/var/log` (888M: haproxy.log/auth.log) is UN-reclaimable without sudo. Reclaimable (celnet-owned) levers, in order: delete stale `/opt/celnet/releases/*` except the `current` target; `rm -rf /opt/celnet/build/gui/node_modules` (npm ci recreates it and needs the freed space to STAGE — pre-deleting is the big win); `npm cache clean --force` (~50M); `rm -rf /opt/celnet/.cargo/registry/cache/* src/*` (crates re-download only if needed). DON'T touch `/opt/celnet/.rustup` (1.7G toolchain) or `build/target` (1.2G incremental cache) unless desperate. That freed 0→900M and npm ci + vite build then passed.

**Symptom 2 — release dirs vanish MID-DEPLOY:** on the re-run, all of `/opt/celnet/releases/` (including the `current` 07-27 target) was deleted at ~12:27 WHILE my `npm ci` ran — by an EXTERNAL actor (a concurrent session's deploy-prune or cleanup; other `celnet` sshd logins were active from the same IP). The role's own prune runs AFTER the `cp dist web` step so it wasn't the cause. Result: `cp -a .../dist .../releases/<rel>/web` failed "No such file or directory" (parent gone), and `current` became a DANGLING symlink → nginx SPA 404s for users, but `celnet-server` keeps running on the held-open deleted binary inode.

**Recovery (fast, tiny race window — preferred over another 3-min rebuild):** all artifacts survive in `build/target/release/{celnet-server,celnet,lp-sim,examples/fix_rfq_client}` + `build/gui/dist`. One atomic SSH: `mkdir -p releases/$REL/bin` → `cp -a` the 4 bins (fix_rfq_client → `fix-rfq-client`) → `cp -a build/gui/dist releases/$REL/web` → write VERSION → `ln -sfn releases/$REL current`. Then verify: nginx `curl 127.0.0.1:8080/` = 200, WS `ss -ltnp | grep 50061`, public `curl -sk https://celnetapp.uat.celnet.uk/` = 200 (cert is self-signed → MUST use `-k`; DNS resolves to the box). No server restart needed when the running commit == HEAD (the 07-27 release was already named `69794e8-…`, same as HEAD, so re-deploying was a no-op for the engine; only the GUI dist was rebuilt).

**Two live hazards to raise with the operator:**
1. **Phantom release-deleter:** something deletes UAT release dirs mid-deploy. Likely a parallel Agent/human session running deploys/cleanup on the shared box. Coordinate before deploying; a lone deploy can be clobbered.
2. **Leaked `lp-sim` procs:** 16 orphaned `/opt/celnet/current/bin/lp-sim` processes running for DAYS (each `--book ust-composite --members 5`), holding deleted inodes and RAM on the 3.8Gi box. The launcher/restart path isn't reaping the old ones. Duplicates may also double-drive the demo book feed.

See [[deploy-ssh-drop-on-silent-build]] and [[fi-agg-book-rfq-gap-and-tiering-backlog]] (the detached-prewarm deploy remedy for this same box).
