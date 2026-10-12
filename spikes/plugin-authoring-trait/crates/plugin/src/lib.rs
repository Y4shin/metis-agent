//! Consumer one: a plugin that implements the throwaway authoring trait.
//!
//! Nothing here mentions the ABI except the table static and the entry point at
//! the bottom. No buffer, no status code, no payload header, no thread.

use std::future::Future;
use std::time::Duration;

use spike_abi::MetisPluginTableV1;
use spike_authoring::executor::sleep;
use spike_authoring::payload::{
    CapabilityDescriptor, CapabilityKind, CompletionRequest, CompletionResponse, EchoRequest,
    EchoResponse, Progress,
};
use spike_authoring::plugin_adapter::{
    CAPABILITY_COMPLETE, CAPABILITY_ECHO, CAPABILITY_UNCANCELLABLE, capability_descriptor, table,
};
use spike_authoring::{Call, Plugin, PluginError};

/// The plugin itself, in the shape an author would write it.
#[derive(Default)]
struct Tools;

impl Plugin for Tools {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![
            capability_descriptor(CAPABILITY_ECHO, CapabilityKind::Tool, 1),
            capability_descriptor(CAPABILITY_COMPLETE, CapabilityKind::Provider, 1),
            capability_descriptor(CAPABILITY_UNCANCELLABLE, CapabilityKind::Tool, 1),
        ]
    }

    fn echo(
        &self,
        _call: Call,
        request: EchoRequest,
    ) -> impl Future<Output = Result<EchoResponse, PluginError>> + Send {
        async move {
            let echo = request.echo.repeat(request.repeat as usize);
            Ok(EchoResponse { bytes: echo.len(), echo })
        }
    }

    fn complete(
        &self,
        call: Call,
        request: CompletionRequest,
    ) -> impl Future<Output = Result<CompletionResponse, PluginError>> + Send {
        async move {
            call.progress(&Progress { stage: "working".to_string() })?;
            sleep(Duration::from_millis(120)).await;
            if call.is_cancelled() {
                return Err(PluginError::Cancelled);
            }
            Ok(CompletionResponse { text: format!("{} ({})", request.prompt, request.max_tokens) })
        }
    }

    fn uncancellable(
        &self,
        _call: Call,
        request: EchoRequest,
    ) -> impl Future<Output = Result<EchoResponse, PluginError>> + Send {
        async move {
            // Deliberately ignores cancellation, so the completion arrives after
            // a successful cancel and the host has to drop it (D12).
            sleep(Duration::from_millis(80)).await;
            Ok(EchoResponse { bytes: request.echo.len(), echo: request.echo })
        }
    }

    fn cancel(&self, _call: Call) -> Result<(), PluginError> {
        Ok(())
    }
}

/// The hand-written wiring: one static table, one entry point.
static TABLE_V1: MetisPluginTableV1 = table::<Tools>();

#[unsafe(no_mangle)]
pub extern "C" fn metis_plugin_entry_v1() -> *const MetisPluginTableV1 {
    &raw const TABLE_V1
}
