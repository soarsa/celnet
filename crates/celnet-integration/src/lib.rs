//! Celnet market-data integration — normalizes external vendor FX-options feeds
//! (ATM / 25Δ&10Δ RR/BF, spot, forward points, NDF fixings) into the canonical
//! Celnet quote vocabulary, and blends multiple sources into one fair,
//! divergence-checked surface input (work-stream WS-H). Skeleton — implementation
//! lands in this lane.
#![forbid(unsafe_code)]
