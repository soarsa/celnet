//! Celnet leader-replicated, deterministic-replay event log
//! (`docs/SCALE-OUT.md` — moves the replicated-log item from *designed* to
//! *built*, behind the honest boundary below).
//!
//! This crate is the distributed-correctness backbone: a **thin
//! leader-replicated log** layered on the dependency-free durable WAL
//! [`celnet_journal`]. A leader durably appends `(term, index)`-stamped
//! [`entry::LogEntry`]s to its journal and streams them, over **real loopback
//! TCP sockets**, to followers who append in the *same order* to their own
//! journals; an entry **commits only on quorum durability** (a strict majority
//! of the cluster has `fsync`'d it). A follower (or a recovered node) replays
//! its journal to rebuild **bit-identical** state (`f64::to_bits` equality), and
//! a caught-up **hot standby** can take over as leader with zero loss of
//! committed entries.
//!
//! # What is built here
//!
//! * [`entry`] — the replicated [`entry::LogEntry`] + its deterministic,
//!   CRC-checked byte codec.
//! * [`state`] — a deterministic priced-book state machine (`u64 → f64`) whose
//!   [`state::BookState::to_bits`] is the cross-node bit-identity oracle.
//! * [`wire`] — length-prefixed [`wire::Message`] framing over real
//!   `std::net::TcpStream` loopback sockets (no shared-memory fake).
//! * [`follower`] — a real TCP-server follower that durably mirrors the leader's
//!   log and applies committed entries.
//! * [`leader`] — the leader: durable-append → replicate → **quorum commit**,
//!   advancing the commit index only on majority durability.
//! * [`standby`] — hot-standby pre-warm + bounded operator-driven failover.
//!
//! # What is explicitly the *next* increment (not half-built here)
//!
//! This is a **thin leader-replicated log, deliberately before full Raft**. The
//! leader is constructed with its term and the standby is promoted by an explicit
//! [`standby::promote`] (a term bump). What is *not* built — and documented here
//! rather than stubbed — is automatic **leader election** (vote RPCs that
//! auto-advance the term on leader loss) and **conflicting-tail truncation**
//! across divergent followers. A standby re-replicates its whole durable prefix
//! to survivors on the first post-promotion proposals; cross-follower log
//! reconciliation under arbitrary divergence is the Raft-election increment.
//!
//! # Honest boundary (reproduced verbatim, never violated)
//!
//! The multi-node replication proof in this crate runs **logical nodes over real
//! loopback (`127.0.0.1`) TCP sockets on ephemeral ports** — genuine OS sockets
//! with kernel framing, not a shared in-memory `Vec` pretending to be a network.
//! Loopback proves the **compute, the wire framing, and the replication / quorum
//! / replay *arithmetic* and relative regression**: it is an **upper bound on
//! compute** and a **lower bound on real cross-host wire latency**. The
//! **absolute cross-host wire p99 / inter-datacentre replication SLO** is
//! provable only on a tuned LAN and stays **DEPLOY-GATED** — it is *never*
//! claimed from this repository. No NVIDIA throughput and no live-JVM-estate
//! claim is made here either. What loopback *does* prove — byte-identical
//! committed logs, `to_bits`-identical replayed state, quorum-commit safety
//! (no false progress under lost quorum), bounded hot-standby takeover with zero
//! committed-entry loss, and crash-recovery — is the honest in-repo distributed-
//! correctness guarantee.

#![forbid(unsafe_code)]

pub mod entry;
pub mod follower;
pub mod leader;
pub mod standby;
pub mod state;
pub mod wire;

pub use entry::{EntryError, Index, LogEntry, Term};
pub use follower::{Follower, FollowerShared};
pub use leader::{Leader, ProposeOutcome, durable_entry_bytes};
pub use standby::{is_caught_up, promote};
pub use state::{BookState, BookUpdate, UpdateError};
pub use wire::{EMPTY_LOG, MAX_FRAME_LEN, Message, WireError};
