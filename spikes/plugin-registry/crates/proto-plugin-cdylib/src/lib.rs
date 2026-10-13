//! PROTOTYPE (throwaway). Not product code.
//!
//! A cdylib plugin that implements `MetisPluginTableV1` directly, using the
//! landed `plugin-contract` ABI types, and always answers the way the real
//! trait path does: `invoke` returns `Pending` and the result arrives later
//! through the host's `complete` callback from a plugin worker thread. Nothing
//! is ever completed inline.
//!
//! It exercises the whole async and lifecycle surface the landed
//! `authoring::HostAdapter` provides: late completion, progress, events, logs,
//! cancellation (including a completion that races a cancel), a completion that
//! never arrives, concurrent in-flight calls, a callback that arrives after
//! `shutdown`, and instance create/use/destroy/reuse.
//!
//! The table the host reads is discovered through `list_capabilities`, which
//! returns descriptors that reference their schemas *by id*; the schema
//! documents travel in one batched `metis.schemas` call. `metis.*` capabilities
//! are host-internal protocol and are deliberately not listed, so the registry
//! never routes them.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use proto_abi::{
    METIS_ABI_MAJOR, METIS_ABI_MINOR, METIS_BUFFER_OWNER_HOST, METIS_BUFFER_OWNER_PLUGIN,
    METIS_LOG_DEBUG, METIS_LOG_ERROR, METIS_LOG_INFO, METIS_LOG_WARN, MetisBuffer,
    MetisCreateInstanceFn, MetisDestroyInstanceFn, MetisHostApiV1, MetisListCapabilitiesFn,
    MetisPluginTableV1, MetisStatus, metis_encode_payload, metis_payload_body,
};
use proto_registry::{Executor, SpawnRefused, Task};
use proto_sdk::{IdlePolicy, PoolExecutor, ThreadPerCallExecutor};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod future;

// --- the capability payload types ------------------------------------------

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EchoRequest {
    pub message: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EchoResponse {
    pub echo: String,
    pub length: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AddRequest {
    pub a: i64,
    pub b: i64,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AddResponse {
    pub sum: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UppercaseRequest {
    pub text: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UppercaseResponse {
    pub upper: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BadResponseRequest {
    pub x: i64,
}
/// The response schema `tool.bad_response` publishes. The plugin does not honour
/// it: see the `Job::BadResponse` arm.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BadResponse {
    /// Published as a string. The plugin returns `42`.
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CompleteRequest {
    pub prompt: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CompleteResponse {
    pub text: String,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DelayRequest {
    pub delay_ms: u64,
    pub tag: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DelayResponse {
    pub tag: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RaceRequest {
    pub delay_ms: u64,
    pub tag: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RaceResponse {
    pub tag: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct NeverRequest {
    pub x: i64,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct NeverResponse {
    pub x: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct StragglerRequest {
    pub delay_ms: u64,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct StragglerResponse {
    pub tag: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct StatsRequest {}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct StatsResponse {
    pub instance: String,
    pub invokes: u64,
    pub config_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EventsRequest {
    pub topic: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EventsResponse {
    pub emitted: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LogsRequest {}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LogsResponse {
    pub levels: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FutureRequest {
    pub x: i64,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FutureResponse {
    pub y: i64,
}

/// The request the host-polled future capabilities take. `delay_ms` is how long
/// the host timer service should take; 0 for the latency loops.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FutureSeamRequest {
    pub x: i64,
    #[serde(default)]
    pub delay_ms: u64,
}

// --- schemas ---------------------------------------------------------------

fn schema_of<T: JsonSchema>() -> Value {
    let mut value = schemars::schema_for!(T).to_value();
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "$schema".to_owned(),
            json!("https://json-schema.org/draft/2020-12/schema"),
        );
    }
    value
}

fn sid(name: &str, side: &str) -> String {
    format!("proto-plugin-cdylib/{name}.{side}")
}

/// A routed capability: name, kind, version, and the two schema ids.
#[derive(Serialize)]
struct WireDescriptor {
    name: &'static str,
    kind: &'static str,
    version: u32,
    request_schema_id: String,
    response_schema_id: String,
}

fn descriptor<Req: JsonSchema, Resp: JsonSchema>(
    name: &'static str,
    kind: &'static str,
    version: u32,
) -> (WireDescriptor, Value, Value) {
    (
        WireDescriptor {
            name,
            kind,
            version,
            request_schema_id: sid(name, "request"),
            response_schema_id: sid(name, "response"),
        },
        schema_of::<Req>(),
        schema_of::<Resp>(),
    )
}

/// The routed capabilities, in declaration order.
fn capabilities() -> Vec<(WireDescriptor, Value, Value)> {
    vec![
        descriptor::<EchoRequest, EchoResponse>("tool.echo", "tool", 1),
        descriptor::<AddRequest, AddResponse>("tool.add", "tool", 1),
        descriptor::<UppercaseRequest, UppercaseResponse>("tool.uppercase", "tool", 1),
        descriptor::<BadResponseRequest, BadResponse>("tool.bad_response", "tool", 1),
        descriptor::<CompleteRequest, CompleteResponse>("provider.complete", "provider", 2),
        descriptor::<DelayRequest, DelayResponse>("tool.delay", "tool", 1),
        descriptor::<RaceRequest, RaceResponse>("tool.races", "tool", 1),
        descriptor::<NeverRequest, NeverResponse>("tool.never", "tool", 1),
        descriptor::<StragglerRequest, StragglerResponse>("tool.straggler", "tool", 1),
        descriptor::<StatsRequest, StatsResponse>("tool.stats", "tool", 1),
        descriptor::<EventsRequest, EventsResponse>("tool.events", "tool", 1),
        descriptor::<LogsRequest, LogsResponse>("tool.logs", "tool", 1),
        descriptor::<FutureRequest, FutureResponse>("tool.future", "tool", 99),
        // The host-polled future seam, lane 1. `tool.poll_future` is the
        // async-ffi object, `tool.hand_future` the hand-written C stand-in;
        // the rest are the cost measurements.
        descriptor::<FutureSeamRequest, FutureResponse>("tool.poll_future", "tool", 1),
        descriptor::<FutureSeamRequest, FutureResponse>("tool.hand_future", "tool", 1),
        descriptor::<FutureSeamRequest, FutureResponse>("tool.future_ready", "tool", 1),
        descriptor::<FutureSeamRequest, FutureResponse>("tool.future_ready_noshim", "tool", 1),
        descriptor::<FutureSeamRequest, FutureResponse>("tool.future_reuse", "tool", 1),
    ]
}

/// The `list_capabilities` body: descriptors by schema id, no schema documents.
fn descriptor_list_body() -> Vec<u8> {
    let list: Vec<WireDescriptor> = capabilities().into_iter().map(|(d, _, _)| d).collect();
    serde_json::to_vec(&json!({ "capabilities": list })).unwrap_or_default()
}

/// The measurement body: descriptors with both schemas inline, the round one
/// shape, so the two descriptor costs can be compared over the same ABI.
fn inline_descriptor_list_body() -> Vec<u8> {
    let list: Vec<Value> = capabilities()
        .into_iter()
        .map(|(d, request, response)| {
            json!({
                "name": d.name,
                "kind": d.kind,
                "version": d.version,
                "request_schema": request,
                "response_schema": response,
            })
        })
        .collect();
    serde_json::to_vec(&json!({ "capabilities": list })).unwrap_or_default()
}

fn schema_document(id: &str) -> Option<Value> {
    let requested = id.to_owned();
    capabilities()
        .into_iter()
        .find_map(|(d, request, response)| {
            if d.request_schema_id == requested {
                Some(request)
            } else if d.response_schema_id == requested {
                Some(response)
            } else {
                None
            }
        })
}

// --- buffers ---------------------------------------------------------------

fn live_buffers() -> &'static Mutex<HashMap<usize, usize>> {
    static LIVE: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static VIOLATIONS: AtomicUsize = AtomicUsize::new(0);
static SUPPRESSED: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn plugin_alloc(bytes: &[u8]) -> MetisBuffer {
    let mut owned = bytes.to_vec();
    let buffer = MetisBuffer {
        ptr: owned.as_mut_ptr(),
        len: owned.len(),
        cap: owned.capacity(),
        owner: METIS_BUFFER_OWNER_PLUGIN,
        reserved: 0,
    };
    std::mem::forget(owned);
    live_buffers()
        .lock()
        .unwrap()
        .insert(buffer.ptr as usize, buffer.cap);
    ALLOCS.fetch_add(1, SeqCst);
    buffer
}

fn accounting_line() -> String {
    format!(
        "accounting allocs={} frees={} live={} violations={} suppressed={}",
        ALLOCS.load(SeqCst),
        FREES.load(SeqCst),
        live_buffers().lock().unwrap().len(),
        VIOLATIONS.load(SeqCst),
        SUPPRESSED.load(SeqCst),
    )
}

pub(crate) unsafe extern "C" fn release_buffer(buffer: MetisBuffer) -> MetisStatus {
    if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
        VIOLATIONS.fetch_add(1, SeqCst);
        return MetisStatus::InvalidArgument;
    }
    if buffer.ptr.is_null() {
        return MetisStatus::Ok;
    }
    let known = live_buffers()
        .lock()
        .unwrap()
        .remove(&(buffer.ptr as usize));
    if known.is_none() {
        VIOLATIONS.fetch_add(1, SeqCst);
        return MetisStatus::InvalidArgument;
    }
    // SAFETY: this plugin allocated the buffer with a Vec of exactly these
    // parts, and this is the only place it is freed.
    drop(unsafe { Vec::from_raw_parts(buffer.ptr, buffer.len, buffer.cap) });
    FREES.fetch_add(1, SeqCst);
    MetisStatus::Ok
}

// --- per request cancellation ----------------------------------------------

struct RequestState {
    cancelled: AtomicBool,
    doorbell: Mutex<()>,
    condvar: Condvar,
}

impl RequestState {
    fn new() -> Self {
        RequestState {
            cancelled: AtomicBool::new(false),
            doorbell: Mutex::new(()),
            condvar: Condvar::new(),
        }
    }

    /// Waits up to `timeout` for a cancel, returning whether one arrived.
    fn park_or_cancel(&self, timeout: Duration) -> bool {
        let guard = self.doorbell.lock().unwrap();
        if self.cancelled.load(SeqCst) {
            return true;
        }
        let _ = self.condvar.wait_timeout(guard, timeout).unwrap();
        self.cancelled.load(SeqCst)
    }

    fn mark_cancelled(&self) {
        self.cancelled.store(true, SeqCst);
        drop(self.doorbell.lock());
        self.condvar.notify_all();
    }
}

// --- instance state --------------------------------------------------------

struct InstanceState {
    host: *const MetisHostApiV1,
    /// The executor that drives this instance's legacy capability futures. The
    /// SDK-injected pool is the default; the thread-per-call fallback is kept
    /// behind the same seam so both are measured in one binary. The host-polled
    /// future seam does not use this executor at all.
    executor: Mutex<Arc<dyn Executor>>,
    /// The current executor mode, for `metis.executor` and the status line.
    executor_label: Mutex<String>,
    /// The configured worker count (pool mode).
    executor_workers: AtomicUsize,
    requests: Mutex<HashMap<u64, Arc<RequestState>>>,
    cancelled_ids: Mutex<HashSet<u64>>,
    config_keys: Mutex<Vec<String>>,
    invokes: AtomicU64,
    shut_down: AtomicBool,
    address: AtomicUsize,
}

// SAFETY: the host table is valid and immutable for the instance's whole life
// (the host keeps its adapter alive until after `shutdown`), and every field
// under the mutexes is `Send`. Nothing in the ABI enforces the first part, as
// round one found.
unsafe impl Send for InstanceState {}
unsafe impl Sync for InstanceState {}

impl InstanceState {
    fn api(&self) -> Option<&MetisHostApiV1> {
        if self.host.is_null() {
            return None;
        }
        // SAFETY: the host table outlives the instance and is immutable.
        Some(unsafe { &*self.host })
    }

    fn log(&self, level: i32, message: &str) {
        let Some(api) = self.api() else { return };
        let bytes = message.as_bytes();
        // SAFETY: the callback signature is the frozen ABI one.
        unsafe { (api.log)(api.user_data, level, bytes.as_ptr(), bytes.len()) };
    }

    fn emit_event(&self, topic: &str, payload: &Value) {
        let Some(api) = self.api() else { return };
        let body = serde_json::to_vec(payload).unwrap_or_default();
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        // SAFETY: the callback signature is the frozen ABI one, and the host
        // releases `buffer`.
        unsafe { (api.emit_event)(api.user_data, topic.as_ptr(), topic.len(), buffer) };
    }

    fn progress(&self, request_id: u64, payload: &Value) {
        let Some(api) = self.api() else { return };
        let body = serde_json::to_vec(payload).unwrap_or_default();
        let buffer = plugin_alloc(&metis_encode_payload(&body));
        // SAFETY: the callback signature is the frozen ABI one.
        unsafe { (api.progress)(api.user_data, request_id, buffer) };
    }

    /// Delivers a completion. Called only from a worker that `shutdown` joined.
    fn complete(&self, request_id: u64, status: MetisStatus, body: Option<&[u8]>) {
        let Some(api) = self.api() else { return };
        let buffer = match body {
            Some(body) => plugin_alloc(&metis_encode_payload(body)),
            None => MetisBuffer::empty(),
        };
        // SAFETY: the callback signature is the frozen ABI one.
        unsafe { (api.complete)(api.user_data, request_id, status, buffer) };
    }

    /// A completion from a thread `shutdown` does not join. Once shutdown has
    /// run, the completion is logged and dropped instead of touching state the
    /// instance no longer owns.
    fn complete_guarded(&self, request_id: u64, status: MetisStatus, body: Option<&[u8]>) {
        if self.shut_down.load(SeqCst) {
            SUPPRESSED.fetch_add(1, SeqCst);
            self.log(
                METIS_LOG_WARN,
                &format!(
                    "proto-plugin-cdylib: a callback for request {request_id} arrived after \
                     shutdown and was ignored"
                ),
            );
            return;
        }
        self.complete(request_id, status, body);
    }

    fn finish(&self, request_id: u64) {
        self.requests.lock().unwrap().remove(&request_id);
    }

    /// Swaps in a new executor and drains the old one outside the executor
    /// lock, so its workers join before this returns. Runs on the caller
    /// thread, never on an executor worker (that would drain the worker's own
    /// pool).
    fn set_executor(&self, mode: &str, workers: usize, idle_ms: u64) {
        let new: Arc<dyn Executor> = match mode {
            "thread" => Arc::new(ThreadPerCallExecutor::new()),
            "pool" | "" => {
                let idle = if idle_ms == 0 {
                    IdlePolicy::Stay
                } else {
                    IdlePolicy::Timeout(Duration::from_millis(idle_ms))
                };
                Arc::new(PoolExecutor::new(workers.max(1), idle))
            }
            other => {
                self.log(
                    METIS_LOG_WARN,
                    &format!("proto-plugin-cdylib: unknown executor mode {other:?}"),
                );
                Arc::new(PoolExecutor::new(workers.max(1), IdlePolicy::Stay))
            }
        };
        self.executor_workers.store(workers.max(1), SeqCst);
        let old = {
            let mut slot = self.executor.lock().unwrap();
            std::mem::replace(&mut *slot, Arc::clone(&new))
        };
        *self.executor_label.lock().unwrap() = mode.to_owned();
        let outcome = old.drain(Some(DRAIN_DEADLINE));
        self.log(
            METIS_LOG_INFO,
            &format!(
                "proto-plugin-cdylib: executor set to {mode} ({workers} workers); previous executor \
                 drained completed={} dropped={} still_running={} timed_out={}",
                outcome.completed, outcome.dropped, outcome.still_running, outcome.timed_out
            ),
        );
    }

    /// The current executor, cloned so the caller does not hold the lock while
    /// spawning or draining.
    fn executor(&self) -> Arc<dyn Executor> {
        Arc::clone(&self.executor.lock().unwrap())
    }
}

/// The default pool size and drain deadline the plugin uses unless it was
/// reconfigured. Four workers is enough for the prototype's eight-host-thread
/// benchmark to stay busy; the sizing question is discussed in the findings.
const DEFAULT_POOL_WORKERS: usize = 4;
const DRAIN_DEADLINE: Duration = Duration::from_secs(5);

/// The SDK default: a bounded pool, created once and reused. The host-polled
/// future seam never touches it.
fn default_executor() -> Arc<dyn Executor> {
    Arc::new(PoolExecutor::new(DEFAULT_POOL_WORKERS, IdlePolicy::Stay))
}

struct Instance {
    state: Arc<InstanceState>,
}

/// Instance handles this plugin has already collected, so a second release is
/// reported rather than becoming a double free, and a reused address is made
/// live again on the next create.
fn released_instances() -> &'static Mutex<HashSet<usize>> {
    static RELEASED: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
    RELEASED.get_or_init(|| Mutex::new(HashSet::new()))
}

// --- the job plan ----------------------------------------------------------

enum Job {
    Echo(String),
    Add(i64, i64),
    Uppercase(String),
    BadResponse,
    Complete(String),
    Delay {
        delay_ms: u64,
        tag: String,
        ignore_cancel: bool,
    },
    Never,
    Straggler {
        delay_ms: u64,
    },
    Stats,
    Events,
    Logs,
    Schemas(Vec<String>),
    DescriptorsInline,
    Configure(Value),
    Accounting,
    FutureStats,
    /// Switch the plugin's legacy executor (`pool` or `thread`). It runs inline
    /// in `invoke`, never on a worker: draining the old executor from one of
    /// its own workers would deadlock.
    SetExecutor {
        mode: String,
        workers: usize,
        idle_ms: u64,
    },
}

fn body_json(body: &[u8]) -> Result<Value, MetisStatus> {
    serde_json::from_slice(body).map_err(|_| MetisStatus::InvalidArgument)
}

fn decode<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, MetisStatus> {
    serde_json::from_slice(body).map_err(|_| MetisStatus::InvalidArgument)
}

fn plan(capability: &str, body: &[u8]) -> Result<Job, MetisStatus> {
    Ok(match capability {
        "tool.echo" => Job::Echo(decode::<EchoRequest>(body)?.message),
        "tool.add" => {
            let request: AddRequest = decode(body)?;
            Job::Add(request.a, request.b)
        }
        "tool.uppercase" => Job::Uppercase(decode::<UppercaseRequest>(body)?.text),
        "tool.bad_response" => {
            let _: BadResponseRequest = decode(body)?;
            Job::BadResponse
        }
        "provider.complete" => Job::Complete(decode::<CompleteRequest>(body)?.prompt),
        "tool.delay" => {
            let request: DelayRequest = decode(body)?;
            Job::Delay {
                delay_ms: request.delay_ms,
                tag: request.tag,
                ignore_cancel: false,
            }
        }
        "tool.races" => {
            let request: RaceRequest = decode(body)?;
            Job::Delay {
                delay_ms: request.delay_ms,
                tag: request.tag,
                ignore_cancel: true,
            }
        }
        "tool.never" => {
            let _: NeverRequest = decode(body)?;
            Job::Never
        }
        "tool.straggler" => Job::Straggler {
            delay_ms: decode::<StragglerRequest>(body)?.delay_ms,
        },
        "tool.stats" => {
            let _: StatsRequest = decode(body)?;
            Job::Stats
        }
        "tool.events" => Job::Events,
        "tool.logs" => Job::Logs,
        "tool.future" => {
            let request: FutureRequest = decode(body)?;
            Job::Echo(format!("future {}", request.x))
        }
        "metis.schemas" => {
            let value = body_json(body)?;
            let ids = value
                .get("ids")
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| id.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            Job::Schemas(ids)
        }
        "metis.descriptors_inline" => Job::DescriptorsInline,
        "metis.configure" => Job::Configure(body_json(body)?),
        "metis.accounting" => Job::Accounting,
        "metis.future_stats" => Job::FutureStats,
        "metis.executor" => {
            let value = body_json(body)?;
            Job::SetExecutor {
                mode: value
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or("pool")
                    .to_owned(),
                workers: value
                    .get("workers")
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_POOL_WORKERS as u64) as usize,
                idle_ms: value.get("idle_ms").and_then(Value::as_u64).unwrap_or(0),
            }
        }
        _ => return Err(MetisStatus::NotFound),
    })
}

fn run_job(state: &InstanceState, request_id: u64, job: Job, request: Arc<RequestState>) {
    match job {
        Job::Echo(message) => {
            let response = json!({ "echo": message, "length": message.len() as u64 });
            complete_json(state, request_id, &response);
        }
        Job::Add(a, b) => complete_json(state, request_id, &json!({ "sum": a + b })),
        Job::Uppercase(text) => {
            let upper = text.to_uppercase();
            complete_json(
                state,
                request_id,
                &json!({ "upper": upper, "bytes": upper.len() as u64 }),
            );
        }
        // Deliberately violates the published response schema: `value` is
        // published as a string and this writes a number.
        Job::BadResponse => complete_json(state, request_id, &json!({ "value": 42 })),
        Job::Complete(prompt) => {
            // Progress first, in order, then the completion. Each progress
            // payload is its own plugin buffer, which the host releases.
            state.progress(request_id, &json!({ "step": 1, "of": 2 }));
            state.progress(request_id, &json!({ "step": 2, "of": 2 }));
            state.log(METIS_LOG_INFO, "proto-plugin-cdylib: provider.complete is answering");
            complete_json(
                state,
                request_id,
                &json!({
                    "text": format!("{prompt} (len {})", prompt.len()),
                    "source": "proto-plugin-cdylib",
                }),
            );
        }
        Job::Delay {
            delay_ms,
            tag,
            ignore_cancel,
        } => {
            let deadline = Instant::now() + Duration::from_millis(delay_ms);
            let cancelled = loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break false;
                }
                if request.park_or_cancel(remaining) {
                    break true;
                }
            };
            if cancelled && !ignore_cancel {
                // The plugin gives up on a call the host cancelled: it logs and
                // suppresses its completion, and allocates nothing.
                SUPPRESSED.fetch_add(1, SeqCst);
                state.log(
                    METIS_LOG_INFO,
                    &format!("proto-plugin-cdylib: observed cancel for request {request_id}"),
                );
                state.finish(request_id);
                return;
            }
            state.log(
                METIS_LOG_INFO,
                &format!("proto-plugin-cdylib: request {request_id} completed after cancel"),
            );
            complete_json(state, request_id, &json!({ "tag": tag }));
        }
        Job::Never => {
            // Handled in `invoke`: no worker is spawned, so no completion is
            // ever delivered.
            unreachable!("a never-completing job is not run on a worker")
        }
        Job::Straggler { .. } => {
            // Handled in `invoke`: the detached thread is spawned there.
            unreachable!("a straggler job is not run on a tracked worker")
        }
        Job::Stats => {
            let keys = state.config_keys.lock().unwrap().clone();
            complete_json(
                state,
                request_id,
                &json!({
                    "instance": format!("0x{:x}", state.address.load(SeqCst)),
                    "invokes": state.invokes.load(SeqCst),
                    "config_keys": keys,
                }),
            );
        }
        Job::Events => {
            state.emit_event(
                "tool.events",
                &json!({ "request_id": request_id, "what": "an event from a capability" }),
            );
            complete_json(state, request_id, &json!({ "emitted": "tool.events" }));
        }
        Job::Logs => {
            state.log(METIS_LOG_DEBUG, "proto-plugin-cdylib: log at debug");
            state.log(METIS_LOG_INFO, "proto-plugin-cdylib: log at info");
            state.log(METIS_LOG_WARN, "proto-plugin-cdylib: log at warn");
            state.log(METIS_LOG_ERROR, "proto-plugin-cdylib: log at error");
            complete_json(state, request_id, &json!({ "levels": 4 }));
        }
        Job::Schemas(ids) => {
            let mut schemas = serde_json::Map::new();
            for id in ids {
                if let Some(document) = schema_document(&id) {
                    schemas.insert(id, document);
                }
            }
            complete_json(state, request_id, &json!({ "schemas": schemas }));
        }
        Job::DescriptorsInline => {
            let body = inline_descriptor_list_body();
            complete_bytes(state, request_id, &body);
        }
        Job::Configure(config) => {
            let keys: Vec<String> = config
                .as_object()
                .map(|object| object.keys().cloned().collect())
                .unwrap_or_default();
            *state.config_keys.lock().unwrap() = keys.clone();
            state.log(
                METIS_LOG_INFO,
                &format!("proto-plugin-cdylib: configured with keys {keys:?}"),
            );
            complete_json(
                state,
                request_id,
                &json!({ "accepted": true, "keys": keys }),
            );
        }
        Job::Accounting => complete_json(
            state,
            request_id,
            &json!({
                "allocs": ALLOCS.load(SeqCst),
                "frees": FREES.load(SeqCst),
                "live": live_buffers().lock().unwrap().len(),
                "violations": VIOLATIONS.load(SeqCst),
                "suppressed": SUPPRESSED.load(SeqCst),
            }),
        ),
        Job::FutureStats => complete_json(state, request_id, &future::stats_json()),
        Job::SetExecutor { .. } => {
            unreachable!("a SetExecutor job runs inline in `invoke`, so it never reaches a worker")
        }
    }
    state.finish(request_id);
}

fn complete_json(state: &InstanceState, request_id: u64, value: &Value) {
    let body = serde_json::to_vec(value).unwrap_or_default();
    state.complete(request_id, MetisStatus::Ok, Some(&body));
}

fn complete_bytes(state: &InstanceState, request_id: u64, body: &[u8]) {
    state.complete(request_id, MetisStatus::Ok, Some(body));
}

// --- exported table entries ------------------------------------------------

unsafe extern "C" fn create_instance(
    host: *const MetisHostApiV1,
    out_instance: *mut *mut c_void,
) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if host.is_null() || out_instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        let state = Arc::new(InstanceState {
            host,
            executor: Mutex::new(default_executor()),
            executor_label: Mutex::new("pool".to_owned()),
            executor_workers: AtomicUsize::new(DEFAULT_POOL_WORKERS),
            requests: Mutex::new(HashMap::new()),
            cancelled_ids: Mutex::new(HashSet::new()),
            config_keys: Mutex::new(Vec::new()),
            invokes: AtomicU64::new(0),
            shut_down: AtomicBool::new(false),
            address: AtomicUsize::new(0),
        });
        let instance = Box::new(Instance {
            state: Arc::clone(&state),
        });
        let raw = Box::into_raw(instance);
        let address = raw as usize;
        // The address is observable through `tool.stats`, so two instances can
        // be shown to be independent.
        state.address.store(address, SeqCst);
        released_instances().lock().unwrap().remove(&address);
        // An event from instance start, before any capability runs.
        state.emit_event(
            "plugin.started",
            &json!({ "plugin": "proto-plugin-cdylib", "instance": format!("0x{address:x}") }),
        );
        state.log(METIS_LOG_INFO, "proto-plugin-cdylib: instance created");
        // SAFETY: the host owns the out parameter and gets the only handle.
        unsafe { *out_instance = raw as *mut c_void };
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn destroy_instance(instance: *mut c_void) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        if !released_instances()
            .lock()
            .unwrap()
            .insert(instance as usize)
        {
            return MetisStatus::NotFound;
        }
        // SAFETY: the handle came from `create_instance`, and the insert above
        // makes this path run exactly once per handle.
        let state = unsafe { Box::from_raw(instance as *mut Instance) };
        state
            .state
            .log(METIS_LOG_INFO, &accounting_line());
        state
            .state
            .log(METIS_LOG_INFO, "proto-plugin-cdylib: instance released");
        drop(state);
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn list_capabilities(
    instance: *mut c_void,
    out_payload: *mut MetisBuffer,
) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() || out_payload.is_null() {
            return MetisStatus::InvalidArgument;
        }
        let body = descriptor_list_body();
        // SAFETY: the host owns the out parameter and releases the buffer.
        unsafe { *out_payload = plugin_alloc(&metis_encode_payload(&body)) };
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn invoke(
    instance: *mut c_void,
    capability: *const u8,
    capability_len: usize,
    request_id: u64,
    payload: MetisBuffer,
    _host: *const MetisHostApiV1,
    out_payload: *mut MetisBuffer,
) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null()
            || capability.is_null()
            || out_payload.is_null()
            || payload.owner != METIS_BUFFER_OWNER_HOST
        {
            return MetisStatus::InvalidArgument;
        }
        // SAFETY: the host passes a UTF-8 capability name for the call.
        let capability =
            match std::str::from_utf8(unsafe { slice::from_raw_parts(capability, capability_len) }) {
                Ok(name) => name,
                Err(_) => return MetisStatus::InvalidArgument,
            };
        // SAFETY: the host owns the buffer and keeps it alive for the call.
        let raw = unsafe { slice::from_raw_parts(payload.ptr, payload.len) };
        let body = match metis_payload_body(raw) {
            Ok(body) => body,
            Err(status) => return status,
        };
        let job = match plan(capability, body) {
            Ok(job) => job,
            Err(status) => return status,
        };

        // SAFETY: the handle is the live instance for this call.
        let instance_ref = unsafe { &*(instance as *const Instance) };
        let state = Arc::clone(&instance_ref.state);
        state.invokes.fetch_add(1, SeqCst);

        // The executor switch is protocol, not work: it runs inline on the
        // caller thread so draining the old executor cannot deadlock on a
        // worker that is draining its own pool.
        let job = match job {
            Job::SetExecutor {
                mode,
                workers,
                idle_ms,
            } => {
                state.set_executor(&mode, workers, idle_ms);
                complete_json(
                    &state,
                    request_id,
                    &json!({ "mode": mode, "workers": workers, "idle_ms": idle_ms }),
                );
                state.finish(request_id);
                return MetisStatus::Pending;
            }
            other => other,
        };

        if let Job::Never = job {
            // A call that will never complete still gets a pending record, so a
            // cancel can reach it, but no worker is spawned.
            let request = Arc::new(RequestState::new());
            state
                .requests
                .lock()
                .unwrap()
                .insert(request_id, request);
            state.log(
                METIS_LOG_WARN,
                &format!("proto-plugin-cdylib: request {request_id} will never complete"),
            );
            return MetisStatus::Pending;
        }

        let request = Arc::new(RequestState::new());
        state
            .requests
            .lock()
            .unwrap()
            .insert(request_id, Arc::clone(&request));

        if let Job::Straggler { delay_ms } = job {
            // Detached on purpose: `shutdown` must not be able to join it, so
            // it is the callback-after-shutdown hazard.
            let state = Arc::clone(&state);
            future::note_thread_spawned();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(delay_ms));
                state.complete_guarded(
                    request_id,
                    MetisStatus::Ok,
                    Some(br#"{"tag":"straggler"}"#),
                );
                state.finish(request_id);
            });
            return MetisStatus::Pending;
        }

        // The legacy path runs through the SDK-injected executor: the default
        // bounded pool, or the thread-per-call fallback, switchable at runtime
        // through `metis.executor`. The host-polled future seam never uses it.
        let worker_state = Arc::clone(&state);
        future::note_thread_spawned();
        let task: Task = Box::pin(async move {
            // A worker is a boundary: a panic becomes an internal-error
            // completion instead of a worker that dies without answering.
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                run_job(&worker_state, request_id, job, request);
            }));
            if outcome.is_err() {
                worker_state.complete(request_id, MetisStatus::Internal, None);
                worker_state.finish(request_id);
            }
        });
        match state.executor().spawn(task) {
            Ok(()) => MetisStatus::Pending,
            Err(SpawnRefused::Draining) => {
                // The executor refused the task, so it will never run. Retire
                // the request record and tell the host, which retires its slot.
                state.finish(request_id);
                state.log(
                    METIS_LOG_WARN,
                    &format!(
                        "proto-plugin-cdylib: executor refused a call to {capability} while draining"
                    ),
                );
                MetisStatus::Cancelled
            }
        }
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn cancel(instance: *mut c_void, request_id: u64) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        // SAFETY: the handle is the live instance for this call.
        let state = unsafe { &*(*(instance as *const Instance)).state };
        if let Some(request) = state.requests.lock().unwrap().get(&request_id).cloned() {
            request.mark_cancelled();
            state.cancelled_ids.lock().unwrap().insert(request_id);
            state.log(
                METIS_LOG_INFO,
                &format!("proto-plugin-cdylib: cancel accepted for request {request_id}"),
            );
            return MetisStatus::Ok;
        }
        // The host-polled future seam keeps its own registry; a cancel for a
        // live future resolves it with `Cancelled` and wakes its host task.
        if future::cancel(request_id) {
            state.log(
                METIS_LOG_INFO,
                &format!("proto-plugin-cdylib: future {request_id} resolved as cancelled"),
            );
            return MetisStatus::Ok;
        }
        // Idempotent: a second cancel for the same, already cancelled request is
        // accepted rather than reported as missing.
        if state.cancelled_ids.lock().unwrap().contains(&request_id) {
            return MetisStatus::Ok;
        }
        MetisStatus::NotFound
    }));
    result.unwrap_or(MetisStatus::Internal)
}

unsafe extern "C" fn shutdown(instance: *mut c_void) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if instance.is_null() {
            return MetisStatus::InvalidArgument;
        }
        if released_instances()
            .lock()
            .unwrap()
            .contains(&(instance as usize))
        {
            return MetisStatus::NotFound;
        }
        // SAFETY: the handle is live, and the check above keeps this once.
        let state = unsafe { &*(*(instance as *const Instance)).state };
        state.shut_down.store(true, SeqCst);
        // Quiescence, in order: refuse new calls (the executor's drain does
        // that), then drain it with a deadline. A deadline cannot preempt a
        // task already inside plugin code, so an expired deadline escalates to
        // an unbounded join before the host may unload the library.
        state.log(
            METIS_LOG_INFO,
            "proto-plugin-cdylib: shutdown started; refusing new calls",
        );
        let executor = state.executor();
        let started = Instant::now();
        let outcome = executor.drain(Some(DRAIN_DEADLINE));
        state.log(
            METIS_LOG_INFO,
            &format!(
                "proto-plugin-cdylib: drain finished in {}us completed={} dropped={} \
                 still_running={} timed_out={}",
                started.elapsed().as_micros(),
                outcome.completed,
                outcome.dropped,
                outcome.still_running,
                outcome.timed_out,
            ),
        );
        if outcome.still_running > 0 {
            state.log(
                METIS_LOG_WARN,
                "proto-plugin-cdylib: the drain deadline expired with tasks still running, so \
                 they are joined before unload",
            );
            let _ = executor.drain(None);
        }
        state.log(
            METIS_LOG_INFO,
            &format!(
                "proto-plugin-cdylib: shutdown joined {} executor task(s)",
                outcome.completed
            ),
        );
        state.log(METIS_LOG_INFO, &accounting_line());
        // SAFETY: as above; shutdown collects the handle it was given, so a
        // later `destroy_instance` reports it as not found.
        unsafe { destroy_instance(instance) }
    }));
    result.unwrap_or(MetisStatus::Internal)
}

// --- the one exported symbol -----------------------------------------------

static TABLE_V1: MetisPluginTableV1 = MetisPluginTableV1 {
    size: std::mem::size_of::<MetisPluginTableV1>(),
    abi_major: METIS_ABI_MAJOR,
    abi_minor: METIS_ABI_MINOR,
    create_instance: create_instance as MetisCreateInstanceFn,
    destroy_instance: destroy_instance as MetisDestroyInstanceFn,
    list_capabilities: list_capabilities as MetisListCapabilitiesFn,
    invoke,
    cancel,
    release_buffer,
    shutdown,
    invoke_future: future::invoke_future,
    service_complete: future::service_complete,
};

#[unsafe(no_mangle)]
pub extern "C" fn metis_plugin_entry_v1() -> *const MetisPluginTableV1 {
    &raw const TABLE_V1
}
