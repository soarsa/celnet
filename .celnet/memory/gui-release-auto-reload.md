---
name: gui-release-auto-reload
description: GUI detects a new deployed release via /version.json and prompts the page to reload onto the fresh bundle
metadata: 
  node_type: memory
  type: project
  originSessionId: 3759f335-a305-4b8e-b3aa-727dea5a173c
---

Shipped 2026-06-27 (`3b09ae7`, deployed UAT). The running SPA detects when a
newer release has been deployed under it and offers a one-click reload — no stale
code after a blue-green cutover.

**Mechanism (no server API — a static deploy artifact, not a versioned contract):**
- `gui/vite.config.ts` emits a never-cached `/version.json` (`{hash, buildTime}`)
  beside index.html, from the SAME identity baked into `__CELNET_BUILD_*`; also
  served in dev via middleware.
- `gui/src/data/versionManifest.ts`: pure `parseReleaseManifest` + `isNewerRelease`
  (unit-tested) and `useVersionWatch(transport)` — polls on mount, every 5 min, on
  tab re-visibility, and on transport reconnect; latches forward.
- `gui/src/app/UpdateBanner.tsx`: a slim dismissible accent pill. **Prompts**, does
  NOT auto-reload (a trading surface — never swap the bundle mid-ticket). Dismiss
  keyed on release hash.
- nginx (`deploy/roles/celnet_web/templates/nginx-celnet.conf.j2`): `no-store` on
  `/version.json` and the index.html SPA entry; fingerprinted `/assets/*` stay
  immutable.

**Bootstrap caveat:** the watcher only catches deploys AFTER the release that
introduced it. Gate GUI with `npm run build` (see [[gui-gate-uses-production-build]]).
