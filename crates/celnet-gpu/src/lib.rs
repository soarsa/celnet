//! Celnet cross-platform GPU compute — a `PricingBackend` over wgpu
//! (Metal/Vulkan/DX12) with a deterministic CPU-SIMD fallback and f64 CPU
//! reconciliation of the f32 GPU path (work-stream WS-E). Skeleton —
//! implementation lands in this lane.
#![forbid(unsafe_code)]
