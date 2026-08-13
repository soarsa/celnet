---
name: uat-login-identity-store-and-reset
description: "UAT login/accounts ops — the server loads a CWD-relative identity.json from /home/celnet (NOT wiped by restart); accounts persist but passwords can be rotated away from the admin@celnet.com/password seed; how to reset a password hash by hand (Argon2id PHC, m=19456,t=2,p=1); login throttle locks an email ~15min after repeated fails (in-memory, clears on server restart)."
metadata:
  node_type: memory
  type: project
  originSessionId: bd2a0685-2d52-4bdc-95d4-4e7872001f3e
---

**UAT identity store lives at `/home/celnet/identity.json`** (287 KB, mode 600, owned by
`celnet`). The server loads it **CWD-relative** — `celnetctl` runs celnet-server with CWD
`/home/celnet`, and there is NO `CELNET_IDENTITY_CONFIG` env set, so `IdentityStore::config_path()`
resolves to `./identity.json` = `/home/celnet/identity.json`. (Beware the decoys under
`/opt/celnet/build/**/identity.json` — the server does NOT read those.) Accounts PERSIST across
restarts (it is a real file, not in-memory), and hold `admin@celnet.com`, `ben@celnet.com`
(role=trader, desks marex+celnet), and several `@marex.com` demo users.

**"invalid email or password" vs the seed:** the seeded default is `admin@celnet.com` /
`password` (config/identity.rs SEED_ADMIN_*), but `ensure_seed_admin` only seeds on a PRISTINE
store — once admin exists its password can be (and here WAS) rotated away from `password`, so the
documented seed no longer logs in. `ben@celnet.com` is a runtime-created user (not in code).

**Login throttle (auth.rs):** after MAX_LOGIN_FAILS consecutive failures an email is locked
~15 min; while locked, login returns a DISTINCT error `"too many failed login attempts; retry in
Ns"` (ResourceExhausted), NOT "invalid email or password" (which = wrong password). The throttle
is **in-memory, keyed by lowercased email** → a server restart clears ALL locks instantly.
Rapid repeated manual attempts self-inflict this lock.

**Reset a password by hand (no CLI exists for it):** password_hash is an **Argon2id PHC string**
`$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>` — RustCrypto `Argon2::default()`. `verify_password`
parses params from the stored string, so ANY valid argon2id PHC (default params) verifies.
Procedure used 2026-08-10:
1. Mint a matching hash with a throwaway cargo bin using crate `argon2 = "0.5"` and
   `Argon2::default().hash_password(pw, &SaltString::generate(&mut OsRng))` (Mac python3.14 was
   broken — no argon2-cffi; the cargo route guarantees identical params). base64 the hash to
   cross SSH safely ($-signs).
2. On box: backup → `cp identity.json identity.json.bak.reset.$(date +%s)`; edit with **python3
   json load/dump** (never sed — it's structured), set the target user's `password_hash`, write
   atomically, `chmod 600`.
3. `/opt/celnet/shared/bin/celnetctl restart` (server only re-reads the file at boot AND the
   restart clears the throttle). CWD stays /home/celnet.
4. Verify with a real WS round-trip: `node --experimental-websocket` (Node 20 on box/Mac),
   `wss://<host>/` (self-signed → `NODE_TLS_REJECT_UNAUTHORIZED=0`), send
   `{"type":"login","email":..,"password":..}`; success returns a capabilities list, failure a
   type:"error". (No grpcurl/websocat/py-websockets available.)
**⚠ CRITICAL — the REAL UAT password is `G00gl312345!`, NOT `password`, and lp-sim depends on it.**
`lp-sim` (and the ESP fix leg) authenticate as **`admin@celnet.com`** every book-poll (~10s) using
the host file **`/home/celnet/.lpsim_pw`** (contents `G00gl312345!`), passed on the daemon cmdline
`--user admin@celnet.com --password G00gl312345!`. If you reset admin's password to anything else,
lp-sim's login FAILS every 10s → floods MAX_LOGIN_FAILS → **admin@celnet.com is permanently
throttle-locked** (browser shows "too many failed login attempts; retry in ~900s" that never
clears). That is exactly what my first reset-to-`password` did — self-inflicted lockout via the sim.
**Correct end state: admin@celnet.com AND ben@celnet.com password = `G00gl312345!`** (matches
`.lpsim_pw`, so the sim's login succeeds and never locks the account). To change admin's password
you must ALSO update `/home/celnet/.lpsim_pw` and restart lp-sim (`lpsimctl restart`), else you
re-trigger the lock. Final verified state 2026-08-10: both accounts log in with `G00gl312345!`;
admin stays unlocked across book-poll cycles. Backups on box: identity.json.bak.reset.1786389482
(the bad password-reset) + a later one restoring G00gl312345!.

Related: box resize/OOM + celnetctl-not-systemd (server needs manual start after reboot) →
[[uat-oom-root-cause-and-swap-fix]].
