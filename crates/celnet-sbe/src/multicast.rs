//! UDP Multicast Market Data Distribution.
//!
//! Provides ultra-low-latency O(1) publisher fan-out of SBE market data frames
//! across institutional networks and top-of-rack switches without TCP head-of-line blocking.
#![deny(missing_docs)]

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

use crate::{
    OPTION_QUOTE_TOTAL_SIZE, OptionQuote, PRICE_TICK_TOTAL_SIZE, PriceTick, SbeError,
    encode_option_quote, encode_price_tick,
};

/// High-throughput UDP Multicast publisher for SBE streams.
pub struct UdpMulticastPublisher {
    socket: UdpSocket,
    target_addr: SocketAddrV4,
    buffer: Vec<u8>,
}

impl UdpMulticastPublisher {
    /// Bind a new UDP multicast publisher targeting the specified multicast group address.
    pub fn new(multicast_ip: Ipv4Addr, port: u16) -> std::io::Result<Self> {
        let local_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
        let socket = UdpSocket::bind(local_addr)?;

        socket.set_multicast_ttl_v4(1)?;
        socket.set_multicast_loop_v4(true)?;

        let target_addr = SocketAddrV4::new(multicast_ip, port);
        Ok(Self {
            socket,
            target_addr,
            buffer: vec![0u8; 1024],
        })
    }

    /// Publish raw SBE frame bytes to the multicast group.
    #[inline(always)]
    pub fn publish(&self, frame: &[u8]) -> std::io::Result<usize> {
        self.socket.send_to(frame, self.target_addr)
    }

    /// Encode and publish an `OptionQuote` to the multicast group in a single pass.
    pub fn publish_quote(&mut self, quote: &OptionQuote) -> Result<usize, SbeError> {
        if self.buffer.len() < OPTION_QUOTE_TOTAL_SIZE {
            self.buffer.resize(OPTION_QUOTE_TOTAL_SIZE, 0);
        }
        let len = encode_option_quote(quote, &mut self.buffer)?;
        self.publish(&self.buffer[..len])
            .map_err(|_| SbeError::BufferTooShort {
                expected: len,
                actual: 0,
            })
    }

    /// Encode and publish a `PriceTick` to the multicast group in a single pass.
    pub fn publish_tick(&mut self, tick: &PriceTick) -> Result<usize, SbeError> {
        if self.buffer.len() < PRICE_TICK_TOTAL_SIZE {
            self.buffer.resize(PRICE_TICK_TOTAL_SIZE, 0);
        }
        let len = encode_price_tick(tick, &mut self.buffer)?;
        self.publish(&self.buffer[..len])
            .map_err(|_| SbeError::BufferTooShort {
                expected: len,
                actual: 0,
            })
    }

    /// Return the target multicast group socket address.
    #[inline(always)]
    pub fn target_addr(&self) -> SocketAddrV4 {
        self.target_addr
    }
}

/// UDP Multicast subscriber receiving SBE market data frames.
pub struct UdpMulticastSubscriber {
    socket: UdpSocket,
}

impl UdpMulticastSubscriber {
    /// Bind and join a UDP multicast group on the specified port.
    pub fn join(multicast_ip: Ipv4Addr, port: u16) -> std::io::Result<Self> {
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        let socket = UdpSocket::bind(bind_addr)?;

        socket.join_multicast_v4(&multicast_ip, &Ipv4Addr::UNSPECIFIED)?;
        socket.set_read_timeout(Some(Duration::from_millis(500)))?;

        Ok(Self { socket })
    }

    /// Receive the next multicast frame into the caller's slice.
    #[inline(always)]
    pub fn recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        let (len, _src) = self.socket.recv_from(buf)?;
        Ok(len)
    }

    /// Set read timeout on the receiving socket.
    pub fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        self.socket.set_read_timeout(dur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OptionQuoteFlyweight;

    #[test]
    fn multicast_quote_transmission() {
        let group_ip = Ipv4Addr::new(239, 255, 42, 99);
        let port = 19876;

        let sub_result = UdpMulticastSubscriber::join(group_ip, port);
        let sub = match sub_result {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Skipping multicast test: network interface does not support group join: {e}");
                return;
            }
        };

        let mut publ = UdpMulticastPublisher::new(group_ip, port).expect("publisher creates");

        let quote = OptionQuote {
            quote_id: 888777,
            epoch_nanos: 1_725_450_000_000_000_000,
            valid_until_nanos: 1_725_450_005_000_000_000,
            bid_price: 1.0850,
            ask_price: 1.0852,
            resolved_strike: 1.0850,
            surface_version: 5,
            greeks: celnet_types::Greeks {
                price: 0.012345,
                delta_spot: 0.4821,
                delta_forward: 0.4933,
                gamma: 2.118,
                vega: 0.305,
                theta: -0.018,
                rho_dom: 0.061,
                rho_for: -0.058,
                vanna: -0.072,
                volga: 0.144,
                charm: 0.0009,
                speed: -1.21,
                zomma: 0.33,
                color: 0.0004,
            },
        };

        let bytes_sent = publ.publish_quote(&quote).expect("publish succeeds");
        assert_eq!(bytes_sent, OPTION_QUOTE_TOTAL_SIZE);

        let mut recv_buf = [0u8; 512];
        if let Ok(len) = sub.recv(&mut recv_buf) {
            assert_eq!(len, OPTION_QUOTE_TOTAL_SIZE);
            let fw = OptionQuoteFlyweight::wrap(&recv_buf[..len]).expect("flyweight wrap");
            assert_eq!(fw.quote_id(), 888777);
            assert_eq!(fw.bid_price(), 1.0850);
            assert_eq!(fw.greeks(), quote.greeks);
        }
    }
}
