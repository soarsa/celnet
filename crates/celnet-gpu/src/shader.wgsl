// Celnet GPU Monte-Carlo vanilla pricer (single-asset GBM) — WGSL compute shader.
//
// One workgroup-cooperative pass:
//   1. Each invocation draws its path's standard normal from the SAME
//      counter-based Philox-4x32-10 stream as the Rust CPU oracle (identical
//      integer arithmetic; only the int->float conversion is f32 here vs f64 on
//      the CPU — the documented reconciliation bound).
//   2. Evolves ln S_T = ln S_0 + (mu - 0.5*sigma^2)*T + sigma*sqrt(T)*Z and
//      evaluates the undiscounted vanilla payoff max(sign*(S_T - K), 0).
//   3. Reduces payoff and payoff^2 per workgroup with a *tree reduction in
//      shared memory* (no float atomics — WGSL atomics are integer only), then
//      each workgroup writes its two partial sums to a per-group output slot.
// The host finishes the reduction across workgroups with the same pairwise
// (tree) sum the CPU backend uses, keeping the accumulation order fixed.
//
// WGSL has no f64; all path math is f32 by design (Metal/WebGPU constraint, see
// docs/ARCHITECTURE.md §4). The Philox round arithmetic is exact u32, so the
// random *integers* match the CPU bit-for-bit; divergence is confined to the f32
// payoff arithmetic and is bounded and tested.

// --- batch parameters (uniform) ---
struct Params {
    ln_spot   : f32,   // ln(S_0)
    mu_term   : f32,   // (mu - 0.5*sigma^2) * T
    vol_sqrt_t: f32,   // sigma * sqrt(T)
    strike    : f32,   // K
    sign      : f32,   // +1 call, -1 put
    paths     : u32,   // number of MC paths
    seed_lo   : u32,   // low 32 bits of the 64-bit run seed
    seed_hi   : u32,   // high 32 bits of the run seed
};

@group(0) @binding(0) var<uniform> params : Params;
// Per-workgroup partial sums: [sum_payoff, sum_payoff_sq] interleaved.
@group(0) @binding(1) var<storage, read_write> group_sums : array<f32>;

const WG_SIZE : u32 = 256u;

// Philox-4x32-10 constants — identical to crate::philox.
const PHILOX_MUL_0 : u32 = 0xD2511F53u;
const PHILOX_MUL_1 : u32 = 0xCD9E8D57u;
const PHILOX_KEY_BUMP_0 : u32 = 0x9E3779B9u;
const PHILOX_KEY_BUMP_1 : u32 = 0xBB67AE85u;
const PHILOX_DOMAIN_TAG : u32 = 0x46584F50u; // "PFXO"
const TAU : f32 = 6.2831853071795864769; // 2*pi (rounds to f32)

// Shared scratch for the per-workgroup tree reduction.
var<workgroup> scratch_sum    : array<f32, WG_SIZE>;
var<workgroup> scratch_sum_sq : array<f32, WG_SIZE>;

// 32x32 -> (hi, lo) multiply via WGSL's split-multiply intrinsics-free path.
// WGSL u32 multiply wraps mod 2^32 (== the low word); we recover the high word
// from 16-bit limbs so the result is exact and matches the CPU's u64 multiply.
fn mul_hi_lo(a : u32, b : u32) -> vec2<u32> {
    let a_lo = a & 0xFFFFu;
    let a_hi = a >> 16u;
    let b_lo = b & 0xFFFFu;
    let b_hi = b >> 16u;

    let lo_lo = a_lo * b_lo;
    let hi_lo = a_hi * b_lo;
    let lo_hi = a_lo * b_hi;
    let hi_hi = a_hi * b_hi;

    // Combine partial products, carrying through 32 bits.
    let cross = hi_lo + (lo_lo >> 16u);
    let cross_lo = cross & 0xFFFFu;
    let cross_hi = cross >> 16u;
    let cross2 = lo_hi + cross_lo;

    let lo = (cross2 << 16u) | (lo_lo & 0xFFFFu);
    let hi = hi_hi + cross_hi + (cross2 >> 16u);
    return vec2<u32>(hi, lo);
}

// One Philox round on counter c under key k (matches crate::philox::philox_round).
fn philox_round(c : vec4<u32>, k : vec2<u32>) -> vec4<u32> {
    let m0 = mul_hi_lo(PHILOX_MUL_0, c.x); // (hi0, lo0)
    let m1 = mul_hi_lo(PHILOX_MUL_1, c.z); // (hi1, lo1)
    return vec4<u32>(
        m1.x ^ c.y ^ k.x,
        m1.y,
        m0.x ^ c.w ^ k.y,
        m0.y,
    );
}

// Full 10-round Philox-4x32-10 bijection (matches crate::philox::philox_4x32_10).
fn philox_4x32_10(counter : vec4<u32>, key_in : vec2<u32>) -> vec4<u32> {
    var c = counter;
    var k = key_in;
    for (var r : u32 = 0u; r < 10u; r = r + 1u) {
        c = philox_round(c, k);
        k.x = k.x + PHILOX_KEY_BUMP_0;
        k.y = k.y + PHILOX_KEY_BUMP_1;
    }
    return c;
}

// Raw uniform integer for (path, step, dim) — matches PhiloxNormals::raw_u32.
fn philox_raw(path : u32, step : u32, dim : u32) -> u32 {
    let counter = vec4<u32>(path, step, dim >> 2u, PHILOX_DOMAIN_TAG);
    let key = vec2<u32>(params.seed_lo, params.seed_hi);
    let block = philox_4x32_10(counter, key);
    let lane = dim & 3u;
    // Index the block lane without dynamic array indexing.
    if (lane == 0u) { return block.x; }
    if (lane == 1u) { return block.y; }
    if (lane == 2u) { return block.z; }
    return block.w;
}

// Open-interval uniform (0,1) in f32 — matches PhiloxNormals::uniform.
fn philox_uniform(path : u32, step : u32, dim : u32) -> f32 {
    let u = philox_raw(path, step, dim);
    // (u + 0.5) * 2^-32 in f32.
    return (f32(u) + 0.5) * (1.0 / 4294967296.0);
}

// Standard normal via Box-Muller cosine branch — matches PhiloxNormals::normal.
fn philox_normal(path : u32, step : u32, dim : u32) -> f32 {
    let u1 = philox_uniform(path, step, 2u * dim);
    let u2 = philox_uniform(path, step, 2u * dim + 1u);
    let r = sqrt(-2.0 * log(u1));
    return r * cos(TAU * u2);
}

@compute @workgroup_size(256)
fn mc_vanilla(
    @builtin(global_invocation_id) gid : vec3<u32>,
    @builtin(local_invocation_index) lid : u32,
    @builtin(workgroup_id) wid : vec3<u32>,
) {
    let path = gid.x;
    var payoff : f32 = 0.0;
    if (path < params.paths) {
        let z = philox_normal(path, 0u, 0u);
        let ln_st = params.ln_spot + params.mu_term + params.vol_sqrt_t * z;
        let st = exp(ln_st);
        payoff = max(params.sign * (st - params.strike), 0.0);
    }

    // Per-workgroup tree reduction in shared memory.
    scratch_sum[lid] = payoff;
    scratch_sum_sq[lid] = payoff * payoff;
    workgroupBarrier();

    var stride : u32 = WG_SIZE / 2u;
    loop {
        if (stride == 0u) { break; }
        if (lid < stride) {
            scratch_sum[lid] = scratch_sum[lid] + scratch_sum[lid + stride];
            scratch_sum_sq[lid] = scratch_sum_sq[lid] + scratch_sum_sq[lid + stride];
        }
        workgroupBarrier();
        stride = stride / 2u;
    }

    if (lid == 0u) {
        group_sums[2u * wid.x] = scratch_sum[0];
        group_sums[2u * wid.x + 1u] = scratch_sum_sq[0];
    }
}
