---
name: cargo-gate-environment-pitfalls
description: "On this M4, cargo nextest orchestration wedges and freshly-built test binaries first-launch-stall under build churn — use cargo test, keep the machine quiet."
metadata: 
  node_type: memory
  type: project
  originSessionId: ce643053-e1eb-472e-820d-f79a68761a05
---

Two reproducible **environment** failure modes hit hard on 2026-06-08 while gating Celnet
(NOT code bugs — they cost hours before diagnosis):

1. **`cargo nextest` orchestration wedges** on multi-crate (`-p a -p b …`) and
   `--workspace` runs: the per-binary `--list` phase sits at **0% CPU forever**. A direct
   `cargo test` (standard libtest harness, runs binaries in-process) works fine, as does a
   direct binary exec. **Fix: gate with `cargo test`, not `cargo nextest`, when nextest
   stalls.** Single-crate nextest sometimes works; broad scopes reliably hang.

2. **macOS first-launch code-signing stall** (`amfid`/dyld): a *freshly-built* test binary
   can hang at `_dyld_start` (0% CPU) on its first launch when the machine is under heavy
   build churn (multiple sessions compiling). Already-launched binaries run fine. **It
   drains once the machine is quiet** — so quiesce, then the same binary launches instantly.
   Symptom looked identical to a hang; `sample <pid>` showing `_dyld_start` is the tell.

3. **Tight tokio test deadlines flake under load:** `celnet-client` conformance/surface
   tests use `tokio::time::timeout(60s, client.price(...))`; under CPU saturation these blow
   the deadline and report as FAILED (`Elapsed`/"timed out") — **not real regressions.**
   Re-run on a quiet machine to disprove.

4. **STALE prebuilt binary masks current code in live probes (2026-06-12):** harnesses that
   boot the server (`verifyHeadless.ts`, ad-hoc probes) prefer `target/release/celnet-server`
   then `target/debug/…`. A live cross-asset probe produced 4 false "anomalies" (perpetual/
   future-option rejected; equity-barrier/crypto-tarf mis-priced) — all artifacts of a
   **Jun-6 release binary** predating the Jun-11 cross-asset WS routing. `strings <bin> | grep
   perpetual` = 0 and `stat -f '%Sm'` vs the source mtime are the tells. **Rebuild (`cargo
   build -p celnet-server`) and confirm binary mtime > source mtime BEFORE trusting a live
   probe** — re-probing against the fresh binary gave `surprises=0` matching the source.

5. **`celnet-gpu` tests contend the single Metal GPU under full nextest parallelism
   (2026-06-26):** a `--workspace --no-fail-fast` run timed out **17 tests — ALL in
   `celnet-gpu`** (batch/gpu/path/pathwise/scenario), tripping the `terminate-after = 4×30s
   = 120s` deadline in `.config/nextest.toml`. They are NOT regressions: serialized,
   `cargo nextest run -p celnet-gpu --test-threads 1` → **35/35 pass** (17 "slow" but none
   timed out). **Run the GPU crate with `--test-threads 1`** (one GPU job at a time) — the
   default profile serializes `engine`/`replog-consensus` but NOT gpu. Same family as the
   tokio-conformance timeouts (#3): heavy resource-bound tests need exclusive access here.

Root cause of all three: **this single M4 cannot run concurrent heavy cargo builds** (mesh
sessions + churn). The lesson that recurs: **serializing is faster than parallelizing** for
heavy Rust gates here — which is exactly what [[w2-parallel-session-collision]] and the
§4.1 "compute courtesy" board rule encode. Keep cargo stages serial; only toolchain-disjoint
(Node/Excel/GUI) lanes truly parallelize.
