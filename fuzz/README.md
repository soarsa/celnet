# celnet-fuzz

Coverage-guided fuzz harness for Celnet's pricing core **and the untrusted-input
byte parsers** (the real socket/disk attack surface).

This is a **standalone crate**, intentionally **not** a member of the root
workspace (it carries its own `[workspace]` table). That keeps
`cargo build --workspace` and the cross-platform CI gate on the pinned
**stable** toolchain (1.96.0) and free of the nightly-only libFuzzer sanitizer
runtime. The fuzzers depend on the product crates **by relative path** and never
re-implement any product logic.

## Targets

| Target                | Under test                                                            | Asserted contract                                                       |
|-----------------------|----------------------------------------------------------------------|------------------------------------------------------------------------|
| `vanilla_inputs`      | `celnet_vanilla::price` + `::greeks`                                  | no panic; price ≥ 0 and finite; all Greeks finite                      |
| `replog_log_entry`    | `celnet_replog::LogEntry::decode`                                     | no panic; `Ok`-or-typed-`Err`; payload bounded by input; round-trips   |
| `replog_snapshot`     | `Snapshot::decode` + `BookState::decode` + `BookUpdate::decode`       | no panic; typed-`Err`; `count*16` overflow guard holds; round-trips    |
| `replog_wire_message` | `celnet_replog::Message::decode` (the Raft RPC frame)                 | no panic; typed-`Err`; entry/snapshot bounded by input; round-trips    |
| `journal_recover`     | `celnet_journal::Journal::open` + `replay` (torn-tail recovery)       | no panic; heal-or-typed-`Err`; recovered len ≤ input; bounded alloc    |
| `proto_convert`       | prost decode of the wire contract + `convert.rs` `TryFrom` mapping    | no panic; `prost` `Ok`-or-`Err`; every domain `TryFrom` is total       |
| `fix_frame_decode`    | `celnet_fix` `FrameReader` (socket delimiter) + `FrameCursor::parse`  | no panic; typed-`Err`; encoder/decoder agreement; 1-byte corruption + truncation rejected; delimiter bounded, terminating, trailer-terminated |

### `vanilla_inputs` — in-domain pricing corners

Uses `arbitrary` to fold raw fuzzer bytes into a **valid-but-adversarial**
`VanillaInputs` in the Garman-Kohlhagen domain (strictly positive
spot/strike/vol/time, finite bounded rates), spending the fuzzing budget on
in-domain corners (deep ITM/OTM, near-zero / century expiries, micro / macro
vols, large rate spreads) rather than trivially-rejected garbage.

### The untrusted-input decode targets

These six feed **arbitrary adversarial bytes** straight into the hand-rolled
byte parsers that consume **socket and disk** input — the genuine external attack
surface. The per-target contract is: **no panic / no UB / Ok-or-typed-Err** on
any input bytes (never an `unwrap`/`expect` on an attacker-controlled length),
**bounded allocation** (a crafted length field can never amplify the allocation
past the input), and — on the accepted path — **decode is a sound inverse of
encode** (re-encoding round-trips). `journal_recover` additionally writes the
arbitrary bytes to a real temp file and drives the actual `open`/scan/torn-tail
recovery code, asserting recovery only ever *shrinks* the file.

`fix_frame_decode` covers the **only parser fed bytes by an external
counterparty** (LP / client FIX sessions over TCP) in three phases per input:
(1) the raw bytes into `framing::FrameCursor::parse`, with the accepted-path
`(tag, value)` sequence re-encoded by the real `FrameEncoder` and re-parsed
(round-trip; byte-identity is not asserted because leading-zero tags re-encode
canonically); (2) a structure-aware phase that folds the bytes via `arbitrary`
into in-domain fields laid out by the real encoder — the output must parse
(encoder/decoder agreement, a permanent foothold past the mod-256 checksum
wall), and a single-byte XOR or strict truncation must be *rejected*; (3) the
same bytes through the real socket delimiter (`transport::FrameReader`) under
adversarial chunk sizes (splitting the `10=NNN<SOH>` trailer across reads),
asserting termination, no invented bytes, well-formed trailers, and that
delimit→parse never panics.

The decode targets are seeded automatically by the unit-test vectors baked into
each decoder's own `#[cfg(test)]` module (round-trip / flipped-byte / truncated
cases in `celnet-replog` and `celnet-journal`); cargo-fuzz also persists a corpus
under `fuzz/corpus/<target>/` as it discovers new coverage. The corpus dirs are
git-ignored (build artefacts); the *property* is what is durable, asserted both
here and (see below) on the stable gate.

## Running (Linux, nightly)

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
cd fuzz
# Any single target:
cargo +nightly fuzz run vanilla_inputs      -- -max_total_time=120
cargo +nightly fuzz run replog_log_entry    -- -max_total_time=120
cargo +nightly fuzz run replog_snapshot     -- -max_total_time=120
cargo +nightly fuzz run replog_wire_message -- -max_total_time=120
cargo +nightly fuzz run journal_recover     -- -max_total_time=120
cargo +nightly fuzz run proto_convert       -- -max_total_time=120
cargo +nightly fuzz run fix_frame_decode    -- -max_total_time=120
```

This is wired as a dedicated **Linux-nightly** CI job — the `fuzz` job in
`.github/workflows/ci.yml` (see also `docs/HARDENING.md`) — which installs
`cargo-fuzz` on nightly and runs every target time-boxed (a smoke budget) on
every push/PR. It is deliberately not part of the stable cross-platform gate and
is not required to build under stable.

## The same property is also gated on the **stable** `just check` lane

Because the fuzz crate is nightly-only and excluded from the workspace gate, the
no-panic / Ok-or-typed-Err / bounded-allocation property of the untrusted
decoders is **also** asserted with `proptest` inside `just check`, so it holds
regardless of whether the nightly fuzz lane ran:

* `crates/celnet-replog/tests/decode_fuzz.rs` — throws adversarial byte buffers
  (uniform-random + structurally-valid-but-corrupted frames) at `LogEntry`,
  `Message`, `Snapshot`, `BookState`, `BookUpdate` decode.
* `crates/celnet-journal/tests/decode_fuzz.rs` — writes adversarial bytes to a
  temp file and drives `Journal::open` + `replay`, asserting heal-or-typed-Err
  and that recovery only shrinks the file.
* `crates/celnet-fix/tests/codec_roundtrip.rs` — throws arbitrary byte buffers
  at `FrameCursor::parse` (no-panic), asserts single-byte corruption of a valid
  frame is rejected, and runs the malformed-frame battery, alongside the typed
  message round-trip proptests.

Coverage-guided fuzzing (this crate) and randomized property testing (the stable
gate) are complementary; the stable gate is the one that *blocks* a merge.

## License / dependency checking

This crate sets `license = "LicenseRef-Proprietary"` only because it is an
internal, **never-published** harness (`publish = false`, version `0.0.0`); it
ships no product code. Because it carries its own `[workspace]` table it is
**excluded from the root workspace** and therefore from the workspace
`cargo-deny` license/advisory gate that runs in the stable CI lane. Its
dependency set is intentionally tiny and pinned in `Cargo.toml`
(`libfuzzer-sys`, `arbitrary`, `libm`, `prost`, `tokio`, plus the product crates
under test by path: `celnet-types`, `celnet-vanilla`, `celnet-replog`,
`celnet-journal`, `celnet-proto`, `celnet-fix`); it is license/advisory-checked on demand by
running `cargo deny check` from inside `fuzz/` (all deps are permissively
licensed: MIT/Apache-2.0/MIT-OR-Apache-2.0).
