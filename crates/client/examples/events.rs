//! Serve the skeleton on the resolved local endpoint and drain one event from
//! its stream.
//!
//! This is the runnable half of the event stream:
//!
//! ```text
//! cargo run -p client --example events
//! ```
//!
//! It starts the skeleton on the resolved endpoint (the `METIS_ENDPOINT`
//! override, else the private base) over a ring holding one event, opens the
//! stream through the wrapper, prints the snapshot the stream opens with, then
//! publishes a second event into the ring and prints the event that arrives.
//! Both steps are the cursorless stream. The third step is the resume: it opens
//! a second stream carrying that event's identifier as its cursor, publishes a
//! third event, and prints the event the resumed stream carries. The fourth step
//! is the cancel: it stops that second stream by the identifier the handle
//! reports and prints the outcome. Anything that fails exits non-zero.

use std::time::Duration;

use api_contracts::cancel::CancelOutcome;
use api_contracts::events::Ring;
use api_contracts::proto::metis::v1::EventKind;

/// The channel the walkthrough streams.
const CHANNEL: &str = "daemon";

/// The event the stream opens with: it is in the ring before the stream exists,
/// so the snapshot is the only thing that can carry it.
const SNAPSHOT_PAYLOAD: &str = "{\"seeded\":true}";

/// The event published while the stream is open.
const EVENT_PAYLOAD: &str = "{\"next\":true}";

/// The event published for the resume step: the second stream carries the
/// identifier of the event just seen as its cursor, so it opens with this event
/// and no snapshot.
const RESUME_PAYLOAD: &str = "{\"resumed\":true}";

/// How many events the walkthrough's ring holds.
const CAPACITY: usize = 64;

#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = client::resolve_endpoint().map_err(client::ClientError::from)?;
    let ring = Ring::new(CAPACITY);
    let snapshot_id = ring.publish(CHANNEL, EventKind::Event, SNAPSHOT_PAYLOAD);

    let server_endpoint = endpoint.clone();
    let served_ring = ring.clone();
    tokio::spawn(async move {
        // The default interval: this walkthrough publishes its event as soon as
        // the snapshot arrives, so no heartbeat is waited on.
        if let Err(err) = client::skeleton::serve_uds_with(
            &server_endpoint,
            served_ring,
            client::skeleton::HEARTBEAT_DEFAULT,
        )
        .await
        {
            eprintln!("skeleton server error: {err}");
        }
    });
    if !client::skeleton::wait_for_socket(&endpoint).await {
        eprintln!(
            "[result] FAIL: the skeleton did not bind {}",
            endpoint.display()
        );
        std::process::exit(1);
    }

    let caller = client::Client::local().await?;
    let mut stream = caller.stream_events(&[CHANNEL], 0).await?;

    let Some(snapshot) = stream.message().await? else {
        eprintln!("[result] FAIL: the stream ended before its snapshot");
        std::process::exit(1);
    };
    println!(
        "[snapshot] endpoint={} id={} kind={:?} payload={:?}",
        endpoint.display(),
        snapshot.id,
        snapshot.kind,
        snapshot.payload
    );
    if snapshot.kind != EventKind::Snapshot || snapshot.id != snapshot_id {
        eprintln!("[result] FAIL: unexpected snapshot {snapshot:?}");
        std::process::exit(1);
    }

    // The snapshot has arrived, so the stream is open and the subscription
    // behind it is live: an event published now is carried by the stream.
    let event_id = ring.publish(CHANNEL, EventKind::Event, EVENT_PAYLOAD);
    let Some(event) = tokio::time::timeout(Duration::from_secs(15), stream.message())
        .await
        .expect("the stream carries the published event")?
    else {
        eprintln!("[result] FAIL: the stream ended before the event");
        std::process::exit(1);
    };
    println!(
        "[event] id={} kind={:?} channel={} payload={:?}",
        event.id, event.kind, event.channel, event.payload
    );
    if event.kind != EventKind::Event || event.id != event_id || event.payload != EVENT_PAYLOAD {
        eprintln!("[result] FAIL: unexpected event {event:?}");
        std::process::exit(1);
    }

    // The resume: a second stream opened with the identifier of the event just
    // seen as its cursor. It replays nothing (the cursor is the newest
    // identifier), carries no snapshot, and begins with what is published next.
    let mut resumed = caller.stream_events(&[CHANNEL], event.id).await?;
    let resumed_id = ring.publish(CHANNEL, EventKind::Event, RESUME_PAYLOAD);
    let Some(resumed_event) = tokio::time::timeout(Duration::from_secs(15), resumed.message())
        .await
        .expect("the resumed stream carries the published event")?
    else {
        eprintln!("[result] FAIL: the resumed stream ended before its event");
        std::process::exit(1);
    };
    println!(
        "[resume] cursor={} id={} kind={:?} payload={:?}",
        event.id, resumed_event.id, resumed_event.kind, resumed_event.payload
    );
    if resumed_event.kind != EventKind::Event
        || resumed_event.id != resumed_id
        || resumed_event.payload != RESUME_PAYLOAD
    {
        eprintln!("[result] FAIL: unexpected resumed event {resumed_event:?}");
        std::process::exit(1);
    }

    // The cancel: the resumed stream is still open, and the identifier the
    // handle reports is what the daemon registered its work under.
    let cancelled = caller.cancel(resumed.request_id()).await?;
    println!(
        "[cancel] request_id={} outcome={cancelled:?}",
        resumed.request_id()
    );
    if cancelled != CancelOutcome::Cancelled {
        eprintln!("[result] FAIL: unexpected cancel outcome {cancelled:?}");
        std::process::exit(1);
    }
    // The stream stops: the next read reports the typed cancellation rather
    // than an event, so the gap a cancellation leaves is never presented as
    // current state.
    if !matches!(
        resumed.message().await,
        Err(client::ClientError::StreamCancelled)
    ) {
        eprintln!("[result] FAIL: the cancelled stream did not stop");
        std::process::exit(1);
    }

    println!("[result] PASS");
    Ok(())
}

/// The local transport is Unix-only in this build; the Windows front-end
/// arrives with its own ticket.
#[cfg(not(unix))]
fn main() {
    eprintln!("[result] FAIL: the local transport is Unix-only in this build");
    std::process::exit(1);
}
