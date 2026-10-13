//! Lane 3: prefix compatibility and append-only versioning, driven through the
//! table.
//!
//! Every check here reads the plugin's table the way a real host would, at the
//! boundary, and every number comes from the crate (`proto-abi`) rather than
//! from a literal, so the checks cannot silently drift from the header.
//!
//! The directions proven:
//!
//! - an older host, which knows only the landed prefix, still drives the newer
//!   plugin through `invoke` plus `Pending` and the `complete` callback;
//! - a newer host sees the appended entries, and the plugin table's `size`
//!   covers them;
//! - an older plugin, whose table stops at the landed tail, is still driven;
//! - a table that does not cover the prefix is refused rather than read.

use std::ffi::c_void;
use std::path::Path;
use std::ptr;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use plugin_contract::abi::{
    METIS_ABI_MAJOR, MetisPluginEntryV1Fn, MetisPluginTableV1 as LandedTable, MetisStatus,
    metis_read_prefix,
};
use plugin_contract::authoring::HostAdapter;
use plugin_contract::payload::{metis_encode_payload, metis_payload_body};
use proto_abi::{
    METIS_BUFFER_OWNER_HOST, METIS_HOST_API_V1_LANDED_SIZE, METIS_PLUGIN_TABLE_V1_LANDED_SIZE,
    MetisBuffer, MetisFutureHandle, MetisPluginTableV1 as CurrentTable, MetisStatus as CurrentStatus,
    MetisTablePrefix, MetisWaker, MetisWakerVtable, metis_host_api_has_service_request,
    metis_plugin_table_has_invoke_future, metis_plugin_table_has_service_complete,
};

/// One line of evidence, printed by the host binary.
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

fn check(name: &str, ok: bool, detail: String) -> Check {
    Check {
        name: name.to_owned(),
        ok,
        detail,
    }
}

/// A loaded cdylib whose table stays mapped for as long as the value lives.
struct Loaded {
    _library: libloading::Library,
    table: *const CurrentTable,
}

impl Loaded {
    fn open(path: &Path) -> Result<Loaded, String> {
        // SAFETY: loading a library runs its initialisers; the caller vets the
        // path, which is the cdylib the workspace just built.
        let library = unsafe { libloading::Library::new(path) }
            .map_err(|err| format!("could not load {}: {err}", path.display()))?;
        let table = {
            // SAFETY: the symbol name is the frozen v1 entry point.
            let entry: libloading::Symbol<MetisPluginEntryV1Fn> =
                unsafe { library.get(plugin_contract::abi::METIS_PLUGIN_ENTRY_V1) }
                    .map_err(|err| format!("no metis_plugin_entry_v1 symbol: {err}"))?;
            // SAFETY: the entry point returns a live table with the landed
            // prefix at its start; the pointer is read as the current table.
            unsafe { entry() as *const CurrentTable }
        };
        Ok(Loaded {
            _library: library,
            table,
        })
    }
}

/// Drives one `tool.echo` call through a table that stops at the landed tail.
///
/// The table is a copy of the real one's first seven entries, the shape an
/// older plugin build would export. The landed `HostAdapter` is the caller, so
/// this is the older-plugin direction from the host side.
fn drive_landed_table(landed: &LandedTable) -> Result<String, String> {
    let adapter = HostAdapter::new();
    adapter.attach(landed as *const LandedTable);

    let mut instance: *mut c_void = ptr::null_mut();
    // SAFETY: the table is live for this call and the out parameter is this
    // frame's. The host table the adapter installs is the landed one.
    let status = unsafe { (landed.create_instance)(adapter.api(), &mut instance) };
    if status != MetisStatus::Ok {
        return Err(format!("create_instance returned {}", status.as_str()));
    }

    let request_id = 0x5eed_0001_u64;
    let awaiting = adapter.register(request_id);
    let body = br#"{"message":"short-table"}"#;
    // SAFETY: the instance came from this table and the adapter registered the
    // request before the call.
    let invoke_status = unsafe { adapter.invoke(instance, "tool.echo", request_id, body) };
    // SAFETY: the instance is live and this is the only destroy.
    let destroy_status = unsafe { (landed.destroy_instance)(instance) };
    if invoke_status != MetisStatus::Pending {
        return Err(format!(
            "invoke returned {} rather than Pending",
            invoke_status.as_str()
        ));
    }
    if destroy_status != MetisStatus::Ok {
        return Err(format!(
            "destroy_instance returned {}",
            destroy_status.as_str()
        ));
    }
    let completed = awaiting
        .wait_timeout(Duration::from_secs(5))
        .ok_or("no completion arrived for the older table")?;
    if completed.status != MetisStatus::Ok {
        return Err(format!(
            "the completion carried {}",
            completed.status.as_str()
        ));
    }
    let parsed: serde_json::Value = serde_json::from_slice(&completed.body)
        .map_err(|err| format!("the completion body did not parse: {err}"))?;
    Ok(parsed["echo"].as_str().unwrap_or_default().to_owned())
}

/// Runs every lane 3 versioning check against the built cdylib.
pub fn run(path: &Path) -> Result<Vec<Check>, String> {
    let loaded = Loaded::open(path)?;

    // SAFETY: the table comes from the entry point and stays mapped for as long
    // as `loaded`.
    let prefix = unsafe { metis_read_prefix(loaded.table as *const LandedTable) }
        .ok_or("the plugin table does not cover the size and version prefix")?;
    let current_size = std::mem::size_of::<CurrentTable>();

    let mut checks = Vec::new();

    // 1. An older host reads the landed prefix and sees a table that grew.
    checks.push(check(
        "versioning: a landed-prefix reader reads size and major, and sees the table grew",
        prefix.abi_major == METIS_ABI_MAJOR
            && prefix.size == current_size
            && prefix.size > METIS_PLUGIN_TABLE_V1_LANDED_SIZE,
        format!(
            "abi_major={} abi_minor={} size={} landed={} current={}",
            prefix.abi_major, prefix.abi_minor, prefix.size, METIS_PLUGIN_TABLE_V1_LANDED_SIZE,
            current_size
        ),
    ));

    // 2. A newer host sees the appended plugin entries in the size and in the
    //    slots.
    // SAFETY: `prefix.size` covers the appended entries, checked here first.
    let (has_future, has_service) = if prefix.size >= current_size {
        let table = unsafe { &*loaded.table };
        (
            table.invoke_future as usize != 0,
            table.service_complete as usize != 0,
        )
    } else {
        (false, false)
    };
    checks.push(check(
        "versioning: a newer host sees invoke_future and service_complete, decided by size",
        metis_plugin_table_has_invoke_future(prefix.size)
            && metis_plugin_table_has_service_complete(prefix.size)
            && has_future
            && has_service,
        format!(
            "invoke_future={} service_complete={} size={current_size}",
            has_future, has_service
        ),
    ));

    // 3. An older host still drives the newer plugin: the landed prefix plus
    //    invoke, Pending and complete. The landed adapter reads only the prefix
    //    it knows, and the plugin's appended entries never interfere.
    checks.push({
        let adapter = HostAdapter::new();
        adapter.attach(loaded.table as *const LandedTable);
        let mut instance: *mut c_void = ptr::null_mut();
        // SAFETY: the table is live and the out parameter is this frame's.
        let created = unsafe {
            ((*(loaded.table as *const LandedTable)).create_instance)(adapter.api(), &mut instance)
        };
        let request_id = 0x5eed_0002_u64;
        let awaiting = adapter.register(request_id);
        let body = br#"{"message":"old-host-new-plugin"}"#;
        // SAFETY: as above; the request was registered before the call.
        let invoked = unsafe { adapter.invoke(instance, "tool.echo", request_id, body) };
        let completed = awaiting.wait_timeout(Duration::from_secs(5));
        // SAFETY: the instance is live and this is the only destroy.
        let destroyed = unsafe {
            ((*(loaded.table as *const LandedTable)).destroy_instance)(instance)
        };
        let echoed = completed
            .as_ref()
            .and_then(|c| serde_json::from_slice::<serde_json::Value>(&c.body).ok())
            .and_then(|v| v["echo"].as_str().map(str::to_owned));
        check(
            "versioning: an older host drives the newer plugin through invoke, Pending and complete",
            created == MetisStatus::Ok
                && invoked == MetisStatus::Pending
                && destroyed == MetisStatus::Ok
                && echoed.as_deref() == Some("old-host-new-plugin"),
            format!(
                "create={} invoke={} destroy={} echo={echoed:?}",
                created.as_str(),
                invoked.as_str(),
                destroyed.as_str()
            ),
        )
    });

    // 4. An older plugin, whose table stops at the landed tail, is still
    //    driven. The copy is the real table's first seven entries with `size`
    //    set to the landed size; a host that respects `size` treats the
    //    appended entries as absent.
    let landed: LandedTable = {
        // SAFETY: the first seven entries and the prefix of the current table
        // have the landed layout, so reading the landed prefix of the mapping
        // is in bounds.
        let copy = unsafe { ptr::read(loaded.table as *const LandedTable) };
        LandedTable {
            size: std::mem::size_of::<LandedTable>(),
            ..copy
        }
    };
    let older_absent = !metis_plugin_table_has_invoke_future(landed.size);
    let echoed = drive_landed_table(&landed);
    checks.push(check(
        "versioning: an older plugin with a shorter table is still driven",
        older_absent && echoed.as_deref() == Ok("short-table"),
        format!("appended_absent={older_absent} echo={echoed:?}"),
    ));

    // 5. A table below the prefix is refused, and the decision is made from
    //    `size` alone: a table that stops exactly at `abi_major` is refused
    //    even though the bytes at `abi_minor`'s offset would be readable.
    let too_short = MetisTablePrefix {
        size: std::mem::size_of::<usize>(),
        abi_major: METIS_ABI_MAJOR,
        abi_minor: 0,
    };
    let short_refused = unsafe { metis_read_prefix(&too_short as *const MetisTablePrefix as *const LandedTable) }
        .is_none();
    // The presence helpers use the same rule, so an appended slot is never
    // reached on a table that does not cover it.
    let slot_rule = !metis_plugin_table_has_invoke_future(METIS_PLUGIN_TABLE_V1_LANDED_SIZE - 1)
        && !metis_plugin_table_has_service_complete(METIS_PLUGIN_TABLE_V1_LANDED_SIZE);
    checks.push(check(
        "versioning: a table that does not cover the prefix is refused, and slots are decided by size",
        short_refused && slot_rule,
        format!("short_refused={short_refused} slot_rule={slot_rule}"),
    ));

    // 6. The appended host entry is absent from a landed host table, by size, so
    //    the plugin can tell an old host from a new one without touching a
    //    pointer the host never wrote.
    let adapter = HostAdapter::new();
    // SAFETY: the adapter's table is live and the size field is readable.
    let host_size = unsafe { (*adapter.api()).size };
    checks.push(check(
        "versioning: a landed host table reports service_request absent, so the plugin cannot call it",
        host_size == METIS_HOST_API_V1_LANDED_SIZE
            && !metis_host_api_has_service_request(host_size),
        format!("host_size={host_size} landed={METIS_HOST_API_V1_LANDED_SIZE}"),
    ));

    // 7. A newer host drives one future end to end: invoke_future, a waker the
    //    host supplies, poll until Ok, then drop.
    checks.push(drive_one_future(&loaded));

    Ok(checks)
}

// --- one host-polled future, driven end to end ----------------------------

/// The host's waker body. The plugin clones it, wakes it from its worker, and
/// the host waits on the same state.
struct PollWaker {
    woken: Mutex<bool>,
    condvar: Condvar,
    wakes: AtomicUsize,
    clones: AtomicUsize,
}

impl PollWaker {
    fn signal(&self) {
        self.wakes.fetch_add(1, SeqCst);
        *self.woken.lock().unwrap() = true;
        self.condvar.notify_all();
    }

    fn wait(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let mut woken = self.woken.lock().unwrap();
        while !*woken {
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            woken = self.condvar.wait_timeout(woken, deadline - now).unwrap().0;
        }
        true
    }
}

// SAFETY: the waker bodies are the boundary, so each one is panic-guarded the
// same way every other callback is.
unsafe extern "C" fn waker_clone(waker: *const MetisWaker) -> MetisWaker {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if waker.is_null() {
            return MetisWaker {
                data: ptr::null_mut(),
                vtable: ptr::null(),
            };
        }
        // SAFETY: the waker data is an `Arc<PollWaker>` raw pointer.
        let source = unsafe { &*waker };
        unsafe { Arc::increment_strong_count(source.data as *const PollWaker) };
        // SAFETY: the increment above makes this the second owner.
        let cloned = unsafe { Arc::from_raw(source.data as *const PollWaker) };
        (cloned.clones).fetch_add(1, SeqCst);
        MetisWaker {
            data: Arc::into_raw(cloned) as *mut c_void,
            vtable: source.vtable,
        }
    }));
    result.unwrap_or(MetisWaker {
        data: ptr::null_mut(),
        vtable: ptr::null(),
    })
}

unsafe extern "C" fn waker_wake(waker: MetisWaker) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if waker.data.is_null() {
            return;
        }
        // SAFETY: consuming the owner `clone` returned.
        let state = unsafe { Arc::from_raw(waker.data as *const PollWaker) };
        state.signal();
        drop(state);
    }));
}

unsafe extern "C" fn waker_wake_by_ref(waker: *const MetisWaker) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if waker.is_null() {
            return;
        }
        // SAFETY: a borrowed waker; the increment and the `from_raw` cancel out.
        let source = unsafe { &*waker };
        unsafe { Arc::increment_strong_count(source.data as *const PollWaker) };
        // SAFETY: the increment above makes this a valid one-shot owner.
        let state = unsafe { Arc::from_raw(source.data as *const PollWaker) };
        state.signal();
        drop(state);
    }));
}

unsafe extern "C" fn waker_drop(waker: MetisWaker) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if waker.data.is_null() {
            return;
        }
        // SAFETY: consuming the owner `clone` returned.
        drop(unsafe { Arc::from_raw(waker.data as *const PollWaker) });
    }));
}

static WAKER_VTABLE: MetisWakerVtable = MetisWakerVtable {
    size: std::mem::size_of::<MetisWakerVtable>(),
    clone: waker_clone,
    wake: waker_wake,
    wake_by_ref: waker_wake_by_ref,
    drop: waker_drop,
};

fn drive_one_future(loaded: &Loaded) -> Check {
    let name = "versioning: a newer host drives one appended future through poll, waker and drop";
    let adapter = HostAdapter::new();
    adapter.attach(loaded.table as *const LandedTable);
    let mut instance: *mut c_void = ptr::null_mut();
    // SAFETY: the table is live and the out parameter is this frame's.
    let created = unsafe {
        ((*(loaded.table as *const LandedTable)).create_instance)(adapter.api(), &mut instance)
    };
    if created != MetisStatus::Ok {
        return check(name, false, format!("create_instance returned {}", created.as_str()));
    }

    let state = Arc::new(PollWaker {
        woken: Mutex::new(false),
        condvar: Condvar::new(),
        wakes: AtomicUsize::new(0),
        clones: AtomicUsize::new(0),
    });
    let waker_data = Arc::into_raw(Arc::clone(&state)) as *mut c_void;
    let waker_vtable: *const MetisWakerVtable = &raw const WAKER_VTABLE;

    let body = metis_encode_payload(br#"{"probe":true}"#);
    let payload = MetisBuffer {
        ptr: body.as_ptr() as *mut u8,
        len: body.len(),
        cap: 0,
        owner: METIS_BUFFER_OWNER_HOST,
        reserved: 0,
    };
    let mut handle = MetisFutureHandle {
        ctx: ptr::null_mut(),
        vtable: ptr::null(),
    };
    // SAFETY: the table is live, the instance is live, and `handle` and
    // `payload` are this frame's.
    let started = unsafe {
        ((*loaded.table).invoke_future)(
            instance,
            b"metis.future_probe".as_ptr(),
            "metis.future_probe".len(),
            0x5eed_0003_u64,
            payload,
            adapter.api() as *const proto_abi::MetisHostApiV1,
            &mut handle,
        )
    };

    let mut polls = 0usize;
    let mut response: Option<Vec<u8>> = None;
    let mut failure = None;
    if started == CurrentStatus::Ok && !handle.ctx.is_null() {
        for _ in 0..8 {
            let mut value = MetisBuffer::empty();
            let borrowed = MetisWaker {
                data: waker_data,
                vtable: waker_vtable,
            };
            // SAFETY: the handle is live until `drop` below, and the waker is
            // valid for the same lifetime.
            let polled = unsafe { ((*handle.vtable).poll)(handle.ctx, borrowed, &mut value) };
            polls += 1;
            match polled {
                CurrentStatus::Ok => {
                    // SAFETY: the buffer is plugin-owned; release it through the
                    // plugin's own `release_buffer`.
                    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) }.to_vec();
                    let released = unsafe { ((*loaded.table).release_buffer)(value) };
                    response = Some(bytes);
                    if released != CurrentStatus::Ok {
                        failure = Some(format!("release_buffer returned {}", released.as_str()));
                    }
                    break;
                }
                CurrentStatus::Pending => {
                    if !state.wait(Duration::from_secs(5)) {
                        failure = Some("the waker was never woken".to_owned());
                        break;
                    }
                }
                other => {
                    failure = Some(format!("poll returned {}", other.as_str()));
                    break;
                }
            }
        }
        if response.is_none() && failure.is_none() {
            failure = Some(format!("the future never became ready in {polls} polls"));
        }
    } else {
        failure = Some(format!("invoke_future returned {}", started.as_str()));
    }

    // SAFETY: the handle was filled by `invoke_future`; `drop` is called once.
    let dropped = if handle.ctx.is_null() {
        CurrentStatus::InvalidArgument
    } else {
        unsafe { ((*handle.vtable).drop)(handle.ctx) }
    };
    // The host's own waker is released last, by the vtable's own `drop`.
    unsafe {
        ((*waker_vtable).drop)(MetisWaker {
            data: waker_data,
            vtable: waker_vtable,
        })
    };
    // SAFETY: the instance is live and this is the only destroy.
    let destroyed = unsafe {
        ((*(loaded.table as *const LandedTable)).destroy_instance)(instance)
    };

    let parsed = response.as_ref().and_then(|bytes| {
        let body = metis_payload_body(bytes).ok()?;
        serde_json::from_slice::<serde_json::Value>(body).ok()
    });
    let ok = created == MetisStatus::Ok
        && started == CurrentStatus::Ok
        && failure.is_none()
        && polls >= 2
        && state.clones.load(SeqCst) >= 1
        && state.wakes.load(SeqCst) >= 1
        && dropped == CurrentStatus::Ok
        && destroyed == MetisStatus::Ok
        && parsed
            .as_ref()
            .map(|v| v["echo"] == "metis.future_probe")
            .unwrap_or(false);
    check(
        name,
        ok,
        format!(
            "invoke_future={} polls={polls} clones={} wakes={} drop={} destroy={} failure={failure:?} response={parsed:?}",
            started.as_str(),
            state.clones.load(SeqCst),
            state.wakes.load(SeqCst),
            dropped.as_str(),
            destroyed.as_str()
        ),
    )
}
