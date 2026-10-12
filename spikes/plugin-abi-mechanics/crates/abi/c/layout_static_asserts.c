/*
 * Static layout assertions for the generated ABI header (D13).
 *
 * This translation unit is compiled through the `cc` crate by
 * `crates/abi/build.rs`, so any build in this spike fails when the generated
 * header stops agreeing with the Rust `const` assertions in
 * `crates/abi/src/layout.rs`.
 *
 * The numbers below are literals on purpose: the Rust side asserts the same
 * literals independently, and `crates/abi/tests/c_layout.rs` compares the
 * `SPIKE_*` symbols further down with `size_of!`/`offset_of!`, so a stale
 * literal is caught as well.
 */

#include <stddef.h>
#include <stdint.h>

#include "metis_plugin_abi.h"

/* --- payload header constants (D4) -------------------------------------- */

_Static_assert(METIS_PAYLOAD_VERSION_MASK == 0x0f, "payload version is the low four bits");
_Static_assert(METIS_PAYLOAD_CODEC_BIT == 0x10, "codec is bit 4");
_Static_assert(METIS_PAYLOAD_RESERVED_MASK == 0xe0, "bits 5 to 7 are reserved");
_Static_assert(METIS_BUFFER_OWNER_HOST == 1, "host owner tag");
_Static_assert(METIS_BUFFER_OWNER_PLUGIN == 2, "plugin owner tag");
_Static_assert(METIS_STATUS_PENDING == 7, "spike addition to the frozen status enum");

/* --- MetisStatus -------------------------------------------------------- */

_Static_assert(sizeof(MetisStatus) == 4, "MetisStatus size");
_Static_assert(_Alignof(MetisStatus) == 4, "MetisStatus alignment");

/* --- MetisBuffer -------------------------------------------------------- */

_Static_assert(sizeof(MetisBuffer) == 32, "MetisBuffer size");
_Static_assert(_Alignof(MetisBuffer) == 8, "MetisBuffer alignment");
_Static_assert(offsetof(MetisBuffer, ptr) == 0, "MetisBuffer.ptr offset");
_Static_assert(offsetof(MetisBuffer, len) == 8, "MetisBuffer.len offset");
_Static_assert(offsetof(MetisBuffer, cap) == 16, "MetisBuffer.cap offset");
_Static_assert(offsetof(MetisBuffer, owner) == 24, "MetisBuffer.owner offset");
_Static_assert(offsetof(MetisBuffer, reserved) == 28, "MetisBuffer.reserved offset");

/* --- MetisTablePrefix --------------------------------------------------- */

_Static_assert(sizeof(MetisTablePrefix) == 16, "MetisTablePrefix size");
_Static_assert(_Alignof(MetisTablePrefix) == 8, "MetisTablePrefix alignment");
_Static_assert(offsetof(MetisTablePrefix, size) == 0, "prefix.size offset");
_Static_assert(offsetof(MetisTablePrefix, abi_major) == 8, "prefix.abi_major offset");
_Static_assert(offsetof(MetisTablePrefix, abi_minor) == 12, "prefix.abi_minor offset");

/* --- MetisPluginTableV1 ------------------------------------------------- */

_Static_assert(sizeof(MetisPluginTableV1) == 72, "MetisPluginTableV1 size");
_Static_assert(_Alignof(MetisPluginTableV1) == 8, "MetisPluginTableV1 alignment");
_Static_assert(offsetof(MetisPluginTableV1, size) == 0, "table.size offset");
_Static_assert(offsetof(MetisPluginTableV1, abi_major) == 8, "table.abi_major offset");
_Static_assert(offsetof(MetisPluginTableV1, abi_minor) == 12, "table.abi_minor offset");
_Static_assert(offsetof(MetisPluginTableV1, create_instance) == 16, "table.create_instance offset");
_Static_assert(offsetof(MetisPluginTableV1, destroy_instance) == 24, "table.destroy_instance offset");
_Static_assert(offsetof(MetisPluginTableV1, list_capabilities) == 32, "table.list_capabilities offset");
_Static_assert(offsetof(MetisPluginTableV1, invoke) == 40, "table.invoke offset");
_Static_assert(offsetof(MetisPluginTableV1, cancel) == 48, "table.cancel offset");
_Static_assert(offsetof(MetisPluginTableV1, release_buffer) == 56, "table.release_buffer offset");
_Static_assert(offsetof(MetisPluginTableV1, shutdown) == 64, "table.shutdown offset");
/* Append-only: the next entry lands exactly after the current tail. */
_Static_assert(offsetof(MetisPluginTableV1, shutdown) + sizeof(void *) == sizeof(MetisPluginTableV1),
               "no padding after the last entry");

/* --- MetisHostApiV1 ----------------------------------------------------- */

_Static_assert(sizeof(MetisHostApiV1) == 48, "MetisHostApiV1 size");
_Static_assert(_Alignof(MetisHostApiV1) == 8, "MetisHostApiV1 alignment");
_Static_assert(offsetof(MetisHostApiV1, size) == 0, "host.size offset");
_Static_assert(offsetof(MetisHostApiV1, user_data) == 8, "host.user_data offset");
_Static_assert(offsetof(MetisHostApiV1, complete) == 16, "host.complete offset");
_Static_assert(offsetof(MetisHostApiV1, progress) == 24, "host.progress offset");
_Static_assert(offsetof(MetisHostApiV1, emit_event) == 32, "host.emit_event offset");
_Static_assert(offsetof(MetisHostApiV1, log) == 40, "host.log offset");
_Static_assert(offsetof(MetisHostApiV1, log) + sizeof(void *) == sizeof(MetisHostApiV1),
               "no padding after the last callback");

/* --- values exported to the Rust test ----------------------------------- */

const size_t SPIKE_size_MetisStatus = sizeof(MetisStatus);
const size_t SPIKE_align_MetisStatus = _Alignof(MetisStatus);
const size_t SPIKE_size_MetisBuffer = sizeof(MetisBuffer);
const size_t SPIKE_align_MetisBuffer = _Alignof(MetisBuffer);
const size_t SPIKE_offset_MetisBuffer_len = offsetof(MetisBuffer, len);
const size_t SPIKE_offset_MetisBuffer_owner = offsetof(MetisBuffer, owner);
const size_t SPIKE_size_MetisTablePrefix = sizeof(MetisTablePrefix);
const size_t SPIKE_size_MetisPluginTableV1 = sizeof(MetisPluginTableV1);
const size_t SPIKE_align_MetisPluginTableV1 = _Alignof(MetisPluginTableV1);
const size_t SPIKE_offset_MetisPluginTableV1_create_instance = offsetof(MetisPluginTableV1, create_instance);
const size_t SPIKE_offset_MetisPluginTableV1_invoke = offsetof(MetisPluginTableV1, invoke);
const size_t SPIKE_offset_MetisPluginTableV1_shutdown = offsetof(MetisPluginTableV1, shutdown);
const size_t SPIKE_size_MetisHostApiV1 = sizeof(MetisHostApiV1);
const size_t SPIKE_align_MetisHostApiV1 = _Alignof(MetisHostApiV1);
const size_t SPIKE_offset_MetisHostApiV1_complete = offsetof(MetisHostApiV1, complete);
const size_t SPIKE_offset_MetisHostApiV1_log = offsetof(MetisHostApiV1, log);

/* --- a C-authored version 1 table -------------------------------------- */

/* Written the way a non-Rust implementer would write it, to prove that the
 * committed header is usable as-is: designated initialisers, one function per
 * entry, no knowledge of Rust. */

static const MetisHostApiV1 *spike_c_host_api;

static void spike_c_complete(void *user_data, uint64_t request_id, MetisStatus status, MetisBuffer payload)
{
    (void)user_data;
    (void)request_id;
    (void)status;
    (void)payload;
}

static void spike_c_progress(void *user_data, uint64_t request_id, MetisBuffer payload)
{
    (void)user_data;
    (void)request_id;
    (void)payload;
}

static void spike_c_emit_event(void *user_data, const uint8_t *topic, size_t topic_len, MetisBuffer payload)
{
    (void)user_data;
    (void)topic;
    (void)topic_len;
    (void)payload;
}

static void spike_c_log(void *user_data, int32_t level, const uint8_t *message, size_t message_len)
{
    (void)user_data;
    (void)level;
    (void)message;
    (void)message_len;
}

static MetisStatus spike_c_create_instance(const MetisHostApiV1 *host, void **out_instance)
{
    spike_c_host_api = host;
    if (out_instance == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    *out_instance = (void *)0x1;
    return METIS_STATUS_OK;
}

static MetisStatus spike_c_destroy_instance(void *instance)
{
    return instance == (void *)0x1 ? METIS_STATUS_OK : METIS_STATUS_NOT_FOUND;
}

static MetisStatus spike_c_list_capabilities(void *instance, MetisBuffer *out_payload)
{
    (void)instance;
    (void)out_payload;
    return METIS_STATUS_OK;
}

static MetisStatus spike_c_invoke(void *instance,
                                  const uint8_t *capability,
                                  size_t capability_len,
                                  uint64_t request_id,
                                  MetisBuffer payload,
                                  const MetisHostApiV1 *host,
                                  MetisBuffer *out_payload)
{
    (void)instance;
    (void)capability;
    (void)capability_len;
    (void)request_id;
    (void)payload;
    (void)host;
    (void)out_payload;
    return METIS_STATUS_OK;
}

static MetisStatus spike_c_cancel(void *instance, uint64_t request_id)
{
    (void)instance;
    (void)request_id;
    return METIS_STATUS_OK;
}

static MetisStatus spike_c_release_buffer(MetisBuffer buffer)
{
    (void)buffer;
    return METIS_STATUS_OK;
}

static MetisStatus spike_c_shutdown(void *instance)
{
    (void)instance;
    return METIS_STATUS_OK;
}

static const MetisPluginTableV1 SPIKE_C_TABLE = {
    .size = sizeof(MetisPluginTableV1),
    .abi_major = METIS_ABI_MAJOR,
    .abi_minor = METIS_ABI_MINOR,
    .create_instance = spike_c_create_instance,
    .destroy_instance = spike_c_destroy_instance,
    .list_capabilities = spike_c_list_capabilities,
    .invoke = spike_c_invoke,
    .cancel = spike_c_cancel,
    .release_buffer = spike_c_release_buffer,
    .shutdown = spike_c_shutdown,
};

/* Mirrors the plugin entry point: the Rust test resolves and drives this. */
const MetisPluginTableV1 *spike_c_entry_v1(void)
{
    return &SPIKE_C_TABLE;
}

/* Unused callbacks, exported so the compiler keeps them. */
const void *spike_c_host_callbacks[] = {
    (const void *)spike_c_complete,
    (const void *)spike_c_progress,
    (const void *)spike_c_emit_event,
    (const void *)spike_c_log,
};
