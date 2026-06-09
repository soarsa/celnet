//! Deterministic, single-threaded gate tests pinning the exact branch boundaries
//! of the conflation / seqlock arithmetic, plus a contention stress test that
//! pins the producer's two-stamp in-progress encoding. These tests close the
//! mutation-gap clusters the broadcast/zero-alloc suites left open (see
//! `.config/mutants-fanout.toml` + docs/HARDENING.md): the conflation-frontier
//! comparisons (`cursor < oldest_live`, the retry-path `cursor < oldest_live2`,
//! the retry-path `cursor >= head2` empty check) and the seqlock stamp encoding
//! (`(seq << 1) | 1`).
//!
//! INDEPENDENT ORACLE: every expected value here is a closed form computed
//! directly from the published-sequence / ring-capacity arithmetic — the
//! half-open live window `[head - capacity, head)` — derived by hand from the
//! conflation contract, NOT read back from the ring under test. A single
//! deterministic producer (no interleaving) makes `oldest_live = head -
//! capacity` an exact integer, so each delivered sequence and each skip count is
//! predicted in closed form and asserted bit-exactly.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use celnet_fanout::{BroadcastRing, Consumer, Producer, RecvError};

/// Drain a consumer fully into a vector (single-threaded; the producer is done,
/// so there is no concurrent overwrite and the loop terminates at `Empty`).
fn drain<T: Copy + Default + Send>(c: &mut Consumer<T>) -> Vec<T> {
    let mut out = Vec::new();
    while let Ok(v) = c.try_recv() {
        out.push(v);
    }
    out
}

/// Exactly-at-the-frontier: publish `capacity + 1` items, so the live window is
/// `[1, capacity + 1)` and `oldest_live == 1`. A from-start consumer (cursor 0)
/// is ONE behind the frontier: the outer `cursor < oldest_live` (0 < 1) must fire
/// and skip exactly that one conflated item (seq 0), then deliver seqs
/// `1..=capacity` in order. This pins the `<` frontier comparison at its tightest
/// boundary (cursor exactly `oldest_live - 1`).
#[test]
fn outer_frontier_skips_exactly_the_conflated_prefix() {
    let cap = 8u64;
    let ring = BroadcastRing::<u64>::new(cap as usize);
    let mut producer: Producer<u64> = ring.into_producer();
    let mut c = producer.subscribe_from_start();

    // Publish capacity + 1 ⇒ oldest_live = (cap + 1) - cap = 1; seq 0 is conflated.
    for i in 0..=cap {
        producer.publish(i);
    }
    // ORACLE: `published()` is the count of items put on the ring = cap + 1.
    assert_eq!(
        producer.published(),
        cap + 1,
        "producer.published() must equal the number of publish() calls"
    );
    assert_eq!(
        producer.capacity() as u64,
        cap,
        "producer.capacity() reports the ring capacity"
    );
    let got = drain(&mut c);

    // ORACLE: delivered = seqs [oldest_live .. head) = [1 .. cap+1) = 1..=cap.
    let want: Vec<u64> = (1..=cap).collect();
    assert_eq!(got, want, "must deliver exactly the live window [1, cap+1)");
    // ORACLE: exactly one item (seq 0) was conflated.
    assert_eq!(c.skipped(), 1, "exactly seq 0 was conflated");
    assert_eq!(c.received(), cap, "delivered the full live window");
    assert_eq!(
        c.received() + c.skipped(),
        cap + 1,
        "conservation == produced"
    );
}

/// Frontier NOT crossed: publish exactly `capacity` items, so the live window is
/// `[0, capacity)` and `oldest_live == 0`. A from-start consumer (cursor 0) is
/// EXACTLY at the frontier: the outer `cursor < oldest_live` (0 < 0) must NOT
/// fire — zero skips, every item delivered. This pins the comparison's behaviour
/// when `cursor == oldest_live` (the `<` vs `<=`/`==` discriminator: a `<=` or an
/// `==` mutant would treat the boundary differently — and any spurious skip here
/// would drop seq 0 or mis-account, which the exact-equality asserts catch).
#[test]
fn at_frontier_no_skip_delivers_everything() {
    let cap = 8u64;
    let ring = BroadcastRing::<u64>::new(cap as usize);
    let mut producer: Producer<u64> = ring.into_producer();
    let mut c = producer.subscribe_from_start();

    for i in 0..cap {
        producer.publish(i);
    }
    let got = drain(&mut c);

    // ORACLE: oldest_live == 0, nothing conflated, full sequence delivered.
    let want: Vec<u64> = (0..cap).collect();
    assert_eq!(got, want, "the full [0, cap) window with no conflation");
    assert_eq!(c.skipped(), 0, "cursor == oldest_live ⇒ no skip");
    assert_eq!(c.received(), cap);
}

/// Resume-then-lapped: a consumer reads PART of the stream (advancing its cursor
/// into the live window), then the producer publishes a large burst that laps it.
/// On the NEXT read the outer frontier skip fires from a NON-zero cursor — the
/// scenario that exercises the outer `cursor < oldest_live` skip independently of
/// the from-start case, with a hand-computed skip count.
#[test]
fn resume_then_lapped_skips_the_exact_gap() {
    let cap = 8u64;
    let ring = BroadcastRing::<u64>::new(cap as usize);
    let mut producer: Producer<u64> = ring.into_producer();
    let mut c = producer.subscribe_from_start();

    // Phase 1: publish 4 (< cap, nothing conflated), read all 4. cursor -> 4.
    for i in 0..4 {
        producer.publish(i);
    }
    let got1 = drain(&mut c);
    assert_eq!(got1, vec![0, 1, 2, 3]);
    assert_eq!(c.skipped(), 0);
    assert_eq!(c.cursor(), 4);

    // Phase 2: publish seqs 4..=19 (16 more ⇒ head = 20). Live window = [12, 20).
    // The consumer is at cursor 4, which is < oldest_live (12), so the outer skip
    // must jump it to 12, skipping seqs 4..=11 (8 items).
    for i in 4..20 {
        producer.publish(i);
    }
    let got2 = drain(&mut c);

    // ORACLE: head = 20, oldest_live = 20 - 8 = 12; delivered = [12, 20).
    let want2: Vec<u64> = (12..20).collect();
    assert_eq!(got2, want2, "second drain = live window [12, 20)");
    // ORACLE: skipped seqs 4..12 = 8 items (cursor 4 -> oldest_live 12).
    assert_eq!(c.skipped(), 8, "exactly seqs 4..12 conflated on resume");
    // Conservation: received (4 + 8) + skipped (8) = 20 produced reached.
    assert_eq!(c.received(), 12, "4 (phase 1) + 8 (phase 2 window)");
    assert_eq!(c.received() + c.skipped(), 20);
}

/// The retry-path EMPTY check (`cursor >= head2`). A from-head consumer created
/// when nothing is published yet, then queried before any publish: the outer
/// `cursor >= head` returns `Empty` immediately; after one publish + one read it
/// is caught up and the next `try_recv` is `Empty` again. We then publish one
/// more and confirm delivery resumes — proving the empty boundary is exact (an
/// off-by-one in the head comparison would either spuriously deliver a
/// non-existent item or wrongly report Empty when an item is available).
#[test]
fn empty_boundary_is_exact_at_the_head() {
    let ring = BroadcastRing::<u64>::new(8);
    let mut producer: Producer<u64> = ring.into_producer();
    let mut c = producer.subscribe_from_start();

    // Nothing published ⇒ Empty.
    assert_eq!(c.try_recv(), Err(RecvError::Empty));
    producer.publish(100);
    assert_eq!(c.try_recv(), Ok(100));
    // Caught up exactly at head ⇒ Empty (cursor == head, not <).
    assert_eq!(c.try_recv(), Err(RecvError::Empty));
    // One more becomes available ⇒ delivered (cursor < head again).
    producer.publish(101);
    assert_eq!(c.try_recv(), Ok(101));
    assert_eq!(c.try_recv(), Err(RecvError::Empty));
    assert_eq!(c.received(), 2);
    assert_eq!(c.skipped(), 0);
}

/// Capacity-2 minimal lap: the tightest ring. Publish 3 over a 2-slot ring; the
/// live window is `[1, 3)`. A from-start consumer must skip exactly seq 0 and
/// deliver seqs 1, 2. This pins the frontier arithmetic at `capacity == 2` (the
/// minimum), where any `<<`/`>>` or `+/-`-style off-by-one in the window math is
/// maximally visible.
#[test]
fn capacity_two_minimal_lap() {
    let ring = BroadcastRing::<u64>::new(2);
    assert_eq!(ring.capacity(), 2);
    let mut producer: Producer<u64> = ring.into_producer();
    let mut c = producer.subscribe_from_start();

    for i in 0..3u64 {
        producer.publish(i);
    }
    let got = drain(&mut c);
    // ORACLE: head = 3, oldest_live = 1; delivered = [1, 3) = {1, 2}.
    assert_eq!(got, vec![1, 2]);
    assert_eq!(c.skipped(), 1, "seq 0 conflated");
    assert_eq!(c.received(), 2);
}

/// CONTENTION stress pinning the producer's two-stamp **in-progress** encoding
/// `(seq << 1) | 1`. A coherent-pair payload `(s, s)` is published over a tiny
/// ring by a fast producer while a concurrent consumer drains; EVERY delivered
/// pair must be coherent (`a == b`) — a torn read (halves from different
/// publishes) is only possible if the odd in-progress stamp is mis-encoded so a
/// reader fails to reject a mid-write slot. Many iterations over a 2-slot ring
/// force continuous in-place overwrites concurrent with reads, so a broken
/// `| 1` / `<< 1` in-progress stamp manifests as a torn pair with high
/// probability. (The exhaustive proof of this is the loom model in
/// `tests/loom_seqlock.rs`; this std test makes the same property a fast,
/// nextest-free mutation-killing gate.)
#[test]
fn concurrent_coherent_pair_never_tears() {
    type Pair = (u64, u64);
    // Enough laps over a 2-slot ring to exercise continuous in-place overwrites
    // concurrent with reads, while keeping the per-mutant cost low so the
    // mutation gate stays fast. (The DEFINITIVE proof that no interleaving tears
    // is the exhaustive loom model in `tests/loom_seqlock.rs`; this is the
    // real-thread regression companion.)
    const ITERS: u64 = 300_000;
    let ring = BroadcastRing::<Pair>::new(2);
    let mut producer: Producer<Pair> = ring.into_producer();
    let consumer: Consumer<Pair> = producer.subscribe_from_start();

    let done = Arc::new(AtomicBool::new(false));
    let published = Arc::new(AtomicU64::new(0));
    let deadline = Instant::now() + Duration::from_secs(30);

    std::thread::scope(|s| {
        let done_p = Arc::clone(&done);
        let published_p = Arc::clone(&published);
        s.spawn(move || {
            for seq in 0..ITERS {
                producer.publish((seq, seq));
                if seq & 0xFFFF == 0 {
                    published_p.store(seq, Ordering::Release);
                }
            }
            done_p.store(true, Ordering::Release);
        });

        let cons = s.spawn(move || {
            let mut c = consumer;
            let mut last: Option<u64> = None;
            let mut delivered = 0u64;
            loop {
                match c.try_recv() {
                    Ok((a, b)) => {
                        // TORN-READ DETECTOR: the two halves must be from the same
                        // publish. A mis-encoded in-progress stamp lets a mid-write
                        // slot be accepted, returning a torn pair.
                        assert_eq!(a, b, "TORN READ: ({a}, {b}) halves disagree");
                        // Strict in-order, never duplicated.
                        if let Some(p) = last {
                            assert!(a > p, "out-of-order/duplicate: {a} after {p}");
                        }
                        last = Some(a);
                        delivered += 1;
                    }
                    Err(RecvError::Empty) => {
                        if done.load(Ordering::Acquire) && c.cursor() >= ITERS {
                            break;
                        }
                        if Instant::now() > deadline {
                            panic!("consumer stalled at cursor {} (regression)", c.cursor());
                        }
                    }
                }
            }
            (delivered, c.received(), c.skipped(), c.cursor())
        });

        let (delivered, received, skipped, cursor) = cons.join().expect("consumer panicked");
        // Conservation across the whole run (independent of how many were lapped).
        assert_eq!(received, delivered, "received counter == delivered count");
        assert_eq!(received + skipped, ITERS, "received + skipped == produced");
        assert_eq!(cursor, ITERS, "cursor reached the end of the sequence");
    });
}
