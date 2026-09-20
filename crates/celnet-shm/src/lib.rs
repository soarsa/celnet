//! `celnet-shm` — Shared-memory IPC broadcast ring.
//!
//! Provides a lock-free, zero-allocation, single-producer multi-consumer (SPMC)
//! broadcast ring buffer over memory-mapped files in `/dev/shm` or POSIX shared memory.
//!
//! Uses the audited seqlock protocol with counted-skip conflation and ARM64
//! `fence(Acquire)` barrier semantics, enabling sub-microsecond cross-process
//! communication with complete fault isolation.
#![deny(missing_docs)]

use std::fs::{File, OpenOptions};
use std::hint::spin_loop;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering, fence};

use memmap2::{Mmap, MmapMut};
use thiserror::Error;

use celnet_sbe::{
    OptionQuote, OptionQuoteFlyweight, PriceTick, PriceTickFlyweight, SbeDispatcher, SbeMessageRef,
    encode_option_quote, encode_price_tick,
};

/// Magic byte constant identifying Celnet Shared Memory (`"CELN_SHM"`).
pub const SHM_MAGIC: u64 = 0x4345_4C4E_5F53_484D;
/// Current layout version.
pub const SHM_VERSION: u32 = 1;
/// Fixed header size (128 bytes, cache-aligned).
pub const SHM_HEADER_SIZE: usize = 128;
/// Unwritten slot stamp sentinel.
pub const UNWRITTEN: u64 = u64::MAX;

/// Errors arising from shared-memory ring creation and mapping.
#[derive(Debug, Error)]
pub enum ShmError {
    /// Underlying filesystem or OS error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// File header magic does not match `SHM_MAGIC`.
    #[error("invalid magic: expected {SHM_MAGIC:#x}, got {0:#x}")]
    InvalidMagic(u64),
    /// Incompatible layout version.
    #[error("unsupported version: expected {SHM_VERSION}, got {0}")]
    UnsupportedVersion(u32),
    /// Ring capacity must be a power of two.
    #[error("capacity {0} is not a power of two")]
    CapacityNotPowerOfTwo(usize),
    /// Payload exceeds slot capacity.
    #[error("payload {actual} bytes exceeds slot limit {max} bytes")]
    PayloadTooLarge {
        /// Maximum allowed payload size.
        max: usize,
        /// Actual attempted payload size.
        actual: usize,
    },
}

/// Errors returned by consumer receive operations.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ShmRecvError {
    /// Ring buffer currently has no new unread messages.
    #[error("ring buffer empty")]
    Empty,
    /// Destination buffer is smaller than the slot payload size.
    #[error("buffer too small: required {required}, provided {provided}")]
    BufferTooSmall {
        /// Required buffer size.
        required: usize,
        /// Provided buffer size.
        provided: usize,
    },
}

/// Slot metadata and stride calculator.
#[inline(always)]
fn calculate_slot_stride(slot_size: usize) -> usize {
    // 8 bytes stamp + 56 bytes padding (to 64-byte line) + (4-byte length prefix + slot_size, rounded to 64 bytes)
    let padded_slot_size = (4 + slot_size + 63) & !63;
    64 + padded_slot_size
}

/// Single-producer shared-memory broadcast ring publisher.
pub struct ShmProducer {
    mmap: MmapMut,
    capacity: usize,
    mask: u64,
    slot_size: usize,
    slot_stride: usize,
    head: u64,
}

impl ShmProducer {
    /// Create a new shared-memory ring buffer file at the given path.
    #[allow(unsafe_code)]
    pub fn create(path: &Path, capacity: usize, slot_size: usize) -> Result<Self, ShmError> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(ShmError::CapacityNotPowerOfTwo(capacity));
        }

        let slot_stride = calculate_slot_stride(slot_size);
        let total_file_size = SHM_HEADER_SIZE + capacity * slot_stride;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        file.set_len(total_file_size as u64)?;

        // SAFETY: File was just created, exclusive ownership, sized exactly.
        let mut mmap = unsafe { MmapMut::map_mut(&file)? };

        // Write header
        mmap[0..8].copy_from_slice(&SHM_MAGIC.to_le_bytes());
        mmap[8..12].copy_from_slice(&SHM_VERSION.to_le_bytes());
        mmap[12..16].copy_from_slice(&(capacity as u32).to_le_bytes());
        mmap[16..20].copy_from_slice(&(slot_size as u32).to_le_bytes());
        mmap[20..24].copy_from_slice(&(slot_stride as u32).to_le_bytes());
        let mask = (capacity - 1) as u64;
        mmap[24..32].copy_from_slice(&mask.to_le_bytes());

        // Zero head at offset 64
        mmap[64..72].fill(0);

        // Initialize all slot stamps to UNWRITTEN
        for i in 0..capacity {
            let slot_offset = SHM_HEADER_SIZE + i * slot_stride;
            mmap[slot_offset..slot_offset + 8].copy_from_slice(&UNWRITTEN.to_le_bytes());
        }

        mmap.flush()?;

        Ok(Self {
            mmap,
            capacity,
            mask,
            slot_size,
            slot_stride,
            head: 0,
        })
    }

    /// Publish a payload into the ring buffer using the seqlock protocol.
    ///
    /// Lock-free, zero-allocation, non-blocking to the caller.
    #[allow(unsafe_code)]
    pub fn publish(&mut self, payload: &[u8]) -> Result<u64, ShmError> {
        if payload.len() > self.slot_size {
            return Err(ShmError::PayloadTooLarge {
                max: self.slot_size,
                actual: payload.len(),
            });
        }

        let seq = self.head;
        let idx = (seq & self.mask) as usize;
        let slot_offset = SHM_HEADER_SIZE + idx * self.slot_stride;

        let stamp_ptr = self.mmap[slot_offset..slot_offset + 8].as_mut_ptr() as *const AtomicU64;
        // SAFETY: slot_offset is aligned and within mapped memory.
        let stamp_atomic = unsafe { &*stamp_ptr };

        // 1. Mark in-progress (odd sequence number)
        let in_progress_stamp = (seq << 1) | 1;
        stamp_atomic.store(in_progress_stamp, Ordering::Release);

        // 2. Write payload length (u32) and bytes into slot payload area (offset + 64)
        let payload_offset = slot_offset + 64;
        let len_bytes = (payload.len() as u32).to_le_bytes();
        self.mmap[payload_offset..payload_offset + 4].copy_from_slice(&len_bytes);
        self.mmap[payload_offset + 4..payload_offset + 4 + payload.len()].copy_from_slice(payload);

        // 3. Reader barrier before releasing final stamp
        fence(Ordering::Release);

        // 4. Mark write complete (even sequence number)
        let complete_stamp = seq << 1;
        stamp_atomic.store(complete_stamp, Ordering::Release);

        // 5. Update global head at offset 64
        let head_ptr = self.mmap[64..72].as_mut_ptr() as *const AtomicU64;
        // SAFETY: offset 64 is 8-byte aligned within mapped memory.
        let head_atomic = unsafe { &*head_ptr };
        head_atomic.store(seq + 1, Ordering::Release);

        self.head = seq + 1;
        Ok(seq)
    }

    /// Current head sequence number.
    #[inline(always)]
    pub fn head(&self) -> u64 {
        self.head
    }

    /// Ring capacity.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Maximum slot payload size.
    #[inline(always)]
    pub fn slot_size(&self) -> usize {
        self.slot_size
    }

    /// Pre-fault and warm up all virtual memory pages backing the shared memory ring buffer.
    ///
    /// By accessing every slot and OS virtual page boundary before live trading starts,
    /// this forces the kernel to commit physical memory pages into the process resident set (RSS)
    /// and pre-populates CPU Translation Lookaside Buffers (TLBs).
    /// Eliminates cold page faults (which typically induce 10-50 microsecond latency spikes).
    pub fn prefault_and_warmup(&mut self) -> usize {
        let mut touched_pages = 0usize;
        let page_size = 4096usize;
        let total_len = self.mmap.len();

        let mut offset = 0;
        while offset < total_len {
            let b = self.mmap[offset];
            std::sync::atomic::compiler_fence(Ordering::SeqCst);
            self.mmap[offset] = b;
            touched_pages += 1;
            offset += page_size;
        }

        for i in 0..self.capacity {
            let slot_offset = SHM_HEADER_SIZE + i * self.slot_stride;
            let b = self.mmap[slot_offset];
            std::sync::atomic::compiler_fence(Ordering::SeqCst);
            self.mmap[slot_offset] = b;
            touched_pages += 1;
        }

        touched_pages
    }

    /// Publish an SBE-encoded `OptionQuote` directly into the ring buffer.
    pub fn publish_sbe_quote(&mut self, quote: &OptionQuote) -> Result<u64, ShmError> {
        let mut buf = [0u8; celnet_sbe::OPTION_QUOTE_TOTAL_SIZE];
        let len = encode_option_quote(quote, &mut buf).map_err(|_| ShmError::PayloadTooLarge {
            max: self.slot_size,
            actual: celnet_sbe::OPTION_QUOTE_TOTAL_SIZE,
        })?;
        self.publish(&buf[..len])
    }

    /// Publish an SBE-encoded `PriceTick` directly into the ring buffer.
    pub fn publish_sbe_tick(&mut self, tick: &PriceTick) -> Result<u64, ShmError> {
        let mut buf = [0u8; celnet_sbe::PRICE_TICK_TOTAL_SIZE];
        let len = encode_price_tick(tick, &mut buf).map_err(|_| ShmError::PayloadTooLarge {
            max: self.slot_size,
            actual: celnet_sbe::PRICE_TICK_TOTAL_SIZE,
        })?;
        self.publish(&buf[..len])
    }
}

/// Multi-consumer shared-memory broadcast ring reader.
pub struct ShmConsumer {
    mmap: Mmap,
    capacity: usize,
    mask: u64,
    slot_size: usize,
    slot_stride: usize,
    cursor: u64,
    received: u64,
    skipped: u64,
}

impl ShmConsumer {
    /// Open an existing shared-memory ring buffer file.
    #[allow(unsafe_code)]
    pub fn open(path: &Path) -> Result<Self, ShmError> {
        let file = File::open(path)?;
        // SAFETY: File opened for read, mapped read-only.
        let mmap = unsafe { Mmap::map(&file)? };

        if mmap.len() < SHM_HEADER_SIZE {
            return Err(ShmError::InvalidMagic(0));
        }

        let magic = u64::from_le_bytes(mmap[0..8].try_into().unwrap());
        if magic != SHM_MAGIC {
            return Err(ShmError::InvalidMagic(magic));
        }

        let version = u32::from_le_bytes(mmap[8..12].try_into().unwrap());
        if version != SHM_VERSION {
            return Err(ShmError::UnsupportedVersion(version));
        }

        let capacity = u32::from_le_bytes(mmap[12..16].try_into().unwrap()) as usize;
        let slot_size = u32::from_le_bytes(mmap[16..20].try_into().unwrap()) as usize;
        let slot_stride = u32::from_le_bytes(mmap[20..24].try_into().unwrap()) as usize;
        let mask = u64::from_le_bytes(mmap[24..32].try_into().unwrap());

        let head_ptr = mmap[64..72].as_ptr() as *const AtomicU64;
        // SAFETY: offset 64 is 8-byte aligned.
        let head_atomic = unsafe { &*head_ptr };
        let current_head = head_atomic.load(Ordering::Acquire);

        Ok(Self {
            mmap,
            capacity,
            mask,
            slot_size,
            slot_stride,
            cursor: current_head, // start from current head
            received: 0,
            skipped: 0,
        })
    }

    /// Open from sequence 0 (replay mode).
    pub fn open_replay(path: &Path) -> Result<Self, ShmError> {
        let mut consumer = Self::open(path)?;
        consumer.cursor = 0;
        Ok(consumer)
    }

    /// Try to read the next unread message into caller's slice.
    ///
    /// Follows the strict seqlock reader barrier protocol:
    /// 1. Pre-stamp check (Acquire)
    /// 2. Payload read
    /// 3. Memory fence (Acquire)
    /// 4. Post-stamp check (Acquire)
    #[allow(unsafe_code)]
    pub fn try_recv(&mut self, out: &mut [u8]) -> Result<usize, ShmRecvError> {
        let head_ptr = self.mmap[64..72].as_ptr() as *const AtomicU64;
        // SAFETY: offset 64 is 8-byte aligned.
        let head_atomic = unsafe { &*head_ptr };
        let head = head_atomic.load(Ordering::Acquire);

        if self.cursor >= head {
            return Err(ShmRecvError::Empty);
        }

        // Check if consumer was lapped (conflation)
        let oldest_live = head.saturating_sub(self.capacity as u64);
        if self.cursor < oldest_live {
            let gap = oldest_live - self.cursor;
            self.skipped += gap;
            self.cursor = oldest_live;
        }

        loop {
            let seq = self.cursor;
            let idx = (seq & self.mask) as usize;
            let slot_offset = SHM_HEADER_SIZE + idx * self.slot_stride;

            let stamp_ptr = self.mmap[slot_offset..slot_offset + 8].as_ptr() as *const AtomicU64;
            // SAFETY: slot_offset is aligned and within mapped memory.
            let stamp_atomic = unsafe { &*stamp_ptr };

            let want = seq << 1;
            let stamp_before = stamp_atomic.load(Ordering::Acquire);

            if stamp_before != want {
                let head2 = head_atomic.load(Ordering::Acquire);
                let oldest_live2 = head2.saturating_sub(self.capacity as u64);
                if self.cursor < oldest_live2 {
                    self.skipped += oldest_live2 - self.cursor;
                    self.cursor = oldest_live2;
                    continue;
                }
                if self.cursor >= head2 {
                    return Err(ShmRecvError::Empty);
                }
                spin_loop();
                continue;
            }

            // Read payload length and bytes
            let payload_offset = slot_offset + 64;
            let payload_len = u32::from_le_bytes(
                self.mmap[payload_offset..payload_offset + 4]
                    .try_into()
                    .unwrap(),
            ) as usize;

            if out.len() < payload_len {
                return Err(ShmRecvError::BufferTooSmall {
                    required: payload_len,
                    provided: out.len(),
                });
            }

            out[..payload_len].copy_from_slice(
                &self.mmap[payload_offset + 4..payload_offset + 4 + payload_len],
            );

            // Seqlock reader barrier on ARM64 and weakly ordered ISAs
            fence(Ordering::Acquire);

            let stamp_after = stamp_atomic.load(Ordering::Acquire);
            if stamp_after != want {
                // Mid-read overwrite detected -> retry
                spin_loop();
                continue;
            }

            self.cursor = seq + 1;
            self.received += 1;
            return Ok(payload_len);
        }
    }

    /// Drain up to `out_buffers.len()` items in a single pass.
    pub fn try_recv_batch(&mut self, out_buffers: &mut [&mut [u8]]) -> usize {
        let mut n = 0;
        for buf in out_buffers.iter_mut() {
            match self.try_recv(buf) {
                Ok(_) => n += 1,
                Err(ShmRecvError::Empty) => break,
                Err(ShmRecvError::BufferTooSmall { .. }) => break,
            }
        }
        n
    }

    /// Read the next unread message in-place without memory allocation or copying.
    ///
    /// The provided closure `viewer` is executed with a slice viewing the payload
    /// directly inside the mapped shared memory buffer. If the producer overwrites
    /// the slot concurrently, the seqlock detects the stamp mutation and retries,
    /// ensuring the viewer only produces a committed, un-torn result.
    #[allow(unsafe_code)]
    pub fn try_recv_view<R, F: FnMut(&[u8]) -> R>(&mut self, mut viewer: F) -> Result<R, ShmRecvError> {
        let head_ptr = self.mmap[64..72].as_ptr() as *const AtomicU64;
        // SAFETY: offset 64 is 8-byte aligned.
        let head_atomic = unsafe { &*head_ptr };
        let head = head_atomic.load(Ordering::Acquire);

        if self.cursor >= head {
            return Err(ShmRecvError::Empty);
        }

        // Check if consumer was lapped (conflation)
        let oldest_live = head.saturating_sub(self.capacity as u64);
        if self.cursor < oldest_live {
            let gap = oldest_live - self.cursor;
            self.skipped += gap;
            self.cursor = oldest_live;
        }

        loop {
            let seq = self.cursor;
            let idx = (seq & self.mask) as usize;
            let slot_offset = SHM_HEADER_SIZE + idx * self.slot_stride;

            let stamp_ptr = self.mmap[slot_offset..slot_offset + 8].as_ptr() as *const AtomicU64;
            // SAFETY: slot_offset is aligned and within mapped memory.
            let stamp_atomic = unsafe { &*stamp_ptr };

            let want = seq << 1;
            let stamp_before = stamp_atomic.load(Ordering::Acquire);

            if stamp_before != want {
                let head2 = head_atomic.load(Ordering::Acquire);
                let oldest_live2 = head2.saturating_sub(self.capacity as u64);
                if self.cursor < oldest_live2 {
                    self.skipped += oldest_live2 - self.cursor;
                    self.cursor = oldest_live2;
                    continue;
                }
                if self.cursor >= head2 {
                    return Err(ShmRecvError::Empty);
                }
                spin_loop();
                continue;
            }

            // In-place payload slice
            let payload_offset = slot_offset + 64;
            let payload_len = u32::from_le_bytes(
                self.mmap[payload_offset..payload_offset + 4]
                    .try_into()
                    .unwrap(),
            ) as usize;

            let payload_slice = &self.mmap[payload_offset + 4..payload_offset + 4 + payload_len];

            // Execute the zero-copy viewer closure
            let result = viewer(payload_slice);

            // Memory fence before checking trailing seqlock stamp
            fence(Ordering::Acquire);

            let stamp_after = stamp_atomic.load(Ordering::Acquire);
            if stamp_after != want {
                // Mid-read overwrite detected -> retry with latest slot data
                spin_loop();
                continue;
            }

            self.cursor = seq + 1;
            self.received += 1;
            return Ok(result);
        }
    }

    /// Read next unread message as an SBE `OptionQuoteFlyweight` directly in-place.
    pub fn try_recv_sbe_quote_view<R, F: FnMut(&OptionQuoteFlyweight<'_>) -> R>(
        &mut self,
        mut viewer: F,
    ) -> Result<R, ShmRecvError> {
        self.try_recv_view(|slice| {
            let fw = OptionQuoteFlyweight::wrap(slice).map_err(|_| ShmRecvError::Empty)?;
            Ok(viewer(&fw))
        })?
    }

    /// Read next unread message as an SBE `PriceTickFlyweight` directly in-place.
    pub fn try_recv_sbe_tick_view<R, F: FnMut(&PriceTickFlyweight<'_>) -> R>(
        &mut self,
        mut viewer: F,
    ) -> Result<R, ShmRecvError> {
        self.try_recv_view(|slice| {
            let fw = PriceTickFlyweight::wrap(slice).map_err(|_| ShmRecvError::Empty)?;
            Ok(viewer(&fw))
        })?
    }

    /// Dispatch next unread message via the zero-allocation SBE message dispatcher.
    pub fn try_recv_sbe_dispatch<R, F: FnMut(SbeMessageRef<'_>) -> R>(
        &mut self,
        mut handler: F,
    ) -> Result<R, ShmRecvError> {
        self.try_recv_view(|slice| {
            let msg = SbeDispatcher::dispatch(slice).map_err(|_| ShmRecvError::Empty)?;
            Ok(handler(msg))
        })?
    }

    /// Number of items successfully received.
    #[inline(always)]
    pub fn received(&self) -> u64 {
        self.received
    }

    /// Number of items skipped due to conflation.
    #[inline(always)]
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Current cursor sequence.
    #[inline(always)]
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Maximum slot payload size.
    #[inline(always)]
    pub fn slot_size(&self) -> usize {
        self.slot_size
    }

    /// Ring capacity in slots.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Pre-fault and warm up all virtual memory pages backing the shared memory ring buffer.
    ///
    /// Reads across all page boundaries and ring slots to eliminate read page faults
    /// during real-time consumption.
    pub fn prefault_and_warmup(&self) -> usize {
        let mut touched = 0usize;
        let page_size = 4096usize;
        let total_len = self.mmap.len();

        let mut offset = 0;
        let mut acc: u8 = 0;
        while offset < total_len {
            acc = acc.wrapping_add(self.mmap[offset]);
            std::sync::atomic::compiler_fence(Ordering::SeqCst);
            touched += 1;
            offset += page_size;
        }

        for i in 0..self.capacity {
            let slot_offset = SHM_HEADER_SIZE + i * self.slot_stride;
            acc = acc.wrapping_add(self.mmap[slot_offset]);
            std::sync::atomic::compiler_fence(Ordering::SeqCst);
            touched += 1;
        }

        std::hint::black_box(acc);
        touched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_shm_prefault_and_warmup() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        let mut producer = ShmProducer::create(path, 64, 256).unwrap();
        let touched_prod = producer.prefault_and_warmup();
        assert!(touched_prod > 0);

        let consumer = ShmConsumer::open(path).unwrap();
        let touched_cons = consumer.prefault_and_warmup();
        assert!(touched_cons > 0);
    }

    #[test]
    fn shm_create_publish_receive_round_trip() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        let mut producer = ShmProducer::create(path, 16, 256).expect("create succeeds");
        let mut consumer = ShmConsumer::open_replay(path).expect("open succeeds");

        let msg1 = b"CELNET_QUOTE_DATA_001";
        let msg2 = b"CELNET_QUOTE_DATA_002";

        producer.publish(msg1).unwrap();
        producer.publish(msg2).unwrap();

        let mut buf = [0u8; 256];
        let n1 = consumer.try_recv(&mut buf).unwrap();
        assert_eq!(&buf[..n1], msg1);

        let n2 = consumer.try_recv(&mut buf).unwrap();
        assert_eq!(&buf[..n2], msg2);

        assert_eq!(consumer.try_recv(&mut buf), Err(ShmRecvError::Empty));
        assert_eq!(consumer.received(), 2);
        assert_eq!(consumer.skipped(), 0);
    }

    #[test]
    fn shm_conflation_accounting() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        let capacity = 8;
        let mut producer = ShmProducer::create(path, capacity, 64).unwrap();
        let mut consumer = ShmConsumer::open_replay(path).unwrap();

        // Publish 20 items into a capacity-8 ring
        for i in 0..20u64 {
            let msg = i.to_le_bytes();
            producer.publish(&msg).unwrap();
        }

        // Consumer reads available items
        let mut buf = [0u8; 64];
        let mut count = 0;
        while let Ok(_) = consumer.try_recv(&mut buf) {
            count += 1;
        }

        // Conflation invariant: received + skipped == 20
        assert_eq!(consumer.received(), count);
        assert_eq!(consumer.received() + consumer.skipped(), 20);
        assert_eq!(consumer.skipped(), 20 - capacity as u64);
    }

    #[test]
    fn shm_full_slot_payload_round_trip() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        let slot_size = 128;
        let mut producer = ShmProducer::create(path, 8, slot_size).unwrap();
        let mut consumer = ShmConsumer::open_replay(path).unwrap();

        // Exactly slot_size bytes (verifies no overrun across slot stride)
        let payload = vec![0xA5u8; slot_size];
        producer.publish(&payload).unwrap();

        let mut buf = vec![0u8; slot_size];
        let n = consumer.try_recv(&mut buf).unwrap();
        assert_eq!(n, slot_size);
        assert_eq!(buf, payload);
    }

    #[test]
    fn shm_zero_copy_view_and_sbe_dispatch() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        let mut producer = ShmProducer::create(path, 16, 256).unwrap();
        let mut consumer = ShmConsumer::open_replay(path).unwrap();

        let tick = PriceTick {
            epoch_nanos: 1_725_000_000,
            pair_id: 10,
            flags: 1,
            bid: 1.0850,
            ask: 1.0852,
        };

        producer.publish_sbe_tick(&tick).unwrap();

        // Zero-copy in-place read with closure
        let bid = consumer
            .try_recv_sbe_tick_view(|fw| fw.bid())
            .expect("read tick succeeds");
        assert_eq!(bid, 1.0850);

        // Test SBE message dispatch
        producer.publish_sbe_tick(&tick).unwrap();
        let matched = consumer
            .try_recv_sbe_dispatch(|msg| match msg {
                SbeMessageRef::PriceTick(fw) => fw.pair_id() == 10,
                _ => false,
            })
            .expect("dispatch succeeds");
        assert!(matched);
    }
}
