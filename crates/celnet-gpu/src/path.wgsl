// Celnet GPU multi-step path kernel — one dispatch evolves a large batch of
// multi-step geometric-Brownian-motion paths under SHARED Sobol' draws and a
// Brownian-bridge construction, reducing an Asian (arithmetic-average) payoff.
//
// This is the GPU-AT-SCALE multi-step path lever (G3): a path-dependent product
// (here the arithmetic-average Asian) priced by quasi-Monte-Carlo. The whole
// point of the kernel is that it consumes the EXACT SAME direction numbers and
// Brownian-bridge plan as the CPU celnet-qmc engine, so feeding the same Sobol
// index to CPU and GPU yields the same path — a CPU<->GPU known-answer test
// (KAT). Divergence is confined to the f32 transcendentals (inverse-normal CDF +
// exp) vs the CPU f64, which path.rs bounds node-by-node with a DERIVED f32
// bound (never a fitted constant).
//
// HONEST BOUNDARY: WGSL/Metal have no f64, so the path math is f32 by design
// (Metal lacks f64). In-repo this proves CORRECTNESS (GPU-f32 == CPU-f64 within
// the derived bound, both == celnet-golden / closed-form within QMC error) and
// RATIOS only. The NVIDIA absolute throughput headline / <=50ms exotic /
// Workload-A/B absolutes are DEFERRED to the CUDA deploy-gate. We NEVER claim
// f64 on Metal.
//
// Determinism: the integer Sobol' core is pure u32 gray-code XOR of the uploaded
// direction numbers, so the per-path Sobol' INTEGERS are bit-identical to the
// CPU (celnet-qmc SobolSequence::point_u32). Only the f32 (u+0.5)*2^-32 ->
// inverse-normal -> bridge -> GBM chain differs in precision.

// Uniform metadata block. `steps` = path dimension `m` (== Sobol' dim ==
// bridge steps); `paths` = number of QMC points; the GBM/payoff parameters.
struct PathMeta {
    paths      : u32,   // number of Sobol' points (paths)
    steps      : u32,   // path steps m (== Sobol' dim == bridge plan length)
    sobol_base : u32,   // first Sobol' index (gray-code offset)
    _p0        : u32,
    ln_spot    : f32,   // ln(S_0)
    drift      : f32,   // mu = r_dom - r_for
    vol        : f32,   // sigma
    strike     : f32,   // K
    sign       : f32,   // +1 call, -1 put (on the average)
    r_dom      : f32,   // domestic rate (discounting handled host-side)
    t_total    : f32,   // total maturity T
    _p1        : f32,
};

// Direction numbers: row-major `steps * 32` u32, dimension j contiguous in
// 32-entry blocks. Uploaded verbatim from celnet-qmc SobolSequence.
@group(0) @binding(0) var<uniform> cfg : PathMeta;
@group(0) @binding(1) var<storage, read> dir_nums : array<u32>;
// Brownian-bridge weight matrix A (row-major `steps * steps` f32) such that the
// path W = A * z. Uploaded verbatim from celnet-qmc BrownianBridge::weight_matrix
// (its public API), so the GPU and CPU use the IDENTICAL bridge — the kernel
// applies W[i] = sum_k A[i][k] * z[k]. (O(m^2), trivial for the small step counts
// path-dependent FX products use; m is bounded by MAX_STEPS.)
@group(0) @binding(2) var<storage, read> bridge_a : array<f32>;
// Per-PATH undiscounted payoff (one f32 per Sobol' point). The host finishes the
// reduction with the same pairwise (tree) sum the CPU oracle uses, keeping the
// accumulation order fixed AND making the reconcile node-by-node (per path),
// which is exactly the CPU<->GPU known-answer comparison.
@group(0) @binding(3) var<storage, read_write> payoffs : array<f32>;

const WG_SIZE : u32 = 256u;
const INV_SQRT_2 : f32 = 0.70710678118654752440;
const MAX_STEPS : u32 = 64u;

// --- Sobol' point (unscrambled) for index i, dimension j -------------------
// Pure u32 gray-code XOR of the direction numbers selected by the set bits of
// g(i) = i ^ (i >> 1). Bit-identical to celnet-qmc SobolSequence::point_u32.
fn sobol_coord(i : u32, j : u32) -> u32 {
    let g = i ^ (i >> 1u);
    var acc : u32 = 0u;
    var bits = g;
    var k : u32 = 0u;
    let base = j * 32u;
    loop {
        if (bits == 0u || k >= 32u) { break; }
        if ((bits & 1u) == 1u) {
            acc = acc ^ dir_nums[base + k];
        }
        bits = bits >> 1u;
        k = k + 1u;
    }
    return acc;
}

// Open-interval uniform (0,1) in f32 from a 32-bit Sobol' integer:
// (u + 0.5) * 2^-32 — biased so a zero integer never maps to exactly 0.0.
fn u32_to_open_unit(u : u32) -> f32 {
    return (f32(u) + 0.5) * (1.0 / 4294967296.0);
}

// --- f32 erf via Abramowitz & Stegun 7.1.26 (mirrors batch.wgsl::erf_as) ----
fn erf_as(x : f32) -> f32 {
    let s = sign(x);
    let z = abs(x);
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

fn norm_cdf(x : f32) -> f32 {
    return 0.5 * (1.0 + erf_as(x * INV_SQRT_2));
}

fn norm_pdf(x : f32) -> f32 {
    return 0.3989422804014327 * exp(-0.5 * x * x);
}

// --- inverse standard-normal CDF in f32 (Acklam seed + one Halley step) ------
// Mirrors celnet-qmc normal.rs::inv_norm_cdf, in f32. The single Halley step
// uses the same norm_cdf/norm_pdf the kernel carries, so the algorithm matches
// the CPU oracle; only the precision (f32 vs f64) differs — the path.rs derived
// bound accounts for it.
fn inv_norm_cdf(p : f32) -> f32 {
    // Acklam coefficients (f32 views of the canonical constants).
    let A0 = -39.69683028665376;
    let A1 = 220.9460984245205;
    let A2 = -275.9285104469687;
    let A3 = 138.357751867269;
    let A4 = -30.66479806614716;
    let A5 = 2.506628277459239;
    let B0 = -54.47609879822406;
    let B1 = 161.5858368580409;
    let B2 = -155.6989798598866;
    let B3 = 66.80131188771972;
    let B4 = -13.28068155288572;
    let C0 = -0.007784894002430293;
    let C1 = -0.3223964580411365;
    let C2 = -2.400758277161838;
    let C3 = -2.549732539343734;
    let C4 = 4.374664141464968;
    let C5 = 2.938163982698783;
    let D0 = 0.007784695709041462;
    let D1 = 0.3224671290700398;
    let D2 = 2.445134137142996;
    let D3 = 3.754408661907416;
    let P_LOW = 0.02425;
    let P_HIGH = 1.0 - 0.02425;

    var x : f32;
    if (p < P_LOW) {
        let q = sqrt(-2.0 * log(p));
        x = (((((C0 * q + C1) * q + C2) * q + C3) * q + C4) * q + C5)
            / ((((D0 * q + D1) * q + D2) * q + D3) * q + 1.0);
    } else if (p <= P_HIGH) {
        let q = p - 0.5;
        let r = q * q;
        x = (((((A0 * r + A1) * r + A2) * r + A3) * r + A4) * r + A5) * q
            / (((((B0 * r + B1) * r + B2) * r + B3) * r + B4) * r + 1.0);
    } else {
        let q = sqrt(-2.0 * log(1.0 - p));
        x = -(((((C0 * q + C1) * q + C2) * q + C3) * q + C4) * q + C5)
            / ((((D0 * q + D1) * q + D2) * q + D3) * q + 1.0);
    }
    // One Halley step.
    let e = norm_cdf(x) - p;
    let u = e / norm_pdf(x);
    x = x - u / (1.0 + 0.5 * x * u);
    return x;
}

// Arithmetic-average Asian payoff for path index `i`. Returns the undiscounted
// max(sign*(avg - K), 0). The asset path is S(t_l) = S0 * exp((mu-0.5 sig^2) t_l
// + sig * W(t_l)); the average is over the m monitoring points t_1..t_m. The
// bridge is applied as the linear map W = A * z (A uploaded from celnet-qmc).
fn asian_payoff(i : u32, m : u32) -> f32 {
    var z : array<f32, MAX_STEPS>;
    // Sobol' draw -> standard normals.
    for (var j : u32 = 0u; j < m; j = j + 1u) {
        let uu = u32_to_open_unit(sobol_coord(i, j));
        z[j] = inv_norm_cdf(uu);
    }
    let dt = cfg.t_total / f32(m);
    let half_var = 0.5 * cfg.vol * cfg.vol;
    var acc : f32 = 0.0;
    for (var l : u32 = 0u; l < m; l = l + 1u) {
        // W(t_l) = sum_k A[l][k] * z[k].
        var w_l : f32 = 0.0;
        let row = l * m;
        for (var k : u32 = 0u; k < m; k = k + 1u) {
            w_l = w_l + bridge_a[row + k] * z[k];
        }
        let t_l = f32(l + 1u) * dt;
        let ln_s = cfg.ln_spot + (cfg.drift - half_var) * t_l + cfg.vol * w_l;
        acc = acc + exp(ln_s);
    }
    let avg = acc / f32(m);
    return max(cfg.sign * (avg - cfg.strike), 0.0);
}

@compute @workgroup_size(256)
fn path_asian(@builtin(global_invocation_id) gid : vec3<u32>) {
    let p = gid.x;
    if (p < cfg.paths) {
        payoffs[p] = asian_payoff(cfg.sobol_base + p, cfg.steps);
    }
}
