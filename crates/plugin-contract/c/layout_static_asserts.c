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
 * Every check below is written with `METIS_C_LAYOUT_ASSERT`, and every
 * alignment is read with `__alignof`, because `cl.exe` compiles `.c` files in
 * its legacy C mode unless a `/std:` flag asks for more, and the recorded MSVC
 * invocation carries no such flag
 * (`docs/bugs/archive/plugin-contract-c-static-assert-msvc.md`). That mode
 * rejects the C11 `_Static_assert` message form and the C11 `_Alignof`
 * operator, which is how the Windows build of `plugin-runtime` failed in this
 * file before it type-checked any Rust. The forms here are valid C89 onwards,
 * so the same source compiles in whatever mode a compiler defaults to and the
 * check cannot be skipped: `scripts/check-c-layout-asserts.sh` compiles this
 * file in strict ISO C89 under gcc and clang with warnings denied, and compiles
 * a deliberately broken copy of it to prove that a mismatch still fails the
 * build.
 *
 * The only include is the generated header, because the header is the artifact
 * a non-Rust implementer compiles against. The presence predicates are written
 * here in the C form the header documents: a `size` compared against the
 * offset of the entry's slot plus one pointer.
 */

#include <stddef.h>
#include <stdint.h>

#include "metis_plugin_abi.h"

/* --- the check every assertion below is written with -------------------- */

/* A false condition makes the array's length negative, which gcc, clang and
 * cl.exe all reject as an error rather than as a warning, so a mismatch fails
 * the build. `name` identifies the check in the compiler's diagnostic. */
#define METIS_C_LAYOUT_ASSERT(condition, name) \
    typedef char metis_c_layout_assert_##name[(condition) ? 1 : -1]

/* --- the assumption every pointer-sized number below rests on ------------ */

METIS_C_LAYOUT_ASSERT(sizeof(size_t) == sizeof(void *), size_t_matches_object_pointers);
METIS_C_LAYOUT_ASSERT(sizeof(MetisInvokeFutureFn) == sizeof(void *), function_pointers_match_object_pointers);

/* --- status ------------------------------------------------------------- */

METIS_C_LAYOUT_ASSERT(sizeof(MetisStatus) == sizeof(uint32_t), MetisStatus_size);
METIS_C_LAYOUT_ASSERT(__alignof(MetisStatus) == __alignof(uint32_t), MetisStatus_align);
METIS_C_LAYOUT_ASSERT(METIS_STATUS_OK == 0, frozen_status_ok);
METIS_C_LAYOUT_ASSERT(METIS_STATUS_INVALID_ARGUMENT == 1, frozen_status_invalid_argument);
METIS_C_LAYOUT_ASSERT(METIS_STATUS_NOT_FOUND == 2, frozen_status_not_found);
METIS_C_LAYOUT_ASSERT(METIS_STATUS_PERMISSION_DENIED == 3, frozen_status_permission_denied);
METIS_C_LAYOUT_ASSERT(METIS_STATUS_CANCELLED == 4, frozen_status_cancelled);
METIS_C_LAYOUT_ASSERT(METIS_STATUS_INTERNAL == 5, frozen_status_internal);
METIS_C_LAYOUT_ASSERT(METIS_STATUS_UNSUPPORTED == 6, frozen_status_unsupported);
METIS_C_LAYOUT_ASSERT(METIS_STATUS_PENDING == 7, pending_joins_the_frozen_status_space);

/* --- buffer owner tags -------------------------------------------------- */

METIS_C_LAYOUT_ASSERT(METIS_BUFFER_OWNER_NONE == 0, owner_tag_none);
METIS_C_LAYOUT_ASSERT(METIS_BUFFER_OWNER_HOST == 1, owner_tag_host);
METIS_C_LAYOUT_ASSERT(METIS_BUFFER_OWNER_PLUGIN == 2, owner_tag_plugin);

/* --- ABI version ------------------------------------------------------- */

METIS_C_LAYOUT_ASSERT(METIS_ABI_MAJOR == 1, major_1_is_the_versioned_entry_symbol);
/* The revision replaced instance creation with the asynchronous initialisation
 * entry under the unchanged symbol and major, so the minor moved to 2. A reader
 * compares the minor additively: this minor and any newer one are accepted, an
 * older one is refused. */
METIS_C_LAYOUT_ASSERT(METIS_ABI_MINOR == 2, minor_2_is_the_asynchronous_initialisation_entry);

/* --- the payload envelope, now in the header --------------------------- */

METIS_C_LAYOUT_ASSERT(METIS_PAYLOAD_VERSION_MASK == 0x0f, low_nibble_is_the_format_version);
METIS_C_LAYOUT_ASSERT(METIS_PAYLOAD_CODEC_BIT == 0x10, bit_4_selects_the_codec);
METIS_C_LAYOUT_ASSERT(METIS_PAYLOAD_RESERVED_MASK == 0xe0, bits_5_to_7_are_reserved);
METIS_C_LAYOUT_ASSERT(METIS_PAYLOAD_FORMAT_VERSION == 1, format_version_1);
METIS_C_LAYOUT_ASSERT(METIS_CODEC_JSON == 0, codec_0_is_json);
METIS_C_LAYOUT_ASSERT(METIS_CODEC_MESSAGEPACK == 1, codec_1_is_messagepack);

/* --- MetisBuffer ------------------------------------------------------- */

METIS_C_LAYOUT_ASSERT(sizeof(MetisBuffer) == 3 * sizeof(void *) + 2 * sizeof(uint32_t), MetisBuffer_size);
METIS_C_LAYOUT_ASSERT(__alignof(MetisBuffer) == __alignof(void *), MetisBuffer_align);
METIS_C_LAYOUT_ASSERT(offsetof(MetisBuffer, ptr) == 0, MetisBuffer_ptr_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisBuffer, len) == sizeof(void *), MetisBuffer_len_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisBuffer, cap) == 2 * sizeof(void *), MetisBuffer_cap_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisBuffer, owner) == 3 * sizeof(void *), MetisBuffer_owner_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisBuffer, reserved) == 3 * sizeof(void *) + sizeof(uint32_t),
                      MetisBuffer_reserved_offset);

/* --- MetisTablePrefix -------------------------------------------------- */

METIS_C_LAYOUT_ASSERT(sizeof(MetisTablePrefix) == sizeof(size_t) + 2 * sizeof(uint32_t),
                      MetisTablePrefix_size);
METIS_C_LAYOUT_ASSERT(__alignof(MetisTablePrefix) == __alignof(size_t), MetisTablePrefix_align);
METIS_C_LAYOUT_ASSERT(offsetof(MetisTablePrefix, size) == 0, MetisTablePrefix_size_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisTablePrefix, abi_major) == sizeof(size_t),
                      MetisTablePrefix_abi_major_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisTablePrefix, abi_minor) == sizeof(size_t) + sizeof(uint32_t),
                      MetisTablePrefix_abi_minor_offset);

/* --- the future and the waker (neither vtable carries a size) ---------- */

METIS_C_LAYOUT_ASSERT(sizeof(MetisFutureVtable) == 2 * sizeof(void *), future_vtable_is_poll_and_drop);
METIS_C_LAYOUT_ASSERT(__alignof(MetisFutureVtable) == __alignof(void *), MetisFutureVtable_align);
METIS_C_LAYOUT_ASSERT(offsetof(MetisFutureVtable, poll) == 0, MetisFutureVtable_poll_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisFutureVtable, drop) == sizeof(void *), MetisFutureVtable_drop_offset);

METIS_C_LAYOUT_ASSERT(sizeof(MetisFutureHandle) == 2 * sizeof(void *), future_handle_is_ctx_and_vtable);
METIS_C_LAYOUT_ASSERT(__alignof(MetisFutureHandle) == __alignof(void *), MetisFutureHandle_align);
METIS_C_LAYOUT_ASSERT(offsetof(MetisFutureHandle, ctx) == 0, MetisFutureHandle_ctx_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisFutureHandle, vtable) == sizeof(void *), MetisFutureHandle_vtable_offset);

METIS_C_LAYOUT_ASSERT(sizeof(MetisWakerVtable) == 4 * sizeof(void *), waker_vtable_is_four_entries);
METIS_C_LAYOUT_ASSERT(__alignof(MetisWakerVtable) == __alignof(void *), MetisWakerVtable_align);
METIS_C_LAYOUT_ASSERT(offsetof(MetisWakerVtable, clone) == 0, MetisWakerVtable_clone_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisWakerVtable, wake) == sizeof(void *), MetisWakerVtable_wake_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisWakerVtable, wake_by_ref) == 2 * sizeof(void *),
                      MetisWakerVtable_wake_by_ref_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisWakerVtable, drop) == 3 * sizeof(void *),
                      MetisWakerVtable_drop_offset);

METIS_C_LAYOUT_ASSERT(sizeof(MetisWaker) == 2 * sizeof(void *), waker_is_data_and_vtable);
METIS_C_LAYOUT_ASSERT(__alignof(MetisWaker) == __alignof(void *), MetisWaker_align);
METIS_C_LAYOUT_ASSERT(offsetof(MetisWaker, data) == 0, MetisWaker_data_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisWaker, vtable) == sizeof(void *), MetisWaker_vtable_offset);

/* --- MetisPluginTableV1 ------------------------------------------------ */

METIS_C_LAYOUT_ASSERT(sizeof(MetisPluginTableV1) == sizeof(MetisTablePrefix) + 8 * sizeof(void *),
                      plugin_table_is_the_prefix_plus_eight_entries);
METIS_C_LAYOUT_ASSERT(__alignof(MetisPluginTableV1) == __alignof(void *), MetisPluginTableV1_align);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, size) == 0, MetisPluginTableV1_size_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, abi_major) == sizeof(size_t),
                      MetisPluginTableV1_abi_major_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, abi_minor) == sizeof(size_t) + sizeof(uint32_t),
                      MetisPluginTableV1_abi_minor_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, init_plugin) == sizeof(MetisTablePrefix),
                      MetisPluginTableV1_init_plugin_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, destroy_instance) == sizeof(MetisTablePrefix) + sizeof(void *),
                      MetisPluginTableV1_destroy_instance_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, list_capabilities) == sizeof(MetisTablePrefix) + 2 * sizeof(void *),
                      MetisPluginTableV1_list_capabilities_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, invoke_future) == sizeof(MetisTablePrefix) + 3 * sizeof(void *),
                      MetisPluginTableV1_invoke_future_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, cancel) == sizeof(MetisTablePrefix) + 4 * sizeof(void *),
                      MetisPluginTableV1_cancel_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, release_buffer) == sizeof(MetisTablePrefix) + 5 * sizeof(void *),
                      MetisPluginTableV1_release_buffer_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, shutdown) == sizeof(MetisTablePrefix) + 6 * sizeof(void *),
                      MetisPluginTableV1_shutdown_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, service_complete) == sizeof(MetisTablePrefix) + 7 * sizeof(void *),
                      MetisPluginTableV1_service_complete_offset);
/* Append-only: the next entry lands exactly after the current tail, so no
 * padding sits between the table a host knows and the entry it does not. */
METIS_C_LAYOUT_ASSERT(offsetof(MetisPluginTableV1, service_complete) + sizeof(void *) == sizeof(MetisPluginTableV1),
                      no_padding_after_the_last_table_entry);

/* The slot constants the header carries are the offsets above divided by the
 * pointer size, which is what makes the C presence predicate possible. */
METIS_C_LAYOUT_ASSERT(METIS_PLUGIN_TABLE_V1_INIT_PLUGIN_SLOT * sizeof(void *) + sizeof(MetisTablePrefix)
                          == offsetof(MetisPluginTableV1, init_plugin),
                      init_plugin_slot_constant);
METIS_C_LAYOUT_ASSERT(METIS_PLUGIN_TABLE_V1_INVOKE_FUTURE_SLOT * sizeof(void *) + sizeof(MetisTablePrefix)
                          == offsetof(MetisPluginTableV1, invoke_future),
                      invoke_future_slot_constant);
METIS_C_LAYOUT_ASSERT(METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_SLOT * sizeof(void *) + sizeof(MetisTablePrefix)
                          == offsetof(MetisPluginTableV1, service_complete),
                      service_complete_slot_constant);
METIS_C_LAYOUT_ASSERT(METIS_PLUGIN_TABLE_V1_SLOTS == 8, plugin_table_has_eight_pointer_slots);

/* --- MetisHostApiV1 --------------------------------------------------- */

METIS_C_LAYOUT_ASSERT(sizeof(MetisHostApiV1) == sizeof(size_t) + 5 * sizeof(void *), MetisHostApiV1_size);
METIS_C_LAYOUT_ASSERT(__alignof(MetisHostApiV1) == __alignof(void *), MetisHostApiV1_align);
METIS_C_LAYOUT_ASSERT(offsetof(MetisHostApiV1, size) == 0, MetisHostApiV1_size_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisHostApiV1, user_data) == sizeof(size_t), MetisHostApiV1_user_data_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisHostApiV1, progress) == sizeof(size_t) + sizeof(void *),
                      MetisHostApiV1_progress_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisHostApiV1, emit_event) == sizeof(size_t) + 2 * sizeof(void *),
                      MetisHostApiV1_emit_event_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisHostApiV1, log) == sizeof(size_t) + 3 * sizeof(void *),
                      MetisHostApiV1_log_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisHostApiV1, service_request) == sizeof(size_t) + 4 * sizeof(void *),
                      MetisHostApiV1_service_request_offset);
METIS_C_LAYOUT_ASSERT(offsetof(MetisHostApiV1, service_request) + sizeof(void *) == sizeof(MetisHostApiV1),
                      no_padding_after_the_last_callback);
METIS_C_LAYOUT_ASSERT(METIS_HOST_API_V1_SERVICE_REQUEST_SLOT * sizeof(void *) + sizeof(size_t)
                          == offsetof(MetisHostApiV1, service_request),
                      service_request_slot_constant);
METIS_C_LAYOUT_ASSERT(METIS_HOST_API_V1_SLOTS == 5, host_table_has_five_pointer_slots);

/* --- the presence rule, in the form the header documents --------------- */

#define METIS_PLUGIN_TABLE_COVERS(size, slot) \
    ((size) >= sizeof(MetisTablePrefix) + ((slot) + 1) * sizeof(void *))
#define METIS_HOST_API_COVERS(size, slot) ((size) >= sizeof(size_t) + ((slot) + 1) * sizeof(void *))

METIS_C_LAYOUT_ASSERT(METIS_PLUGIN_TABLE_COVERS(sizeof(MetisPluginTableV1), METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_SLOT),
                      full_plugin_table_covers_every_slot);
METIS_C_LAYOUT_ASSERT(METIS_HOST_API_COVERS(sizeof(MetisHostApiV1), METIS_HOST_API_V1_SERVICE_REQUEST_SLOT),
                      full_host_table_covers_every_slot);
METIS_C_LAYOUT_ASSERT(!METIS_PLUGIN_TABLE_COVERS(sizeof(MetisTablePrefix), METIS_PLUGIN_TABLE_V1_INIT_PLUGIN_SLOT),
                      bare_prefix_covers_no_entry);

/* --- the measurements Rust compares against ---------------------------- */

/* These are the numbers above, measured by the C compiler instead of
 * asserted, so `tests/c_layout.rs` can compare both sides. */

#define METIS_C_MEASURE_SIZE(type) const size_t metis_c_size_##type = sizeof(type)
#define METIS_C_MEASURE_ALIGN(type) const size_t metis_c_align_##type = __alignof(type)
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
METIS_C_MEASURE_OFFSET(MetisPluginTableV1, init_plugin);
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
