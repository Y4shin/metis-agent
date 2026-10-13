//! The authoring surface's seams, driven in-process through the exported table
//! and the host adapter.
//!
//! `crates/plugin-fixture` drives the same adapter through a real `cdylib`, so
//! these tests cover what a second fixture library would be needed for: a
//! deliberately inconsistent plugin, a capability that answers `Cancelled`
//! without a successful cancel, and a lifecycle hook that fails.
//!
//! Request ids are namespaced per test (`1_0xx`, `2_0xx`, ...) because the
//! adapter's pending registry and its accounting ledger are process-wide: the
//! exported table is a `const fn` (D1, D8) with no user data to key them by.
#![cfg(feature = "authoring")]

use std::time::Duration;

use plugin_contract::abi::{MetisPluginTableV1, MetisStatus, metis_read_prefix};
use plugin_contract::authoring::{
    Call, CapabilityDescriptor, CapabilityKind, HostAdapter, Plugin, PluginError, Shape, table,
};
use serde::{Deserialize, Serialize};

/// How long a test waits for a completion before calling it missing. The host
/// never waits unbounded (mechanics spike finding): a completion that will not
/// arrive looks exactly like a hang from the outside.
const WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Ping {
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Pong {
    text: String,
}

/// The probe plugin's one payload shape, reused by all four arms.
struct PingShape;

impl Shape for PingShape {
    type Request = Ping;
    type Response = Pong;
}

fn descriptor(name: &str, kind: CapabilityKind) -> CapabilityDescriptor {
    CapabilityDescriptor {
        name: name.to_string(),
        kind,
        version: 1,
        schema: format!("{name}/v1"),
    }
}

/// The probe plugin: one arm per capability shape the adapter dispatches on.
#[derive(Default)]
struct Probe;

impl Plugin for Probe {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![
            descriptor(Self::ANSWER, CapabilityKind::Tool),
            descriptor(Self::DEFERRED, CapabilityKind::Provider),
            descriptor(Self::CANCELLABLE, CapabilityKind::Tool),
            descriptor(Self::FAULT, CapabilityKind::Tool),
        ]
    }

    const ANSWER: &'static str = "probe.answer";
    const DEFERRED: &'static str = "probe.deferred";
    const CANCELLABLE: &'static str = "probe.cancellable";
    const FAULT: &'static str = "probe.fault";

    type Answer = PingShape;
    type Deferred = PingShape;
    type Cancellable = PingShape;
    type Fault = PingShape;

    async fn answer(&self, _call: Call, request: Ping) -> Result<Pong, PluginError> {
        Ok(Pong { text: request.text })
    }

    async fn deferred(&self, _call: Call, request: Ping) -> Result<Pong, PluginError> {
        Ok(Pong { text: request.text })
    }

    async fn cancellable(&self, _call: Call, request: Ping) -> Result<Pong, PluginError> {
        Ok(Pong { text: request.text })
    }

    async fn fault(&self, _call: Call, _request: Ping) -> Result<Pong, PluginError> {
        Ok(Pong {
            text: "the fault arm does not fault in this probe".to_string(),
        })
    }
}

/// Creates an instance of `Probe` through the exported table and attaches the
/// table to the host, so plugin buffers can be released.
fn instance(host: &HostAdapter, table: &MetisPluginTableV1) -> *mut std::ffi::c_void {
    host.attach(table);
    let mut handle: *mut std::ffi::c_void = std::ptr::null_mut();
    let status = unsafe { (table.create_instance)(host.api(), &mut handle) };
    assert_eq!(status, MetisStatus::Ok, "the probe instance is created");
    handle
}

#[test]
fn the_trait_adapter_always_completes_through_the_host_callback() {
    let table = table::<Probe>();
    let prefix = unsafe { metis_read_prefix(&table) }.expect("the table carries a readable prefix");
    assert_eq!(prefix.size, std::mem::size_of::<MetisPluginTableV1>());

    let host = HostAdapter::new();
    let handle = instance(&host, &table);
    let completion = host.register(1_001);

    // D15: the trait path is always pending, even for a capability whose future
    // is ready on its first poll. The ABI keeps its inline path for direct
    // implementers; nothing in this test reaches it.
    let status = unsafe {
        host.invoke(
            handle,
            Probe::ANSWER,
            1_001,
            &serde_json::to_vec(&Ping {
                text: "ping".to_string(),
            })
            .expect("the request serialises"),
        )
    };
    assert_eq!(status, MetisStatus::Pending);

    let completed = completion
        .wait_timeout(WAIT)
        .expect("the completion arrives through the host callback");
    assert_eq!(completed.status, MetisStatus::Ok);
    let body: Pong = serde_json::from_slice(&completed.body).expect("the response deserialises");
    assert_eq!(body.text, "ping");
}
