//! The fixture plugin: a real cdylib written through the authoring trait.
//!
//! It is test support, not product code. `src/bin/abi-harness.rs` loads it as a
//! shared library and drives it, and nothing outside this crate depends on it.
//! It exists so the plugin boundary is proven by a real library loaded through
//! `plugin-contract`, not by a mock or by a second copy of the ABI.
//!
//! This file is an author's file: it names no buffer, no status, no callback, no
//! thread, and no payload header. The five capabilities it exposes, and what
//! each one proves:
//!
//! - `tool.echo`, the capability that used to finish inline: through the trait
//!   it completes through the host's `complete` callback like every other arm
//!   (D15), and it emits an event, which is the capability half of the event
//!   route.
//! - `tool.pending`, the deferred half of D11: a `progress` callback, then a
//!   result through `complete`. It does not honour a cancel, which is what makes
//!   the host drop a completion that arrives after one (D12).
//! - `tool.cancellable`, which parks on the cancellation token and is woken by
//!   the cancel (D15); its own budget is the backstop that makes the other half
//!   of the suppression rule observable, because the same `Cancelled` value
//!   returned without a successful cancel still completes.
//! - `tool.panic`, a deliberate panic inside a capability, which the adapter's
//!   worker turns into an internal-error completion instead of losing the call
//!   (D7).
//!
//! The wiring at the bottom is the whole ABI footprint: one `const fn` table and
//! one entry point.

pub mod payload;

use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use plugin_contract::abi::MetisPluginTableV1;
use plugin_contract::authoring::{
    Call, CancellationToken, Cancelled, CapabilityDescriptor, CapabilityKind, Host, LogLevel,
    Plugin, PluginError, Shape, accounting, table,
};

use crate::payload::{EchoRequest, EchoResponse, Progress};

/// How long the cancellable arm waits for a cancel before giving up on its own.
///
/// Long enough that the harness's cancel always wins the race and short enough
/// that an uncancelled call still ends.
const CANCELLABLE_BUDGET: Duration = Duration::from_millis(1500);

/// How long the pending arm stays in flight. Long enough that a cancel arriving
/// right after the pending return is observed before the completion.
const PENDING_DELAY: Duration = Duration::from_millis(120);

/// The fixture plugin.
#[derive(Default)]
pub struct Fixture;

/// The one payload shape the fixture's capabilities use.
///
/// Public because it appears in the plugin's associated types, which are part of
/// the public implementation; it is still fixture-local, and no payload type
/// reaches `plugin-contract`.
pub struct Echo;

impl Shape for Echo {
    type Request = EchoRequest;
    type Response = EchoResponse;
}

fn descriptor(name: &str, kind: CapabilityKind) -> CapabilityDescriptor {
    CapabilityDescriptor {
        name: name.to_string(),
        kind,
        version: 1,
        schema: format!("{name}/v1"),
    }
}

impl Plugin for Fixture {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![
            descriptor(Self::ANSWER, CapabilityKind::Tool),
            descriptor(Self::DEFERRED, CapabilityKind::Tool),
            descriptor(Self::CANCELLABLE, CapabilityKind::Tool),
            descriptor(Self::FAULT, CapabilityKind::Tool),
        ]
    }

    const ANSWER: &'static str = "tool.echo";
    const DEFERRED: &'static str = "tool.pending";
    const CANCELLABLE: &'static str = "tool.cancellable";
    const FAULT: &'static str = "tool.panic";

    type Answer = Echo;
    type Deferred = Echo;
    type Cancellable = Echo;
    type Fault = Echo;

    async fn answer(&self, call: Call, request: EchoRequest) -> Result<EchoResponse, PluginError> {
        let echo = request.echo.repeat(request.repeat as usize);
        call.emit_event(
            "fixture.answered",
            &serde_json::json!({ "bytes": echo.len() }),
        )?;
        Ok(EchoResponse {
            bytes: echo.len(),
            echo,
        })
    }

    async fn deferred(
        &self,
        call: Call,
        _request: EchoRequest,
    ) -> Result<EchoResponse, PluginError> {
        call.progress(&Progress {
            stage: "working".to_string(),
        })?;
        // Cancellation is not honoured here on purpose: the completion arrives
        // anyway and the host has to drop it (D12). The worker thread belongs to
        // this call and to no other, so a sleep is what "in flight" means here.
        thread::sleep(PENDING_DELAY);
        Ok(EchoResponse {
            echo: "async".to_string(),
            bytes: 5,
        })
    }

    async fn cancellable(
        &self,
        call: Call,
        _request: EchoRequest,
    ) -> Result<EchoResponse, PluginError> {
        let parked = Instant::now();
        let outcome = WaitFor::new(call.cancellation(), CANCELLABLE_BUDGET).await;
        match outcome {
            GiveUp::Token => call.log(
                LogLevel::Info,
                &format!(
                    "fixture: cancellable observed the cancel after {}ms",
                    parked.elapsed().as_millis()
                ),
            ),
            GiveUp::Budget => call.log(
                LogLevel::Info,
                &format!(
                    "fixture: cancellable gave up without a cancel after {}ms",
                    parked.elapsed().as_millis()
                ),
            ),
        }
        // The same value either way: the adapter's rule is what decides whether
        // the host hears about it, because a give-up after a successful cancel
        // has a host that is no longer waiting.
        Err(PluginError::Cancelled)
    }

    async fn fault(&self, _call: Call, _request: EchoRequest) -> Result<EchoResponse, PluginError> {
        panic!("fixture: deliberate panic inside a capability")
    }

    fn cancel(&self, _call: Call) -> Result<(), PluginError> {
        Ok(())
    }

    fn started(&self, host: &Host) -> Result<(), PluginError> {
        // An event at creation time is how the lifecycle half of the event route
        // is exercised: the callback belongs to the host table, not to a request.
        host.emit_event("fixture.ready", &serde_json::json!({ "ready": true }))
    }

    fn stopped(&self, host: &Host) {
        // The adapter owns every buffer the plugin hands out, so the ledger it
        // keeps is the plugin's side of the ownership check (D6). This runs
        // after `shutdown` has joined every worker, which is what makes the
        // numbers final.
        host.log(
            LogLevel::Info,
            &format!("FIXTURE_ACCOUNT {}", accounting().line()),
        );
    }
}

// --- the fixture's bounded wait --------------------------------------------

/// Which half of the wait ended it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GiveUp {
    /// The call was cancelled and the token woke the future.
    Token,
    /// The fixture's own budget ran out, with no cancel.
    Budget,
}

/// Waits for a cancellation token, and no longer than a budget.
///
/// The contract ships no select combinator, so this fixture-local future races
/// the two endings itself: it polls the token's future, which registers the
/// worker's waker with the token (so a cancel wakes it promptly), and a timer
/// thread wakes the worker at the deadline so an uncancelled call still ends.
///
/// The timer thread is joined when the future is dropped, which happens on the
/// worker before it exits, so no thread of this library can run after the host
/// unloads it.
struct WaitFor {
    token: Cancelled,
    deadline: Instant,
    timer: Option<Timer>,
}

impl WaitFor {
    fn new(token: CancellationToken, budget: Duration) -> Self {
        WaitFor {
            token: token.cancelled(),
            deadline: Instant::now() + budget,
            timer: None,
        }
    }
}

impl Future for WaitFor {
    type Output = GiveUp;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<GiveUp> {
        let this = self.get_mut();
        if Pin::new(&mut this.token).poll(context).is_ready() {
            return Poll::Ready(GiveUp::Token);
        }
        if Instant::now() >= this.deadline {
            return Poll::Ready(GiveUp::Budget);
        }
        if this.timer.is_none() {
            this.timer = Some(Timer::until(this.deadline, context.waker().clone()));
        }
        Poll::Pending
    }
}

/// A thread that wakes a task at a deadline, and stops when the task says so.
struct Timer {
    abandoned: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Timer {
    fn until(deadline: Instant, waker: Waker) -> Timer {
        let abandoned = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&abandoned);
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                thread::park_timeout(deadline - now);
            }
            if !flag.load(Ordering::SeqCst) {
                waker.wake();
            }
        });
        Timer {
            abandoned,
            handle: Some(handle),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.abandoned.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            // The park is bounded by the deadline, and an unpark returns from it
            // at once, so this join is brief.
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

// --- the wiring (the fixture's whole ABI footprint) -------------------------

static PLUGIN_TABLE_V1: MetisPluginTableV1 = table::<Fixture>();

#[unsafe(no_mangle)]
pub extern "C" fn metis_plugin_entry_v1() -> *const MetisPluginTableV1 {
    install_panic_hook();
    catch_unwind(AssertUnwindSafe(|| &raw const PLUGIN_TABLE_V1)).unwrap_or(std::ptr::null())
}

/// Reports a caught panic in one line instead of the default hook's notice.
///
/// The adapter catches a capability panic and turns it into a completion (D7),
/// so the panic is a designed failure mode rather than a crash. A panic hook
/// cannot reach the host: it has no context, which is why the adapter logs the
/// catch through the host callback and this one only quiets the process.
fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("[fixture] a deliberate panic was caught at the boundary: {info}");
        }));
    });
}
