//! The client error type: a single, current, transport-and-protocol error union
//! the SDK surfaces to callers instead of leaking raw `tonic` / wire detail.
//!
//! There is exactly one error enum (no versioned error contract): every fallible
//! SDK call returns [`ClientResult`]. A [`tonic::Status`] (a server-side
//! application error) and a [`tonic::transport::Error`] (a connect/transport
//! failure) are both folded into [`ClientError`], with the protocol-shape
//! violations the SDK detects (a missing required field on a server message, an
//! out-of-range enum) carried as dedicated variants so a caller can branch on the
//! *kind* of failure without string-matching.

use celnet_proto::convert::WireError;

/// A failure raised by a `celnet-client` call.
#[derive(Debug)]
#[non_exhaustive]
pub enum ClientError {
    /// The gRPC transport could not connect, was reset, or the endpoint was
    /// malformed.
    Transport(tonic::transport::Error),
    /// The server returned an application-level status (e.g. `not_found` for an
    /// unknown quote id, `deadline_exceeded` for a last-look expiry,
    /// `unavailable` when the edge is draining). Boxed to keep the error enum
    /// small (a [`tonic::Status`] is large), so a `Result<_, ClientError>` stays
    /// cheap to pass by value.
    Status(Box<tonic::Status>),
    /// A server message was missing a field the contract requires to be present
    /// (e.g. a `Quote` with no `price`, a `Snapshot` with no `subscription`).
    MissingField(&'static str),
    /// A wire enum / value the contract carried could not be mapped to the typed
    /// domain vocabulary (an out-of-range proto3 enum tag, a malformed currency).
    Wire(WireError),
    /// The RFS stream closed before the SDK could establish a baseline snapshot,
    /// or closed for a non-recoverable reason while a subscription was open.
    StreamClosed,
    /// A click-to-trade [`crate::ExecuteOutcome`] could not be resolved because the
    /// session re-dialed (a `DRAINING` blue-green cutover) while the click was in
    /// flight: the in-flight token belonged to the pre-cutover session and can no
    /// longer be booked, so the click is failed promptly rather than left to hang.
    /// The caller re-clicks the *current* line on the reconnected session.
    Reconnected,
    /// The endpoint URI handed to [`crate::Client::connect`] was malformed.
    InvalidEndpoint(String),
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ClientError::Transport(e) => write!(f, "transport error: {e}"),
            ClientError::Status(s) => {
                write!(f, "server status {:?}: {}", s.code(), s.message())
            }
            ClientError::MissingField(field) => {
                write!(f, "server message missing required field `{field}`")
            }
            ClientError::Wire(e) => write!(f, "wire decode error: {e}"),
            ClientError::StreamClosed => write!(f, "RFS stream closed unexpectedly"),
            ClientError::Reconnected => write!(
                f,
                "session re-dialed (blue-green cutover) while the click-to-trade was in flight; re-click the current line"
            ),
            ClientError::InvalidEndpoint(uri) => write!(f, "invalid endpoint URI: {uri}"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Transport(e) => Some(e),
            ClientError::Status(e) => Some(e.as_ref()),
            ClientError::Wire(e) => Some(e),
            _ => None,
        }
    }
}

impl From<tonic::transport::Error> for ClientError {
    fn from(value: tonic::transport::Error) -> Self {
        ClientError::Transport(value)
    }
}

impl From<tonic::Status> for ClientError {
    fn from(value: tonic::Status) -> Self {
        ClientError::Status(Box::new(value))
    }
}

impl From<WireError> for ClientError {
    fn from(value: WireError) -> Self {
        ClientError::Wire(value)
    }
}

/// The result alias every fallible `celnet-client` call returns.
pub type ClientResult<T> = Result<T, ClientError>;
