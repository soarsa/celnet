//! Atomics/cell shim: loom-instrumented primitives under `cfg(loom)`, the
//! zero-cost std primitives otherwise. The ONLY place the ring's choice of
//! atomic/cell type is made — so the loom model-check (`tests/loom_seqlock.rs`,
//! built only with `RUSTFLAGS="--cfg loom"`) instruments every shared access,
//! while the production build is byte-for-byte the std seqlock (loom never
//! enters a release build; `cfg(loom)` is set by the loom test invocation only).
//!
//! ## Why the payload cell is modeled with ATOMIC LANES under loom (not `UnsafeCell`)
//!
//! A seqlock's payload read is, by design, a *race* with the producer's
//! overwrite: the reader copies the payload while a writer may concurrently
//! rewrite it, then the post-copy stamp re-check (guarded by the reader's
//! `Acquire` [`fence`]) DISCARDS any copy that overlapped a write. In the strict
//! C11 model that overlap on a *non-atomic* location is undefined behaviour —
//! `loom::cell::UnsafeCell`'s causality checker (correctly) refuses to bless it
//! ("Concurrent read and write accesses"), so loom cannot model a seqlock whose
//! payload is a plain `UnsafeCell`. (This is the well-known "seqlock data race":
//! the production `*cell.get()` copy is sound on a coherent CPU — aarch64/x86 —
//! but is a data race in the abstract machine; see *Rust Atomics and Locks*,
//! Mara Bos, ch. "Building Our Own Seqlock".)
//!
//! The model-faithful rendering is therefore a set of **atomic lanes** covering
//! `size_of::<T>()`. To verify the PROTOCOL LOGIC under test — that the two-stamp
//! straddle plus the reader fence REJECTS every torn interleaving and never
//! RETURNS a mixed-publish payload — the lanes carry the same `Release`/`Acquire`
//! coupling the production `Acquire` fence realises on hardware: the producer stores each
//! lane with `Release` (sequenced after the in-progress stamp), the consumer
//! loads each lane with `Acquire`. That coupling is exactly what makes "observed
//! a newer payload byte ⇒ observe the newer stamp" hold, which on real hardware
//! the `Acquire` fence + cache coherence provide for the plain copy. loom then
//! explores every interleaving of the stamp protocol exhaustively and proves no
//! interleaving returns a torn pair, with strict in-order, conserving delivery.
//!
//! Honesty note (recorded in `docs/HARDENING.md`): the loom model proves the
//! **stamp/fence rejection protocol** is correct over all interleavings; it does
//! NOT (and cannot, in strict C11) bless the production *non-atomic* payload copy
//! itself — that copy's soundness rests on hardware coherence + the `Acquire`
//! fence, which is the deliberate, documented design choice in `ring.rs`
//! §"Seqlock reader barrier". The std unit + conflation-stress tests exercise the
//! real `UnsafeCell` copy; the loom model exhaustively exercises the ordering
//! skeleton that guards it.
//!
//! Under `not(loom)` the cell is the production `UnsafeCell<T>` with a plain
//! `Copy` read/write — byte-for-byte the original hot path, zero added cost.

#[cfg(loom)]
pub(crate) use loom::sync::Arc;
#[cfg(loom)]
pub(crate) use loom::sync::atomic::{AtomicU64, Ordering, fence};

#[cfg(not(loom))]
pub(crate) use std::sync::Arc;
#[cfg(not(loom))]
pub(crate) use std::sync::atomic::{AtomicU64, Ordering, fence};

/// Spin-wait hint inside the seqlock retry loops.
///
/// * **std** — `std::hint::spin_loop()`: the zero-cost CPU pause the hot path
///   always used; emits a `YIELD`/`PAUSE` and nothing else.
/// * **loom** — `loom::thread::yield_now()`: tells the model checker this is a
///   busy-wait YIELD POINT so it schedules the writer to make progress instead
///   of exploring the spin as an unbounded branch (without this, loom treats the
///   retry as a non-terminating spin and blows past `LOOM_MAX_BRANCHES`). This is
///   the canonical loom rendering of `spin_loop` — it changes only the model's
///   scheduling view, never the production code.
#[inline]
#[cfg(not(loom))]
pub(crate) fn spin_loop() {
    std::hint::spin_loop();
}
#[inline]
#[cfg(loom)]
pub(crate) fn spin_loop() {
    loom::thread::yield_now();
}

// ---------------------------------------------------------------------------
// Production payload cell (std): a plain `UnsafeCell<T>` with `Copy` r/w. This
// is identical codegen to the original `ring.rs` (`*slot.value.get()`).
// ---------------------------------------------------------------------------

/// The seqlock payload cell for one ring slot.
///
/// * **std** — a transparent wrapper over `std::cell::UnsafeCell<T>`; the read
///   and write are the plain `Copy` deref the ring always used.
/// * **loom** — relaxed-atomic lanes covering `size_of::<T>()` (see the module
///   doc) so loom can model the benign seqlock payload race.
#[cfg(not(loom))]
pub(crate) struct PayloadCell<T> {
    inner: std::cell::UnsafeCell<T>,
}

#[cfg(not(loom))]
impl<T: Copy> PayloadCell<T> {
    #[inline]
    pub(crate) fn new(v: T) -> Self {
        Self {
            inner: std::cell::UnsafeCell::new(v),
        }
    }

    /// Read the payload (single coherent `Copy` out).
    ///
    /// # Safety
    /// The caller upholds the seqlock read discipline: the returned value is only
    /// trusted after the post-copy stamp re-check confirms no overlapping write.
    #[inline]
    #[allow(unsafe_code)]
    pub(crate) unsafe fn read(&self) -> T {
        // SAFETY: as documented on the fn; the raw deref is the same access
        // `ring.rs` made directly before the shim existed.
        unsafe { *self.inner.get() }
    }

    /// Write the payload (single-producer exclusive, under the in-progress stamp).
    ///
    /// # Safety
    /// The caller is the single producer holding exclusive write access between
    /// raising and clearing the in-progress stamp.
    #[inline]
    #[allow(unsafe_code)]
    pub(crate) unsafe fn write(&self, v: T) {
        // SAFETY: as documented on the fn; exclusive single-producer write.
        unsafe {
            *self.inner.get() = v;
        }
    }
}

// SAFETY: identical justification to the original `unsafe impl Sync for Slot`:
// the only mutable access is the single producer, ordered with readers by the
// seqlock stamp protocol. `T: Send` carries no thread-affine ownership.
#[cfg(not(loom))]
#[allow(unsafe_code)]
unsafe impl<T: Send> Sync for PayloadCell<T> {}

// ---------------------------------------------------------------------------
// Loom payload cell: relaxed-atomic lanes covering `size_of::<T>()`. The value
// is (de)composed via `bytemuck`-free, dependency-free byte copies into u64
// lanes so loom tracks each lane as an independent relaxed-atomic location and
// a torn read manifests as two lanes from DIFFERENT publishes.
// ---------------------------------------------------------------------------

#[cfg(loom)]
pub(crate) struct PayloadCell<T> {
    /// One relaxed-atomic lane per 8 bytes of `T` (rounded up). The seqlock
    /// stamp + reader fence provide all ordering; the lanes themselves are
    /// `Relaxed`, so loom is free to interleave/reorder them — which is exactly
    /// how it surfaces the torn-read / reorder window.
    lanes: Box<[AtomicU64]>,
    _marker: core::marker::PhantomData<T>,
}

#[cfg(loom)]
impl<T: Copy> PayloadCell<T> {
    #[inline]
    fn lane_count() -> usize {
        core::mem::size_of::<T>().div_ceil(8).max(1)
    }

    pub(crate) fn new(v: T) -> Self {
        let n = Self::lane_count();
        let lanes: Box<[AtomicU64]> = (0..n).map(|_| AtomicU64::new(0)).collect();
        let cell = Self {
            lanes,
            _marker: core::marker::PhantomData,
        };
        // SAFETY: exclusive access at construction (no other thread observes the
        // cell yet); seeds the lanes with the initial value's bytes.
        #[allow(unsafe_code)]
        unsafe {
            cell.write(v);
        }
        cell
    }

    /// Read the payload by `Relaxed`-loading every lane and reassembling the
    /// bytes. A torn read returns lanes from different publishes; the caller's
    /// post-copy stamp re-check (after the [`fence`]) is what rejects it.
    ///
    /// # Safety
    /// As the std variant: the result is only trusted after the stamp re-check.
    #[inline]
    #[allow(unsafe_code)]
    pub(crate) unsafe fn read(&self) -> T {
        let n = Self::lane_count();
        let mut bytes = vec![0u8; n * 8];
        for (i, lane) in self.lanes.iter().enumerate() {
            // `Acquire` lane load: couples "observed a newer payload byte" to
            // "observe the newer stamp" — the coherence the production `Acquire`
            // fence + cache coherence give the plain copy on hardware. This is
            // what lets the stamp re-check reject every torn read in the model.
            let word = lane.load(Ordering::Acquire).to_le_bytes();
            bytes[i * 8..i * 8 + 8].copy_from_slice(&word);
        }
        // SAFETY: `bytes` holds at least `size_of::<T>()` bytes laid out as the
        // little-endian lane image written by `write`; `T: Copy` is plain-old-data
        // for the purposes of the model. We read it back as a `T`.
        unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const T) }
    }

    /// Write the payload by decomposing it into lanes, each stored `Relaxed`.
    ///
    /// # Safety
    /// Single-producer exclusive write under the in-progress stamp.
    #[inline]
    #[allow(unsafe_code)]
    pub(crate) unsafe fn write(&self, v: T) {
        let n = Self::lane_count();
        let mut bytes = vec![0u8; n * 8];
        // SAFETY: copy the `size_of::<T>()` value bytes into the zero-padded lane
        // image; `T: Copy`.
        unsafe {
            core::ptr::copy_nonoverlapping(
                (&v as *const T) as *const u8,
                bytes.as_mut_ptr(),
                core::mem::size_of::<T>(),
            );
        }
        for (i, lane) in self.lanes.iter().enumerate() {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
            // `Release` lane store (sequenced after the in-progress stamp): pairs
            // with the consumer's `Acquire` lane load (see `read`).
            lane.store(u64::from_le_bytes(word), Ordering::Release);
        }
    }
}
