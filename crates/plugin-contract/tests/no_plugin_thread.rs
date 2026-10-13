//! The no-executor seam, in a binary of its own.
//!
//! The authoring path used to own an executor: a bounded pool with a
//! thread-per-call fallback, and a host adapter that parked on a completion
//! callback. The revision deletes all of it. The observable consequence is that
//! a capability is driven to completion without the plugin starting a thread,
//! and the honest way to assert that is a thread count taken across the drive.
//!
//! This lives in its own integration binary on purpose: the test harness starts
//! a thread per test, so a count taken in a binary that runs other cases in
//! parallel would measure the harness rather than the plugin.

#![cfg(feature = "authoring")]

use std::ffi::c_void;
use std::mem::size_of;
use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};

use plugin_contract::abi::{
    METIS_BUFFER_OWNER_HOST, MetisBuffer, MetisFutureHandle, MetisHostApiV1, MetisPluginTableV1,
    MetisStatus, MetisWaker,
};
use plugin_contract::authoring::{
    Call, CapabilityDescriptor, CapabilityKind, Plugin, PluginError, Request, Response, table,
};
use plugin_contract::payload::{metis_encode_payload, metis_payload_body};

/// The one capability this probe advertises.
#[derive(Default)]
struct Probe;

impl Plugin for Probe {
    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![CapabilityDescriptor {
            name: "tool.probe".to_string(),
            kind: CapabilityKind::Tool,
            version: 1,
            request_schema: plugin_contract::authoring::schemars::schema_for!(Body),
            response_schema: plugin_contract::authoring::schemars::schema_for!(Body),
        }]
    }

    async fn call(&self, _call: Call, request: Request<'_>) -> Result<Response, PluginError> {
        let body: Body = request.json()?;
        Response::json(&Body {
            text: format!("{}:{}", request.capability, body.text),
        })
    }
}

#[derive(
    serde::Serialize, serde::Deserialize, plugin_contract::authoring::schemars::JsonSchema,
)]
struct Body {
    text: String,
}

static PROBE: MetisPluginTableV1 = table::<Probe>();

// --- the host side --------------------------------------------------------

/// A host with no state beyond its table: enough to create an instance, poll a
/// future, and release what the plugin allocated.
struct Host {
    table: *const MetisPluginTableV1,
    api: MetisHostApiV1,
    released: AtomicUsize,
}

impl Host {
    fn new(table: *const MetisPluginTableV1) -> Box<Host> {
        let mut host = Box::new(Host {
            table,
            api: MetisHostApiV1 {
                size: size_of::<MetisHostApiV1>(),
                user_data: std::ptr::null_mut(),
                progress: noop_progress,
                emit_event: noop_emit_event,
                log: noop_log,
                service_request: noop_service_request,
            },
            released: AtomicUsize::new(0),
        });
        host.api.user_data = &*host as *const Host as *mut c_void;
        host
    }

    fn api(&self) -> *const MetisHostApiV1 {
        &self.api
    }

    fn waker(&self) -> MetisWaker {
        MetisWaker {
            data: self as *const Host as *mut c_void,
            vtable: &WAKER_VTABLE,
        }
    }

    /// Frees a plugin-owned buffer through the table, which is the only way a
    /// host may free one.
    fn release(&self, buffer: MetisBuffer) {
        if buffer.ptr.is_null() {
            return;
        }
        // SAFETY: the table is the live one, and the buffer is the plugin's.
        let status = unsafe { ((*self.table).release_buffer)(buffer) };
        assert_eq!(status, MetisStatus::Ok);
        self.released.fetch_add(1, Ordering::SeqCst);
    }
}

unsafe extern "C" fn noop_progress(_user_data: *mut c_void, _id: u64, _payload: MetisBuffer) {}

unsafe extern "C" fn noop_emit_event(
    _user_data: *mut c_void,
    _topic: *const u8,
    _topic_len: usize,
    _payload: MetisBuffer,
) {
}

unsafe extern "C" fn noop_log(
    _user_data: *mut c_void,
    _level: i32,
    _message: *const u8,
    _message_len: usize,
) {
}

unsafe extern "C" fn noop_service_request(
    _user_data: *mut c_void,
    _service: *const u8,
    _service_len: usize,
    _request_id: u64,
    _payload: MetisBuffer,
) -> MetisStatus {
    MetisStatus::Unsupported
}

unsafe extern "C" fn waker_clone(waker: *const MetisWaker) -> MetisWaker {
    // The fields are read one at a time: a clone is this entry, not a Rust
    // copy of the struct.
    // SAFETY: the waker is live for the poll.
    MetisWaker {
        data: unsafe { (*waker).data },
        vtable: unsafe { (*waker).vtable },
    }
}

unsafe extern "C" fn waker_wake(_waker: MetisWaker) {}

unsafe extern "C" fn waker_wake_by_ref(_waker: *const MetisWaker) {}

unsafe extern "C" fn waker_drop(_waker: MetisWaker) {}

static WAKER_VTABLE: plugin_contract::abi::MetisWakerVtable =
    plugin_contract::abi::MetisWakerVtable {
        clone: waker_clone,
        wake: waker_wake,
        wake_by_ref: waker_wake_by_ref,
        drop: waker_drop,
    };

/// The number of threads in this process, which is how "no plugin thread" is
/// measured rather than argued.
fn thread_count() -> usize {
    std::fs::read_dir("/proc/self/task")
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[test]
fn a_capability_completes_with_no_plugin_thread() {
    let host = Host::new(&raw const PROBE);
    let mut instance = std::ptr::null_mut();
    // SAFETY: the host table lives in the box and stays valid for the
    // instance's lifetime.
    let status = unsafe { (PROBE.create_instance)(host.api(), &mut instance) };
    assert_eq!(status, MetisStatus::Ok);

    let body = serde_json::to_vec(&Body {
        text: "probe".to_string(),
    })
    .unwrap();
    let encoded = metis_encode_payload(&body);

    let before = thread_count();
    let mut handle = MetisFutureHandle::null();
    // SAFETY: the instance is live and the request buffer is host-owned.
    let status = unsafe {
        (PROBE.invoke_future)(
            instance,
            b"tool.probe".as_ptr(),
            "tool.probe".len(),
            1,
            MetisBuffer {
                ptr: encoded.as_ptr() as *mut u8,
                len: encoded.len(),
                cap: encoded.len(),
                owner: METIS_BUFFER_OWNER_HOST,
                reserved: 0,
            },
            host.api(),
            &mut handle,
        )
    };
    assert_eq!(status, MetisStatus::Ok);

    let mut out = MetisBuffer::empty();
    // SAFETY: the handle came from this table's `invoke_future`.
    let status = unsafe { ((*handle.vtable).poll)(handle.ctx, host.waker(), &mut out) };
    assert_eq!(
        status,
        MetisStatus::Ok,
        "the capability is ready on its first poll"
    );
    let response = unsafe { slice::from_raw_parts(out.ptr, out.len) }.to_vec();
    host.release(out);
    // SAFETY: the handle came from this table's `invoke_future`.
    let status = unsafe { ((*handle.vtable).drop)(handle.ctx) };
    assert_eq!(status, MetisStatus::Ok);
    let after = thread_count();

    assert_eq!(
        before, after,
        "the authoring path owns no executor, so driving a capability to completion \
         starts no thread"
    );
    assert_eq!(host.released.load(Ordering::SeqCst), 1);
    let body = metis_payload_body(&response).expect("the response carries an envelope");
    let body: Body = serde_json::from_slice(body).expect("the response is JSON");
    assert_eq!(body.text, "tool.probe:probe");

    // SAFETY: the instance is live.
    let _ = unsafe { (PROBE.shutdown)(instance) };
    // SAFETY: the instance is live.
    let _ = unsafe { (PROBE.destroy_instance)(instance) };
}
