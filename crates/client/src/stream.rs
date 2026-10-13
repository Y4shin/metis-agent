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
/// The stream opens with the snapshot for the channels it selected, then carries
/// the events that follow, with heartbeats on an idle stretch. Its identifiers
/// never decrease. Dropping the stream drops the connection, and opening a new
/// one is how a caller resyncs.
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
    /// the same headers.
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
