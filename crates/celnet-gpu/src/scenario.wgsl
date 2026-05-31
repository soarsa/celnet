// Celnet GPU batched scenario pricer — one dispatch prices an entire
// spot×vol shock grid of a single vanilla, vectorised over the grid.
//
// This is the kernel behind the risk-cube's bump-and-revalue scenarios and the
// GUI Risk grid: a node `(i, j)` shocks the base spot by `spot_mult[i]`
// (multiplicative) and the base vol by `vol_bump[j]` (additive), and prices the
// same vanilla payoff under that shocked GBM. Every node draws from the SAME
// counter-based Philox-4x32-10 stream as crate::scenario / crate::cpu, so the
// nodes share common random numbers: the Monte-Carlo noise is common to all
// nodes and cancels in node-to-node differences, making the priced grid a
// smooth risk surface rather than a noise field. This is exactly the property a
// finite-difference (bump-and-revalue) Greek or a scenario ladder needs.
//
// Dispatch geometry: workgroups are (x = path-block, y = node). Each workgroup
// tree-reduces WG_SIZE paths of ONE node in shared memory (no float atomics —
// WGSL atomics are integer only) and writes two partials (sum, sum_sq) to a
// per-(node, x-block) output slot. The host finishes the per-node reduction
// across x-blocks with the same deterministic pairwise sum the CPU backend
// uses, keeping the accumulation order fixed and the result reproducible.
//
// WGSL has no f64; all path math is f32 (Metal/WebGPU constraint, see
// docs/ARCHITECTURE.md §4). The Philox round arithmetic is exact u32, so the
// random *integers* match the CPU bit-for-bit across every node; divergence is
// confined to the f32 payoff arithmetic and is bounded and tested against the
// f64 CPU oracle.

// --- base batch parameters (uniform) ---
//
// These describe the UN-shocked vanilla. Per-node spot/vol are derived on-device
// from the base spot and base vol so the shock vectors stay small storage
// buffers and the heavy GBM constants are recomputed per node from first
// principles (keeping the host upload minimal and the int->float path identical
// to the single-node kernel).
struct Params {
    base_spot   : f32,   // S_0 (un-shocked)
    base_vol    : f32,   // sigma (un-shocked)
    mu_drift    : f32,   // mu = r_dom - r_for (drift, vol-independent)
    t           : f32,   // time to expiry in years
    sqrt_t      : f32,   // sqrt(T) (precomputed host-side in f64, narrowed)
    strike      : f32,   // K
    sign        : f32,   // +1 call, -1 put
    paths       : u32,   // MC paths per node
    n_spot      : u32,   // number of spot-shock nodes (grid rows)
    n_vol       : u32,   // number of vol-shock nodes (grid cols)
    x_blocks    : u32,   // path-blocks per node = ceil(paths / WG_SIZE)
    seed_lo     : u32,   // low 32 bits of the 64-bit run seed
    seed_hi     : u32,   // high 32 bits of the run seed
};

@group(0) @binding(0) var<uniform> params : Params;
// Multiplicative spot shocks, one per grid row (S_0 -> base_spot * spot_mult[i]).
@group(0) @binding(1) var<storage, read> spot_mult : array<f32>;
// Additive vol bumps, one per grid col (sigma -> base_vol + vol_bump[j]).
@group(0) @binding(2) var<storage, read> vol_bump : array<f32>;
// Per-(node, x-block) partial sums: [sum_payoff, sum_payoff_sq] interleaved.
// Node n occupies the contiguous span [n*x_blocks*2, (n+1)*x_blocks*2).
@group(0) @binding(3) var<storage, read_write> group_sums : array<f32>;

const WG_SIZE : u32 = 256u;

// Philox-4x32-10 constants — identical to crate::counter_rng / shader.wgsl.
const PHILOX_MUL_0 : u32 = 0xD2511F53u;
const PHILOX_MUL_1 : u32 = 0xCD9E8D57u;
const PHILOX_KEY_BUMP_0 : u32 = 0x9E3779B9u;
const PHILOX_KEY_BUMP_1 : u32 = 0xBB67AE85u;
const PHILOX_DOMAIN_TAG : u32 = 0x46584F50u; // "PFXO"
const TAU : f32 = 6.2831853071795864769; // 2*pi (rounds to f32)

var<workgroup> scratch_sum    : array<f32, WG_SIZE>;
var<workgroup> scratch_sum_sq : array<f32, WG_SIZE>;

// 32x32 -> (hi, lo) exact multiply via 16-bit limbs (matches crate CPU u64 mul).
fn mul_hi_lo(a : u32, b : u32) -> vec2<u32> {
    let a_lo = a & 0xFFFFu;
    let a_hi = a >> 16u;
    let b_lo = b & 0xFFFFu;
    let b_hi = b >> 16u;

    let lo_lo = a_lo * b_lo;
    let hi_lo = a_hi * b_lo;
    let lo_hi = a_lo * b_hi;
    let hi_hi = a_hi * b_hi;

    let cross = hi_lo + (lo_lo >> 16u);
    let cross_lo = cross & 0xFFFFu;
    let cross_hi = cross >> 16u;
    let cross2 = lo_hi + cross_lo;

    let lo = (cross2 << 16u) | (lo_lo & 0xFFFFu);
    let hi = hi_hi + cross_hi + (cross2 >> 16u);
    return vec2<u32>(hi, lo);
}

fn counter_round(c : vec4<u32>, k : vec2<u32>) -> vec4<u32> {
    let m0 = mul_hi_lo(PHILOX_MUL_0, c.x);
    let m1 = mul_hi_lo(PHILOX_MUL_1, c.z);
    return vec4<u32>(
        m1.x ^ c.y ^ k.x,
        m1.y,
        m0.x ^ c.w ^ k.y,
        m0.y,
    );
}

fn counter_block(counter : vec4<u32>, key_in : vec2<u32>) -> vec4<u32> {
    var c = counter;
    var k = key_in;
    for (var r : u32 = 0u; r < 10u; r = r + 1u) {
        c = counter_round(c, k);
        k.x = k.x + PHILOX_KEY_BUMP_0;
        k.y = k.y + PHILOX_KEY_BUMP_1;
    }
    return c;
}

fn counter_raw(path : u32, step : u32, dim : u32) -> u32 {
    let counter = vec4<u32>(path, step, dim >> 2u, PHILOX_DOMAIN_TAG);
    let key = vec2<u32>(params.seed_lo, params.seed_hi);
    let block = counter_block(counter, key);
    let lane = dim & 3u;
    if (lane == 0u) { return block.x; }
    if (lane == 1u) { return block.y; }
    if (lane == 2u) { return block.z; }
    return block.w;
}

fn counter_uniform(path : u32, step : u32, dim : u32) -> f32 {
    let u = counter_raw(path, step, dim);
    return (f32(u) + 0.5) * (1.0 / 4294967296.0);
}

// Standard normal via Box-Muller cosine branch — matches CounterNormals::normal.
// The normal is drawn at the path's coordinate and is INDEPENDENT of the node:
// every node reuses the same Z_p for path p (common random numbers), so the only
// thing that varies across the grid is the deterministic GBM mapping of Z_p.
fn counter_normal(path : u32, step : u32, dim : u32) -> f32 {
    let u1 = counter_uniform(path, step, 2u * dim);
    let u2 = counter_uniform(path, step, 2u * dim + 1u);
    let r = sqrt(-2.0 * log(u1));
    return r * cos(TAU * u2);
}

@compute @workgroup_size(256)
fn mc_scenario(
    @builtin(global_invocation_id) gid : vec3<u32>,
    @builtin(local_invocation_index) lid : u32,
    @builtin(workgroup_id) wid : vec3<u32>,
) {
    let path = gid.x;          // path index within the node's batch
    let node = wid.y;          // grid node index (row-major: node = i*n_vol + j)
    let total_nodes = params.n_spot * params.n_vol;

    var payoff : f32 = 0.0;
    if (node < total_nodes && path < params.paths) {
        // Decode node -> (spot row i, vol col j) and apply the shocks.
        let i = node / params.n_vol;
        let j = node % params.n_vol;
        let spot = params.base_spot * spot_mult[i];
        let vol = params.base_vol + vol_bump[j];

        // Per-node GBM constants (recomputed on-device from the shocked inputs).
        let ln_spot = log(spot);
        let mu_term = (params.mu_drift - 0.5 * vol * vol) * params.t;
        let vol_sqrt_t = vol * params.sqrt_t;

        // Shared normal across the grid: common random numbers.
        let z = counter_normal(path, 0u, 0u);
        let ln_st = ln_spot + mu_term + vol_sqrt_t * z;
        let st = exp(ln_st);
        payoff = max(params.sign * (st - params.strike), 0.0);
    }

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
        // Output slot for (node, x-block wid.x): node-major, then x-block.
        let slot = (node * params.x_blocks + wid.x) * 2u;
        group_sums[slot] = scratch_sum[0];
        group_sums[slot + 1u] = scratch_sum_sq[0];
    }
}
