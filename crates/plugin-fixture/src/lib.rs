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
//!
//! Seven capabilities, not four: the routed `call` has no ceiling, and the
//! harness drives every one by name.
//!
//! The wiring at the bottom is the whole ABI footprint: one `const fn` table and
//! one entry point.

pub mod payload;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;
use std::time::Duration;

use plugin_contract::abi::MetisPluginTableV1;
use plugin_contract::authoring::{
    Call, CapabilityDescriptor, CapabilityKind, Host, LogLevel, Plugin, PluginError, Request,
    Response, accounting, schemars, table,
};

use crate::payload::{EchoRequest, EchoResponse, Progress, TimerAnswer, TimerRequest};

/// How long the blocking capability blocks, which is longer than the host's
/// watchdog window on purpose.
const BLOCKING: Duration = Duration::from_millis(250);

/// The fixture plugin.
#[derive(Default)]
pub struct Fixture;

/// One capability descriptor, with the inline schemas its bodies imply.
fn descriptor(name: &str) -> CapabilityDescriptor {
    CapabilityDescriptor {
        name: name.to_string(),
        kind: CapabilityKind::Tool,
        version: 1,
        request_schema: schemars::schema_for!(EchoRequest),
        response_schema: schemars::schema_for!(EchoResponse),
    }
}

/// The capability names, in the order the plugin declares them.
pub const CAPABILITIES: [&str; 7] = [
    "tool.echo",
    "tool.pending",
    "tool.cancellable",
    "tool.stubborn",
    "tool.panic",
    "tool.panic_drop",
    "tool.blocking",
];

/// The guard whose destructor panics, so the panic lands in the host's `drop`
/// call rather than in a poll.
struct PanicOnDrop;

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("fixture: a deliberate panic inside a future destructor")
    }
}

impl Plugin for Fixture {
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
            other => Err(PluginError::NotFound(other.to_string())),
        }
    }

    fn started(&self, host: &Host) -> Result<(), PluginError> {
        // An event at creation time is how the lifecycle half of the event route
        // is exercised: the callback belongs to the host table, not to a request.
        host.emit_event("fixture.ready", &serde_json::json!({ "ready": true }))
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
