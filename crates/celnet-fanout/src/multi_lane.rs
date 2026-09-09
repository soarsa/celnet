//! Multi-Lane Sharded Broadcast Ring
//!
//! Partitions counterparty reader sessions across multiple independent ring lanes
//! to eliminate cross-core cache invalidation storms and NUMA interconnect saturation.

use crate::padded_ring::{CachePaddedBroadcastRing, CachePaddedConsumer, CachePaddedProducer};
use crate::ring::RecvError;

/// A multi-lane sharded broadcast ring for ultra-high fan-out (1,000+ consumers).
pub struct MultiLaneBroadcastRing<T> {
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Copy + Default + Send> MultiLaneBroadcastRing<T> {
    /// Create a multi-lane broadcast ring with `num_lanes` independent lanes.
    pub fn new(
        num_lanes: usize,
        capacity_per_lane: usize,
    ) -> (MultiLaneProducer<T>, MultiLaneConsumerFactory<T>) {
        assert!(num_lanes >= 1, "must have at least one lane");
        let mut producers = Vec::with_capacity(num_lanes);
        let mut consumers = Vec::with_capacity(num_lanes);

        for _ in 0..num_lanes {
            let (p, c) = CachePaddedBroadcastRing::new(capacity_per_lane);
            producers.push(p);
            consumers.push(c);
        }

        let factory = MultiLaneConsumerFactory {
            consumers,
            num_lanes,
        };

        let producer = MultiLaneProducer {
            lanes: producers,
            num_lanes,
        };

        (producer, factory)
    }
}

/// The multi-lane producer end. Publishes items to all lanes concurrently.
pub struct MultiLaneProducer<T> {
    lanes: Vec<CachePaddedProducer<T>>,
    num_lanes: usize,
}

impl<T: Copy + Default + Send> MultiLaneProducer<T> {
    /// Publish an item to all lanes.
    #[inline]
    pub fn publish(&mut self, item: T) {
        for lane in &mut self.lanes {
            lane.publish(item);
        }
    }

    /// Total items published so far (from lane 0).
    #[inline]
    pub fn published(&self) -> u64 {
        self.lanes[0].published()
    }

    /// Number of parallel lanes.
    #[inline]
    pub fn num_lanes(&self) -> usize {
        self.num_lanes
    }
}

/// Factory for creating consumers sharded by subscriber/session ID.
pub struct MultiLaneConsumerFactory<T> {
    consumers: Vec<CachePaddedConsumer<T>>,
    num_lanes: usize,
}

impl<T: Copy + Default + Send> Clone for MultiLaneConsumerFactory<T> {
    fn clone(&self) -> Self {
        Self {
            consumers: self.consumers.clone(),
            num_lanes: self.num_lanes,
        }
    }
}

impl<T: Copy + Default + Send> MultiLaneConsumerFactory<T> {
    /// Subscribe a counterparty session to its designated lane.
    pub fn subscribe(&self, subscriber_id: usize) -> MultiLaneConsumer<T> {
        let lane_idx = subscriber_id % self.num_lanes;
        let consumer = self.consumers[lane_idx].fork_from_head();
        MultiLaneConsumer {
            consumer,
            lane_idx,
        }
    }

    /// Number of lanes configured.
    #[inline]
    pub fn num_lanes(&self) -> usize {
        self.num_lanes
    }
}

/// A consumer pinned to a specific ring lane.
pub struct MultiLaneConsumer<T> {
    consumer: CachePaddedConsumer<T>,
    lane_idx: usize,
}

impl<T: Copy + Default + Send> MultiLaneConsumer<T> {
    /// Try to receive the next item from this consumer's lane.
    #[inline]
    pub fn try_recv(&mut self) -> Result<T, RecvError> {
        self.consumer.try_recv()
    }

    /// The assigned lane index.
    #[inline]
    pub fn lane(&self) -> usize {
        self.lane_idx
    }

    /// Number of received items.
    #[inline]
    pub fn received(&self) -> u64 {
        self.consumer.received()
    }

    /// Number of skipped items.
    #[inline]
    pub fn skipped(&self) -> u64 {
        self.consumer.skipped()
    }
}
