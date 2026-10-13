//! The event stream the wrapper hands out.
//!
//! A caller drains an [`EventStream`] one event at a time and never names the
//! transport, the generated client, or a buffa view type: the events it yields
//! are [`api_contracts::events::Event`], the same type the daemon's ring
//! publishes, so the two sides of the stream cannot drift on what an event is.

use api_contracts::events::Event;
use api_contracts::proto::metis::v1::{Event as EventMessage, EventView};
use connectrpc::client::ServerStream;

use crate::ClientError;

/// The response body the local and remote transports answer with. It is named
/// here and nowhere else in the wrapper: no caller sees it.
type Body = hyper::body::Incoming;

/// A live event stream from the daemon.
///
/// A stream opened without a cursor begins with the snapshot for the channels it
/// selected (one empty snapshot message when they hold nothing); a stream opened
/// with a cursor begins with the events that follow it and carries no snapshot.
/// Either way the live events follow, with heartbeats on an idle stretch, and
/// the identifiers never decrease.
///
/// The cursor a caller resumes by is the highest `EVENT` identifier it has
/// processed: a `SNAPSHOT` advances it only for the channels that message names,
/// and a `HEARTBEAT` advances nothing even though it carries the daemon's latest
/// identifier. Dropping the stream drops the connection, and reconnecting with
/// that cursor replays what the ring still holds. A cursor the ring can no longer
/// serve is refused at [`Client::stream_events`](crate::Client::stream_events)
/// as [`ClientError::CursorExpired`](crate::ClientError::CursorExpired), whose
/// answer is a cursorless stream, which begins with the snapshot.
pub struct EventStream {
    inner: ServerStream<Body, EventView<'static>>,
}

impl std::fmt::Debug for EventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventStream")
            .field("stream", &self.inner)
            .finish()
    }
}

impl EventStream {
    /// Wrap the generated streaming call.
    pub(crate) fn new(inner: ServerStream<Body, EventView<'static>>) -> Self {
        Self { inner }
    }

    /// Fetch the next event, or `None` when the stream ends cleanly.
    ///
    /// An error is terminal: the stream will not carry another event, and
    /// recovering means opening a new stream. The daemon's own refusals are
    /// reported as the typed errors a unary call reports, because they travel in
    /// the same headers. The cursor to resume by is the highest `EVENT`
    /// identifier seen, and a refusal this stream cannot earn (an expired cursor
    /// is refused at [`Client::stream_events`](crate::Client::stream_events)) is
    /// answered with a cursorless stream.
    ///
    /// # Errors
    ///
    /// Returns the wrapper's typed error for a refusal the daemon labelled, and
    /// [`ClientError::Call`] when the stream fails for any other reason.
    pub async fn message(&mut self) -> Result<Option<Event>, ClientError> {
        match self.inner.message::<EventMessage>().await {
            Ok(Some(message)) => Ok(Some(Event::from_message(&message.to_owned_message()))),
            Ok(None) => Ok(None),
            Err(err) => Err(crate::refusal(err)),
        }
    }
}
