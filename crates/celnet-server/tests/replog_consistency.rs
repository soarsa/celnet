//! ADR-0015 — the **configurable consistency tier** wired at the server state sinks.
//!
//! Proves the activation of the dormant `celnet-replog` Raft as the engine of the
//! `Strong` tier, over a **real ≥3-node loopback-socket cluster** (the crate's own
//! multi-node harness shape), asserting the four load-bearing properties:
//!
//! 1. **Strong ⇒ quorum-committed + applied.** A `Strong`-tier FX booking routes its
//!    authoritative write through `RaftNode::propose` and lands in the committed,
//!    bit-identically-applied replicated `BookState` on the leader **and** every
//!    follower (the `to_bits` oracle) — keyed by `position_id`, valued at the
//!    must-order economic size.
//! 2. **Local ⇒ byte-identical, no Raft.** A book that resolves `Local` — even with the
//!    consensus node booted and shared — never touches the quorum log (its key is absent
//!    from the applied state), and a store with no handle at all books exactly as today.
//!    "Wired everywhere, forced nowhere."
//! 3. **Pricing / marking is unaffected by the consistency choice** (the §4.3 hard
//!    invariant at the booking sink): the priced/marked leaf of a booked position is
//!    bit-identical whether its write went through quorum (`Strong`) or the fast path
//!    (`Local`).
//! 4. **Rates too.** A `Strong` linear-rates cell commits its signed PV01 through the
//!    SAME shared node, into the bit-63-tagged disjoint key range.
//!
//! Each test body is hard wall-clock bounded so a regression fails fast, never hangs.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use celnet_replog::{RaftConfig, RaftNode};

use celnet_proto::{
    AttributionRecord, BookId, OisInstrument, Owner, RatesInstrument, RatesPosition, Side, owner,
    rates_instrument,
};
use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

use celnet_server::config::consistency::{ConsistencyLevel, ConsistencyPolicy};
use celnet_server::services::consensus::{ConsensusHandle, fx_book_key, rates_book_key};
use celnet_server::services::rates_book::RatesPositionStore;
use celnet_server::services::risk::store::{BookedPosition, PositionStore};

/// The hard per-test wall-clock bound (leader election + replication over loopback).
///
/// Raised 20 s → 45 s: loaded-t2 M4 contention can starve the multi-node Raft
/// convergence past the original limit (not a real regression — passes uncontended).
const DEADLINE: Duration = Duration::from_secs(45);

fn eurusd() -> CcyPair {
    CcyPair::new(Ccy::EUR, Ccy::USD)
}

fn booked(id: u64, notional: f64) -> BookedPosition {
    BookedPosition {
        position_id: id,
        pair: eurusd(),
        option: OptionType::Call,
        notional_base: notional,
        inputs: VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        quoted_delta: DeltaConvention::SpotUnadjusted,
        premium_style: PremiumStyle::DomesticPips,
        surface_version: 1,
    }
}

/// An attribution chain whose holder book is `book` — the identifier the FX sink
/// resolves the consistency level from.
fn attribution(book: &str, trader: &str) -> AttributionRecord {
    AttributionRecord {
        held_by: Some(BookId {
            book: book.to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::Trader(trader.to_owned())),
            }),
        }),
        ..Default::default()
    }
}

/// A 5-year OIS rates position in `book`, `side` (Buy = pay-fixed), notional 10mm.
fn ois_position(id: u64, entity: u32, book: u32, side: Side) -> RatesPosition {
    RatesPosition {
        position_id: id,
        entity,
        book,
        instrument: Some(RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                tenor_years: 5,
                fixed_rate: 0.04,
                notional: 10_000_000.0,
                side: side as i32,
            })),
        }),
        ..Default::default()
    }
}

/// Boot an N-node fully-meshed cluster over real loopback sockets, mirroring the
/// `celnet-replog` multi-node harness: bind every listener first (so all addresses are
/// known for fixed membership), then hand each pre-bound listener into
/// [`RaftNode::boot_on`] with the full peer-address plan. Journals live under `dir`.
fn boot_cluster(n: usize, dir: &std::path::Path) -> Vec<Arc<RaftNode>> {
    let mut listeners = Vec::with_capacity(n);
    let mut addrs = Vec::with_capacity(n);
    for _ in 0..n {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral loopback");
        addrs.push(l.local_addr().unwrap());
        listeners.push(l);
    }
    let mut nodes = Vec::with_capacity(n);
    for (i, listener) in listeners.into_iter().enumerate() {
        let peers: Vec<_> = addrs
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, &a)| a)
            .collect();
        let journal = dir.join(format!("node-{i}.journal"));
        let node = RaftNode::boot_on(listener, journal, &peers, n, RaftConfig::default())
            .expect("node boots on loopback");
        nodes.push(Arc::new(node));
    }
    nodes
}

/// Wait for a single, stably-established leader (leader continuously for a short window
/// so it is the sole, heartbeating leader). Returns its index. Deadline-bounded.
fn establish_stable_leader(start: Instant, nodes: &[Arc<RaftNode>]) -> usize {
    let stable_window = Duration::from_millis(250);
    loop {
        assert!(
            start.elapsed() < DEADLINE,
            "no stable leader within the deadline"
        );
        let leaders: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.is_leader())
            .map(|(i, _)| i)
            .collect();
        let Some(&leader) = leaders.first().filter(|_| leaders.len() == 1) else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        let term = nodes[leader].term();
        let window_start = Instant::now();
        let mut stable = true;
        while window_start.elapsed() < stable_window {
            assert!(
                start.elapsed() < DEADLINE,
                "leader destabilized past the deadline"
            );
            let count = nodes.iter().filter(|n| n.is_leader()).count();
            if count != 1 || !nodes[leader].is_leader() || nodes[leader].term() != term {
                stable = false;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if stable {
            return leader;
        }
    }
}

/// Build a shared consensus handle from an already-established leader node with the given
/// Strong policy (generous deadlines for the loopback cluster).
///
/// Commit-wait 10 s → 30 s, I/O 5 s → 15 s: loaded-t2 M4 contention can delay Raft
/// RPCs past the original limits.
fn handle_on_leader(leader: Arc<RaftNode>, policy: ConsistencyPolicy) -> Arc<ConsensusHandle> {
    Arc::new(ConsensusHandle::new(
        leader,
        policy,
        Duration::from_secs(30),
        Duration::from_secs(15),
    ))
}

/// Poll `get` on `node`'s applied replicated `BookState` until it returns `expected`, or
/// the deadline elapses. Returns whether it converged (followers apply asynchronously).
fn wait_applied(start: Instant, node: &RaftNode, key: u64, expected: f64) -> bool {
    while start.elapsed() < DEADLINE {
        if node.applied_state().get(key) == Some(expected) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    node.applied_state().get(key) == Some(expected)
}

/// (1) A `Strong`-tier FX booking is proposed through the leader, quorum-commits, and is
/// applied bit-identically on the leader AND every follower.
#[test]
fn strong_fx_booking_is_quorum_committed_and_replicated() {
    let start = Instant::now();
    let dir = tempfile::tempdir().expect("temp raft dir");
    let nodes = boot_cluster(3, dir.path());
    let leader_idx = establish_stable_leader(start, &nodes);

    let mut policy = ConsistencyPolicy::new();
    policy.set_book("STRONG-BOOK", ConsistencyLevel::Strong);
    let handle = handle_on_leader(Arc::clone(&nodes[leader_idx]), policy);

    let store = PositionStore::new().with_consensus(Arc::clone(&handle));
    let notional = 10_000_000.0;
    store
        .book_from_attribution(booked(1, notional), &attribution("STRONG-BOOK", "jdoe"))
        .expect("a Strong booking quorum-commits and books");

    // The position is in the live server book...
    assert_eq!(
        store.len(),
        1,
        "the Strong booking also lands in the live book"
    );
    // ...and its must-order economic size is committed + applied on the leader...
    let key = fx_book_key(1);
    assert_eq!(
        nodes[leader_idx].applied_state().get(key),
        Some(notional),
        "the Strong write is quorum-applied on the leader"
    );
    // ...and replicated to bit-identical applied state on every follower.
    for (i, node) in nodes.iter().enumerate() {
        if i == leader_idx {
            continue;
        }
        assert!(
            wait_applied(start, node, key, notional),
            "follower {i} did not converge on the committed Strong write"
        );
    }
}

/// (2) With the SAME booted consensus node shared, a book that resolves `Local` never
/// touches the quorum log (its key is absent from the applied state), and a store with no
/// handle books exactly as today. "Wired everywhere, forced nowhere."
#[test]
fn local_books_never_touch_the_quorum_log() {
    let start = Instant::now();
    let dir = tempfile::tempdir().expect("temp raft dir");
    let nodes = boot_cluster(3, dir.path());
    let leader_idx = establish_stable_leader(start, &nodes);

    // Only STRONG-BOOK is Strong; the platform default stays Local.
    let mut policy = ConsistencyPolicy::new();
    policy.set_book("STRONG-BOOK", ConsistencyLevel::Strong);
    let handle = handle_on_leader(Arc::clone(&nodes[leader_idx]), policy);

    // A Local book, through a store that DOES hold the consensus handle.
    let store = PositionStore::new().with_consensus(Arc::clone(&handle));
    store
        .book_from_attribution(booked(42, 7_000_000.0), &attribution("G10-LOCAL", "asmith"))
        .expect("a Local booking books without any quorum round-trip");
    assert_eq!(store.len(), 1, "the Local booking lands in the live book");
    assert_eq!(
        nodes[leader_idx].applied_state().get(fx_book_key(42)),
        None,
        "a Local book is never proposed to the quorum log"
    );

    // A store with NO consensus handle at all is the pure-Local default path.
    let plain = PositionStore::new();
    plain
        .book_from_attribution(booked(43, 3_000_000.0), &attribution("ANY", "x"))
        .expect("no-consensus store books as today");
    assert_eq!(plain.len(), 1);
    assert_eq!(
        nodes[leader_idx].applied_state().get(fx_book_key(43)),
        None,
        "a store without a handle never replicates"
    );
}

/// (3) The consistency choice never changes the priced / marked position — the leaf of a
/// booking is bit-identical whether its write went through quorum (`Strong`) or the fast
/// path (`Local`). The §4.3 hard invariant, asserted at the booking sink.
#[test]
fn consistency_choice_does_not_change_the_marked_position() {
    let start = Instant::now();
    let dir = tempfile::tempdir().expect("temp raft dir");
    let nodes = boot_cluster(3, dir.path());
    let leader_idx = establish_stable_leader(start, &nodes);

    let mut policy = ConsistencyPolicy::new();
    policy.set_book("STRONG-BOOK", ConsistencyLevel::Strong);
    let handle = handle_on_leader(Arc::clone(&nodes[leader_idx]), policy);

    // Same economic booking, once Strong (through the quorum) and once Local (no handle).
    let strong = PositionStore::new().with_consensus(handle);
    strong
        .book_from_attribution(booked(7, 12_500_000.0), &attribution("STRONG-BOOK", "t"))
        .expect("strong marks + commits");
    let local = PositionStore::new();
    local
        .book_from_attribution(booked(7, 12_500_000.0), &attribution("G10-LOCAL", "t"))
        .expect("local marks");

    let strong_leaf = strong.snapshot().facts[0].measure.leaf.premium_quote;
    let local_leaf = local.snapshot().facts[0].measure.leaf.premium_quote;
    assert_eq!(
        strong_leaf.to_bits(),
        local_leaf.to_bits(),
        "the marked/priced leaf must be bit-identical regardless of the consistency tier"
    );
}

/// (4) A `Strong` linear-rates cell commits its signed PV01 through the SAME shared node,
/// into the bit-63-tagged disjoint rates key range.
#[test]
fn strong_rates_booking_is_quorum_committed() {
    let start = Instant::now();
    let dir = tempfile::tempdir().expect("temp raft dir");
    let nodes = boot_cluster(3, dir.path());
    let leader_idx = establish_stable_leader(start, &nodes);

    let mut policy = ConsistencyPolicy::new();
    policy.set_rates_book(77, ConsistencyLevel::Strong);
    let handle = handle_on_leader(Arc::clone(&nodes[leader_idx]), policy);

    let store = RatesPositionStore::new().with_consensus(Arc::clone(&handle));
    // A 5y pay-fixed (Buy) 10mm OIS in the Strong rates book 77: signed linear PV01 =
    // 10mm · 5 · 1bp = +5000.0 (deterministic, curve-free).
    let booked = store
        .book(ois_position(0, 1, 77, Side::Buy))
        .expect("a Strong rates cell quorum-commits and books");
    assert_ne!(
        booked.position_id, 0,
        "the id is assigned up front for the Strong write"
    );
    assert_eq!(store.len(), 1);

    let expected_pv01 = 10_000_000.0 * 5.0 * 1e-4;
    let key = rates_book_key(booked.position_id);
    assert_eq!(
        nodes[leader_idx].applied_state().get(key),
        Some(expected_pv01),
        "the Strong rates write is quorum-applied under the tagged key"
    );
    for (i, node) in nodes.iter().enumerate() {
        if i == leader_idx {
            continue;
        }
        assert!(
            wait_applied(start, node, key, expected_pv01),
            "follower {i} did not converge on the committed Strong rates write"
        );
    }

    // A rates cell in a non-Strong book (default Local) is not replicated.
    let local = store
        .book(ois_position(0, 1, 99, Side::Buy))
        .expect("a Local rates cell books without quorum");
    assert_eq!(
        nodes[leader_idx]
            .applied_state()
            .get(rates_book_key(local.position_id)),
        None,
        "a Local rates book never touches the quorum log"
    );
}
