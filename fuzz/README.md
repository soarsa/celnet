# celnet-fuzz

Coverage-guided fuzz harness for Celnet's pricing core.

This is a **standalone crate**, intentionally **not** a member of the root
workspace (it carries its own `[workspace]` table). That keeps
`cargo build --workspace` and the cross-platform CI gate on the pinned
**stable** toolchain (1.96.0) and free of the nightly-only libFuzzer sanitizer
runtime. The fuzzers depend on the product crates **by relative path** and never
re-implement any product logic.

## Targets

| Target           | Under test                                  | Asserted contract                                    |
|------------------|---------------------------------------------|------------------------------------------------------|
| `vanilla_inputs` | `celnet_vanilla::price` + `::greeks`        | no panic; price ≥ 0 and finite; all 13 Greeks finite |

The `vanilla_inputs` target uses `arbitrary` to fold raw fuzzer bytes into a
**valid-but-adversarial** `VanillaInputs` in the Garman-Kohlhagen domain
(strictly positive spot/strike/vol/time, finite bounded rates), spending the
fuzzing budget on in-domain corners (deep ITM/OTM, near-zero / century
expiries, micro / macro vols, large rate spreads) rather than trivially-rejected
garbage.

## Running (Linux, nightly)

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz run vanilla_inputs -- -max_total_time=120
```

This is wired as a dedicated **Linux-nightly** CI job — the `fuzz` job in
`.github/workflows/ci.yml` (see also `docs/HARDENING.md`) — which installs
`cargo-fuzz` on nightly and runs `vanilla_inputs` time-boxed (a 60 s smoke
budget) on every push/PR. It is deliberately not part of the stable
cross-platform gate and is not required to build under stable.

## License / dependency checking

This crate sets `license = "LicenseRef-Proprietary"` only because it is an
internal, **never-published** harness (`publish = false`, version `0.0.0`); it
ships no product code. Because it carries its own `[workspace]` table it is
**excluded from the root workspace** and therefore from the workspace
`cargo-deny` license/advisory gate that runs in the stable CI lane. Its
dependency set is intentionally tiny and pinned in `Cargo.toml`
(`libfuzzer-sys`, `arbitrary`, `libm`, plus the two product crates by path); it
is license/advisory-checked on demand by running `cargo deny check` from inside
`fuzz/` (all deps are permissively licensed: MIT/Apache-2.0/MIT-OR-Apache-2.0).
