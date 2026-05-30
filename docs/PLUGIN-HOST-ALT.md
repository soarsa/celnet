# Celnet Plugin-Host — Alternative Architecture (wasmtime-blocked unblock plan)

> **Status:** research + design (no code in this doc). Decision is ADR-grade and ready to
> implement. This supersedes the wasmtime-based plan recorded in `docs/ARCHITECTURE.md` §6
> while keeping the **frozen `celnet-plugin-api` contract unchanged** (Rust traits +
> `wit/celnet.wit`). The contract was deliberately runtime-agnostic; only the *host runtime*
> choice changes here.
>
> **Date:** 2026-05-30. **Owner workstream:** WS-G (`celnet-plugin-host`).
> **One-line decision:** ship a **tiered host** — a native trait registry for first-party
> models (the hot path) + **`wasmi` (pure-Rust, fuel-metered) as the untrusted sandbox**,
> with **`stabby`-loaded signed native `.so`** as a *trusted-partner* tier and **OS-process
> isolation (Landlock/seccomp)** as an optional Linux defense-in-depth ring. **`wasmtime`,
> `wasmer-singlepass`, and `wasm-bridge`-style component shims are not adopted.**

---

## 1. Why this document exists

`celnet-plugin-host` is the last GA-blocking gap (`docs/CAPABILITIES-VS-COMPETITION.md`).
The original design (ARCHITECTURE §6.1) named **`wasmtime 45` + Component Model + WASI 0.2.x**
as the untrusted-plugin runtime. Guardrail 7 (OSS-only, advisory-clean) and our `cargo-deny`
gate **block wasmtime**: as of May 2026 it carries multiple open RustSec advisories, several
of them sandbox-relevant. We must therefore pick a *different* untrusted-execution substrate
without touching the contract that the engine, the native registry, and the WIT mirror all
depend on.

The plugin host must let desks run **custom vol models, payoffs, and calibrations in-engine**,
keeping their IP private, while satisfying five non-negotiables:

| # | Requirement | What it means concretely |
|---|---|---|
| R1 | **Determinism** | Identical inputs ⇒ **bit-identical** output, replayable forever; the replay harness (fixed market-data snapshot → identical price/Greeks) must pass. |
| R2 | **Capability sandbox** | No ambient authority. A plugin can touch *only* the market-data/pricing primitives the host explicitly grants — no filesystem, clock, network, RNG, or threads unless seeded/granted. |
| R3 | **Resource metering** | A per-call **compute/latency budget** that deterministically interrupts a runaway plugin (the budget *is* the plugin's SLA). Never stalls the pinned hot core. |
| R4 | **OSS + `cargo-deny`-clean** | MIT/Apache/BSD/etc.; **zero** open RustSec advisories; license in our allow-set. |
| R5 | **Implements `celnet-plugin-api`** | Same `PricingModel` / `SmileModel` / `Calibration` shape native models use, so first-party and user plugins are interchangeable behind **one** `ModelRegistry`. |

---

## 2. The wasmtime block (evidence)

`wasmtime` (and everything that vendors its runtime, including its in-tree **Pulley**
pure-Rust interpreter, which ships *inside* the `wasmtime` crate and so inherits its advisory
surface) is rejected on **R4**. May-2026 RustSec advisories against `wasmtime`/`wasmtime-wasi`:

| Advisory | Class | Note |
|---|---|---|
| RUSTSEC-2026-0006 | **Sandbox escape-class** | Segfault / *out-of-sandbox load* via `f64.copysign` on x86-64 (CVE-2026-24116). Directly defeats R2. |
| RUSTSEC-2026-0149 | **High (CVSS 7.5)** | `wasi:filesystem` `path_open(TRUNCATE)` bypasses `FilePerms::WRITE` host restriction — capability containment failure. |
| RUSTSEC-2026-0092 / -0093 | Component-Model memory safety | Panic / heap OOB read transcoding UTF-16 component-model strings — the exact Component-Model path the old plan relied on. |
| RUSTSEC-2026-0021 | DoS panic | `wasi:http/types.fields` excessive-fields panic. |

Even pinning to a patched release leaves us **chasing a high-cadence advisory stream on a
JIT with a native-codegen attack surface**, which is the wrong risk posture for a
mission-critical pricing core. We want the *smallest, audited, advisory-clean* substrate.

---

## 3. Candidate evaluation

Scored against R1–R5. "deny-vet" = the exact `cargo-deny` / `cargo-audit` check to run before
adoption.

### 3.1 `wasmi` — pure-Rust Wasm interpreter — **CHOSEN sandbox tier**

- **What it is.** `wasmi-labs/wasmi` **v1.0.9** (Feb 2026), a pure-Rust, `no_std`-capable
  WebAssembly **interpreter** (no JIT, no native codegen). Maintained, sponsored by the
  Stellar Development Foundation; **two independent security audits** (SRLabs 2023 on 0.31,
  Runtime Verification 2024 on 0.36–0.38). Used in blockchain consensus engines, where
  *deterministic, bounded, sandboxed* execution of untrusted Wasm is the entire job — exactly
  our threat model.
- **R1 Determinism.** Design goal #1 ("simple, correct and **deterministic** execution"). As a
  pure interpreter with no platform-specific codegen, the same module + same fuel interrupts at
  the same instruction on every target, including our aarch64-apple-darwin dev box and Linux CI.
  We pin float behavior at the boundary (see §6): f64-only, NaN-canonicalize host↔guest, and the
  contract already forbids `==`/`NaN` compares and routes transcendentals through
  `celnet_core::math` (libm) — so two engines replay bit-identically.
- **R2 Capability sandbox.** Linear-memory isolation is the Wasm guarantee. wasmi's `Linker`
  exposes **only** the host functions we register; with **no WASI** linked, a guest has **zero**
  ambient authority — no clock, no files, no net, no RNG, no threads. We register a tiny,
  audited host-import surface (market-data lookups, `celnet_core` math shims if desired).
- **R3 Resource metering.** **Built-in fuel metering** + resumable calls (guest *yields* to the
  host when fuel runs out). Fuel is fully deterministic (R1-compatible), unlike wall-clock
  epoch interruption. We set a per-call fuel budget = the plugin's compute SLA; exhaustion
  returns `PluginError::DidNotConverge("fuel budget exhausted")` deterministically. We *also*
  wrap the call in an off-hot-core OS watchdog as belt-and-suspenders (the interpreter loop is
  cooperatively interruptible).
- **R4 OSS/advisory.** **MIT OR Apache-2.0** (in our allow-set). **No RustSec advisories**
  against `wasmi` in the database as of May 2026. Pure-`#![forbid(unsafe_code)]`-leaning,
  audited — minimal attack surface.
- **R5 Contract.** Hosts **core Wasm modules** (Component Model is WIP / not supported). The
  `celnet-plugin-api` contract is `Copy`-POD-in / `Copy`-POD-out (`vanilla-inputs`, `greeks`,
  `option-type` are flat numeric records/enums; `calibrate` takes a `list<calibration-target>`),
  which lowers cleanly to a **core-module ABI** — pass scalars in registers/stack, marshal the
  one list through guest linear memory with an explicit `(ptr,len)` calling convention the host
  controls. We do **not** need the Component Model to satisfy the contract.
- **Cost.** ~10×–50× slower than a JIT. **Acceptable by design:** ARCHITECTURE §6.2 already
  reserves Wasm for *user-supplied / less-hot-path* models; ultra-hot per-tick first-party
  models run native (Tier 0). For a calibration or a bespoke smile evaluated per-quote (not
  per-tick), interpreter latency sits comfortably inside the fuel budget.
- **deny-vet.** `cargo deny check advisories bans licenses` on a scratch crate adding
  `wasmi = "1"`; confirm: (a) zero advisories on `wasmi` + its small dep tree (`spin`,
  `wasmparser` — itself a Bytecode-Alliance crate, check its advisory line separately),
  (b) every transitive license ∈ allow-set, (c) `cargo tree -e no-dev` surface is small and JIT-free.

### 3.2 `wasmtime` (+ Pulley) — **REJECTED (R4)**

Open 2026 advisories (§2), including an out-of-sandbox-load class and a High WASI bypass.
JIT + large native-codegen surface = wrong risk posture. Pulley does not help: it lives in the
`wasmtime` crate, so adopting it pulls the advisory surface and the deny gate still fails.

### 3.3 `wasmer` — **REJECTED (R4 license + risk surface)**

Core `wasmer` is MIT, but the high-perf **Singlepass backend relicensed to BUSL-1.1** (a
non-OSS source-available licence **outside** our `deny.toml` allow-set) — adopting Wasmer
realistically pulls toward Singlepass for perf, and the Cranelift backend reintroduces a JIT
attack surface comparable to wasmtime's. Larger, more entangled dependency tree than wasmi for
no determinism/security advantage over a pure interpreter. **deny-vet would reject** any tree
that resolves the BUSL Singlepass crate.

### 3.4 Native trait-registry + signed `.so` via `stabby` — **CHOSEN, but trusted-only tier**

- **What it is.** `stabby` **72.x** (ZettaScale, actively maintained May 2026; `abi_stable` is
  effectively unmaintained and remains banned) gives a **stable Rust ABI** so a partner's
  `.dylib`/`.so` exporting `celnet-plugin-api` trait objects loads into the engine at full
  native speed.
- **R1/R2/R3:** **NONE.** Native code has no sandbox, no determinism guarantee, and no
  metering — a faulty plugin can corrupt memory or stall the core. Therefore admissible **only**
  for **code-signed, trusted-publisher-allowlisted** partners (and our own first-party models),
  **never** for untrusted user code.
- **R4:** MIT/Apache; `stabby` is advisory-clean — **deny-vet** `stabby = "72"` for advisories +
  license, and confirm the loader uses `#[forbid(unsafe_code)]` *except* the audited `dlopen`
  shim (`libloading`, advisory-clean) gated behind the signature check.
- **R5:** Implements the contract directly as native trait objects — *identical* shape to Tier 0.

### 3.5 OS-process isolation (Landlock + seccomp-bpf, e.g. `sandlock`/`hakoniwa`) — **OPTIONAL Linux defense-in-depth ring, not the primary mechanism**

- **What it is.** Run a plugin in a child process confined by **Landlock** (filesystem/net/IPC
  scope) + **seccomp-bpf** (syscall allow-list), communicating over a fixed IPC channel.
- **R1 Determinism:** a separate process does **not** make computation deterministic — float/RNG/
  scheduling vary; you still need a deterministic *compute* layer (i.e. wasmi) inside it.
- **R2:** Strong *kernel-enforced* capability containment — an excellent **second ring** around
  the wasmi sandbox for the highest-distrust deployments.
- **R3:** Coarse (cgroup v2 CPU/mem, wall-clock kill) — not the fine, deterministic per-call
  budget fuel gives.
- **R4:** `landlock`, `sandlock`/`sandlock-core`, `hakoniwa` are OSS — but **Linux-only**
  (kernel ≥ 5.13/5.19). **Not available on our M4/macOS dev target**, so it cannot be the
  primary portable mechanism; it is a CI/Linux-prod hardening option.
- **Decision.** Keep as an **optional outer ring** (config-gated, Linux only) wrapping the wasmi
  host process for paranoid multi-tenant deployments; do not make it load-bearing for R1–R3.

### 3.6 2026 capability/component-model options — **WATCH, not adopt**

- **`jco`/`wasmtime`-component path:** the only mature Component-Model host today is wasmtime's
  — advisory-blocked (§2). WASI 0.3 (native async) lands ~Feb 2026 but on wasmtime.
- **wasmi Component Model:** listed as a WIP feature; **not** ready. We design the host so that
  *if/when* wasmi ships a clean Component-Model layer we can lift our existing WIT `world`
  directly — but we **build on core modules now** and do not depend on experimental features
  (consistent with ARCHITECTURE §6.3's "build on stable" rule).

### 3.7 Scorecard

| Option | R1 det. | R2 cap-sandbox | R3 metering | R4 OSS/clean | R5 contract | Verdict |
|---|---|---|---|---|---|---|
| **wasmi** | ✅ pure interp | ✅ no-WASI Linker | ✅ fuel | ✅ MIT/Apache, **0 advisories** | ✅ core-module ABI | **Sandbox tier (chosen)** |
| wasmtime/Pulley | ✅ | ⚠️ (escape advisory) | ✅ fuel | ❌ open 2026 advisories | ✅ CM | **Rejected** |
| wasmer | ✅ | ✅ | ✅ | ❌ Singlepass BUSL / JIT surface | ✅/⚠️ | **Rejected** |
| stabby `.so` | ❌ | ❌ | ❌ | ✅ | ✅ | **Trusted tier only** |
| Landlock/seccomp | ❌ (alone) | ✅ kernel | ⚠️ coarse | ✅ but Linux-only | n/a | **Optional outer ring** |

---

## 4. Decision — the tiered host

```
                         ┌─────────────────── celnet-engine (pinned hot core) ───────────────────┐
                         │                         one  ModelRegistry  (celnet-plugin-api)        │
                         └───────────────┬───────────────────┬───────────────────┬───────────────┘
                                         │                   │                   │
                   Tier 0 (hottest)      │  Tier 1 (trusted) │  Tier 2 (UNTRUSTED, default for users)
              native compiled-in models  │  signed .so/.dylib│  wasmi sandbox  (+ optional Tier 3 ring)
                 `inventory` registry     │  via `stabby` 72  │  fuel-metered, no-WASI Linker
                 full native speed        │  allowlist+sign   │  deterministic replay harness
                                                                     └─ optional Linux outer ring:
                                                                        child process + Landlock + seccomp
```

- **Tier 0 — native trait registry (first-party, hot path).** `inventory`-collected
  `dyn PricingModel`/`SmileModel`/`Calibration`. Unchanged from the current plan; this is where
  per-tick models live. No ABI/sandbox concern.
- **Tier 1 — trusted native dynamic (`stabby`).** Code-signed, publisher-allowlisted partner
  `.so`/`.dylib`. Native speed, **no sandbox** → trusted only.
- **Tier 2 — untrusted sandbox (`wasmi`).** **The default for any user/third-party model.**
  Pure-Rust interpreter, fuel-metered, no-WASI capability Linker, deterministic replay. This is
  the wasmtime replacement.
- **Tier 3 — optional OS ring (Linux).** Landlock+seccomp child process around Tier 2 for
  highest-distrust multi-tenant prod. Config-gated; never required for correctness.

All four tiers present the **same `celnet-plugin-api` shape** behind **one registry**, so
the engine routes by `ModelDescriptor` and never knows or cares which tier served a call (R5).

---

## 5. Build plan (`celnet-plugin-host` + guest crate)

> All crates flat in the workspace per current layout. Gates: `just check` green; the four
> WS-G gates from ROADMAP §7 (sandbox-escape/capability-denial, fuel-exhaustion bounded
> runtime, replay bit-identity, first-party-vs-sandbox interchangeability).

1. **deny-vet first (blocking).** Scratch-crate `cargo deny check` for `wasmi = "1"`,
   `stabby = "72"`, `libloading` (and `wasmparser` transitive). Record results in an ADR note.
   **Do not add a dependency before this passes.** (`cargo-deny` already wired in `deny.toml`.)
2. **`celnet-plugin-host` crate.**
   - `registry` — the unified `ModelRegistry` impl that fans Tier 0/1/2(/3) entries into one
     lookup keyed by `ModelId`. Tier 0 via `inventory`.
   - `native_dyn` (Tier 1) — `stabby`-typed loader behind a signature/allowlist check; feature-
     gated; the only `unsafe` (the `dlopen` shim) is isolated and audited.
   - `wasm` (Tier 2) — wasmi `Engine` with `Config` fuel-on; a **no-WASI** `Linker` exposing only
     an explicit, audited host-import set; a `Store` per call (or pooled, reset deterministically);
     ABI marshalling that lowers the WIT records to the core-module `(ptr,len)` convention.
   - `replay` — deterministic replay harness: fixed market-data snapshot + fixed fuel ⇒ assert
     bit-identical price/Greeks across two independent runs and across platforms (CI: macOS+Linux).
3. **`celnet-plugin-guest` crate.** Guest-side bindings + a tiny SDK so a desk writes a model
   in Rust, compiles to `wasm32-unknown-unknown` (a *core module*, not a component), and the host
   loads it. Ship the **reference flat-vol plugin** (mirrors `example.rs`) as the conformance
   fixture proving Tier-2 == Tier-0 numerically.
4. **Gates / tests (no mocks).**
   - *Capability denial:* a guest that tries any non-granted import fails to link / traps —
     proves R2.
   - *Fuel exhaustion:* an infinite-loop guest is interrupted within budget and returns the
     deterministic error — proves R3, bounded runtime.
   - *Replay bit-identity:* same inputs+fuel ⇒ identical bits, twice, cross-platform — proves R1.
   - *Interchangeability:* the flat-vol model via Tier 0 (native) and Tier 2 (wasm) produce
     identical prices/Greeks through one registry — proves R5.
5. **Docs sync (guardrail 10).** Update ARCHITECTURE §6.1/§6.3 to name **wasmi** (not wasmtime)
   as the untrusted runtime and to note "core modules, not Component Model, today"; update
   ROADMAP §7 WS-G deliverables; flip CAPABILITIES-VS-COMPETITION's "top GA gap"; update the
   header comment in `wit/celnet.wit` (host is wasmi; bindings target core-module ABI). Re-index
   codebase-memory after the crates land.

---

## 6. Determinism contract for the wasmi tier (host obligations)

- **f64 canonical scalar**, matching the Rust/WIT policy; the host canonicalizes NaN on every
  value crossing the boundary so a guest cannot smuggle a non-canonical NaN bit pattern into a
  replay.
- **No FMA contraction divergence:** guests route transcendentals through host-provided
  `celnet_core::math` (libm) shims (or compile libm into the guest) so `exp`/`ln`/`erf` are
  bit-identical to native models — the contract already mandates this.
- **No non-deterministic imports:** the Linker exposes **no** clock/RNG/thread/file/net. Any
  randomness a model needs is a *seeded, host-supplied* deterministic stream.
- **Fuel = SLA:** a fixed per-call fuel budget; exhaustion is a deterministic
  `PluginError::DidNotConverge`. Fuel accounting is part of the replay invariant (same fuel in ⇒
  same interruption point).

---

## 7. ADR — ADR-00NN: Untrusted plugin runtime = wasmi (supersedes wasmtime)

- **Context.** Guardrail 7 + `cargo-deny` block `wasmtime` (open 2026 RustSec advisories,
  incl. an out-of-sandbox-load class). The `celnet-plugin-api` contract is runtime-agnostic and
  stays frozen; only the untrusted-execution substrate must change.
- **Decision.** Adopt a **tiered host**: native `inventory` registry (Tier 0) + `stabby`-loaded
  signed native dynamic libs for trusted partners (Tier 1) + **`wasmi` pure-Rust fuel-metered
  interpreter for all untrusted user models** (Tier 2) + optional Linux Landlock/seccomp child-
  process ring (Tier 3). Build on **core Wasm modules**, not the Component Model (wasmi CM is
  WIP; we don't depend on experimental features). **Reject** wasmtime/Pulley (advisories), and
  **reject** wasmer (Singlepass BUSL + JIT surface).
- **Consequences.** (+) Advisory-clean, audited, deterministic, sandboxed, OSS — passes the deny
  gate and all four WS-G gates; the contract and WIT mirror are unchanged. (−) Interpreter is
  ~10–50× slower than a JIT, accepted because untrusted Wasm is reserved for non-per-tick models
  (Tier 0 carries the hot path). (−) We hand-roll the WIT→core-module ABI lowering instead of
  using the Component Model; revisit if wasmi ships an advisory-clean Component Model.
- **Crates to vet before building (deny-vet checklist):** `wasmi` (^1), `stabby` (^72),
  `libloading` (for the Tier-1 `dlopen` shim), `wasmparser` (wasmi transitive); Linux-only
  outer ring: `landlock` + a seccomp crate (`seccompiler`/`sandlock-core`). Adopt none before
  `cargo deny check advisories bans licenses` is green for each.

---

## 8. Sources

- RustSec — wasmtime advisories index and 2026 entries (RUSTSEC-2026-0006 out-of-sandbox load /
  CVE-2026-24116; -0149 WASI path_open WRITE bypass; -0092/-0093 component-model UTF-16 OOB;
  -0021 wasi:http fields panic): <https://rustsec.org/packages/wasmtime.html>,
  <https://rustsec.org/advisories/RUSTSEC-2026-0006.html>,
  <https://rustsec.org/advisories/RUSTSEC-2026-0149.html>.
- wasmi — repo, features, license, audits, v1.0.9 (Feb 2026):
  <https://github.com/wasmi-labs/wasmi>, <https://lib.rs/crates/wasmi>,
  <https://wasmi-labs.github.io/blog/posts/wasmi-v1.0/>.
- Wasmtime Pulley interpreter (ships in the wasmtime crate; ~10× slowdown):
  <https://docs.wasmtime.dev/examples-pulley.html>.
- Wasmer Singlepass BUSL-1.1 relicensing: <https://wasmer.io/posts/singlepass-relicensing>.
- stabby stable ABI (ZettaScaleLabs, v72.x, maintained 2026):
  <https://github.com/ZettaScaleLabs/stabby>, <https://docs.rs/crate/stabby/latest>.
- Landlock/seccomp Rust sandboxing (sandlock, hakoniwa, rust-landlock; Linux-only):
  <https://landlock.io/rust-landlock/landlock/>, <https://lib.rs/crates/hakoniwa>,
  <https://arxiv.org/html/2605.26298v1>.
- WASI Component Model / 0.2.x vs core modules, WASI 0.3 (~Feb 2026) status:
  <https://component-model.bytecodealliance.org/>, <https://wasi.dev/roadmap>.
