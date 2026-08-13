---
name: run-dev-single-entry-point
description: "./run_dev.sh is THE local bring-up (server + lp-sim + FIX RFQ/ESP legs + GUI); it auto-discovers the FIX port, provisions the aggregated book, and sweeps a stale stack on entry."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 8d80fdd4-c0ec-4f58-9709-ea1816fd9391
  modified: 2026-08-13T18:12:49.428Z
---

`./run_dev.sh` at the repo root is the single local dev entry point, added
2026-08-13. It **replaced and deleted** `tools/dev.sh` and `tools/dev-sims.sh` —
do not recreate either.

```
./run_dev.sh                # server + lp-sim + FIX RFQ leg + FIX ESP leg + GUI
./run_dev.sh --skip-build   # cached debug binaries
./run_dev.sh --no-sims      # server + GUI only
./run_dev.sh --demo-edge    # the demo_edge example (what the e2e suites drive)
./run_dev.sh --no-sweep     # do NOT kill a stack already running
```

Defaults: gRPC `127.0.0.1:50551`, ws `127.0.0.1:8081`, GUI `localhost:5173`,
sign in `admin@celnet.com` / `password`, logs under `target/dev/`.

**It provisions two things that otherwise look identical to a broken feed:**
- The FIX acceptor's PORT is operator-created and environment specific, so the
  script ASKS the running server for it over the WS contract (preferring a
  fixed-income venue) instead of hardcoding. On this machine it resolves to
  **9100** (`fi-quote-venue-…`); there is also an Options acceptor `celer` on
  51001.
- An lp-sim provider only streams into a book that lists it as a MEMBER — with no
  such book the feed connects, logs `0 streams` and quotes nothing. The script
  ensures `lp-sim-book` exists with `LP-SIM-01..04`.

Both idempotent.

**It sweeps on ENTRY, not just exit.** Teardown-on-exit cannot help with the
actual failure — a process that did NOT exit cleanly, which leaves ports bound and
the server dying on `Os { code: 48, AddrInUse }`. The sweep kills by PORT
OWNERSHIP (via `lsof`) plus by process IMAGE for the simulators (they bind
nothing), then WAITS for release, because SIGKILL is asynchronous and returning
early re-creates the very race. Same lesson `celnetctl restart` already carries.

**Bash gotcha this exposed:** `[[ $FLAG -eq 1 ]] && do_thing` evaluates false when
the flag is off, and a false statement at top level under `set -e` **exits the
script**. Use `if/then`.

A healthy run reports:
`141 cash bond(s) + 18 Treasury future(s) + 6 OIS curve point(s) + 8 SOFR STIR
contract(s)` then `692 stream(s) over 4 member(s) × 173 instrument(s)`.

Related: [[open-items-dropdowns-esp-tag-futures-roll]].
