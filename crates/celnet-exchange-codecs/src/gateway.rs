//! Unified Reactive Exchange Gateway & Market Data Feed SPI.
//!
//! Provides a protocol-agnostic service provider interface (SPI) unifying
//! packet decoding, gap detection, snapshot synchronization, and book updates.

use crate::ExchangeCodecError;

/// Book update notification dispatched to matching or pricing engines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BookAction {
    /// Add or update price level quantity.
    UpdateLevel {
        /// Level price.
        price: f64,
        /// New aggregate quantity.
        quantity: f64,
        /// Side: true = bid, false = ask.
        is_bid: bool,
    },
    /// Delete price level.
    DeleteLevel {
        /// Level price to delete.
        price: f64,
        /// Side: true = bid, false = ask.
        is_bid: bool,
    },
    /// Clear entire order book (e.g. prior to snapshot load).
    ClearBook,
}

/// Dispatcher interface implemented by order book state machines.
pub trait BookUpdateDispatcher {
    /// Apply an atomic book action.
    fn dispatch_action(&mut self, instrument_id: u32, action: BookAction);
}

/// Action to take when a message sequence gap is detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapAction {
    /// Sequence is contiguous, continue normal processing.
    None,
    /// Recoverable packet drop; request TCP replay from `from_seq` to `to_seq`.
    RequestReplay {
        /// Starting missing sequence number.
        from_seq: u64,
        /// Ending missing sequence number.
        to_seq: u64,
    },
    /// Non-recoverable sequence jump; flush book and trigger out-of-band snapshot.
    TriggerSnapshot,
}

/// Feed health status of the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedHealth {
    /// Operating normally with zero gaps.
    Healthy,
    /// In synchronization / snapshot recovery.
    Synchronizing,
    /// Connection degraded or packets dropped.
    Degraded,
}

/// Standardized Market Data Feed Provider SPI.
pub trait MarketDataFeedHandler: Send + Sync {
    /// Stable feed identifier (e.g. "CME-MDP3", "NASDAQ-ITCH50", "EUREX-T7").
    fn feed_id(&self) -> &'static str;

    /// Process a raw wire packet slice (zero-allocation hot path).
    ///
    /// Returns number of messages processed.
    fn on_packet(
        &mut self,
        packet: &[u8],
        timestamp_ns: u64,
        dispatcher: &mut dyn BookUpdateDispatcher,
    ) -> Result<usize, ExchangeCodecError>;

    /// Check and handle packet sequence continuity.
    fn check_sequence(&mut self, incoming_seq: u64) -> GapAction;

    /// Report current feed operational health.
    fn feed_health(&self) -> FeedHealth;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockDispatcher {
        actions: Vec<(u32, BookAction)>,
    }
    impl BookUpdateDispatcher for MockDispatcher {
        fn dispatch_action(&mut self, instrument_id: u32, action: BookAction) {
            self.actions.push((instrument_id, action));
        }
    }

    struct SimpleMdpHandler {
        expected_seq: u64,
        health: FeedHealth,
    }

    impl SimpleMdpHandler {
        fn new() -> Self {
            Self {
                expected_seq: 1,
                health: FeedHealth::Healthy,
            }
        }
    }

    impl MarketDataFeedHandler for SimpleMdpHandler {
        fn feed_id(&self) -> &'static str {
            "CME-MDP3-SIM"
        }

        fn on_packet(
            &mut self,
            packet: &[u8],
            _timestamp_ns: u64,
            dispatcher: &mut dyn BookUpdateDispatcher,
        ) -> Result<usize, ExchangeCodecError> {
            if packet.len() < 4 {
                return Err(ExchangeCodecError::BufferUnderflow {
                    expected: 4,
                    actual: packet.len(),
                });
            }
            // Mock dispatch
            dispatcher.dispatch_action(
                1001,
                BookAction::UpdateLevel {
                    price: 1.0850,
                    quantity: 50.0,
                    is_bid: true,
                },
            );
            Ok(1)
        }

        fn check_sequence(&mut self, incoming_seq: u64) -> GapAction {
            if incoming_seq == self.expected_seq {
                self.expected_seq += 1;
                GapAction::None
            } else if incoming_seq > self.expected_seq {
                let from_seq = self.expected_seq;
                let to_seq = incoming_seq - 1;
                self.expected_seq = incoming_seq + 1;
                GapAction::RequestReplay { from_seq, to_seq }
            } else {
                // Duplicate or old packet
                GapAction::None
            }
        }

        fn feed_health(&self) -> FeedHealth {
            self.health
        }
    }

    #[test]
    fn test_feed_handler_spi() {
        let mut handler = SimpleMdpHandler::new();
        assert_eq!(handler.feed_id(), "CME-MDP3-SIM");
        assert_eq!(handler.feed_health(), FeedHealth::Healthy);

        // Test normal sequence
        assert_eq!(handler.check_sequence(1), GapAction::None);

        // Test sequence gap
        assert_eq!(
            handler.check_sequence(5),
            GapAction::RequestReplay {
                from_seq: 2,
                to_seq: 4
            }
        );

        // Test packet handling
        let mut dispatcher = MockDispatcher { actions: vec![] };
        let packet = [0x01, 0x02, 0x03, 0x04];
        let processed = handler.on_packet(&packet, 1_000_000, &mut dispatcher).unwrap();
        assert_eq!(processed, 1);
        assert_eq!(dispatcher.actions.len(), 1);
    }
}
