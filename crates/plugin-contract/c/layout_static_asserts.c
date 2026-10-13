/*
 * Static layout assertions for the generated ABI header.
 *
 * This translation unit is compiled by `crates/plugin-contract/build.rs`
 * through the `cc` crate, so any build of the crate fails when the committed
 * header stops agreeing with the pointer-size formulas in
 * `crates/plugin-contract/src/abi/layout.rs`.
 *
 * Every number is written in terms of `sizeof(void *)`, `sizeof(size_t)` and
 * `sizeof(uint32_t)` rather than as a 64-bit literal, so the assertions hold on
 * a 32-bit target too. At the bottom the same numbers are exported as
 * `metis_c_*` symbols, which `tests/c_layout.rs` compares with Rust's
 * `size_of`, `align_of` and `offset_of`: that catches a stale formula here as
 * well as a layout change.
 *
 * The only include is the generated header, because the header is the artifact
 * a non-Rust implementer compiles against. The presence predicates are written
 * here in the C form the header documents: a `size` compared against the
 * offset of the entry's slot plus one pointer.
 */

#include <stddef.h>
#include <stdint.h>

#include "metis_plugin_abi.h"

/* --- the assumption every pointer-sized number below rests on ------------ */

_Static_assert(sizeof(size_t) == sizeof(void *), "size_t and object pointers are the same size");
_Static_assert(sizeof(MetisInvokeFutureFn) == sizeof(void *), "function pointers are the size of object pointers");

/* --- status ------------------------------------------------------------- */

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

/* --- buffer owner tags -------------------------------------------------- */

_Static_assert(METIS_BUFFER_OWNER_NONE == 0, "owner tag none");
_Static_assert(METIS_BUFFER_OWNER_HOST == 1, "owner tag host");
_Static_assert(METIS_BUFFER_OWNER_PLUGIN == 2, "owner tag plugin");

/* --- ABI version ------------------------------------------------------- */

_Static_assert(METIS_ABI_MAJOR == 1, "major 1 is the versioned entry symbol");
/* The revision changed the entry order under the unchanged symbol and major,
 * so the minor moved to 1. A reader compares the minor additively: this minor
 * and any newer one are accepted, an older one is refused. */
_Static_assert(METIS_ABI_MINOR == 1, "minor 1 is the revised entry order");

/* --- the payload envelope, now in the header --------------------------- */

_Static_assert(METIS_PAYLOAD_VERSION_MASK == 0x0f, "the low nibble is the format version");
_Static_assert(METIS_PAYLOAD_CODEC_BIT == 0x10, "bit 4 selects the codec");
_Static_assert(METIS_PAYLOAD_RESERVED_MASK == 0xe0, "bits 5 to 7 are reserved");
_Static_assert(METIS_PAYLOAD_FORMAT_VERSION == 1, "format version 1");
_Static_assert(METIS_CODEC_JSON == 0, "codec 0 is JSON");
_Static_assert(METIS_CODEC_MESSAGEPACK == 1, "codec 1 is MessagePack");

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

/* --- the future and the waker (neither vtable carries a size) ---------- */

_Static_assert(sizeof(MetisFutureVtable) == 2 * sizeof(void *),
               "the future vtable is poll and drop, and nothing else");
_Static_assert(_Alignof(MetisFutureVtable) == _Alignof(void *), "MetisFutureVtable alignment");
_Static_assert(offsetof(MetisFutureVtable, poll) == 0, "future vtable poll offset");
_Static_assert(offsetof(MetisFutureVtable, drop) == sizeof(void *), "future vtable drop offset");

_Static_assert(sizeof(MetisFutureHandle) == 2 * sizeof(void *),
               "the handle is a context pointer and a vtable pointer");
_Static_assert(_Alignof(MetisFutureHandle) == _Alignof(void *), "MetisFutureHandle alignment");
_Static_assert(offsetof(MetisFutureHandle, ctx) == 0, "future handle ctx offset");
_Static_assert(offsetof(MetisFutureHandle, vtable) == sizeof(void *), "future handle vtable offset");

_Static_assert(sizeof(MetisWakerVtable) == 4 * sizeof(void *),
               "the waker vtable is clone, wake, wake_by_ref and drop");
_Static_assert(_Alignof(MetisWakerVtable) == _Alignof(void *), "MetisWakerVtable alignment");
_Static_assert(offsetof(MetisWakerVtable, clone) == 0, "waker vtable clone offset");
_Static_assert(offsetof(MetisWakerVtable, wake) == sizeof(void *), "waker vtable wake offset");
_Static_assert(offsetof(MetisWakerVtable, wake_by_ref) == 2 * sizeof(void *),
               "waker vtable wake_by_ref offset");
_Static_assert(offsetof(MetisWakerVtable, drop) == 3 * sizeof(void *), "waker vtable drop offset");

_Static_assert(sizeof(MetisWaker) == 2 * sizeof(void *),
               "the waker is a data pointer and a vtable pointer");
_Static_assert(_Alignof(MetisWaker) == _Alignof(void *), "MetisWaker alignment");
_Static_assert(offsetof(MetisWaker, data) == 0, "waker data offset");
_Static_assert(offsetof(MetisWaker, vtable) == sizeof(void *), "waker vtable offset");

/* --- MetisPluginTableV1 ------------------------------------------------ */

_Static_assert(sizeof(MetisPluginTableV1) == sizeof(MetisTablePrefix) + 8 * sizeof(void *),
               "the plugin table is the prefix plus eight entries");
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
_Static_assert(offsetof(MetisPluginTableV1, invoke_future) == sizeof(MetisTablePrefix) + 3 * sizeof(void *),
               "table.invoke_future offset");
_Static_assert(offsetof(MetisPluginTableV1, cancel) == sizeof(MetisTablePrefix) + 4 * sizeof(void *),
               "table.cancel offset");
_Static_assert(offsetof(MetisPluginTableV1, release_buffer) == sizeof(MetisTablePrefix) + 5 * sizeof(void *),
               "table.release_buffer offset");
_Static_assert(offsetof(MetisPluginTableV1, shutdown) == sizeof(MetisTablePrefix) + 6 * sizeof(void *),
               "table.shutdown offset");
_Static_assert(offsetof(MetisPluginTableV1, service_complete) == sizeof(MetisTablePrefix) + 7 * sizeof(void *),
               "table.service_complete offset");
/* Append-only: the next entry lands exactly after the current tail, so no
 * padding sits between the table a host knows and the entry it does not. */
_Static_assert(offsetof(MetisPluginTableV1, service_complete) + sizeof(void *) == sizeof(MetisPluginTableV1),
               "no padding after the last table entry");

/* The slot constants the header carries are the offsets above divided by the
 * pointer size, which is what makes the C presence predicate possible. */
_Static_assert(METIS_PLUGIN_TABLE_V1_INVOKE_FUTURE_SLOT * sizeof(void *) + sizeof(MetisTablePrefix)
                   == offsetof(MetisPluginTableV1, invoke_future),
               "invoke_future slot constant");
_Static_assert(METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_SLOT * sizeof(void *) + sizeof(MetisTablePrefix)
                   == offsetof(MetisPluginTableV1, service_complete),
               "service_complete slot constant");
_Static_assert(METIS_PLUGIN_TABLE_V1_SLOTS == 8, "the plugin table has eight pointer slots");

/* --- MetisHostApiV1 --------------------------------------------------- */

_Static_assert(sizeof(MetisHostApiV1) == sizeof(size_t) + 5 * sizeof(void *), "MetisHostApiV1 size");
_Static_assert(_Alignof(MetisHostApiV1) == _Alignof(void *), "MetisHostApiV1 alignment");
_Static_assert(offsetof(MetisHostApiV1, size) == 0, "host.size offset");
_Static_assert(offsetof(MetisHostApiV1, user_data) == sizeof(size_t), "host.user_data offset");
_Static_assert(offsetof(MetisHostApiV1, progress) == sizeof(size_t) + sizeof(void *),
               "host.progress offset");
_Static_assert(offsetof(MetisHostApiV1, emit_event) == sizeof(size_t) + 2 * sizeof(void *),
               "host.emit_event offset");
_Static_assert(offsetof(MetisHostApiV1, log) == sizeof(size_t) + 3 * sizeof(void *), "host.log offset");
_Static_assert(offsetof(MetisHostApiV1, service_request) == sizeof(size_t) + 4 * sizeof(void *),
               "host.service_request offset");
_Static_assert(offsetof(MetisHostApiV1, service_request) + sizeof(void *) == sizeof(MetisHostApiV1),
               "no padding after the last callback");
_Static_assert(METIS_HOST_API_V1_SERVICE_REQUEST_SLOT * sizeof(void *) + sizeof(size_t)
                   == offsetof(MetisHostApiV1, service_request),
               "service_request slot constant");
_Static_assert(METIS_HOST_API_V1_SLOTS == 5, "the host table has five pointer slots");

/* --- the presence rule, in the form the header documents --------------- */

#define METIS_PLUGIN_TABLE_COVERS(size, slot) \
    ((size) >= sizeof(MetisTablePrefix) + ((slot) + 1) * sizeof(void *))
#define METIS_HOST_API_COVERS(size, slot) ((size) >= sizeof(size_t) + ((slot) + 1) * sizeof(void *))

_Static_assert(METIS_PLUGIN_TABLE_COVERS(sizeof(MetisPluginTableV1), METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_SLOT),
               "the full plugin table covers every slot");
_Static_assert(METIS_HOST_API_COVERS(sizeof(MetisHostApiV1), METIS_HOST_API_V1_SERVICE_REQUEST_SLOT),
               "the full host table covers every slot");
_Static_assert(!METIS_PLUGIN_TABLE_COVERS(sizeof(MetisTablePrefix), METIS_PLUGIN_TABLE_V1_CREATE_INSTANCE_SLOT),
               "the bare prefix covers no entry");

/* --- the measurements Rust compares against ---------------------------- */

/* These are the numbers above, measured by the C compiler instead of
 * asserted, so `tests/c_layout.rs` can compare both sides. */

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

METIS_C_MEASURE_SIZE(MetisFutureVtable);
METIS_C_MEASURE_ALIGN(MetisFutureVtable);
METIS_C_MEASURE_OFFSET(MetisFutureVtable, poll);
METIS_C_MEASURE_OFFSET(MetisFutureVtable, drop);

METIS_C_MEASURE_SIZE(MetisFutureHandle);
METIS_C_MEASURE_ALIGN(MetisFutureHandle);
METIS_C_MEASURE_OFFSET(MetisFutureHandle, ctx);
METIS_C_MEASURE_OFFSET(MetisFutureHandle, vtable);

METIS_C_MEASURE_SIZE(MetisWakerVtable);
METIS_C_MEASURE_ALIGN(MetisWakerVtable);
METIS_C_MEASURE_OFFSET(MetisWakerVtable, clone);
METIS_C_MEASURE_OFFSET(MetisWakerVtable, wake);
METIS_C_MEASURE_OFFSET(MetisWakerVtable, wake_by_ref);
METIS_C_MEASURE_OFFSET(MetisWakerVtable, drop);

METIS_C_MEASURE_SIZE(MetisWaker);
METIS_C_MEASURE_ALIGN(MetisWaker);
METIS_C_MEASURE_OFFSET(MetisWaker, data);
METIS_C_MEASURE_OFFSET(MetisWaker, vtable);

METIS_C_MEASURE_SIZE(MetisPluginTableV1);
METIS_C_MEASURE_ALIGN(MetisPluginTableV1);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, size);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, abi_major);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, abi_minor);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, create_instance);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, destroy_instance);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, list_capabilities);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, invoke_future);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, cancel);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, release_buffer);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, shutdown);
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, service_complete);

METIS_C_MEASURE_SIZE(MetisHostApiV1);
METIS_C_MEASURE_ALIGN(MetisHostApiV1);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, size);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, user_data);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, progress);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, emit_event);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, log);
METIS_C_MEASURE_OFFSET(MetisHostApiV1, service_request);
