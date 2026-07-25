---
name: gui-formatting-settings-shipped
description: "Compact number formatting, blotter search/filter, notifications clear+auto-clear, UI settings panel, and maturity-sorted curve pillars — shipped and deployed to UAT (9677a56)."
metadata: 
  node_type: memory
  type: project
  originSessionId: 24534380-8fb6-4089-a501-c7168ebd8613
---

Shipped 2026-07-07 in commit **9677a56** (local `main`), deployed live to UAT
(release `9677a56-20260707T071244Z`, host 136.114.170.5). Combined GUI build + full
vitest (1062/1062) green.

Features:
- **`fmtCompact`** in `gui/src/lib/format.ts` (k/m/b/t, trailing-zero-trimmed) — applied
  to notionals/quantities across blotters, tickets, RFQ inbox, book, risk. Rates/prices/
  strikes/vols/ids/dates deliberately NOT abbreviated. Unit-tested in `gui/test/format.test.ts`.
- **Blotter search/filter**: reusable `useTableFilter` hook + `TableSearch` component;
  wired into positions (`RatesBookWorkspace`), orders (RFQ inbox in `QuotingWorkspace`),
  deals, quotes. "N of M" + clear.
- **Notifications** (`NotificationCenter` + new `useNotificationStore`): Clear-all,
  per-item dismiss, auto-clear (terminal-state linkage by requestId + TTL sweep).
- **UI Settings panel** (`gui/src/settings/` + `SettingsPanel`): alerts, sounds+volume,
  min-qty threshold, growl/desktop notifications, auto-clear TTL. Persisted to
  localStorage key `celnet.settings.v1`; gates in-app/sound/desktop alerts + min-qty
  suppression.
- **Curve pillars** (`CurveWorkspace`): held in true maturity order (months/years/broken
  dates) via `sortByMaturity` + stable ids, so 1M sorts above 1Y and the strictly-
  increasing bootstrap invariant holds; INSPECT HORIZON chips follow. Also fixed the RFQ
  inbox selection highlight to align to the row (dropped `translateX`).

Deploy op notes ([[fi-rfs-streaming-and-booking]]): use `deploy/celnet-deploy.sh -t uat
release` (ships+builds BOTH server and GUI on-box, atomic symlink swap, restart; NOT the
heavier `full`/site.yml which re-provisions OS/HAProxy). No vault/`.vault_pass`, `become:
false` → non-interactive. **UAT box is small (3.8 GiB RAM); npm ci can drop SSH under
memory pressure — the deploy is idempotent, just re-run `release`** (it did OOM-drop once
at npm ci, succeeded on retry). Verify via served `https://<host>/version.json` buildTime
(GUI hash is placeholder `v0.0.0`; buildTime is the real discriminator). Backend runs via
`celnetctl` on 127.0.0.1:8080 behind nginx — NOT a `celnet-server` systemd unit, so
`systemctl --user is-active celnet-server` falsely reads inactive.
