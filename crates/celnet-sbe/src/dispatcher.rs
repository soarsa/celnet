//! Zero-allocation SBE message dispatcher.
//!
//! Provides high-throughput fragment dispatching modeled after Aeron's
//! `FragmentHandler` and Agrona's `ControlledFragmentHandler`.
#![deny(missing_docs)]

use crate::{
    ExecutionReportFlyweight, MessageHeader, OptionQuoteFlyweight, PriceTickFlyweight, SbeError,
    TemplateId,
};

/// An enumerated wrapper over all SBE message flyweight types.
#[derive(Debug)]
pub enum SbeMessageRef<'a> {
    /// Spot price tick.
    PriceTick(PriceTickFlyweight<'a>),
    /// Two-way option quote with full Greek sensitivities.
    OptionQuote(OptionQuoteFlyweight<'a>),
    /// Confirmed execution report.
    ExecutionReport(ExecutionReportFlyweight<'a>),
    /// Raw unparsed SBE payload frame.
    RawFrame {
        /// Header of the unparsed message.
        header: MessageHeader,
        /// Entire slice buffer.
        payload: &'a [u8],
    },
}

/// Zero-allocation SBE message fragment dispatcher.
pub struct SbeDispatcher;

impl SbeDispatcher {
    /// Parse and wrap an SBE frame into an `SbeMessageRef` without heap allocation.
    #[inline(always)]
    pub fn dispatch(buffer: &[u8]) -> Result<SbeMessageRef<'_>, SbeError> {
        let header = MessageHeader::decode(buffer)?;
        let template = TemplateId::from_u16(header.template_id)
            .ok_or(SbeError::UnknownTemplateId(header.template_id))?;

        match template {
            TemplateId::PriceTick => {
                let fw = PriceTickFlyweight::wrap(buffer)?;
                Ok(SbeMessageRef::PriceTick(fw))
            }
            TemplateId::OptionQuote => {
                let fw = OptionQuoteFlyweight::wrap(buffer)?;
                Ok(SbeMessageRef::OptionQuote(fw))
            }
            TemplateId::ExecutionReport => {
                let fw = ExecutionReportFlyweight::wrap(buffer)?;
                Ok(SbeMessageRef::ExecutionReport(fw))
            }
            _ => Ok(SbeMessageRef::RawFrame {
                header,
                payload: buffer,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PriceTick, encode_price_tick};

    #[test]
    fn test_dispatcher_routing() {
        let tick = PriceTick {
            epoch_nanos: 1_725_000_000,
            pair_id: 1,
            flags: 0,
            bid: 1.0850,
            ask: 1.0852,
        };
        let mut buf = [0u8; 128];
        let len = encode_price_tick(&tick, &mut buf).unwrap();

        let msg = SbeDispatcher::dispatch(&buf[..len]).unwrap();
        match msg {
            SbeMessageRef::PriceTick(fw) => {
                assert_eq!(fw.pair_id(), 1);
                assert_eq!(fw.bid(), 1.0850);
                assert_eq!(fw.ask(), 1.0852);
            }
            _ => panic!("Expected PriceTick"),
        }
    }
}
