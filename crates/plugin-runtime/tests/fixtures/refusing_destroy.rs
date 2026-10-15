//! A test-only library that refuses to destroy its instance.
//!
//! The host's recovery from a failed initialisation ends in `destroy_instance`,
//! and no library in this tree refuses that entry: the fixture's adapter always
//! destroys the instance it was handed, and the statuses it answers for a
//! refusal, a null handle and a doubled release are not reachable from a load.
//! The outcome that stops start-up therefore cannot be produced with a library
//! this tree wrote, so the test compiles this one, which does nothing but
//! refuse.
//!
//! This file is not compiled as part of any target. `tests/support/mod.rs` reads
//! it, compiles it with `rustc` as a cdylib and hands the test its path, exactly
//! as the manifest tests compile `fixtures/patched_table.rs`.
//!
//! The shapes below are this file's own reading of the contract rather than a
//! use of any type in this tree, which is the point: the numbers must not come
//! from the reader. Every entry the host calls is implemented, because the host
//! binds them all, and only the three the load actually reaches do anything:
//! `init_plugin` hands out an instance and a future, the future's first poll
//! resolves the initialisation as a terminal failure, and `destroy_instance`
//! refuses. The offsets are asserted at the bottom, so a table this file
//! restated wrongly fails to compile rather than failing at run time.

#![allow(dead_code, private_interfaces)]

use core::ffi::c_void;

/// The status space, as the members this library answers with.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok = 0,
    Internal = 5,
}

/// A byte buffer crossing the boundary. This library never reads or allocates
/// one: it accepts the initialisation payload and ignores it.
#[repr(C)]
#[derive(Clone, Copy)]
struct Buffer {
    ptr: *mut u8,
    len: usize,
    cap: usize,
    owner: u32,
    reserved: u32,
}

/// A host waker, by value. This library never clones, wakes or drops one, so it
/// only has to have the same size and shape as the host's.
#[repr(C)]
struct Waker {
    data: *mut c_void,
    vtable: *const c_void,
}

/// Polls a future's context, and releases it exactly once.
type PollFn = unsafe extern "C" fn(*mut c_void, Waker, *mut Buffer) -> Status;
type DropFn = unsafe extern "C" fn(*mut c_void) -> Status;

#[repr(C)]
struct FutureVtable {
    poll: PollFn,
    drop: DropFn,
}

/// A plugin-owned future: an opaque context plus the vtable it is polled
/// through.
#[repr(C)]
#[derive(Clone, Copy)]
struct FutureHandle {
    ctx: *mut c_void,
    vtable: *const FutureVtable,
}

type InitPluginFn =
    unsafe extern "C" fn(*const c_void, Buffer, *mut *mut c_void, *mut FutureHandle) -> Status;
type DestroyInstanceFn = unsafe extern "C" fn(*mut c_void) -> Status;
type ListCapabilitiesFn = unsafe extern "C" fn(*mut c_void, *mut Buffer) -> Status;
type InvokeFutureFn = unsafe extern "C" fn(
    *mut c_void,
    *const u8,
    usize,
    u64,
    Buffer,
    *const c_void,
    *mut FutureHandle,
) -> Status;
type CancelFn = unsafe extern "C" fn(*mut c_void, u64) -> Status;
type ReleaseBufferFn = unsafe extern "C" fn(Buffer) -> Status;
type ShutdownFn = unsafe extern "C" fn(*mut c_void) -> Status;
type ServiceCompleteFn = unsafe extern "C" fn(*mut c_void, u64, Status, Buffer) -> Status;

/// The plugin table: the prefix, then the eight entries in slot order.
#[repr(C)]
struct Table {
    size: usize,
    abi_major: u32,
    abi_minor: u32,
    init_plugin: InitPluginFn,
    destroy_instance: DestroyInstanceFn,
    list_capabilities: ListCapabilitiesFn,
    invoke_future: InvokeFutureFn,
    cancel: CancelFn,
    release_buffer: ReleaseBufferFn,
    shutdown: ShutdownFn,
    service_complete: ServiceCompleteFn,
}

/// The instance this library hands out.
///
/// It is a static byte and never dereferenced: the host only passes the pointer
/// back to `destroy_instance`, which refuses without reading it.
static INSTANCE: u8 = 0;

/// The one future this library hands out: it resolves on its first poll.
static FUTURE: FutureVtable = FutureVtable {
    poll: poll,
    drop: future_drop,
};

/// The initialisation entry: it accepts the payload, hands out an instance and a
/// future, and reports `ok`, so the host drives the future and reaches the
/// recovery path when that future fails.
unsafe extern "C" fn init_plugin(
    _host: *const c_void,
    _payload: Buffer,
    out_instance: *mut *mut c_void,
    out_future: *mut FutureHandle,
) -> Status {
    if out_instance.is_null() || out_future.is_null() {
        return Status::Internal;
    }
    // SAFETY: the host owns both out parameters and they are checked above.
    unsafe {
        *out_instance = &raw const INSTANCE as *mut c_void;
        *out_future = FutureHandle {
            ctx: core::ptr::null_mut(),
            vtable: &raw const FUTURE,
        };
    }
    Status::Ok
}

/// The future's first poll: the initialisation is terminal here, which is the
/// failure the host has to recover from.
unsafe extern "C" fn poll(_ctx: *mut c_void, _waker: Waker, _out: *mut Buffer) -> Status {
    Status::Internal
}

/// The future's release, which this library accepts.
unsafe extern "C" fn future_drop(_ctx: *mut c_void) -> Status {
    Status::Ok
}

/// The refusal this library exists for: the instance it handed out is not
/// destroyed, which is what stops start-up.
unsafe extern "C" fn destroy_instance(_instance: *mut c_void) -> Status {
    Status::Internal
}

/// The entries below are never reached by a load that fails initialisation. They
/// answer a terminal status rather than misbehaving quietly.
unsafe extern "C" fn list_capabilities(_instance: *mut c_void, _out: *mut Buffer) -> Status {
    Status::Internal
}

unsafe extern "C" fn invoke_future(
    _instance: *mut c_void,
    _capability: *const u8,
    _capability_len: usize,
    _request_id: u64,
    _payload: Buffer,
    _host: *const c_void,
    _out: *mut FutureHandle,
) -> Status {
    Status::Internal
}

unsafe extern "C" fn cancel(_instance: *mut c_void, _request_id: u64) -> Status {
    Status::Internal
}

unsafe extern "C" fn release_buffer(_buffer: Buffer) -> Status {
    Status::Internal
}

unsafe extern "C" fn shutdown(_instance: *mut c_void) -> Status {
    Status::Internal
}

unsafe extern "C" fn service_complete(
    _instance: *mut c_void,
    _request_id: u64,
    _status: Status,
    _payload: Buffer,
) -> Status {
    Status::Internal
}

/// The table this library exports, with the size this build's layout has.
static TABLE: Table = Table {
    size: core::mem::size_of::<Table>(),
    abi_major: 1,
    abi_minor: 2,
    init_plugin: init_plugin,
    destroy_instance: destroy_instance,
    list_capabilities: list_capabilities,
    invoke_future: invoke_future,
    cancel: cancel,
    release_buffer: release_buffer,
    shutdown: shutdown,
    service_complete: service_complete,
};

/// The one versioned entry symbol, for ABI major 1.
#[unsafe(no_mangle)]
pub extern "C" fn metis_plugin_entry_v1() -> *const Table {
    &raw const TABLE
}

/// The prefix is a size and two version fields, and the entries follow it in
/// slot order. A table this file laid out differently fails to compile here.
const _: () = {
    assert!(core::mem::offset_of!(Table, size) == 0);
    assert!(core::mem::offset_of!(Table, abi_major) == core::mem::size_of::<usize>());
    assert!(
        core::mem::offset_of!(Table, abi_minor) == core::mem::size_of::<usize>() + 4
    );
    assert!(core::mem::offset_of!(Table, init_plugin) == 16);
    assert!(core::mem::offset_of!(Table, destroy_instance) == 24);
    assert!(core::mem::offset_of!(Table, service_complete) == 16 + 7 * core::mem::size_of::<usize>());
};
