//! A minimal [`futures_core::Stream`] adaptor over a bounded
//! [`tokio::sync::mpsc::Receiver`], used as the server→client side of the RFS
//! bidirectional gRPC stream.
//!
//! `tonic` accepts any `Stream<Item = Result<T, Status>> + Send` as a streaming
//! response. Rather than pull in a `tokio-stream` dependency, this wraps a
//! receiver and forwards [`tokio::sync::mpsc::Receiver::poll_recv`] into
//! [`futures_core::Stream::poll_next`] — a few lines, no extra crate.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use tokio::sync::mpsc::Receiver;

/// A [`Stream`] yielding every item sent on the wrapped bounded channel until the
/// sender side is dropped and the buffer drains.
#[derive(Debug)]
pub struct ReceiverStream<T> {
    rx: Receiver<T>,
}

impl<T> ReceiverStream<T> {
    /// Wrap a bounded receiver as a stream.
    #[must_use]
    pub fn new(rx: Receiver<T>) -> Self {
        Self { rx }
    }
}

impl<T> Stream for ReceiverStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.rx.poll_recv(cx)
    }
}
