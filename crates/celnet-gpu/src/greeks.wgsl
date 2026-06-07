// Celnet GPU pathwise / likelihood-ratio Greeks kernel — one dispatch produces
// the per-path Monte-Carlo Greek estimators for a single-step terminal GBM
// vanilla under SHARED Sobol' draws, so the host averages them into delta/vega.
//
// This is the GPU-AT-SCALE pathwise/LR-Greeks lever (G6). Two estimator families
// share one terminal draw:
//
//   * PATHWISE (smooth payoff, here the call (S_T - K)^+): differentiate the
//     payoff along the path. The discontinuity of max() at the strike is a
//     measure-zero event so pathwise is valid for the call.
//        d payoff / d S0    = 1{S_T>K} * S_T / S0
//        d payoff / d sigma = 1{S_T>K} * S_T * (sqrt(T) * Z - sigma * T)
//
//   * LIKELIHOOD-RATIO (DISCONTINUOUS payoff, here the digital 1{S_T>K}):
//     pathwise fails (the payoff derivative is a Dirac), so differentiate the
//     density instead. With ln S_T ~ Normal(mean = ln S0 + (mu-0.5 sig^2)T,
//     std s = sigma*sqrt(T)) and Z the standard normal driving the draw:
//        d 1{S_T>K} / d S0 (LR) = payoff * Z / (S0 * s)
//
// All Greeks are reconciled three ways: GPU-f32 == CPU-f64 (same Sobol' draws,
// node-by-node within a derived f32 bound), CPU-MC == analytic GK Greek
// (pathwise) / == an INDEPENDENT central finite-difference on the SAME QMC price
// (both families). HONEST BOUNDARY: Metal has no f64 ⇒ f32 only; in-repo proves
// CORRECTNESS + RATIOS; NVIDIA absolutes DEFERRED to the CUDA deploy-gate.

struct GreeksMeta {
    paths      : u32,
    sobol_base : u32,
    _p0        : u32,
    _p1        : u32,
    ln_spot    : f32,   // ln(S_0)
    spot       : f32,   // S_0
    drift      : f32,   // mu = r_dom - r_for
    vol        : f32,   // sigma
    strike     : f32,   // K
    t_total    : f32,   // T
    sqrt_t     : f32,   // sqrt(T)
    _p2        : f32,
};

@group(0) @binding(0) var<uniform> cfg : GreeksMeta;
@group(0) @binding(1) var<storage, read> dir_nums : array<u32>; // 32 u32 (dim 1)
// Per-path estimators, FIVE f32 per path, interleaved:
//   [0] call payoff           (S_T - K)^+
//   [1] pathwise delta        1{S_T>K} * S_T / S0
//   [2] pathwise vega         1{S_T>K} * S_T * (sqrt(T) Z - sigma T)
//   [3] digital payoff        1{S_T>K}
//   [4] LR delta of digital   1{S_T>K} * Z / (S0 * sigma sqrt(T))
@group(0) @binding(2) var<storage, read_write> est : array<f32>;

const WG_SIZE : u32 = 256u;
const INV_SQRT_2 : f32 = 0.70710678118654752440;
const N_EST : u32 = 5u;

fn sobol_coord(i : u32) -> u32 {
    let g = i ^ (i >> 1u);
    var acc : u32 = 0u;
    var bits = g;
    var k : u32 = 0u;
    loop {
        if (bits == 0u || k >= 32u) { break; }
        if ((bits & 1u) == 1u) {
            acc = acc ^ dir_nums[k];
        }
        bits = bits >> 1u;
        k = k + 1u;
    }
    return acc;
}

fn u32_to_open_unit(u : u32) -> f32 {
    return (f32(u) + 0.5) * (1.0 / 4294967296.0);
}

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

fn inv_norm_cdf(p : f32) -> f32 {
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
    let e = norm_cdf(x) - p;
    let u = e / norm_pdf(x);
    x = x - u / (1.0 + 0.5 * x * u);
    return x;
}

@compute @workgroup_size(256)
fn pathwise_lr_greeks(@builtin(global_invocation_id) gid : vec3<u32>) {
    let p = gid.x;
    if (p >= cfg.paths) { return; }

    let z = inv_norm_cdf(u32_to_open_unit(sobol_coord(cfg.sobol_base + p)));
    let half_var = 0.5 * cfg.vol * cfg.vol;
    let s = cfg.vol * cfg.sqrt_t;                 // std of ln S_T
    let ln_st = cfg.ln_spot + (cfg.drift - half_var) * cfg.t_total + s * z;
    let st = exp(ln_st);

    let in_money = select(0.0, 1.0, st > cfg.strike);
    let call = max(st - cfg.strike, 0.0);

    let base = N_EST * p;
    est[base + 0u] = call;
    est[base + 1u] = in_money * st / cfg.spot;                          // pathwise delta
    est[base + 2u] = in_money * st * (cfg.sqrt_t * z - cfg.vol * cfg.t_total); // pathwise vega
    est[base + 3u] = in_money;                                           // digital payoff
    est[base + 4u] = in_money * z / (cfg.spot * s);                     // LR delta of digital
}
