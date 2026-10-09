---
name: run-dev-single-entry-point
description: "./run_dev.sh is THE local bring-up (server + lp-sim + FIX RFQ leg + FIX RFS stream leg + GUI); it provisions BOTH FI acceptors + a desk + the book, supplies the RFS service credential, and sweeps a stale stack on entry."
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
./run_dev.sh                # server + lp-sim + FIX RFQ leg + FIX RFS stream leg + GUI
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
  **9100** (`fi-quote-venue-…`); there is also an Options acceptor `celnet` on
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


## The two FI venues are SEPARATE acceptors — fixed 2026-08-13

The server gates each dialect on the acceptor's own kind (`rates_intent_for_kind`):
`on_market_data_request` returns immediately unless the acceptor is
`FixedIncomeStream`; `on_quote_request` returns immediately when it IS. A leg
pointed at the wrong kind is dropped with **no reject and no log line**.

The script used to discover ONE acceptor and point both legs at it, so one of the
two flows was always silently dead while the stack looked healthy. It now ensures
one venue of EACH kind — `dev-fi-rfq` (kind 1) and `dev-fi-rfs` (kind 2),
idempotent — and gives each leg its own port. It CREATES a missing venue rather
than skipping the leg.

**The streaming leg had never actually run from this script**: it downloads its
instrument list over gRPC first, so it needs a service credential, and without one
it exits instantly with `no RFS service credential configured`. The client refuses
a password on argv by design and prefers a `0600` file, so the script writes
`target/dev/.fixsim_pw` (gitignored) and passes `FIXSIM_USER` /
`FIXSIM_PASSWORD_FILE`.

**Desk matters:** a venue whose desk has no user on it has its RFQ/deal
notifications DROPPED (the server warns at boot). Provisioning prefers a desk the
sim user belongs to (`login.user.desk_ids` / `all_desks`).

Sim client flag is now `--asset rfs` (was `esp`) — see
[[open-items-dropdowns-esp-tag-futures-roll]] for why that venue is RFS.

## What the fleet actually streams (measured, not assumed)

`lp-sim` already covers futures — `lpsim::quotable_lines` builds
**141 cash bonds + 18 Treasury futures + 6 OIS curve points + 8 SOFR STIR
contracts** = 173 priceable, 692 streams over 4 members.

## Open, characterised not guessed

- **RFS lifts never fill.** 16 subscribes produced 16 published snapshots in
  `pricing.log`, but `orders.log` and `executions.log` are EMPTY — no
  NewOrderSingle reaches the order path. Not yet diagnosed.
- The RFS leg's top-N picks `ust-2y-note` / `acme-5y-corp`, the two instruments
  the server warns are TRADEABLE BUT UNQUOTABLE.
- The MD venue's bond arm keys on `SecurityType(167)=BOND`, so **futures are not
  streamable over the RFS venue** even though lp-sim quotes them — a platform
  capability gap, not a script gap.
