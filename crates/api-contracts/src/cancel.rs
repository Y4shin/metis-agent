//! The in-flight work a request identifier names.
//!
//! A caller that wants to be able to stop a call declares a
//! [`RequestId`](crate::request::RequestId) as request metadata. The work that
//! call starts registers itself here for as long as it runs, and the `Cancel`
//! RPC stops it by name. The registry is in the shared crate because the daemon
//! and the client's in-process test service both need one, and it is the only
//! crate both reach.
//!
//! Work is keyed by the declaring surface and the identifier. A cancel stops
//! only work its own surface started: a cancel naming another surface's request
//! finds nothing in flight and is answered as already finished, which is both
//! the idempotent answer and no way to learn whether an identifier exists.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use tokio::sync::Notify;

use crate::proto::metis::v1::CancelOutcome as WireOutcome;
use crate::request::RequestId;

/// What a cancel call did.
///
/// The distinction is the one a caller acts on: work that was in flight is
/// being stopped, and everything else, a request that finished, one that was
/// already cancelled, and one that was never seen, reads the same so the answer
/// is idempotent and tells a caller nothing about another caller's identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// An in-flight request under that identifier was found and is being
    /// stopped.
    Cancelled,
    /// Nothing was in flight under that identifier.
    AlreadyFinished,
}

impl CancelOutcome {
    /// The wire value this outcome crosses as.
    #[must_use]
    pub fn to_proto(self) -> WireOutcome {
        match self {
            Self::Cancelled => WireOutcome::Cancelled,
            Self::AlreadyFinished => WireOutcome::AlreadyFinished,
        }
    }

    /// The outcome a wire value carries, or `None` when it names nothing this
    /// build can act on.
    ///
    /// `None` covers the contract's `UNSPECIFIED` and any value a later version
    /// adds: an answer that names no outcome is not guessed at, so a caller
    /// keeps the transport error rather than reading it as work that finished.
    #[must_use]
    pub fn from_proto(outcome: WireOutcome) -> Option<Self> {
        match outcome {
            WireOutcome::Cancelled => Some(Self::Cancelled),
            WireOutcome::AlreadyFinished => Some(Self::AlreadyFinished),
            WireOutcome::Unspecified => None,
        }
    }
}

/// The daemon's in-flight work.
///
/// The handle is clonable, so the service keeps one and a test or the daemon
/// effort reads the same registry.
#[derive(Clone)]
pub struct Registry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    state: Mutex<State>,
}

struct State {
    /// The work in flight, keyed by the declaring surface and the identifier.
    in_flight: HashMap<Key, Arc<Entry>>,
    /// How many cancels found work to stop.
    cancellations: u64,
}

/// The key one request's work is registered under.
type Key = (String, RequestId);

/// One request's entry: whether it was cancelled and the signal that says so.
struct Entry {
    cancelled: AtomicBool,
    notify: Notify,
}

/// One request's in-flight work.
///
/// The guard is the entry's lifetime: register at the start of the work and
/// hold it for as long as the work runs, and the entry leaves the registry when
/// the guard drops, however the work ended.
pub struct InFlight {
    key: Key,
    entry: Arc<Entry>,
    registry: Weak<RegistryInner>,
}

impl InFlight {
    /// The identifier this work was registered under.
    #[must_use]
    pub fn request_id(&self) -> &RequestId {
        &self.key.1
    }

    /// Whether this work has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.entry.cancelled.load(Ordering::SeqCst)
    }

    /// The signal the work selects on: it resolves when this request is
    /// cancelled, and never otherwise.
    ///
    /// The waiter is registered before the flag is read, so a cancellation that
    /// arrives at any point, before this call, during it, or after it, is seen
    /// exactly once.
    pub async fn cancelled(&self) {
        let notified = self.entry.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let Some(registry) = self.registry.upgrade() else {
            return;
        };
        let mut state = lock(&registry);
        // Only this guard's own entry is removed: work registered under the same
        // surface and identifier after it must keep its own entry.
        let mine = state
            .in_flight
            .get(&self.key)
            .is_some_and(|entry| Arc::ptr_eq(entry, &self.entry));
        if mine {
            state.in_flight.remove(&self.key);
        }
    }
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                state: Mutex::new(State {
                    in_flight: HashMap::new(),
                    cancellations: 0,
                }),
            }),
        }
    }

    /// Register one request's work, and report the guard that keeps it in
    /// flight.
    ///
    /// The entry lives exactly as long as the guard, so work that finishes, is
    /// cancelled, or is dropped leaves nothing behind. A second registration
    /// under the same surface and identifier replaces the first entry: the
    /// identifiers this daemon hands out are generated, so a collision is a
    /// caller reusing a name deliberately, and the newest registration is the
    /// one a cancel should reach.
    #[must_use]
    pub fn register(&self, surface: &str, id: RequestId) -> InFlight {
        let key = (surface.to_owned(), id);
        let entry = Arc::new(Entry {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
        });
        lock(&self.inner)
            .in_flight
            .insert(key.clone(), Arc::clone(&entry));
        InFlight {
            key,
            entry,
            registry: Arc::downgrade(&self.inner),
        }
    }

    /// Stop the work `surface` started under `id`.
    ///
    /// The entry is removed before its signal fires, so the answer and what the
    /// work sees cannot disagree: the first call answers [`CancelOutcome::Cancelled`]
    /// and every later call, and every call that names another surface's work,
    /// answers [`CancelOutcome::AlreadyFinished`].
    #[must_use]
    pub fn cancel(&self, surface: &str, id: &RequestId) -> CancelOutcome {
        let mut state = lock(&self.inner);
        let key = (surface.to_owned(), id.clone());
        let Some(entry) = state.in_flight.remove(&key) else {
            return CancelOutcome::AlreadyFinished;
        };
        entry.cancelled.store(true, Ordering::SeqCst);
        entry.notify.notify_waiters();
        state.cancellations += 1;
        CancelOutcome::Cancelled
    }

    /// How many requests are in flight.
    ///
    /// The count is what a test asserts a disconnect did not change: a dropped
    /// stream removes its own entry and no other.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        lock(&self.inner).in_flight.len()
    }

    /// How many cancels found work to stop.
    #[must_use]
    pub fn cancellations(&self) -> u64 {
        lock(&self.inner).cancellations
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// Take the registry's state, tolerating a lock poisoned by a panicking
/// request: the entries are still readable, and a guard that drops afterwards
/// cannot corrupt them.
fn lock(registry: &RegistryInner) -> std::sync::MutexGuard<'_, State> {
    registry
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{CancelOutcome, Registry};
    use crate::proto::metis::v1::CancelOutcome as WireOutcome;
    use crate::request::RequestId;
    use std::time::Duration;

    fn id(text: &str) -> RequestId {
        RequestId::parse(text).expect("a printable identifier")
    }

    /// Work is in flight exactly as long as its guard lives, and the identifier
    /// it was registered under is the one it reports.
    #[test]
    fn work_is_in_flight_until_its_guard_drops() {
        let registry = Registry::new();
        assert_eq!(registry.in_flight(), 0);

        let guard = registry.register("cli", id("cli-1"));
        assert_eq!(registry.in_flight(), 1);
        assert_eq!(guard.request_id().as_str(), "cli-1");
        assert!(!guard.is_cancelled(), "nothing has cancelled this work");

        drop(guard);
        assert_eq!(
            registry.in_flight(),
            0,
            "the entry lives exactly as long as its guard"
        );
        assert_eq!(
            registry.cancel("cli", &id("cli-1")),
            CancelOutcome::AlreadyFinished,
            "work that finished is not work that was cancelled"
        );
        assert_eq!(
            registry.cancellations(),
            0,
            "a request leaving the registry is not a cancellation"
        );
    }

    /// A cancel stops the work it names, answers once, and is idempotent: the
    /// second call finds nothing in flight and says so.
    #[test]
    fn a_cancel_stops_the_work_and_a_second_call_is_already_finished() {
        let registry = Registry::new();
        let guard = registry.register("cli", id("cli-1"));

        assert_eq!(
            registry.cancel("cli", &id("cli-1")),
            CancelOutcome::Cancelled
        );
        assert!(guard.is_cancelled(), "the work sees its own cancellation");
        assert_eq!(
            registry.in_flight(),
            0,
            "cancelled work leaves the registry"
        );
        assert_eq!(registry.cancellations(), 1);

        assert_eq!(
            registry.cancel("cli", &id("cli-1")),
            CancelOutcome::AlreadyFinished,
            "a repeated cancel is answered rather than refused"
        );
        assert_eq!(
            registry.cancellations(),
            1,
            "only the call that stopped work counts"
        );

        drop(guard);
        assert_eq!(registry.cancellations(), 1);
    }

    /// The signal a handler selects on is pending while the work runs and
    /// resolves when the work is cancelled.
    #[tokio::test]
    async fn the_cancellation_signal_resolves_when_the_work_is_cancelled() {
        let registry = Registry::new();
        let guard = registry.register("cli", id("cli-1"));

        assert!(
            tokio::time::timeout(Duration::from_millis(50), guard.cancelled())
                .await
                .is_err(),
            "work that is not cancelled never signals"
        );

        assert_eq!(
            registry.cancel("cli", &id("cli-1")),
            CancelOutcome::Cancelled
        );
        tokio::time::timeout(Duration::from_secs(5), guard.cancelled())
            .await
            .expect("a cancellation wakes the work it stopped");
    }

    /// Work is keyed by the declaring surface: a cancel naming another
    /// surface's request finds nothing, answers as already finished, and leaves
    /// that work running.
    #[test]
    fn a_cancel_for_another_surface_touches_nothing() {
        let registry = Registry::new();
        let guard = registry.register("cli", id("cli-1"));

        assert_eq!(
            registry.cancel("ui", &id("cli-1")),
            CancelOutcome::AlreadyFinished,
            "another surface's identifier is not an oracle"
        );
        assert_eq!(registry.in_flight(), 1, "the work is untouched");
        assert!(!guard.is_cancelled());
        assert_eq!(registry.cancellations(), 0);

        assert_eq!(
            registry.cancel("cli", &id("cli-1")),
            CancelOutcome::Cancelled,
            "the declaring surface still stops its own work"
        );
    }

    /// The outcome crosses the wire as the contract's own value, and a value
    /// this build cannot act on is not guessed at.
    #[test]
    fn the_outcome_round_trips_through_the_wire_value() {
        assert_eq!(
            CancelOutcome::Cancelled.to_proto(),
            WireOutcome::Cancelled,
            "an in-flight request that is being stopped"
        );
        assert_eq!(
            CancelOutcome::AlreadyFinished.to_proto(),
            WireOutcome::AlreadyFinished,
            "nothing was in flight"
        );
        assert_eq!(
            CancelOutcome::from_proto(WireOutcome::Cancelled),
            Some(CancelOutcome::Cancelled)
        );
        assert_eq!(
            CancelOutcome::from_proto(WireOutcome::AlreadyFinished),
            Some(CancelOutcome::AlreadyFinished)
        );
        assert_eq!(
            CancelOutcome::from_proto(WireOutcome::Unspecified),
            None,
            "an answer that names nothing is left alone rather than guessed at"
        );
    }
}
