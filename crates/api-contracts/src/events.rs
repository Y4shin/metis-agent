//! The daemon's event ring: one monotonic identifier space, one bounded window
//! of recent events, and the live subscription a stream is served from.
//!
//! The ring is in the shared crate because both sides need it: the daemon
//! serves it, and the client's in-process service and walkthrough example drive
//! a stream from one. There is one ring per daemon, not one per channel, so
//! identifiers compare across every channel and a stream can carry several at
//! once.
//!
//! The ring is deliberately in memory and bounded. It is a window on what
//! happened, not the source of truth: past its capacity the oldest event is
//! dropped and [`Ring::earliest_resumable`] advances, and a restart empties it.
//!
//! A cursor is a caller's position: the highest event identifier it has
//! processed. A stream opened with one resumes, replaying what the window still
//! holds after it and carrying no snapshot, and a cursor the window has moved
//! past, or a restart has invalidated, is refused as [`CursorError::Expired`]
//! before the stream opens. The answer to that refusal is a cursorless stream,
//! which begins with the snapshot.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use connectrpc::{ConnectError, ErrorCode};
use futures_util::StreamExt as _;
use futures_util::stream::{self, Stream};
use http::{HeaderMap, HeaderValue};
use tokio::sync::broadcast;

use crate::cancel::InFlight;
use crate::proto::metis::v1::{self, EventKind};

/// Why a cursor cannot be served.
///
/// One value covers both ways a cursor expires, because a caller's answer is
/// the same either way: open a cursorless stream, which begins with the
/// snapshot. The window's bounds are in the value rather than in prose, so a
/// caller can say how far the daemon has moved on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorError {
    /// The cursor is outside what the ring can still serve: the window moved
    /// past it, or the daemon restarted and the identifier space began again.
    Expired {
        /// The cursor the caller presented.
        cursor: u64,
        /// The oldest identifier the ring can still resume from.
        earliest_resumable: u64,
        /// The newest identifier the ring has assigned.
        latest_id: u64,
    },
}

impl fmt::Display for CursorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expired {
                cursor,
                earliest_resumable,
                latest_id,
            } => {
                if cursor > latest_id {
                    write!(
                        f,
                        "cursor {cursor} is expired: the daemon restarted and its identifier space began again, so the newest identifier is {latest_id} and the window begins at {earliest_resumable}"
                    )
                } else {
                    write!(
                        f,
                        "cursor {cursor} is expired: the window moved past it, and now begins at {earliest_resumable} and ends at {latest_id}"
                    )
                }
            }
        }
    }
}

/// The response header a cursor refusal carries.
///
/// A refusal's meaning crosses the wire as this header rather than as message
/// text, so a client maps it to a typed error instead of parsing prose. A
/// cursor is refused before the stream opens, so the header travels on the
/// response the stream was established with, exactly as the credential and
/// version refusals do.
pub const CURSOR_ERROR_HEADER: &str = "metis-cursor-error";

/// How a cursor was refused, as it crosses the wire.
///
/// The enum is non-exhaustive: a later way to refuse a cursor adds itself
/// without breaking a caller that matches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CursorRefusal {
    /// The cursor is outside what the ring can still serve.
    Expired,
}

impl CursorRefusal {
    /// The header value this refusal crosses the wire as.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
        }
    }

    /// Read a refusal from the header value it crossed the wire as.
    ///
    /// `None` means the value is not a refusal this build knows, which leaves
    /// the caller with the transport error rather than a guess.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }
}

impl fmt::Display for CursorRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The response headers a cursor refusal carries.
#[must_use]
pub fn cursor_refusal_headers(error: &CursorError) -> HeaderMap {
    let mut headers = HeaderMap::new();
    match error {
        CursorError::Expired { .. } => headers.insert(
            CURSOR_ERROR_HEADER,
            HeaderValue::from_static(CursorRefusal::Expired.as_str()),
        ),
    };
    headers
}

impl std::error::Error for CursorError {}

/// Why a stream ended in the middle rather than carrying what was published.
///
/// A stream opens before any of this can happen, so a refusal here is not the
/// answer to the call that opened it: it is the stream's own end, and its
/// meaning crosses the wire as
/// [`STREAM_ERROR_HEADER`] in the stream's trailing metadata rather than as an
/// HTTP status on a response that was already sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamError {
    /// The caller fell behind: it did not read the stream fast enough, and the
    /// daemon's bounded window dropped events before they could be carried.
    SlowConsumer {
        /// How many events the window dropped before the caller could read
        /// them. A gap is never silent: the count is named here and in the
        /// message the caller is answered with.
        skipped: u64,
    },
    /// The request that owned this stream was cancelled: a `Cancel` call named
    /// the identifier the stream registered under.
    Cancelled,
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SlowConsumer { skipped } => write!(
                f,
                "the caller fell behind the stream and {skipped} events were dropped before they could be carried; open a fresh stream to resync"
            ),
            Self::Cancelled => f.write_str(
                "the request that owned this stream was cancelled, so the stream stopped",
            ),
        }
    }
}

impl std::error::Error for StreamError {}

/// The trailer a stream refusal carries.
///
/// The meaning crosses the wire as this header and never as message text, so a
/// client maps it to a typed error instead of parsing prose. A stream refusal
/// happens after the response was established, so the header travels in the
/// trailing metadata of a Connect stream, which is the only place a mid-stream
/// failure has to put it.
pub const STREAM_ERROR_HEADER: &str = "metis-stream-error";

/// How a stream was dropped, as it crosses the wire.
///
/// The enum is non-exhaustive: a later way to end a stream adds itself without
/// breaking a caller that matches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamRefusal {
    /// The caller did not read the stream fast enough.
    SlowConsumer,
    /// The request that owned the stream was cancelled.
    Cancelled,
}

impl StreamRefusal {
    /// The header value this refusal crosses the wire as.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SlowConsumer => "slow-consumer",
            Self::Cancelled => "cancelled",
        }
    }

    /// Read a refusal from the header value it crossed the wire as.
    ///
    /// `None` means the value is not a refusal this build knows, which leaves
    /// the caller with the transport error rather than a guess.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "slow-consumer" => Some(Self::SlowConsumer),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

impl fmt::Display for StreamRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The trailer a stream refusal carries.
#[must_use]
pub fn stream_refusal_headers(error: &StreamError) -> HeaderMap {
    let mut headers = HeaderMap::new();
    match error {
        StreamError::SlowConsumer { .. } => headers.insert(
            STREAM_ERROR_HEADER,
            HeaderValue::from_static(StreamRefusal::SlowConsumer.as_str()),
        ),
        StreamError::Cancelled => headers.insert(
            STREAM_ERROR_HEADER,
            HeaderValue::from_static(StreamRefusal::Cancelled.as_str()),
        ),
    };
    headers
}

/// Refuse a stream that cannot continue, in the shape every serving path uses.
///
/// A stream refusal happens after the response was established, so the meaning
/// travels in the trailing metadata, and the status code names the reason for a
/// caller that reads no header.
fn stream_refusal(error: &StreamError) -> ConnectError {
    let code = match error {
        StreamError::SlowConsumer { .. } => ErrorCode::ResourceExhausted,
        StreamError::Cancelled => ErrorCode::Canceled,
    };
    ConnectError::new(code, error.to_string()).with_trailers(stream_refusal_headers(error))
}

/// One event: the Rust mirror of the contract's `Event` message.
///
/// `kind` is the generated enum, so the two sides cannot drift on what a
/// snapshot, a heartbeat, or an event is. The payload is the event's structured
/// content as text, opaque to the ring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// The event's daemon-wide identifier.
    pub id: u64,
    /// What this event is.
    pub kind: EventKind,
    /// The channel the event belongs to.
    pub channel: String,
    /// The event's structured content.
    pub payload: String,
}

impl Event {
    /// The wire message for this event.
    #[must_use]
    pub fn to_message(&self) -> v1::Event {
        v1::Event {
            id: self.id,
            kind: self.kind.into(),
            channel: self.channel.clone(),
            payload: self.payload.clone(),
            ..Default::default()
        }
    }

    /// The event a wire message carries.
    ///
    /// A kind this build does not know reads as
    /// [`EventKind::Unspecified`], which is the contract's own "cannot act on
    /// this" value, rather than as a guess.
    #[must_use]
    pub fn from_message(message: &v1::Event) -> Self {
        Self {
            id: message.id,
            kind: message.kind.as_known().unwrap_or_default(),
            channel: message.channel.clone(),
            payload: message.payload.clone(),
        }
    }
}

/// The daemon's events: one identifier space, one bounded window, one live
/// notification fan-out.
///
/// The handle is clonable, so the daemon can publish into the same ring its
/// service serves and a test can hold a second handle to it.
#[derive(Clone)]
pub struct Ring {
    inner: Arc<RingInner>,
}

struct RingInner {
    /// The most events the window holds; the oldest is dropped past it.
    capacity: usize,
    state: Mutex<State>,
    /// The live fan-out. Every subscriber gets every published event; the
    /// capacity is the ring's, so a subscriber that fell behind loses exactly
    /// what the ring dropped.
    notifications: broadcast::Sender<Event>,
}

struct State {
    events: VecDeque<Event>,
    /// The identifier the next published event gets. Identifiers start at one,
    /// so zero means "nothing published yet".
    next_id: u64,
}

impl State {
    /// The oldest identifier a cursor can still resume from: the identifier of
    /// the event that opened the window, or `0` when the ring holds nothing.
    fn earliest_resumable(&self) -> u64 {
        self.events.front().map_or(0, |event| event.id)
    }

    /// The identifier most recently assigned, or `0` when nothing has been
    /// published.
    fn latest_id(&self) -> u64 {
        self.next_id.saturating_sub(1)
    }

    /// The current head of each selected channel, in identifier order.
    ///
    /// A channel with nothing in the window contributes nothing, a channel
    /// named twice contributes once, and an empty selection selects nothing.
    fn heads(&self, channels: &[&str]) -> Vec<Event> {
        let mut heads: HashMap<&str, Event> = HashMap::new();
        for event in &self.events {
            if channels.contains(&event.channel.as_str()) {
                heads.insert(event.channel.as_str(), event.clone());
            }
        }
        let mut heads: Vec<Event> = heads.into_values().collect();
        heads.sort_unstable_by_key(|event| event.id);
        heads
    }

    /// Every held event after `id`, for the selected channels, in identifier
    /// order.
    fn since(&self, id: u64, channels: &[&str]) -> Vec<Event> {
        self.events
            .iter()
            .filter(|event| event.id > id && channels.contains(&event.channel.as_str()))
            .cloned()
            .collect()
    }
}

impl Ring {
    /// A ring holding at most `capacity` events.
    ///
    /// A capacity of zero is raised to one: a window that holds nothing cannot
    /// carry an identifier, so there would be nothing to serve.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Arc::new(RingInner {
                capacity,
                state: Mutex::new(State {
                    events: VecDeque::new(),
                    next_id: 1,
                }),
                notifications: broadcast::channel(capacity).0,
            }),
        }
    }

    /// Publish one event and report the identifier it was assigned.
    ///
    /// The identifier is daemon-wide: it does not depend on the channel. Past
    /// the ring's capacity the oldest event leaves the window.
    pub fn publish(
        &self,
        channel: impl Into<String>,
        kind: EventKind,
        payload: impl Into<String>,
    ) -> u64 {
        let mut event = Event {
            id: 0,
            kind,
            channel: channel.into(),
            payload: payload.into(),
        };
        let id = {
            let mut state = self.lock();
            let id = state.next_id;
            state.next_id = id.saturating_add(1);
            event.id = id;
            state.events.push_back(event.clone());
            while state.events.len() > self.inner.capacity {
                state.events.pop_front();
            }
            id
        };
        // A ring with no subscriber is not an error: the window is the record,
        // and a stream that is not open missed nothing.
        let _ = self.inner.notifications.send(event);
        id
    }

    /// The identifier most recently assigned, or `0` when nothing has been
    /// published.
    #[must_use]
    pub fn latest_id(&self) -> u64 {
        self.lock().latest_id()
    }

    /// The oldest identifier a cursor can still resume from: the identifier of
    /// the event that opened the window, or `0` when the ring holds nothing.
    #[must_use]
    pub fn earliest_resumable(&self) -> u64 {
        self.lock().earliest_resumable()
    }

    /// The current head of each selected channel, in identifier order.
    ///
    /// A channel with nothing in the window contributes nothing, a channel
    /// named twice contributes once, and an empty selection selects nothing.
    #[must_use]
    pub fn snapshot(&self, channels: &[&str]) -> Vec<Event> {
        self.lock().heads(channels)
    }

    /// Every held event after `id`, for the selected channels, in identifier
    /// order.
    #[must_use]
    pub fn since(&self, id: u64, channels: &[&str]) -> Vec<Event> {
        self.lock().since(id, channels)
    }

    /// A live subscription: every event published from now on, in identifier
    /// order.
    #[must_use]
    pub fn subscribe(&self) -> Subscription {
        Subscription {
            receiver: self.inner.notifications.subscribe(),
        }
    }

    /// The stream one subscriber is served.
    ///
    /// A stream opened without a cursor opens with the snapshot of `channels`:
    /// a selected channel holding an event contributes its head, and a
    /// selection holding nothing contributes one empty snapshot message
    /// carrying the daemon's latest identifier. A stream opened with a cursor
    /// is a resume: it carries what the ring still holds after that identifier,
    /// in identifier order and typed `EVENT`, and no snapshot.
    ///
    /// Either way the subscription is taken before the window is read, so an
    /// event published between the two is not lost; the floor is the highest
    /// identifier the opening messages carried, so no identifier is carried
    /// twice. Heartbeats fill an idle stretch.
    ///
    /// The item is a `Result` because that is the body a streaming handler
    /// answers with: nothing fails today, and the ring going away ends the
    /// stream cleanly, but a later drop policy can refuse in-stream without a
    /// signature change.
    ///
    /// # Errors
    ///
    /// Returns [`CursorError::Expired`] when a non-zero `cursor` names
    /// something the ring can no longer serve. The decision is made under the
    /// same lock that reads the window and builds the opening messages, and it
    /// is returned before the stream opens rather than as its first item.
    pub fn stream(
        &self,
        channels: &[&str],
        cursor: u64,
        heartbeat: Duration,
    ) -> Result<impl Stream<Item = Result<v1::Event, ConnectError>> + Send + 'static, CursorError>
    {
        // Taken before the window is read, so an event published between the
        // two is neither lost nor carried twice below.
        let subscription = self.subscribe();
        // One lock decides the cursor and builds the opening messages from the
        // window it decided against, so the decision and the stream cannot
        // disagree.
        let (pending, floor) = {
            let state = self.lock();
            let earliest = state.earliest_resumable();
            let latest = state.latest_id();
            if cursor != 0 && (cursor > latest || cursor.saturating_add(1) < earliest) {
                return Err(CursorError::Expired {
                    cursor,
                    earliest_resumable: earliest,
                    latest_id: latest,
                });
            }
            if cursor == 0 {
                let heads = state.heads(channels);
                let floor = heads.iter().map(|event| event.id).max().unwrap_or(0);
                let mut pending: VecDeque<(Event, EventKind)> = heads
                    .into_iter()
                    .map(|event| (event, EventKind::Snapshot))
                    .collect();
                if pending.is_empty() {
                    // Nothing to report, and the stream still opens with the
                    // snapshot: the empty one names no channel and carries no
                    // payload, and its identifier is the daemon's latest, so a
                    // stream opened before anything was published begins with
                    // the snapshot rather than with live traffic.
                    pending.push_back((
                        Event {
                            id: latest,
                            kind: EventKind::Snapshot,
                            channel: String::new(),
                            payload: String::new(),
                        },
                        EventKind::Snapshot,
                    ));
                }
                (pending, floor)
            } else {
                // The replay: what the window still holds after the cursor,
                // typed as the ordinary events it is. The floor is the cursor
                // at least, so a live copy of a replayed event is suppressed.
                let replay = state.since(cursor, channels);
                let floor = replay.iter().map(|event| event.id).max().unwrap_or(0);
                let pending: VecDeque<(Event, EventKind)> = replay
                    .into_iter()
                    .map(|event| (event, EventKind::Event))
                    .collect();
                (pending, floor.max(cursor))
            }
        };
        let state = StreamState {
            pending,
            selected: channels
                .iter()
                .map(|channel| (*channel).to_owned())
                .collect(),
            subscription,
            heartbeat: tokio::time::interval_at(tokio::time::Instant::now() + heartbeat, heartbeat),
            floor,
            last: floor,
            done: false,
            ring: Arc::downgrade(&self.inner),
        };
        Ok(stream::unfold(state, |mut state| async move {
            loop {
                if state.done {
                    // The stream already ended with its terminal refusal.
                    return None;
                }
                if let Some((event, kind)) = state.pending.pop_front() {
                    state.last = state.last.max(event.id);
                    // The kind describes the message, not the event: a message
                    // the opening messages carry is the snapshot when the
                    // stream has no cursor and an ordinary event when it has
                    // one, whatever the stored event was published as.
                    let mut message = event.to_message();
                    message.kind = kind.into();
                    return Some((Ok(message), state));
                }
                // The subscription is polled first, so a real event always wins
                // over a heartbeat that came due while the stream was busy.
                let next = tokio::select! {
                    biased;
                    received = state.subscription.recv() => match received {
                        Some(Live::Event(event)) => Next::Event(event),
                        Some(Live::Lagged { skipped }) => Next::Lagged { skipped },
                        None => Next::Ended,
                    },
                    _ = state.heartbeat.tick() => Next::Heartbeat,
                };
                match next {
                    // The ring is gone: the daemon is going away too, so the
                    // stream ends rather than hanging on a promise nobody keeps.
                    Next::Ended => return None,
                    // The caller fell behind the window. A gap is not a state a
                    // caller can work from, so the stream ends with the typed
                    // refusal that names what it lost; the caller's answer is a
                    // fresh stream.
                    Next::Lagged { skipped } => {
                        state.done = true;
                        return Some((
                            Err(stream_refusal(&StreamError::SlowConsumer { skipped })),
                            state,
                        ));
                    }
                    Next::Heartbeat => {
                        let id = state.latest_id();
                        state.last = state.last.max(id);
                        return Some((
                            Ok(v1::Event {
                                id,
                                kind: EventKind::Heartbeat.into(),
                                ..Default::default()
                            }),
                            state,
                        ));
                    }
                    Next::Event(event) => {
                        if event.id > state.floor
                            && state
                                .selected
                                .iter()
                                .any(|channel| channel == &event.channel)
                        {
                            state.last = state.last.max(event.id);
                            return Some((Ok(event.to_message()), state));
                        }
                    }
                }
            }
        }))
    }

    /// Take the state, tolerating a lock poisoned by a panicking publisher: the
    /// window is still readable, and a stream that ended cannot corrupt it.
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// What one poll of the serving stream learned.
enum Next {
    /// An event the ring published.
    Event(Event),
    /// The stream was idle for the heartbeat interval.
    Heartbeat,
    /// The subscriber fell behind the window, which dropped this many events
    /// before they could be carried.
    Lagged {
        /// How many events were dropped before they could be carried.
        skipped: u64,
    },
    /// The ring is gone, so nothing more will be published.
    Ended,
}

/// The serving stream's state: the messages not yet sent, the selection, the
/// live subscription, the heartbeat clock, and the identifiers already carried.
struct StreamState {
    /// The opening messages, in identifier order, until they have all been sent,
    /// each with the kind the message carries.
    pending: VecDeque<(Event, EventKind)>,
    /// The channels this stream carries, as strings, because the state outlives
    /// the request it was built from.
    selected: Vec<String>,
    subscription: Subscription,
    heartbeat: tokio::time::Interval,
    /// Events at or below this identifier have already been carried by the
    /// snapshot or the cursor, so they are suppressed.
    floor: u64,
    /// The highest identifier carried so far.
    last: u64,
    /// Whether the stream has already ended with its terminal refusal, so the
    /// poll after that one ends it rather than carrying anything else.
    done: bool,
    /// The ring, weakly: the serving stream must not keep the ring alive, or a
    /// dropped ring would never end the streams it opened.
    ring: Weak<RingInner>,
}

impl StreamState {
    /// The identifier a heartbeat carries: the daemon's latest, so identifiers
    /// never decrease, or the stream's own last when the ring has gone.
    fn latest_id(&self) -> u64 {
        self.ring
            .upgrade()
            .map_or(self.last, |inner| inner.latest_id())
    }
}

impl RingInner {
    fn latest_id(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .latest_id()
    }
}

/// What one live receive learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Live {
    /// The next published event.
    Event(Event),
    /// The subscriber fell behind the ring's window, which dropped `skipped`
    /// events before they could be carried. The receiver stays subscribed: it
    /// is the caller that must decide whether to resync.
    Lagged {
        /// How many events were dropped before they could be carried.
        skipped: u64,
    },
}

/// A live subscription to a ring: the events published after it was taken.
///
/// A subscriber that fell behind the ring's capacity is told what the window
/// lost rather than silently carried forward: [`Subscription::recv`] reports
/// the lag, and the serving stream turns the first one into its terminal
/// refusal, because a gap in a stream is not a state a caller can work from.
pub struct Subscription {
    receiver: broadcast::Receiver<Event>,
}

impl Subscription {
    /// The next live delivery, or `None` when the ring is gone.
    ///
    /// A lag is reported rather than hidden: the subscriber stays subscribed
    /// and the count of what the window dropped is in the value, so the caller
    /// decides whether to carry on across the gap or to resync.
    pub async fn recv(&mut self) -> Option<Live> {
        match self.receiver.recv().await {
            Ok(event) => Some(Live::Event(event)),
            Err(broadcast::error::RecvError::Lagged(skipped)) => Some(Live::Lagged { skipped }),
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

/// Gate a serving stream on one request's cancellation.
///
/// The stream carries what `inner` carries until the request's work is
/// cancelled, and then ends with the typed cancelled refusal. The guard is held
/// for as long as the stream runs, so the work is in flight exactly as long as
/// the stream that serves it, and it is dropped, together with the registry
/// entry it owns, the moment the stream ends for any reason.
///
/// Both the daemon's own service and the client's in-process service gate their
/// streams here, so a cancellation ends a stream the same way whichever one is
/// served.
pub fn cancellable<S>(
    inner: S,
    guard: Option<InFlight>,
) -> impl Stream<Item = Result<v1::Event, ConnectError>> + Send
where
    S: Stream<Item = Result<v1::Event, ConnectError>> + Send + 'static,
{
    // Boxed so the gate can poll the inner stream from the state it owns
    // without bounding it to `Unpin`; the ring's own stream is an opaque type
    // whose pinning a caller cannot name.
    let state = Serving {
        inner: Box::pin(inner),
        guard,
        done: false,
    };
    stream::unfold(state, |mut state| async move {
        if state.done {
            // The stream already ended with its terminal refusal.
            return None;
        }
        let step = match state.guard.as_ref() {
            // No identifier was declared, so nothing can cancel this stream.
            None => Step::Item(state.inner.next().await),
            Some(guard) => tokio::select! {
                // The inner stream is polled first, so a message the ring
                // already produced is carried before a cancellation that
                // arrived at the same moment ends the stream.
                biased;
                item = state.inner.next() => Step::Item(item),
                () = guard.cancelled() => Step::Cancelled,
            },
        };
        match step {
            Step::Item(Some(item)) => Some((item, state)),
            Step::Item(None) => None,
            Step::Cancelled => {
                state.done = true;
                Some((Err(stream_refusal(&StreamError::Cancelled)), state))
            }
        }
    })
}

/// The gate's state: the stream it carries, the guard that keeps the request in
/// flight, and whether the terminal refusal has already been carried.
struct Serving<S> {
    inner: std::pin::Pin<Box<S>>,
    guard: Option<InFlight>,
    done: bool,
}

/// What one poll of the gate learned.
enum Step {
    /// The inner stream produced an item, or ended.
    Item(Option<Result<v1::Event, ConnectError>>),
    /// The request was cancelled.
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::{
        CURSOR_ERROR_HEADER, CursorError, CursorRefusal, Event, Live, Ring, STREAM_ERROR_HEADER,
        StreamError, StreamRefusal, Subscription, cancellable, cursor_refusal_headers,
        stream_refusal_headers,
    };
    use crate::cancel::{CancelOutcome, Registry};
    use crate::proto::metis::v1::EventKind;
    use crate::request::RequestId;
    use connectrpc::{ConnectError, ErrorCode};
    use futures_util::StreamExt as _;
    use http::HeaderValue;
    use std::time::Duration;

    /// The next event a subscription delivers. The tests that expect a lag read
    /// the variant themselves.
    async fn next_event(live: &mut Subscription) -> Event {
        match live.recv().await.expect("the subscription is live") {
            Live::Event(event) => event,
            Live::Lagged { skipped } => panic!("an unexpected lag of {skipped}"),
        }
    }

    /// Every event published carries the next daemon-wide identifier, whatever
    /// channel it belongs to, and the first one is `1`.
    #[test]
    fn identifiers_are_daemon_wide_and_start_at_one() {
        let ring = Ring::new(8);

        assert_eq!(ring.publish("daemon", EventKind::Event, "{\"a\":1}"), 1);
        assert_eq!(ring.publish("session:7", EventKind::Event, "{}"), 2);
        assert_eq!(ring.publish("daemon", EventKind::Heartbeat, "{}"), 3);

        let daemon = ring.since(0, &["daemon"]);
        assert_eq!(
            daemon.iter().map(|event| event.id).collect::<Vec<_>>(),
            vec![1, 3],
            "one counter serves every channel"
        );
        assert_eq!(daemon[0].channel, "daemon");
        assert_eq!(daemon[0].kind, EventKind::Event);
        assert_eq!(daemon[0].payload, "{\"a\":1}");
        assert_eq!(daemon[1].kind, EventKind::Heartbeat);
    }

    /// The ring is bounded: past capacity the oldest event is dropped, and the
    /// earliest resumable identifier follows the oldest event still held.
    #[test]
    fn the_ring_drops_the_oldest_event_past_its_capacity() {
        let ring = Ring::new(2);

        assert_eq!(ring.publish("daemon", EventKind::Event, "1"), 1);
        assert_eq!(ring.earliest_resumable(), 1);
        assert_eq!(ring.publish("daemon", EventKind::Event, "2"), 2);
        assert_eq!(ring.earliest_resumable(), 1, "nothing is dropped yet");

        assert_eq!(ring.publish("daemon", EventKind::Event, "3"), 3);
        assert_eq!(
            ring.earliest_resumable(),
            2,
            "the ring holds the newest two"
        );
        assert_eq!(
            ring.since(0, &["daemon"])
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );

        assert_eq!(ring.publish("daemon", EventKind::Event, "4"), 4);
        assert_eq!(ring.publish("daemon", EventKind::Event, "5"), 5);
        assert_eq!(ring.latest_id(), 5);
        assert_eq!(ring.earliest_resumable(), 4);
    }

    /// An empty ring has nothing to resume from and nothing to report.
    #[test]
    fn an_empty_ring_has_no_identifiers_yet() {
        let ring = Ring::new(2);

        assert_eq!(ring.latest_id(), 0);
        assert_eq!(ring.earliest_resumable(), 0);
        assert!(ring.since(0, &["daemon"]).is_empty());
        assert!(ring.snapshot(&["daemon"]).is_empty());
    }

    /// The snapshot is the head of each selected channel, in identifier order,
    /// and a channel the selection does not name contributes nothing. An empty
    /// selection selects nothing rather than everything.
    #[test]
    fn the_snapshot_is_the_head_of_each_selected_channel() {
        let ring = Ring::new(8);
        ring.publish("daemon", EventKind::Event, "first");
        ring.publish("session:7", EventKind::Event, "session");
        ring.publish("daemon", EventKind::Event, "head");

        let daemon = ring.snapshot(&["daemon"]);
        assert_eq!(daemon.len(), 1);
        assert_eq!(daemon[0].payload, "head");
        assert_eq!(daemon[0].id, 3);

        let both = ring.snapshot(&["daemon", "session:7"]);
        assert_eq!(
            both.iter().map(|event| event.id).collect::<Vec<_>>(),
            vec![2, 3],
            "identifier order, not selection order"
        );

        assert!(ring.snapshot(&["task:9"]).is_empty());
        assert!(ring.snapshot(&[]).is_empty());
    }

    /// `since` is what follows a cursor, for the selected channels only.
    #[test]
    fn since_returns_what_follows_the_cursor() {
        let ring = Ring::new(8);
        ring.publish("daemon", EventKind::Event, "1");
        ring.publish("session:7", EventKind::Event, "2");
        ring.publish("daemon", EventKind::Event, "3");
        ring.publish("daemon", EventKind::Event, "4");

        assert_eq!(
            ring.since(0, &["daemon"])
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            vec![1, 3, 4]
        );
        assert_eq!(
            ring.since(3, &["daemon"])
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            vec![4]
        );
        assert!(ring.since(4, &["daemon"]).is_empty());
        assert_eq!(
            ring.since(1, &["session:7"])
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }

    /// A subscriber receives what is published after it subscribed, in
    /// identifier order, and it receives the event itself rather than a copy of
    /// the ring's whole window.
    #[tokio::test]
    async fn a_subscription_delivers_published_events_in_identifier_order() {
        let ring = Ring::new(4);
        let mut live = ring.subscribe();

        assert_eq!(ring.publish("daemon", EventKind::Event, "1"), 1);
        assert_eq!(ring.publish("session:7", EventKind::Event, "2"), 2);

        let first = next_event(&mut live).await;
        assert_eq!(first.id, 1);
        assert_eq!(first.channel, "daemon");
        let second = next_event(&mut live).await;
        assert_eq!(second.id, 2);
    }

    /// A subscriber that fell behind is told what the window dropped instead of
    /// being carried forward across a gap nobody named.
    #[tokio::test]
    async fn a_subscription_reports_what_a_lag_skipped() {
        let ring = Ring::new(2);
        let mut live = ring.subscribe();

        // The channel's buffer is the ring's window, so three sends with
        // nothing read overwrite the oldest one.
        ring.publish("daemon", EventKind::Event, "1");
        ring.publish("daemon", EventKind::Event, "2");
        ring.publish("daemon", EventKind::Event, "3");

        match live.recv().await.expect("the subscription is live") {
            Live::Lagged { skipped } => {
                assert_eq!(skipped, 1, "the event the window overwrote")
            }
            other => panic!("expected a reported lag, got {other:?}"),
        }
    }

    /// The stream refusal crosses the wire as one trailer value, and a value
    /// this build does not know parses to nothing rather than to a guess.
    #[test]
    fn the_stream_refusal_round_trips_as_a_trailer() {
        let headers = stream_refusal_headers(&StreamError::SlowConsumer { skipped: 3 });
        assert_eq!(
            headers.get(STREAM_ERROR_HEADER),
            Some(&HeaderValue::from_static("slow-consumer")),
            "the meaning crosses the wire as the trailer, not as message text"
        );
        let headers = stream_refusal_headers(&StreamError::Cancelled);
        assert_eq!(
            headers.get(STREAM_ERROR_HEADER),
            Some(&HeaderValue::from_static("cancelled"))
        );

        assert_eq!(StreamRefusal::SlowConsumer.as_str(), "slow-consumer");
        assert_eq!(StreamRefusal::Cancelled.as_str(), "cancelled");
        assert_eq!(StreamRefusal::Cancelled.to_string(), "cancelled");
        assert_eq!(
            StreamRefusal::parse("slow-consumer"),
            Some(StreamRefusal::SlowConsumer)
        );
        assert_eq!(
            StreamRefusal::parse(" cancelled "),
            Some(StreamRefusal::Cancelled)
        );
        assert_eq!(
            StreamRefusal::parse("something-else"),
            None,
            "a refusal this build does not know is left as the transport error"
        );
        assert!(
            StreamError::SlowConsumer { skipped: 3 }
                .to_string()
                .contains("3 events were dropped"),
            "the message names what was lost"
        );
    }

    /// A serving stream that finds its subscriber past the window ends with the
    /// typed slow-consumer refusal rather than carrying a gap: the count is in
    /// the message, the meaning is in the trailer, and nothing follows it.
    #[tokio::test]
    async fn a_stream_ends_with_the_slow_consumer_refusal_when_it_fell_behind()
    -> Result<(), CursorError> {
        let ring = Ring::new(2);
        let mut stream = Box::pin(ring.stream(&["daemon"], 0, Duration::from_secs(60))?);

        // The stream opens with the empty snapshot, and nothing is read while
        // the ring publishes, so the subscriber falls behind the window.
        let opening = stream.next().await.expect("a stream").expect("a message");
        assert_eq!(opening.kind, EventKind::Snapshot);
        ring.publish("daemon", EventKind::Event, "1");
        ring.publish("daemon", EventKind::Event, "2");
        ring.publish("daemon", EventKind::Event, "3");

        let refusal = stream
            .next()
            .await
            .expect("the stream carries the refusal")
            .expect_err("a subscriber past the window is refused");
        assert_eq!(
            refusal.code,
            ErrorCode::ResourceExhausted,
            "the stream ran out of room for this caller"
        );
        assert_eq!(
            refusal.trailers().get(STREAM_ERROR_HEADER),
            Some(&HeaderValue::from_static("slow-consumer")),
            "a mid-stream refusal travels in the trailing metadata"
        );
        let message = refusal.message.clone().unwrap_or_default();
        assert!(
            message.contains("1 events were dropped"),
            "the message names the loss rather than skipping it in silence: {message}"
        );
        assert!(
            stream.next().await.is_none(),
            "nothing follows the refusal: the gap is never presented as state"
        );
        Ok(())
    }

    /// A reader that keeps up never sees the refusal, under the same load that
    /// drops a stalled one: the bound is the window, not the traffic.
    #[tokio::test]
    async fn a_stream_that_keeps_up_is_never_refused() -> Result<(), CursorError> {
        let ring = Ring::new(2);
        let mut stream = Box::pin(ring.stream(&["daemon"], 0, Duration::from_secs(60))?);
        let opening = stream.next().await.expect("a stream").expect("a message");
        assert_eq!(opening.kind, EventKind::Snapshot);

        // Published and read one at a time, past the capacity of the window.
        for expected in 1..=8 {
            let id = ring.publish("daemon", EventKind::Event, "{}");
            assert_eq!(id, expected);
            let event = stream.next().await.expect("a stream").expect("a message");
            assert_eq!(event.kind, EventKind::Event);
            assert_eq!(event.id, id);
        }
        Ok(())
    }

    /// A registered request's stream ends with the typed cancelled refusal as
    /// soon as the request is cancelled, carries nothing after it, and keeps
    /// the work in flight for as long as it runs.
    #[tokio::test]
    async fn a_cancelled_request_ends_its_stream() -> Result<(), CursorError> {
        let ring = Ring::new(4);
        let registry = Registry::new();
        let id = RequestId::parse("cli-7").expect("a printable identifier");
        let guard = registry.register("cli", id.clone());

        let inner = ring.stream(&["daemon"], 0, Duration::from_secs(60))?;
        let mut stream = Box::pin(cancellable(inner, Some(guard)));
        assert_eq!(
            registry.in_flight(),
            1,
            "the guard the stream carries keeps the work in flight"
        );

        let opening = stream.next().await.expect("a stream").expect("a message");
        assert_eq!(opening.kind, EventKind::Snapshot);

        assert_eq!(registry.cancel("cli", &id), CancelOutcome::Cancelled);
        let refusal = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("the cancellation ends the stream")
            .expect("the stream carries the refusal")
            .expect_err("a cancelled stream is refused");
        assert_eq!(refusal.code, ErrorCode::Canceled);
        assert_eq!(
            refusal.trailers().get(STREAM_ERROR_HEADER),
            Some(&HeaderValue::from_static("cancelled"))
        );
        assert!(
            stream.next().await.is_none(),
            "a stopped stream carries nothing more"
        );
        Ok(())
    }

    /// A stream whose request declared no identifier is not cancellable, and it
    /// is served exactly as the ring serves it.
    #[tokio::test]
    async fn a_stream_with_no_registered_request_is_served_unchanged() -> Result<(), CursorError> {
        let ring = Ring::new(4);
        let inner = ring.stream(&["daemon"], 0, Duration::from_secs(60))?;
        let mut stream = Box::pin(cancellable(inner, None));

        let opening = stream.next().await.expect("a stream").expect("a message");
        assert_eq!(opening.kind, EventKind::Snapshot);

        let published = ring.publish("daemon", EventKind::Event, "{}");
        let event = stream.next().await.expect("a stream").expect("a message");
        assert_eq!(event.id, published);
        Ok(())
    }

    /// Dropping the stream drops the guard, so the work leaves the registry
    /// when the stream ends for any reason: a disconnect cancels nothing and
    /// leaves nothing behind.
    #[tokio::test]
    async fn dropping_the_stream_leaves_the_registry() -> Result<(), CursorError> {
        let ring = Ring::new(4);
        let registry = Registry::new();
        let id = RequestId::parse("cli-7").expect("a printable identifier");
        let guard = registry.register("cli", id.clone());
        let inner = ring.stream(&["daemon"], 0, Duration::from_secs(60))?;
        let stream = cancellable(inner, Some(guard));

        drop(stream);
        assert_eq!(
            registry.in_flight(),
            0,
            "the work is not in flight any more"
        );
        assert_eq!(
            registry.cancellations(),
            0,
            "a client going away is not a cancellation"
        );
        assert_eq!(
            registry.cancel("cli", &id),
            CancelOutcome::AlreadyFinished,
            "work that ended is answered as already finished"
        );
        Ok(())
    }

    /// The cursor refusal crosses the wire as one header value, and a value
    /// this build does not know parses to nothing rather than to a guess.
    #[test]
    fn the_cursor_refusal_round_trips_as_a_header() {
        let error = CursorError::Expired {
            cursor: 1,
            earliest_resumable: 3,
            latest_id: 4,
        };
        let headers = cursor_refusal_headers(&error);
        assert_eq!(
            headers.get(CURSOR_ERROR_HEADER),
            Some(&HeaderValue::from_static("expired")),
            "the meaning crosses the wire as the header, not as message text"
        );

        assert_eq!(CursorRefusal::Expired.as_str(), "expired");
        assert_eq!(CursorRefusal::Expired.to_string(), "expired");
        assert_eq!(
            CursorRefusal::parse("expired"),
            Some(CursorRefusal::Expired)
        );
        assert_eq!(
            CursorRefusal::parse(" expired "),
            Some(CursorRefusal::Expired)
        );
        assert_eq!(
            CursorRefusal::parse("something-else"),
            None,
            "a refusal this build does not know is left as the transport error"
        );
    }

    /// A cursor the window has moved past, and a cursor beyond the newest
    /// identifier, are both expired and both carry the window's bounds in the
    /// value; a cursor of zero is never expired, because it means "no cursor".
    #[tokio::test]
    async fn a_cursor_outside_the_window_is_expired_and_names_both_bounds() {
        let ring = Ring::new(2);
        ring.publish("daemon", EventKind::Event, "1");
        ring.publish("daemon", EventKind::Event, "2");
        ring.publish("daemon", EventKind::Event, "3");
        ring.publish("daemon", EventKind::Event, "4");
        assert_eq!(ring.earliest_resumable(), 3, "the window holds 3 and 4");
        assert_eq!(ring.latest_id(), 4);

        // Older than the window: the event that follows this cursor is gone.
        let past = match ring.stream(&["daemon"], 1, Duration::from_secs(60)) {
            Ok(_) => panic!("the window moved past cursor 1"),
            Err(error) => error,
        };
        assert_eq!(
            past,
            CursorError::Expired {
                cursor: 1,
                earliest_resumable: 3,
                latest_id: 4,
            }
        );
        assert!(
            past.to_string().contains("window moved past it"),
            "the message names the moved window: {past}"
        );

        // Beyond the newest identifier: the identifier space began again, as it
        // does when the daemon restarts.
        let beyond = match ring.stream(&["daemon"], 9, Duration::from_secs(60)) {
            Ok(_) => panic!("cursor 9 is beyond the newest identifier"),
            Err(error) => error,
        };
        assert_eq!(
            beyond,
            CursorError::Expired {
                cursor: 9,
                earliest_resumable: 3,
                latest_id: 4,
            }
        );
        assert!(
            beyond.to_string().contains("restarted"),
            "the message names the restarted identifier space: {beyond}"
        );

        // No cursor is never expired, however far the window has moved.
        assert!(
            ring.stream(&["daemon"], 0, Duration::from_secs(60)).is_ok(),
            "zero means no cursor"
        );
    }

    /// The edges of the window are servable: a cursor one below the oldest held
    /// identifier resumes with the event that opened the window, and a cursor at
    /// the newest identifier resumes with nothing and carries what is published
    /// next.
    #[tokio::test]
    async fn the_edges_of_the_window_are_servable() {
        let ring = Ring::new(2);
        ring.publish("daemon", EventKind::Event, "1");
        ring.publish("daemon", EventKind::Event, "2");
        ring.publish("daemon", EventKind::Event, "3");

        // `cursor == earliest_resumable() - 1`: the event that opened the window
        // is exactly what follows it, so there is something to replay.
        let mut resumed = Box::pin(
            ring.stream(&["daemon"], 1, Duration::from_secs(60))
                .expect("a cursor one below the window is servable"),
        );
        let first = resumed.next().await.expect("a stream").expect("a message");
        assert_eq!(first.id, 2);
        let second = resumed.next().await.expect("a stream").expect("a message");
        assert_eq!(second.id, 3);

        // `cursor == latest_id()`: nothing follows it yet, so the stream replays
        // nothing and carries the next live event.
        let mut current = Box::pin(
            ring.stream(&["daemon"], 3, Duration::from_secs(60))
                .expect("the newest identifier is servable"),
        );
        let published = ring.publish("daemon", EventKind::Event, "4");
        assert_eq!(published, 4);
        let next = current.next().await.expect("a stream").expect("a message");
        assert_eq!(next.kind, EventKind::Event);
        assert_eq!(next.id, 4);
    }

    /// A stream opened with a cursor replays exactly the held events that
    /// follow it, in identifier order and typed `EVENT`. It carries no
    /// snapshot: the caller that resumes already holds the state a snapshot
    /// would restate, and an identifier the snapshot carried would arrive a
    /// second time from the replay.
    #[tokio::test]
    async fn a_servable_cursor_replays_only_what_follows_it() -> Result<(), CursorError> {
        let ring = Ring::new(8);
        ring.publish("daemon", EventKind::Event, "1");
        ring.publish("session:7", EventKind::Event, "2");
        ring.publish("daemon", EventKind::Event, "3");

        let mut stream = Box::pin(ring.stream(&["daemon"], 1, Duration::from_secs(60))?);

        let first = stream.next().await.expect("a stream").expect("a message");
        assert_eq!(
            first.kind,
            EventKind::Event,
            "a resumed stream carries no snapshot"
        );
        assert_eq!(first.id, 3, "only what follows the cursor");
        assert_eq!(first.payload, "3");

        // The live subscription follows the replay, so the stream stays open
        // and carries what is published next, once.
        ring.publish("daemon", EventKind::Event, "4");
        let next = stream.next().await.expect("a stream").expect("a message");
        assert_eq!(next.kind, EventKind::Event);
        assert_eq!(next.id, 4);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), stream.next())
                .await
                .is_err(),
            "the replayed and the live copy are one message, carried once"
        );
        Ok(())
    }

    /// The serving stream opens with the snapshot and then carries the events
    /// that follow it, with a heartbeat on an idle stream. The heartbeat
    /// carries the latest identifier, so identifiers never decrease.
    #[tokio::test]
    async fn the_serving_stream_opens_with_the_snapshot_and_then_follows_the_ring()
    -> Result<(), ConnectError> {
        let ring = Ring::new(8);
        ring.publish("daemon", EventKind::Event, "before");

        let mut stream = Box::pin(
            ring.stream(&["daemon"], 0, Duration::from_millis(10))
                .expect("no cursor is never expired"),
        );

        let snapshot = stream.next().await.expect("a stream")?;
        assert_eq!(snapshot.kind, EventKind::Snapshot);
        assert_eq!(snapshot.id, 1);
        assert_eq!(snapshot.payload, "before");

        ring.publish("daemon", EventKind::Event, "after");
        let event = stream.next().await.expect("a stream")?;
        assert_eq!(event.kind, EventKind::Event);
        assert_eq!(event.id, 2);
        assert_eq!(event.payload, "after");

        let heartbeat = stream.next().await.expect("a stream")?;
        assert_eq!(heartbeat.kind, EventKind::Heartbeat);
        assert_eq!(heartbeat.id, 2, "a heartbeat never decreases the id");
        assert!(heartbeat.channel.is_empty(), "a heartbeat has no channel");
        Ok(())
    }

    /// A stream over a selection that holds nothing opens with the empty
    /// snapshot, and a closed ring ends the stream rather than hanging it.
    #[tokio::test]
    async fn the_serving_stream_opens_with_the_empty_snapshot_and_ends_with_the_ring()
    -> Result<(), ConnectError> {
        let ring = Ring::new(4);
        let mut stream = Box::pin(
            ring.stream(&["daemon"], 0, Duration::from_secs(60))
                .expect("no cursor is never expired"),
        );

        let snapshot = stream.next().await.expect("the stream is live")?;
        assert_eq!(snapshot.kind, EventKind::Snapshot);
        assert_eq!(snapshot.id, 0, "nothing has been published yet");
        assert!(snapshot.channel.is_empty());
        assert!(snapshot.payload.is_empty());

        drop(ring);
        assert!(stream.next().await.is_none());
        Ok(())
    }

    /// The message mirror keeps the wire message's fields, and the unknown
    /// spelling of an enum value this build does not know reads as unspecified
    /// rather than as a guess.
    #[test]
    fn an_event_mirrors_the_wire_message() {
        let event = Event {
            id: 7,
            kind: EventKind::Snapshot,
            channel: "daemon".to_owned(),
            payload: "{\"a\":1}".to_owned(),
        };

        let message = event.to_message();
        assert_eq!(message.id, 7);
        assert_eq!(message.channel, "daemon");
        assert_eq!(message.payload, "{\"a\":1}");
        assert_eq!(Event::from_message(&message), event);

        let mut unknown = message;
        unknown.kind = buffa::EnumValue::Unknown(99);
        assert_eq!(
            Event::from_message(&unknown).kind,
            EventKind::Unspecified,
            "a kind this build does not know is not guessed at"
        );
    }
}
