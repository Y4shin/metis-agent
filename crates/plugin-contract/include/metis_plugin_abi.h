#ifndef METIS_PLUGIN_ABI_H
#define METIS_PLUGIN_ABI_H

#pragma once

#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * ABI major version. It is part of the exported symbol name (D8).
 */
#define METIS_ABI_MAJOR 1

/**
 * ABI minor version. It travels in the function table (D8).
 */
#define METIS_ABI_MINOR 0

/**
 * No owner: an unset buffer, such as a null out parameter.
 */
#define METIS_BUFFER_OWNER_NONE 0

/**
 * Allocated by the host, freed by the host after the call returns.
 */
#define METIS_BUFFER_OWNER_HOST 1

/**
 * Allocated by the plugin, released through the table's `release_buffer`.
 */
#define METIS_BUFFER_OWNER_PLUGIN 2

/**
 * Diagnostic level for the host `log` callback.
 */
#define METIS_LOG_DEBUG 0

/**
 * Informational level for the host `log` callback.
 */
#define METIS_LOG_INFO 1

/**
 * Warning level for the host `log` callback.
 */
#define METIS_LOG_WARN 2

/**
 * Error level for the host `log` callback.
 */
#define METIS_LOG_ERROR 3

/**
 * The status space, frozen within ABI major 1 (D7).
 *
 * The members are an enum because every entry of the function table returns
 * one, and because a frozen integer space is what lets a host written in
 * another language handle a plugin it was not compiled against.
 */
typedef enum MetisStatus {
    METIS_STATUS_OK = 0,
    METIS_STATUS_INVALID_ARGUMENT = 1,
    METIS_STATUS_NOT_FOUND = 2,
    METIS_STATUS_PERMISSION_DENIED = 3,
    METIS_STATUS_CANCELLED = 4,
    METIS_STATUS_INTERNAL = 5,
    METIS_STATUS_UNSUPPORTED = 6,
    /**
     * The result did not finish inline and will arrive through the host's
     * `complete` callback (D11). This member joins the frozen list because no
     * other member can express a deferred outcome.
     */
    METIS_STATUS_PENDING = 7,
} MetisStatus;

/**
 * A byte buffer crossing the boundary (D6).
 *
 * The owner tag is what makes "each side releases only what it allocated"
 * checkable at runtime: a receiver never frees a buffer it does not own, it
 * hands a plugin-owned buffer back through `release_buffer`, and a host-owned
 * buffer is only valid for the duration of the call that received it.
 */
typedef struct MetisBuffer {
    /**
     * The first byte of the payload, or null for an empty buffer.
     */
    uint8_t *ptr;
    /**
     * Valid bytes at `ptr`.
     */
    size_t len;
    /**
     * Allocated capacity at `ptr`, which only the allocator may free with.
     */
    size_t cap;
    /**
     * One of the `METIS_BUFFER_OWNER_*` constants.
     */
    uint32_t owner;
    /**
     * Reserved, zero.
     */
    uint32_t reserved;
} MetisBuffer;

/**
 * The smallest prefix of any table: the size and the version fields.
 *
 * A consumer that knows only this much reads only this much, which is what
 * makes the table append-only. `size` is the first field so it is readable
 * before anything else is touched, and a table whose `size` does not cover
 * this prefix is rejected rather than read.
 */
typedef struct MetisTablePrefix {
    /**
     * Size in bytes of the table the plugin returned.
     */
    size_t size;
    /**
     * ABI major version of the table.
     */
    uint32_t abi_major;
    /**
     * ABI minor version of the table.
     */
    uint32_t abi_minor;
} MetisTablePrefix;

/**
 * A completion, inline or late, carrying a plugin-allocated payload (D11).
 *
 * The receiver releases the payload through `release_buffer` even when it
 * discards it, for example a completion that arrived after a successful
 * cancel.
 */
typedef void (*MetisCompleteFn)(void *user_data,
                                uint64_t request_id,
                                enum MetisStatus status,
                                struct MetisBuffer payload);

/**
 * Progress for a pending request, carrying a plugin-allocated payload.
 */
typedef void (*MetisProgressFn)(void *user_data, uint64_t request_id, struct MetisBuffer payload);

/**
 * A plugin-originated event, owned by the plugin until it is released.
 */
typedef void (*MetisEmitEventFn)(void *user_data,
                                 const uint8_t *topic,
                                 size_t topic_len,
                                 struct MetisBuffer payload);

/**
 * A log line borrowed from the plugin for the duration of the call.
 */
typedef void (*MetisLogFn)(void *user_data,
                           int32_t level,
                           const uint8_t *message,
                           size_t message_len);

/**
 * The version 1 host callback table (D1).
 *
 * Same rules as the plugin table: append-only, presence decided by `size`, and
 * entries are plain function pointers. `user_data` is what makes the callbacks
 * usable: without it a host has nowhere to keep the state the callback needs.
 * A host that does not support a callback installs a no-op stub rather than a
 * null pointer.
 *
 * Each callback is an `extern "C"` frame reached from plugin code, so a panic
 * inside one unwinds into plugin frames and aborts across the boundary. The
 * host must wrap every callback body in `catch_unwind` and turn a panic into a
 * status the plugin sees, exactly as the plugin side wraps its exported
 * adapters (D7 as amended by the mechanics spike); only the callback's own
 * frame can do this, because the plugin that called it catches nothing.
 */
typedef struct MetisHostApiV1 {
    /**
     * Size in bytes of this table. First field, always.
     */
    size_t size;
    /**
     * Opaque host state handed back to every callback.
     */
    void *user_data;
    /**
     * Completion of a pending request (D11).
     */
    MetisCompleteFn complete;
    /**
     * Progress of a pending request.
     */
    MetisProgressFn progress;
    /**
     * A plugin-originated event.
     */
    MetisEmitEventFn emit_event;
    /**
     * A log line.
     */
    MetisLogFn log;
} MetisHostApiV1;

/**
 * Creates an instance and hands it the host table it will call back through.
 */
typedef enum MetisStatus (*MetisCreateInstanceFn)(const struct MetisHostApiV1 *host,
                                                  void **out_instance);

/**
 * Destroys an instance. A destroyed handle is not used again.
 */
typedef enum MetisStatus (*MetisDestroyInstanceFn)(void *instance);

/**
 * Returns a plugin-allocated payload holding the capability descriptors (D9).
 */
typedef enum MetisStatus (*MetisListCapabilitiesFn)(void *instance, struct MetisBuffer *out_payload);

/**
 * One generic call, addressed by capability name (D1, D9, D11).
 *
 * Returns `Ok` with a plugin-allocated payload for an inline result, `Pending`
 * when the result will arrive through the host's `complete` callback, or a
 * failure status. `host` is passed per call as well as retained by the
 * instance, so an implementation can use either without a cost.
 */
typedef enum MetisStatus (*MetisInvokeFn)(void *instance,
                                          const uint8_t *capability,
                                          size_t capability_len,
                                          uint64_t request_id,
                                          struct MetisBuffer payload,
                                          const struct MetisHostApiV1 *host,
                                          struct MetisBuffer *out_payload);

/**
 * Requests cancellation of a pending request (D11, D12).
 */
typedef enum MetisStatus (*MetisCancelFn)(void *instance, uint64_t request_id);

/**
 * Releases a buffer this plugin allocated (D6).
 *
 * The host calls it for every plugin-owned buffer it received, including a
 * payload it decided to drop.
 */
typedef enum MetisStatus (*MetisReleaseBufferFn)(struct MetisBuffer buffer);

/**
 * Shuts an instance down: no callback may arrive after it returns (D12).
 */
typedef enum MetisStatus (*MetisShutdownFn)(void *instance);

/**
 * The version 1 function table (D1, D8).
 *
 * Append-only evolution: a new entry is appended after `shutdown` and `size`
 * grows to cover it. A reader compares `size` against the prefix it knows
 * before touching a field, and never decides presence from a null pointer:
 * the fields are plain function pointers, because `Option<NamedFnAlias>` has
 * no C rendering cbindgen can emit (measured).
 */
typedef struct MetisPluginTableV1 {
    /**
     * Size in bytes of this table. First field, always.
     */
    size_t size;
    /**
     * ABI major version, which must match the entry symbol's naming.
     */
    uint32_t abi_major;
    /**
     * ABI minor version, which may exceed the one a host was built with.
     */
    uint32_t abi_minor;
    /**
     * Instance lifecycle.
     */
    MetisCreateInstanceFn create_instance;
    /**
     * Instance lifecycle.
     */
    MetisDestroyInstanceFn destroy_instance;
    /**
     * Capability discovery (D9).
     */
    MetisListCapabilitiesFn list_capabilities;
    /**
     * The single generic call (D1).
     */
    MetisInvokeFn invoke;
    /**
     * Cancellation (D11).
     */
    MetisCancelFn cancel;
    /**
     * Buffer release (D6).
     */
    MetisReleaseBufferFn release_buffer;
    /**
     * Shutdown (D12).
     */
    MetisShutdownFn shutdown;
} MetisPluginTableV1;

/**
 * The signature of `metis_plugin_entry_v1`.
 *
 * Argument-free and frozen within a major version: the symbol name carries
 * only the major, so a signature that changed inside major 1 could not be
 * detected before the call. No entry point takes `MetisPluginTableV1` in a
 * parameter either, because cbindgen then emits the typedef before the struct
 * and C scopes the struct tag to the prototype (measured).
 */
typedef const struct MetisPluginTableV1 *(*MetisPluginEntryV1Fn)(void);

#endif  /* METIS_PLUGIN_ABI_H */
