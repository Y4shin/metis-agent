//! The fixture plugin: a real cdylib written through the routed authoring
//! trait.
//!
//! It is test support, not product code. `src/bin/abi-harness.rs` loads it as a
//! shared library and drives it, and nothing outside this crate depends on it.
//! It exists so the plugin boundary is proven by a real library loaded through
//! `plugin-contract`, not by a mock or by a second copy of the ABI.
//!
//! This file is an author's file: it names no buffer, no status, no callback, no
//! thread it did not start itself, and no payload header. The capabilities it
//! exposes, and what each one proves:
//!
//! - `tool.echo`, ready on its first poll, and it emits an event.
//! - `tool.pending`, which asks the host for a timer and answers when
//!   `service_complete` arrives.
//! - `tool.cancellable`, which parks on its cancellation token and resolves as
//!   `Cancelled`.
//! - `tool.stubborn`, which never honours a cancel, so the host's deadline
//!   expires and it records a missed cancel.
//! - `tool.panic`, which panics inside its poll.
//! - `tool.panic_drop`, whose future owns a guard whose destructor panics.
//! - `tool.blocking`, which blocks inside its poll, which the host's watchdog
//!   has to detect.
//! - `tool.newer`, whose descriptor version is one a host of this revision does
//!   not support, so the host refuses that one capability and keeps the rest of
//!   the plugin.
//! - `tool.deferred`, which keeps one clone of the waker it is polled with and
//!   stays pending, so that clone outlives the call.
//! - `tool.wake_deferred`, which wakes that kept clone; when it is called after
//!   the deferred call was dropped, it is the wake-after-drop case.
//!
//! Ten capabilities, not four: the routed `call` has no ceiling, and the
//! harness drives every one by name.
//!
//! The wiring at the bottom is the whole ABI footprint: one `const fn` table and
//! one entry point.

pub mod payload;

use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Mutex, Once};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use plugin_contract::abi::MetisPluginTableV1;
use plugin_contract::authoring::{
    Call, CapabilityDescriptor, Host, HostContext, Initialisation, LogLevel, Plugin, PluginError,
    Request, Response, accounting, schema_of, table,
};

use crate::payload::{
    EchoRequest, EchoResponse, FixtureConfig, Progress, TimerAnswer, TimerRequest,
};

/// How long the blocking capability blocks, which is longer than the host's
/// watchdog window on purpose.
const BLOCKING: Duration = Duration::from_millis(250);

/// The fixture plugin.
///
/// It is a unit value with no `Default`: the only way to obtain one is
/// [`Fixture::init`], which is what makes "an instance that exists has been
/// initialised" true for this plugin.
pub struct Fixture;

/// The descriptor version every capability but the gated one advertises.
const CAPABILITY_VERSION: u32 = 1;

/// The capability a newer plugin build advertises: its descriptor version is
/// one a host of this revision does not support.
///
/// It is one capability and not a plugin-wide fact, so refusing it must leave
/// the plugin's other capabilities listed and serving.
pub const NEWER_CAPABILITY: &str = "tool.newer";

/// The descriptor version of [`NEWER_CAPABILITY`].
pub const NEWER_CAPABILITY_VERSION: u32 = 2;

/// One capability descriptor, with the inline schemas its bodies imply.
fn descriptor(name: &str) -> CapabilityDescriptor {
    let version = if name == NEWER_CAPABILITY {
        NEWER_CAPABILITY_VERSION
    } else {
        CAPABILITY_VERSION
    };
    CapabilityDescriptor::tool(
        name,
        version,
        schema_of::<EchoRequest>(),
        schema_of::<EchoResponse>(),
    )
}

/// The capability names, in the order the plugin declares them.
pub const CAPABILITIES: [&str; 10] = [
    "tool.echo",
    "tool.pending",
    "tool.cancellable",
    "tool.stubborn",
    "tool.panic",
    "tool.panic_drop",
    "tool.blocking",
    DEFERRED_CAPABILITY,
    WAKE_DEFERRED_CAPABILITY,
    NEWER_CAPABILITY,
];

/// The capability that keeps one clone of the waker it is polled with.
pub const DEFERRED_CAPABILITY: &str = "tool.deferred";

/// The capability that wakes the clone [`DEFERRED_CAPABILITY`] kept.
pub const WAKE_DEFERRED_CAPABILITY: &str = "tool.wake_deferred";

/// One clone of a poll's waker, kept by the plugin rather than by the call.
///
/// It lives here and not in the future on purpose: a clone the future holds is
/// released when the future is dropped, while a clone the plugin keeps outlives
/// the call. Waking it afterwards is the wake-after-drop case the host has to
/// record rather than crash on.
static DEFERRED_WAKER: Mutex<Option<Waker>> = Mutex::new(None);

/// The future `tool.deferred` parks: it keeps one clone of the waker it is
/// polled with and never resolves on its own.
struct Deferred;

impl Future for Deferred {
    type Output = Result<Response, PluginError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        *DEFERRED_WAKER.lock().unwrap() = Some(context.waker().clone());
        Poll::Pending
    }
}

/// The guard whose destructor panics, so the panic lands in the host's `drop`
/// call rather than in a poll.
struct PanicOnDrop;

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("fixture: a deliberate panic inside a future destructor")
    }
}

impl Plugin for Fixture {
    async fn init(host: Host, initialisation: Initialisation) -> Result<Self, PluginError> {
        // The two parts of the one payload: the configuration the daemon
        // resolved for this instance, and what the host says about itself.
        let config: FixtureConfig = initialisation.config()?;
        let context: HostContext = initialisation.host_context()?;
        if config.instance != context.instance_name {
            // The "configuration does not match what the plugin declared"
            // class: the configuration was resolved for another instance.
            return Err(PluginError::InvalidArgument(format!(
                "the configuration was resolved for {} and this instance is {}",
                config.instance, context.instance_name
            )));
        }
        // A log and an event at initialisation time are how the lifecycle half
        // of both routes is exercised: the callbacks belong to the host table,
        // not to a request.
        host.log(
            LogLevel::Info,
            &format!("fixture: initialised for {}", context.instance_name),
        );
        host.emit_event("fixture.ready", &serde_json::json!({ "ready": true }))?;
        Ok(Fixture)
    }

    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        CAPABILITIES.iter().map(|name| descriptor(name)).collect()
    }

    async fn call(&self, call: Call, request: Request<'_>) -> Result<Response, PluginError> {
        match request.capability {
            "tool.echo" => echo(&call, &request),
            "tool.pending" => pending(&call).await,
            "tool.cancellable" => cancellable(&call).await,
            "tool.stubborn" => stubborn(&call).await,
            "tool.panic" => panic!("fixture: a deliberate panic inside a capability"),
            "tool.panic_drop" => {
                let _guard = PanicOnDrop;
                std::future::pending::<()>().await;
                unreachable!("the guard is only dropped when the host drops the future")
            }
            "tool.blocking" => {
                std::thread::sleep(BLOCKING);
                Response::json(&EchoResponse {
                    echo: "blocked".to_string(),
                    bytes: 0,
                })
            }
            // The kept clone is woken here. When this capability runs after the
            // deferred call was dropped, that wake is the late one.
            WAKE_DEFERRED_CAPABILITY => {
                if let Some(waker) = DEFERRED_WAKER.lock().unwrap().take() {
                    waker.wake();
                }
                Response::json(&EchoResponse {
                    echo: "woke the deferred call".to_string(),
                    bytes: "woke the deferred call".len(),
                })
            }
            DEFERRED_CAPABILITY => Deferred.await,
            // The gated capability still serves when it is called: the refusal
            // belongs to the host's version gate, and this arm is what lets the
            // harness prove the plugin was never the refusing party.
            NEWER_CAPABILITY => Response::json(&EchoResponse {
                echo: NEWER_CAPABILITY.to_string(),
                bytes: NEWER_CAPABILITY.len(),
            }),
            other => Err(PluginError::NotFound(other.to_string())),
        }
    }

    fn stopped(&self, host: &Host) {
        // The adapter owns every buffer the plugin hands out, so the ledger it
        // keeps is the plugin's side of the ownership check. This runs when the
        // host has dropped every future, which is what makes the numbers final.
        host.log(
            LogLevel::Info,
            &format!("FIXTURE_ACCOUNT {}", accounting().line()),
        );
    }
}

fn echo(call: &Call, request: &Request<'_>) -> Result<Response, PluginError> {
    let request: EchoRequest = request.json()?;
    let echo = request.echo.repeat(request.repeat as usize);
    call.emit_event(
        "fixture.answered",
        &serde_json::json!({ "bytes": echo.len() }),
    )?;
    Response::json(&EchoResponse {
        bytes: echo.len(),
        echo,
    })
}

async fn pending(call: &Call) -> Result<Response, PluginError> {
    call.progress(&Progress {
        stage: "working".to_string(),
    })?;
    let answer: TimerAnswer = call
        .service_request("timer", &TimerRequest { delay_ms: 5 })?
        .await?;
    Response::json(&EchoResponse {
        echo: format!("timer {}", answer.delay_ms),
        bytes: 6,
    })
}

async fn cancellable(call: &Call) -> Result<Response, PluginError> {
    call.cancellation().cancelled().await;
    call.log(LogLevel::Info, "fixture: cancellable observed the cancel");
    Err(PluginError::Cancelled)
}

async fn stubborn(call: &Call) -> Result<Response, PluginError> {
    // Cancellation is not honoured here on purpose: the host's deadline expires
    // and it records a missed cancel, then answers the timer anyway so the call
    // can still finish.
    let answer: TimerAnswer = call
        .service_request("timer", &TimerRequest { delay_ms: 20 })?
        .await?;
    Response::json(&EchoResponse {
        echo: format!("stubborn {}", answer.delay_ms),
        bytes: 9,
    })
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
/// The adapter catches a panic in a poll and turns it into a terminal status, so
/// the panic is a designed failure mode rather than a crash. A panic hook cannot
/// reach the host: it has no context, which is why the adapter logs the catch
/// through the host callback and this one only quiets the process.
fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("[fixture] a deliberate panic was caught at the boundary: {info}");
        }));
    });
}
