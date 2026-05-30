//! Celnet executable competitive-parity matrix — the claims in
//! `docs/CAPABILITIES-VS-COMPETITION.md` rendered as gated tests, so "meets or
//! beats Synoption / Fenics / Bloomberg" is continuously *proven*, not asserted.
//! Each capability row maps to a test that reprices a reference input, checks a
//! convention/arbitrage invariant, or demonstrates a feature competitors lack.
//! Skeleton — the matrix tests land in this lane.
#![forbid(unsafe_code)]
