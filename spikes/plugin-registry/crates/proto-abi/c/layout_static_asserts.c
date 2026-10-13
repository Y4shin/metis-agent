/*
 * Static layout assertions for the generated ABI header (lane 3).
 *
 * This translation unit is compiled by `crates/proto-abi/build.rs` through the
 * `cc` crate on every build, so any build of the crate fails when the committed
 * header stops agreeing with the pointer-size formulas in
 * `crates/proto-abi/src/layout.rs`.
 *
 * Every number below is written in terms of `sizeof(void *)`, `sizeof(size_t)`
 * and `sizeof(uint32_t)` rather than as a 64-bit literal, so the assertions
 * hold on a 32-bit target too. At the bottom the same numbers are exported as
 * `metis_c_*` symbols, which `tests/layout.rs` compares with Rust's `size_of`,
 * `align_of` and `offset_of`: that catches a stale formula here as well as a
 * layout change.
 *
 * The new host-polled future types are asserted in the same style: the future
 * handle, the future vtable, the waker, the waker vtable, and the two appended
 * plugin-table entries plus the appended host-table entry.
 *
 * The only include is the generated header, because the header is the artifact
 * a non-Rust implementer compiles against.
 */

#include <stddef.h>
#include <stdint.h>

#include "metis_plugin_abi.h"

/* --- the assumption every pointer-sized number below rests on ------------ */

_Static_assert(sizeof(size_t) == sizeof(void *), "size_t and object pointers are the same size");
_Static_assert(sizeof(MetisInvokeFn) == sizeof(void *), "function pointers are the size of object pointers");
_Static_assert(sizeof(MetisFuturePollFn) == sizeof(void *), "future poll is one pointer slot");
_Static_assert(sizeof(MetisWakerCloneFn) == sizeof(void *), "waker clone is one pointer slot");

/* --- status (D7) -------------------------------------------------------- */

_Static_assert(sizeof(MetisStatus) == sizeof(uint32_t), "MetisStatus is one 32-bit word");
_Static_assert(_Alignof(MetisStatus) == _Alignof(uint32_t), "MetisStatus alignment");
_Static_assert(METIS_STATUS_OK == 0, "frozen status ok");
_Static_assert(METIS_STATUS_INVALID_ARGUMENT == 1, "frozen status invalid argument");
_Static_assert(METIS_STATUS_NOT_FOUND == 2, "frozen status not found");
_Static_assert(METIS_STATUS_PERMISSION_DENIED == 3, "frozen status permission denied");
_Static_assert(METIS_STATUS_CANCELLED == 4, "frozen status cancelled");
_Static_assert(METIS_STATUS_INTERNAL == 5, "frozen status internal");
_Static_assert(METIS_STATUS_UNSUPPORTED == 6, "frozen status unsupported");
_Static_assert(METIS_STATUS_PENDING == 7, "pending joins the frozen status space");

/* --- buffer owner tags (D6) -------------------------------------------- */

_Static_assert(METIS_BUFFER_OWNER_NONE == 0, "owner tag none");
_Static_assert(METIS_BUFFER_OWNER_HOST == 1, "owner tag host");
_Static_assert(METIS_BUFFER_OWNER_PLUGIN == 2, "owner tag plugin");

/* --- ABI version ------------------------------------------------------- */

_Static_assert(METIS_ABI_MAJOR == 1, "major 1 is the versioned entry symbol");
_Static_assert(METIS_ABI_MINOR == 0, "minor 0 at first");

/* --- MetisBuffer ------------------------------------------------------- */

_Static_assert(sizeof(MetisBuffer) == 3 * sizeof(void *) + 2 * sizeof(uint32_t), "MetisBuffer size");
_Static_assert(_Alignof(MetisBuffer) == _Alignof(void *), "MetisBuffer alignment");
_Static_assert(offsetof(MetisBuffer, ptr) == 0, "MetisBuffer.ptr offset");
_Static_assert(offsetof(MetisBuffer, len) == sizeof(void *), "MetisBuffer.len offset");
_Static_assert(offsetof(MetisBuffer, cap) == 2 * sizeof(void *), "MetisBuffer.cap offset");
_Static_assert(offsetof(MetisBuffer, owner) == 3 * sizeof(void *), "MetisBuffer.owner offset");
_Static_assert(offsetof(MetisBuffer, reserved) == 3 * sizeof(void *) + sizeof(uint32_t),
               "MetisBuffer.reserved offset");

/* --- MetisTablePrefix -------------------------------------------------- */

_Static_assert(sizeof(MetisTablePrefix) == sizeof(size_t) + 2 * sizeof(uint32_t),
               "MetisTablePrefix size");
_Static_assert(_Alignof(MetisTablePrefix) == _Alignof(size_t), "MetisTablePrefix alignment");
_Static_assert(offsetof(MetisTablePrefix, size) == 0, "prefix.size offset");
_Static_assert(offsetof(MetisTablePrefix, abi_major) == sizeof(size_t), "prefix.abi_major offset");
_Static_assert(offsetof(MetisTablePrefix, abi_minor) == sizeof(size_t) + sizeof(uint32_t),
               "prefix.abi_minor offset");

/* --- MetisPluginTableV1, with the two appended entries ----------------- */

_Static_assert(sizeof(MetisPluginTableV1) == sizeof(MetisTablePrefix) + 9 * sizeof(void *),
               "MetisPluginTableV1 is the prefix plus nine entries");
_Static_assert(_Alignof(MetisPluginTableV1) == _Alignof(void *), "MetisPluginTableV1 alignment");
_Static_assert(offsetof(MetisPluginTableV1, size) == 0, "table.size offset");
_Static_assert(offsetof(MetisPluginTableV1, abi_major) == sizeof(size_t), "table.abi_major offset");
_Static_assert(offsetof(MetisPluginTableV1, abi_minor) == sizeof(size_t) + sizeof(uint32_t),
               "table.abi_minor offset");
_Static_assert(offsetof(MetisPluginTableV1, create_instance) == sizeof(MetisTablePrefix),
               "table.create_instance offset");
_Static_assert(offsetof(MetisPluginTableV1, destroy_instance) == sizeof(MetisTablePrefix) + sizeof(void *),
               "table.destroy_instance offset");
_Static_assert(offsetof(MetisPluginTableV1, list_capabilities) == sizeof(MetisTablePrefix) + 2 * sizeof(void *),
               "table.list_capabilities offset");
_Static_assert(offsetof(MetisPluginTableV1, invoke) == sizeof(MetisTablePrefix) + 3 * sizeof(void *),
               "table.invoke offset");
_Static_assert(offsetof(MetisPluginTableV1, cancel) == sizeof(MetisTablePrefix) + 4 * sizeof(void *),
               "table.cancel offset");
_Static_assert(offsetof(MetisPluginTableV1, release_buffer) == sizeof(MetisTablePrefix) + 5 * sizeof(void *),
               "table.release_buffer offset");
_Static_assert(offsetof(MetisPluginTableV1, shutdown) == sizeof(MetisTablePrefix) + 6 * sizeof(void *),
               "table.shutdown offset");
/* Append-only: the landed table ends exactly at `shutdown`, the first appended
 * entry starts there, and nothing is padded between them. */
_Static_assert(offsetof(MetisPluginTableV1, invoke_future) == sizeof(MetisTablePrefix) + 7 * sizeof(void *),
               "table.invoke_future offset");
_Static_assert(offsetof(MetisPluginTableV1, service_complete) == sizeof(MetisTablePrefix) + 8 * sizeof(void *),
               "table.service_complete offset");
_Static_assert(offsetof(MetisPluginTableV1, service_complete) + sizeof(void *) == sizeof(MetisPluginTableV1),
               "no padding after the last table entry");

/* --- MetisHostApiV1, with the appended service entry ------------------- */

_Static_assert(sizeof(MetisHostApiV1) == sizeof(size_t) + 6 * sizeof(void *), "MetisHostApiV1 size");
_Static_assert(_Alignof(MetisHostApiV1) == _Alignof(void *), "MetisHostApiV1 alignment");
_Static_assert(offsetof(MetisHostApiV1, size) == 0, "host.size offset");
_Static_assert(offsetof(MetisHostApiV1, user_data) == sizeof(size_t), "host.user_data offset");
_Static_assert(offsetof(MetisHostApiV1, complete) == sizeof(size_t) + sizeof(void *), "host.complete offset");
_Static_assert(offsetof(MetisHostApiV1, progress) == sizeof(size_t) + 2 * sizeof(void *),
               "host.progress offset");
_Static_assert(offsetof(MetisHostApiV1, emit_event) == sizeof(size_t) + 3 * sizeof(void *),
               "host.emit_event offset");
_Static_assert(offsetof(MetisHostApiV1, log) == sizeof(size_t) + 4 * sizeof(void *), "host.log offset");
_Static_assert(offsetof(MetisHostApiV1, service_request) == sizeof(size_t) + 5 * sizeof(void *),
               "host.service_request offset");
_Static_assert(offsetof(MetisHostApiV1, service_request) + sizeof(void *) == sizeof(MetisHostApiV1),
               "no padding after the last callback");

/* --- the new host-polled future types ---------------------------------- */

/* MetisFutureHandle: ctx and vtable. */
_Static_assert(sizeof(MetisFutureHandle) == 2 * sizeof(void *), "MetisFutureHandle size");
_Static_assert(_Alignof(MetisFutureHandle) == _Alignof(void *), "MetisFutureHandle alignment");
_Static_assert(offsetof(MetisFutureHandle, ctx) == 0, "handle.ctx offset");
_Static_assert(offsetof(MetisFutureHandle, vtable) == sizeof(void *), "handle.vtable offset");

/* MetisFutureVtable: size, poll, drop. */
_Static_assert(sizeof(MetisFutureVtable) == sizeof(size_t) + 2 * sizeof(void *),
               "MetisFutureVtable size");
_Static_assert(_Alignof(MetisFutureVtable) == _Alignof(void *), "MetisFutureVtable alignment");
_Static_assert(offsetof(MetisFutureVtable, size) == 0, "future_vtable.size offset");
_Static_assert(offsetof(MetisFutureVtable, poll) == sizeof(size_t), "future_vtable.poll offset");
_Static_assert(offsetof(MetisFutureVtable, drop) == sizeof(size_t) + sizeof(void *),
               "future_vtable.drop offset");
_Static_assert(offsetof(MetisFutureVtable, drop) + sizeof(void *) == sizeof(MetisFutureVtable),
               "no padding after future_vtable.drop");

/* MetisWaker: data and vtable. */
_Static_assert(sizeof(MetisWaker) == 2 * sizeof(void *), "MetisWaker size");
_Static_assert(_Alignof(MetisWaker) == _Alignof(void *), "MetisWaker alignment");
_Static_assert(offsetof(MetisWaker, data) == 0, "waker.data offset");
_Static_assert(offsetof(MetisWaker, vtable) == sizeof(void *), "waker.vtable offset");

/* MetisWakerVtable: size, clone, wake, wake_by_ref, drop. */
_Static_assert(sizeof(MetisWakerVtable) == sizeof(size_t) + 4 * sizeof(void *),
               "MetisWakerVtable size");
_Static_assert(_Alignof(MetisWakerVtable) == _Alignof(void *), "MetisWakerVtable alignment");
_Static_assert(offsetof(MetisWakerVtable, size) == 0, "waker_vtable.size offset");
_Static_assert(offsetof(MetisWakerVtable, clone) == sizeof(size_t), "waker_vtable.clone offset");
_Static_assert(offsetof(MetisWakerVtable, wake) == sizeof(size_t) + sizeof(void *),
               "waker_vtable.wake offset");
_Static_assert(offsetof(MetisWakerVtable, wake_by_ref) == sizeof(size_t) + 2 * sizeof(void *),
               "waker_vtable.wake_by_ref offset");
_Static_assert(offsetof(MetisWakerVtable, drop) == sizeof(size_t) + 3 * sizeof(void *),
               "waker_vtable.drop offset");
_Static_assert(offsetof(MetisWakerVtable, drop) + sizeof(void *) == sizeof(MetisWakerVtable),
               "no padding after waker_vtable.drop");

/* --- the measurements Rust compares against ---------------------------- */

/* These are the numbers above, measured by the C compiler instead of
 * asserted, so `tests/layout.rs` can compare both sides. */

#define METIS_C_MEASURE_SIZE(type) const size_t metis_c_size_##type = sizeof(type)
#define METIS_C_MEASURE_ALIGN(type) const size_t metis_c_align_##type = _Alignof(type)
#define METIS_C_MEASURE_OFFSET(type, field) const size_t metis_c_offset_##type##_##field = offsetof(type, field)

METIS_C_MEASURE_SIZE(MetisStatus);
METIS_C_MEASURE_ALIGN(MetisStatus);

METIS_C_MEASURE_SIZE(MetisBuffer);
METIS_C_MEASURE_ALIGN(MetisBuffer);
METIS_C_MEASURE_OFFSET(MetisBuffer, ptr);
METIS_C_MEASURE_OFFSET(MetisBuffer, len);
METIS_C_MEASURE_OFFSET(MetisBuffer, cap);
METIS_C_MEASURE_OFFSET(MetisBuffer, owner);
METIS_C_MEASURE_OFFSET(MetisBuffer, reserved);

METIS_C_MEASURE_SIZE(MetisTablePrefix);
METIS_C_MEASURE_ALIGN(MetisTablePrefix);
METIS_C_MEASURE_OFFSET(MetisTablePrefix, size);
METIS_C_MEASURE_OFFSET(MetisTablePrefix, abi_major);
METIS_C_MEASURE_OFFSET(MetisTablePrefix, abi_minor);

METIS_C_MEASURE_SIZE(MetisPluginTableV1);
METIS_C_MEASURE_ALIGN(MetisPluginTableV1);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, size);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, abi_major);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, abi_minor);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, create_instance);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, destroy_instance);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, list_capabilities);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, invoke);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, cancel);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, release_buffer);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, shutdown);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, invoke_future);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, service_complete);

METIS_C_MEASURE_SIZE(MetisHostApiV1);
METIS_C_MEASURE_ALIGN(MetisHostApiV1);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, size);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, user_data);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, complete);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, progress);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, emit_event);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, log);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, service_request);

METIS_C_MEASURE_SIZE(MetisFutureHandle);
METIS_C_MEASURE_ALIGN(MetisFutureHandle);
METIS_C_MEASURE_OFFSET(MetisFutureHandle, ctx);
METIS_C_MEASURE_OFFSET(MetisFutureHandle, vtable);

METIS_C_MEASURE_SIZE(MetisFutureVtable);
METIS_C_MEASURE_ALIGN(MetisFutureVtable);
METIS_C_MEASURE_OFFSET(MetisFutureVtable, size);
METIS_C_MEASURE_OFFSET(MetisFutureVtable, poll);
METIS_C_MEASURE_OFFSET(MetisFutureVtable, drop);

METIS_C_MEASURE_SIZE(MetisWaker);
METIS_C_MEASURE_ALIGN(MetisWaker);
METIS_C_MEASURE_OFFSET(MetisWaker, data);
METIS_C_MEASURE_OFFSET(MetisWaker, vtable);

METIS_C_MEASURE_SIZE(MetisWakerVtable);
METIS_C_MEASURE_ALIGN(MetisWakerVtable);
METIS_C_MEASURE_OFFSET(MetisWakerVtable, size);
METIS_C_MEASURE_OFFSET(MetisWakerVtable, clone);
METIS_C_MEASURE_OFFSET(MetisWakerVtable, wake);
METIS_C_MEASURE_OFFSET(MetisWakerVtable, wake_by_ref);
METIS_C_MEASURE_OFFSET(MetisWakerVtable, drop);
