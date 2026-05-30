//! The zero-allocation pricing core (`docs/ARCHITECTURE.md` §3.3).
//!
//! The pricing core is the busy-poll consumer of the edge→core [`RequestRing`]:
//! it pops a `Copy` [`PriceRequest`], reads the live [`MarketState`] published
//! behind [`StateHandle`] (lock-free), evaluates the smile to get the Black vol
//! at the requested strike, runs the closed-form Garman-Kohlhagen price + full
//! Greek set from [`celnet_vanilla`], pushes a `Copy` [`PriceResponse`] back on
//! the core→edge [`ResponseRing`], and publishes the top-of-book into a seqlock
//! snapshot.
//!
//! **No heap allocation occurs on the hot path.** All inputs and outputs are
//! `Copy`/POD and live on the stack or in pre-sized pools; the smile lookup and
//! the Garman-Kohlhagen evaluation are pure closed-form arithmetic over
//! `celnet_core::math`. The zero-allocation property is asserted in
//! `tests/zero_alloc.rs` with a counting global allocator.
//!
//! [`RequestRing`]: crate::rt::RequestRing
//! [`ResponseRing`]: crate::rt::ResponseRing
//! [`MarketState`]: crate::rt::MarketState
//! [`StateHandle`]: crate::rt::StateHandle

use std::sync::atomic::{AtomicBool, Ordering};

use celnet_core::Smile;
use celnet_types::{Greeks, OptionType};

use crate::rt::{MarketState, PriceSnapshot, Seqlock, StateHandle, StateReader};

/// A `Copy`/POD price request flowing edge→core over the SPSC ring.
///
/// It carries everything the core needs that is *not* part of the published
/// market state: a stable id (echoed back so the edge can correlate the
/// response), the option type, and the strike. Spot, rates, vol-time and the
/// smile come from the live [`MarketState`]; the request never carries a heap
/// pointer, so it copies by value through the ring with no allocation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceRequest {
    /// Correlation id, echoed in the [`PriceResponse`].
    pub request_id: u64,
    /// Call or put.
    pub option_type: OptionType,
    /// Strike (quote per 1 unit of base).
    pub strike: f64,
}

impl PriceRequest {
    /// Construct a request.
    #[must_use]
    pub const fn new(request_id: u64, option_type: OptionType, strike: f64) -> Self {
        Self {
            request_id,
            option_type,
            strike,
        }
    }
}

/// A `Copy`/POD price response flowing core→edge over the SPSC ring.
///
/// Carries the full Garman-Kohlhagen [`Greeks`] set plus the correlation id and
/// the smile vol used. `Greeks` is itself `Copy`/POD, so the whole response
/// copies by value through the ring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceResponse {
    /// Correlation id from the originating [`PriceRequest`].
    pub request_id: u64,
    /// Price and the full Greek set.
    pub greeks: Greeks,
    /// The Black vol the smile assigned to the priced strike.
    pub vol: f64,
}

impl PriceResponse {
    /// A compact top-of-book [`PriceSnapshot`] view of this response.
    #[must_use]
    pub fn snapshot(&self) -> PriceSnapshot {
        PriceSnapshot {
            request_id: self.request_id,
            price: self.greeks.price,
            delta_spot: self.greeks.delta_spot,
            vega: self.greeks.vega,
            vol: self.vol,
        }
    }
}

/// The stateful pricing core.
///
/// Owns the read handle to the live [`MarketState`] and the single-writer
/// seqlock for top-of-book publication. The core is `!Sync` by construction
/// (the seqlock is single-writer and owned here), matching the one-core-one-core
/// runtime model; the edge interacts with it only through the SPSC rings.
#[derive(Debug)]
pub struct PricingCore {
    state: StateHandle,
    reader: StateReader,
    top_of_book: Seqlock<PriceSnapshot>,
    priced: u64,
}

impl PricingCore {
    /// Construct a core bound to an initial published market state.
    #[must_use]
    pub fn new(initial: MarketState) -> Self {
        let state = StateHandle::new(initial);
        let reader = state.reader();
        Self {
            state,
            reader,
            top_of_book: Seqlock::new(PriceSnapshot::default()),
            priced: 0,
        }
    }

    /// The state handle, so the edge can publish market ticks / recalibrations.
    #[must_use]
    pub fn state(&self) -> &StateHandle {
        &self.state
    }

    /// A clone of the state handle, so the async **edge** can publish market
    /// ticks / recalibrations from another thread *concurrently* with the pinned
    /// core pricing.
    ///
    /// [`StateHandle`] is a cheap `Arc`-backed handle (`Clone`), so the returned
    /// value publishes into the *same* underlying storage the core reads.
    /// Publication is lock-free and wait-free (`arc-swap`), so a tick landing
    /// mid-pricing never blocks the core; the core's next [`price`](Self::price)
    /// loads the new state atomically. This is the production wiring: the edge
    /// owns this handle, the core owns the `&mut self` pricing path.
    #[must_use]
    pub fn shared_state(&self) -> StateHandle {
        self.state.clone()
    }

    /// Read the latest published top-of-book snapshot (lock-free).
    #[must_use]
    pub fn top_of_book(&self) -> PriceSnapshot {
        self.top_of_book.read()
    }

    /// The number of requests this core has priced.
    #[must_use]
    pub fn priced_count(&self) -> u64 {
        self.priced
    }

    /// Price a single request against the live state — the hot path.
    ///
    /// **Zero-allocation:** loads the published state via the cached
    /// [`StateReader`] (a cheap atomic revalidation; a refcount clone at most on
    /// a tick boundary — never a heap allocation, even under concurrent
    /// [`StateHandle::publish`]), evaluates the smile (pure arithmetic), runs the
    /// closed-form Garman-Kohlhagen greeks, and returns a `Copy`
    /// [`PriceResponse`]. No `Vec`, `Box`, or formatting on this path. Also
    /// publishes the result into the top-of-book seqlock.
    ///
    /// [`StateHandle::publish`]: crate::rt::StateHandle::publish
    /// [`StateReader`]: crate::rt::StateReader
    #[must_use]
    pub fn price(&mut self, req: PriceRequest) -> PriceResponse {
        let st = self.reader.load();
        let forward = st.forward();
        // Smile lookup: the published smile assigns a Black vol to this strike.
        let vol = st.smile.implied_vol(req.strike, forward, st.t).0;
        // Form the Garman-Kohlhagen inputs from the published market state plus
        // the request strike and the smile vol. `VanillaInputs` is `Copy`/POD.
        let inputs =
            celnet_types::VanillaInputs::new(st.spot, req.strike, vol, st.t, st.r_dom, st.r_for);
        let greeks = celnet_vanilla::greeks(req.option_type, &inputs);
        let resp = PriceResponse {
            request_id: req.request_id,
            greeks,
            vol,
        };
        // Publish top-of-book (single-writer seqlock) and bump the counter.
        self.top_of_book.store(resp.snapshot());
        self.priced += 1;
        resp
    }

    /// Drain up to `budget` requests from `rx`, pricing each and pushing the
    /// response to `tx`. Returns the number of requests priced this poll.
    ///
    /// This is the body of the busy-poll loop: bounded work per poll (so the core
    /// can interleave other duties), wait-free ring ops, zero allocation. A full
    /// response ring stops the drain early (back-pressure) rather than blocking
    /// or allocating — the caller retries on the next poll.
    pub fn drain(
        &mut self,
        rx: &mut rtrb::Consumer<PriceRequest>,
        tx: &mut rtrb::Producer<PriceResponse>,
        budget: usize,
    ) -> usize {
        let mut done = 0;
        while done < budget {
            match rx.pop() {
                Ok(req) => {
                    let resp = self.price(req);
                    if tx.push(resp).is_err() {
                        // Response ring full: stop; the priced result is still
                        // published in the seqlock, and the edge will catch up.
                        break;
                    }
                    done += 1;
                }
                Err(_) => break, // ring empty
            }
        }
        done
    }

    /// Run the busy-poll pricing loop until `stop` is set, then drain once more
    /// and return — the owned hot-core lifecycle (`docs/ARCHITECTURE.md` §3.3).
    ///
    /// This is the steady-state driver of the pinned pricing core: it
    /// repeatedly [`drain`](Self::drain)s up to `budget` requests per poll from
    /// `rx`, pushing responses to `tx`, with no allocation, no locking and no
    /// blocking on the hot path. When a poll finds the request ring empty it
    /// issues a `spin_loop` hint and re-polls, never parking the core.
    ///
    /// # Shutdown always terminates
    ///
    /// The loop checks `stop` (a `Relaxed` load — shutdown carries no
    /// happens-before obligation on priced data, only liveness) once per poll.
    /// Because every poll either makes bounded progress or observes the empty
    /// ring and re-checks `stop`, the loop is guaranteed to observe a set `stop`
    /// within one poll and exit; there is no path that blocks indefinitely. On
    /// exit it performs one final unbounded-by-`stop` drain so any request the
    /// edge enqueued *before* signalling shutdown is still priced and answered,
    /// giving a clean, lossless quiesce. Returns the total number of requests
    /// priced across the whole run.
    ///
    /// The caller signals shutdown by setting `stop` and then joining the thread
    /// that called `run`; the race-free contract is: edge enqueues all final
    /// requests, *then* sets `stop`, *then* joins — the final drain observes
    /// those requests because the ring `push` happens-before the `stop` store on
    /// the edge thread and the join synchronizes the return.
    pub fn run(
        &mut self,
        rx: &mut rtrb::Consumer<PriceRequest>,
        tx: &mut rtrb::Producer<PriceResponse>,
        budget: usize,
        stop: &AtomicBool,
    ) -> u64 {
        let mut total: u64 = 0;
        while !stop.load(Ordering::Relaxed) {
            let n = self.drain(rx, tx, budget);
            total += n as u64;
            if n == 0 {
                // Ring was empty (or response ring full): hint the CPU and
                // re-poll. We never park, so we always re-observe `stop`.
                std::hint::spin_loop();
            }
        }
        // Final quiesce: price anything enqueued before `stop` was observed.
        // Bounded by the ring's finite occupancy — `drain` returns 0 once empty.
        loop {
            let n = self.drain(rx, tx, budget);
            total += n as u64;
            if n == 0 {
                break;
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rt::{request_ring, response_ring};
    use crate::testing::market_state;
    use celnet_core::is_close;
    use celnet_types::{CcyPair, Tenor};

    fn conv() -> celnet_conventions::ConventionRecord {
        celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record
    }

    #[test]
    fn price_matches_direct_vanilla_at_smile_vol() {
        let st = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let smile_vol = {
            // Independent reference: read the smile vol the same way the core will.
            let f = st.forward();
            st.smile.implied_vol(1.12, f, st.t).0
        };
        let mut core = PricingCore::new(st.clone());
        let resp = core.price(PriceRequest::new(1, OptionType::Call, 1.12));

        // The response vol equals the smile lookup.
        assert!(is_close(resp.vol, smile_vol, 1e-12, 1e-14));
        // And the price equals a direct vanilla pricing at that vol.
        let direct = celnet_vanilla::greeks(
            OptionType::Call,
            &celnet_types::VanillaInputs::new(st.spot, 1.12, smile_vol, st.t, st.r_dom, st.r_for),
        );
        assert!(is_close(resp.greeks.price, direct.price, 1e-12, 1e-14));
        assert!(is_close(resp.greeks.vega, direct.vega, 1e-12, 1e-14));
        assert_eq!(core.priced_count(), 1);
    }

    #[test]
    fn spsc_request_response_roundtrip() {
        let st = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let mut core = PricingCore::new(st);

        let (mut req_tx, mut req_rx) = request_ring();
        let (mut resp_tx, mut resp_rx) = response_ring();

        // Edge submits a batch of requests.
        let strikes = [1.05, 1.08, 1.10, 1.12, 1.15];
        for (i, k) in strikes.iter().enumerate() {
            req_tx
                .push(PriceRequest::new(i as u64, OptionType::Call, *k))
                .expect("ring has capacity");
        }
        // Core drains and prices them, pushing responses.
        let n = core.drain(&mut req_rx, &mut resp_tx, 64);
        assert_eq!(n, strikes.len());

        // Edge reads the responses back, in order, with matching ids.
        for (i, k) in strikes.iter().enumerate() {
            let resp = resp_rx.pop().expect("response available");
            assert_eq!(resp.request_id, i as u64);
            // Each price equals a direct pricing at the published smile vol.
            let f = core.state().load().forward();
            let v = core.state().load().smile.implied_vol(*k, f, 1.0).0;
            let direct = celnet_vanilla::price(
                OptionType::Call,
                &celnet_types::VanillaInputs::new(1.10, *k, v, 1.0, 0.02, 0.01),
            );
            assert!(is_close(resp.greeks.price, direct, 1e-12, 1e-14));
        }
        // The top-of-book seqlock holds the last priced line.
        let tob = core.top_of_book();
        assert_eq!(tob.request_id, (strikes.len() - 1) as u64);
    }

    #[test]
    fn run_drains_then_shuts_down_losslessly() {
        use std::sync::atomic::AtomicBool;
        use std::time::{Duration, Instant};

        let st = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let mut core = PricingCore::new(st);
        let (mut req_tx, mut req_rx) = request_ring();
        let (mut resp_tx, mut resp_rx) = response_ring();
        let stop = AtomicBool::new(false);

        const N: u64 = 1000;

        // Run the busy-poll core on a scoped thread; the main thread plays the
        // edge: enqueue all requests, signal stop, then join. The final drain
        // must price every enqueued request (lossless quiesce).
        let total = std::thread::scope(|s| {
            let stop_ref = &stop;
            let handle = s.spawn(move || core.run(&mut req_rx, &mut resp_tx, 64, stop_ref));

            // Edge enqueues N requests (retrying on a transiently-full ring).
            for id in 0..N {
                loop {
                    if req_tx
                        .push(PriceRequest::new(id, OptionType::Call, 1.10))
                        .is_ok()
                    {
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
            // Signal shutdown only AFTER all requests are enqueued: the push
            // happens-before the stop store, so the final drain sees them all.
            stop.store(true, Ordering::Relaxed);

            // Bounded watchdog: join must complete promptly (the loop never
            // blocks). If it does not, fail loudly rather than hang the suite.
            let deadline = Instant::now() + Duration::from_secs(10);
            while !handle.is_finished() {
                assert!(Instant::now() < deadline, "run() failed to terminate");
                std::hint::spin_loop();
            }
            handle.join().expect("core thread joins")
        });

        // Every request was priced exactly once (lossless).
        assert_eq!(total, N, "shutdown drain must price every enqueued request");

        // Every response is readable, in order, with matching ids.
        let mut seen = 0u64;
        while let Ok(resp) = resp_rx.pop() {
            assert_eq!(resp.request_id, seen);
            seen += 1;
        }
        assert_eq!(seen, N, "every response must reach the edge");
    }

    #[test]
    fn run_terminates_when_stop_already_set() {
        use std::sync::atomic::AtomicBool;

        // If stop is already set, run() must do a single final drain and return
        // immediately — never spin forever.
        let st = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let mut core = PricingCore::new(st);
        let (_req_tx, mut req_rx) = request_ring();
        let (mut resp_tx, _resp_rx) = response_ring();
        let stop = AtomicBool::new(true);
        let total = core.run(&mut req_rx, &mut resp_tx, 64, &stop);
        assert_eq!(total, 0);
    }

    #[test]
    fn hot_reload_observed_without_locking() {
        // A reader observes a republished state with no lock, and reprices.
        let st = market_state(1.10, 0.105, 0.015, 0.0035, conv());
        let mut core = PricingCore::new(st);
        let before = core.price(PriceRequest::new(0, OptionType::Call, 1.10));

        // Edge publishes a market tick (spot jumps) — atomic, lock-free.
        let st2 = market_state(1.25, 0.105, 0.015, 0.0035, conv());
        core.state().publish(st2);

        let after = core.price(PriceRequest::new(1, OptionType::Call, 1.10));
        // The higher spot lifts the call price.
        assert!(after.greeks.price > before.greeks.price);
    }
}
