# Celnet — Master End-to-End Plan (exceed every competitor, close every gap)

> **SUPERSEDED — historical record (2026-05-30).** This was the GA-era consolidated plan
> (rev 2: 21 crates, 555 tests). Its Gap→closure map (G-A…G-G) is now fully closed and its
> live-status numbers are stale; it is retained as a record of intent, not as current state.
> The live source of truth is: **`docs/POST-GA-ROADMAP.md`** (which explicitly supersedes the
> Gap→closure map for post-GA scope — see its §7), the **`docs/COMPLETION-PROGRAM.md`** 12-wave
> program, the **`docs/POST-COMPLETION-AUDIT.md`** gap audit, and the `GUIDE.md` ledger for
> day-to-day status. Celnet is now functionally complete in-repo (34 crates; `just check`
> 1306/1306). Do not treat the "Where we are" / sequencing below as current.

> The single consolidated execution plan from GA-readiness (rev 2) to a full production GA
> that **out-functions, out-intuits, and out-performs** the incumbents — especially SynOption.
> Source of truth for remaining work *(as of 2026-05-30 — now superseded; see banner above)*;
> each item has an owner crate, a dependency gate, and the
> **validation that proves it done**. Updated as items land (zero-legacy).

## Where we are (verified, 2026-05-30)
21 crates + `gui/`, **555 tests green**, `just check` terminating; QuantLib-gated ~1e-10;
~88% mutation kill (vanilla); 96% core coverage; OSS-clean (wasmtime hard-banned); plugin-host
(wasmi) + trader GUI built; executable competitive parity matrix (15 rows). GA verdict: **GO**
for the pricing platform with one gating caveat (end-to-end latency-under-load proof).

## Gap → competitor closure map
| Gap (GA-READINESS) | Why it matters vs competition | Owner | Validation gate |
|---|---|---|---|
| **G-A Exotics breadth** — TARF / accumulator / target-redemption / pivot / quanto / lookback | SynOption Optimus & Fenics kACE ship these; our LSV+MC+PDE substrate is ready — closes the largest *functional* breadth gap | `celnet-exotics` (+`celnet-golden`) | each payoff QuantLib- or MC-cross-validated; TARF target-redemption/gap-risk + accumulator gearing tested; gated in `celnet-parity` |
| **G-B End-to-end latency under load** (the GA gating caveat) | Converts the microsecond claim from micro-bench to a wire-path p50/p99/p99.9 under sustained streaming load — the headline no competitor publishes | `celnet-bench` (drives the server edge) + CI | HdrHistogram wire-path histogram under load; CI bench-regression gate so latency can't silently regress |
| **G-C WebSocket mirror + live GUI** | Browser/GUI parity with the gRPC contract; turns the GUI from mock-fed to live | `celnet-server` (WS JSON mirror) → `gui/` live seam | WS round-trip = gRPC result; GUI consumes the live edge |
| **G-D Cross-node fleet / scale-out** | Substantiates the IB-portfolio-scale claim beyond per-node headroom | new `celnet-router` (HRW partition map, stateless replica routing, hot-standby, backpressure) | partition-map balance + failover + no-loss handoff tests; throughput vs §1.2 |
| **G-E Mutation + coverage CI gates** | Closes the 54 `celnet-vanilla` solver survivors; makes test-strength a CI gate, not a snapshot | `celnet-vanilla` tests + CI | kill-rate ≥ 95% on vanilla; coverage/mutation thresholds enforced in CI |
| **G-F GPU perf-at-scale** | Live-service batch capability no competitor exposes | `celnet-gpu` | large-batch GPU vs f64-CPU reconciliation + throughput (real-HW numbers are CI/container, honestly scoped) |
| **G-G Live CelNet / FIX integration** | Native trade-lifecycle STP — a structural edge over SaaS/venue incumbents | `celnet-integration` | FIX dialect round-trip + distributor adapter against a simulated estate (live wiring = staging, scoped) |

## Sequencing (respects lane discipline — no two concurrent lanes mutate a crate the other compiles)
- **Wave 1 (parallel, disjoint dirs):** G-A `celnet-exotics`+`celnet-golden` · G-C `celnet-server` WS-mirror · G-D new `celnet-router`.
- **Wave 2 (after Wave 1; vanilla/server now stable):** G-B latency harness + CI gate · G-E close vanilla mutants + CI mutation/coverage gates · G-C GUI live-seam wiring.
- **Wave 3:** G-F GPU batch numbers · G-G FIX/distributor against a simulated estate.
- **GA tag** once G-A/B/C/D/E are green and the latency caveat is closed.

## Honesty boundary
GPU-on-real-hardware (Metal lacks f64; CUDA is CI/container-only) and live-CelNet-estate wiring
are inherently deployment/hardware-dependent: we **build + validate against simulation/CI**
here and flag the real-hardware/live-estate step as a deployment gate, never claimed as done
from this environment.
