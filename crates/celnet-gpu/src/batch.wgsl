// Celnet GPU batch closed-form vanilla kernel — one dispatch prices a large
// BATCH of independent vanilla FX options by the Garman-Kohlhagen closed form,
// one GPU thread per instrument.
//
// This is the GPU-AT-SCALE Workload A (docs/GPU-AT-SCALE-PLAN.md §2.1 / G2): a
// smooth, exact, embarrassingly-parallel batch. Unlike the Monte-Carlo kernels
// (shader.wgsl / scenario.wgsl) this draws NO random numbers — it evaluates the
// analytic price directly — so there is no MC noise, only the f32-vs-f64
// round-off the crate reconciles node-by-node against the f64 CPU oracle and the
// celnet-vanilla (QuantLib-golden-validated) closed form.
//
// WGSL/Metal have no f64, so every instrument is priced in f32. WGSL also has no
// native erf/erfc, so the normal CDF is evaluated in-kernel. The SAME A&S-7.1.26
// erf is mirrored on the CPU oracle side (batch.rs::cpu_batch_with_as_erf, in
// f64) so the reconciliation isolates pure f32 round-off from any algorithmic
// difference: the GPU f32 result is bounded against that *same-algorithm* f64
// oracle by batch.rs::derived_batch_bound. The A&S algorithmic error is a
// SEPARATE bracket — the f64 A&S oracle vs the production libm::erfc path
// (itself QuantLib-golden-gated) — bounded by batch.rs::as_erf_price_bound.
//
// Layout: one input struct per instrument in a read-only storage array; one f32
// price per instrument in a read-write storage array. A single 1-D dispatch of
// ceil(n / WG_SIZE) workgroups covers the whole batch; thread `gid.x` prices
// instrument `gid.x` (guarded by `gid.x < n`).

// Per-instrument inputs (must match the #[repr(C)] BatchInstrument in batch.rs:
// six f32 then one f32 sign = seven f32, 28 bytes; the host pads the array
// stride to a 16-byte multiple via a trailing f32 pad to keep std430 happy).
struct Instrument {
    spot   : f32,   // S_0
    strike : f32,   // K
    vol    : f32,   // sigma (annualized, absolute)
    t      : f32,   // time to expiry in years
    r_dom  : f32,   // domestic continuously-compounded rate
    r_for  : f32,   // foreign continuously-compounded rate
    sign   : f32,   // +1 call, -1 put
    _pad   : f32,   // stride pad to 32 bytes (std430 array alignment)
};

struct Meta {
    n : u32,        // number of instruments
    _p0 : u32,
    _p1 : u32,
    _p2 : u32,
};

@group(0) @binding(0) var<uniform> batch_meta : Meta;
@group(0) @binding(1) var<storage, read> insts : array<Instrument>;
@group(0) @binding(2) var<storage, read_write> prices : array<f32>;

const WG_SIZE : u32 = 256u;
const INV_SQRT_2 : f32 = 0.70710678118654752440;

// Standard-normal CDF in f32 via erf. WGSL has no erf, so use the
// Abramowitz & Stegun 7.1.26 rational-times-Gaussian approximation, whose
// maximum absolute error is 1.5e-7 — at the f32 unit-round-off floor
// (f32::EPSILON ~= 5.96e-8), so it adds at most ~2.5 ULP-scale absolute error to
// Phi, which the derived reconciliation bound accounts for explicitly. The SAME
// approximation is mirrored on the CPU oracle (batch.rs) so the GPU<->oracle
// comparison measures pure f32 arithmetic round-off, while a SEPARATE check
// brackets that oracle against the production libm::erfc Phi within the A&S
// algorithmic-error term.
fn erf_as(x : f32) -> f32 {
    // A&S 7.1.26 is stated for x >= 0; erf is odd, so reflect.
    let s = sign(x);
    let z = abs(x);
    // Coefficients written so the f32 the shader parses is bit-identical to the
    // CPU oracle's (batch.rs::erf_as_oracle) — see that fn's note. The decimals
    // round to exactly the same f32 as the canonical A&S 7.1.26 constants.
    let p = 0.3275911;
    let a1 = 0.2548296;
    let a2 = -0.28449672;
    let a3 = 1.4214138;
    let a4 = -1.4531521;
    let a5 = 1.0614054;
    let tt = 1.0 / (1.0 + p * z);
    let poly = ((((a5 * tt + a4) * tt + a3) * tt + a2) * tt + a1) * tt;
    let y = 1.0 - poly * exp(-z * z);
    return s * y;
}

fn norm_cdf_f32(x : f32) -> f32 {
    return 0.5 * (1.0 + erf_as(x * INV_SQRT_2));
}

@compute @workgroup_size(256)
fn batch_vanilla(@builtin(global_invocation_id) gid : vec3<u32>) {
    let idx = gid.x;
    if (idx >= batch_meta.n) {
        return;
    }
    let it = insts[idx];

    let sqt = sqrt(it.t);
    let vsqt = it.vol * sqt;
    // Garman-Kohlhagen d1/d2 (see celnet-vanilla::aux).
    let d1 = (log(it.spot / it.strike)
        + (it.r_dom - it.r_for + 0.5 * it.vol * it.vol) * it.t) / vsqt;
    let d2 = d1 - vsqt;

    let df_dom = exp(-it.r_dom * it.t);
    let df_for = exp(-it.r_for * it.t);
    let s_disc = it.spot * df_for;
    let k_disc = it.strike * df_dom;

    var px : f32;
    if (it.sign > 0.0) {
        // Call = S e^{-r_f t} Phi(d1) - K e^{-r_d t} Phi(d2)
        px = s_disc * norm_cdf_f32(d1) - k_disc * norm_cdf_f32(d2);
    } else {
        // Put = K e^{-r_d t} Phi(-d2) - S e^{-r_f t} Phi(-d1)
        px = k_disc * norm_cdf_f32(-d2) - s_disc * norm_cdf_f32(-d1);
    }
    // Premium is non-negative; clamp tiny negative f32 round-off at the OTM floor.
    prices[idx] = max(px, 0.0);
}
