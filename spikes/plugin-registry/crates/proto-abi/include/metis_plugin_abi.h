#ifndef METIS_PLUGIN_ABI_H
#define METIS_PLUGIN_ABI_H

#pragma once

#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * ABI major version. Part of the exported symbol name.
 */
#define METIS_ABI_MAJOR 1

/**
 * ABI minor version. Travels in the function table.
 */
#define METIS_ABI_MINOR 0

#define METIS_BUFFER_OWNER_NONE 0

#define METIS_BUFFER_OWNER_HOST 1

#define METIS_BUFFER_OWNER_PLUGIN 2

#define METIS_LOG_DEBUG 0

#define METIS_LOG_INFO 1

#define METIS_LOG_WARN 2

#define METIS_LOG_ERROR 3

typedef enum MetisStatus {
    METIS_STATUS_OK = 0,
    METIS_STATUS_INVALID_ARGUMENT = 1,
    METIS_STATUS_NOT_FOUND = 2,
    METIS_STATUS_PERMISSION_DENIED = 3,
    METIS_STATUS_CANCELLED = 4,
    METIS_STATUS_INTERNAL = 5,
    METIS_STATUS_UNSUPPORTED = 6,
    METIS_STATUS_PENDING = 7,
} MetisStatus;

typedef struct MetisBuffer {
    uint8_t *ptr;
    size_t len;
    size_t cap;
    uint32_t owner;
    uint32_t reserved;
} MetisBuffer;

typedef struct MetisTablePrefix {
    size_t size;
    uint32_t abi_major;
    uint32_t abi_minor;
} MetisTablePrefix;

typedef void (*MetisCompleteFn)(void *user_data,
                                uint64_t request_id,
                                enum MetisStatus status,
                                struct MetisBuffer payload);

typedef void (*MetisProgressFn)(void *user_data, uint64_t request_id, struct MetisBuffer payload);

typedef void (*MetisEmitEventFn)(void *user_data,
                                 const uint8_t *topic,
                                 size_t topic_len,
                                 struct MetisBuffer payload);

typedef void (*MetisLogFn)(void *user_data,
                           int32_t level,
                           const uint8_t *message,
                           size_t message_len);

/**
 * Appended (lane 3): the plugin asks the host for a timer or an IO operation.
 * The host answers later through the plugin's `service_complete`, and the
 * plugin wakes the future's waker when that answer arrives.
 *
 * Present only when the host table's `size` covers it; a host built before the
 * seam installs the old four callbacks and the plugin must not call this.
 */
typedef enum MetisStatus (*MetisServiceRequestFn)(void *user_data,
                                                  const uint8_t *service,
                                                  size_t service_len,
                                                  uint64_t request_id,
                                                  struct MetisBuffer payload);

typedef struct MetisHostApiV1 {
    size_t size;
    void *user_data;
    MetisCompleteFn complete;
    MetisProgressFn progress;
    MetisEmitEventFn emit_event;
    MetisLogFn log;
    /**
     * Appended (lane 3): host-owned services the plugin awaits. Present only
     * when `size` covers it.
     */
    MetisServiceRequestFn service_request;
} MetisHostApiV1;

typedef enum MetisStatus (*MetisCreateInstanceFn)(const struct MetisHostApiV1 *host,
                                                  void **out_instance);

typedef enum MetisStatus (*MetisDestroyInstanceFn)(void *instance);

typedef enum MetisStatus (*MetisListCapabilitiesFn)(void *instance, struct MetisBuffer *out_payload);

typedef enum MetisStatus (*MetisInvokeFn)(void *instance,
                                          const uint8_t *capability,
                                          size_t capability_len,
                                          uint64_t request_id,
                                          struct MetisBuffer payload,
                                          const struct MetisHostApiV1 *host,
                                          struct MetisBuffer *out_payload);

typedef enum MetisStatus (*MetisCancelFn)(void *instance, uint64_t request_id);

typedef enum MetisStatus (*MetisReleaseBufferFn)(struct MetisBuffer buffer);

typedef enum MetisStatus (*MetisShutdownFn)(void *instance);

/**
 * Produces a second owning waker from a borrowed one.
 */
typedef struct MetisWaker (*MetisWakerCloneFn)(const struct MetisWaker *waker);

/**
 * Consumes a waker and schedules the future's task.
 */
typedef void (*MetisWakerWakeFn)(struct MetisWaker waker);

/**
 * Schedules the future's task without consuming the waker.
 */
typedef void (*MetisWakerWakeByRefFn)(const struct MetisWaker *waker);

/**
 * Consumes a waker without waking it.
 */
typedef void (*MetisWakerDropFn)(struct MetisWaker waker);

/**
 * The host's waker vtable. `size` is first, like every table.
 */
typedef struct MetisWakerVtable {
    /**
     * Size in bytes of this table. First field, always.
     */
    size_t size;
    /**
     * Produces a second owning waker from a borrowed one.
     */
    MetisWakerCloneFn clone;
    /**
     * Consumes a waker and schedules the future's task. May be called from any
     * thread; must be cheap and non-blocking.
     */
    MetisWakerWakeFn wake;
    /**
     * Schedules the future's task without consuming the waker.
     */
    MetisWakerWakeByRefFn wake_by_ref;
    /**
     * Consumes a waker without waking it.
     */
    MetisWakerDropFn drop;
} MetisWakerVtable;

/**
 * A waker the host hands to `poll`. The plugin may call `wake` from any
 * thread, and every clone it makes must be `drop`ped.
 *
 * `wake` and `drop` consume the waker they are given; `clone` and `wake_by_ref`
 * borrow it.
 */
typedef struct MetisWaker {
    /**
     * Opaque to the plugin: host-owned state behind the waker.
     */
    void *data;
    /**
     * The host's waker vtable.
     */
    const struct MetisWakerVtable *vtable;
} MetisWaker;

/**
 * Polls a future's `ctx`. Returns `Ok` with a plugin-allocated buffer in
 * `out_value` when ready, `Pending` when not ready (the plugin will call
 * `waker.wake` when it is), and any other status when the call failed.
 *
 * `waker` is valid until the future is dropped. The plugin may clone it (each
 * clone is a `clone` call) and must `drop` every clone. `poll` must not block.
 */
typedef enum MetisStatus (*MetisFuturePollFn)(void *ctx,
                                              struct MetisWaker waker,
                                              struct MetisBuffer *out_value);

/**
 * Releases a future's `ctx`. Called exactly once by the host, and it must
 * release every plugin-owned buffer the future still holds.
 */
typedef enum MetisStatus (*MetisFutureDropFn)(void *ctx);

/**
 * The plugin's future vtable. `size` is first, like every table.
 */
typedef struct MetisFutureVtable {
    /**
     * Size in bytes of this table. First field, always.
     */
    size_t size;
    /**
     * The single non-blocking poll entry.
     */
    MetisFuturePollFn poll;
    /**
     * The exactly-once release entry.
     */
    MetisFutureDropFn drop;
} MetisFutureVtable;

/**
 * A plugin-owned future context plus the vtable the host polls it through.
 *
 * The plugin allocates `ctx` and owns it; the host never dereferences it, it
 * only passes it back to `poll` and calls `drop` on it exactly once.
 */
typedef struct MetisFutureHandle {
    /**
     * Opaque to the host: plugin-owned state for one in-flight future.
     */
    void *ctx;
    /**
     * The plugin's future vtable.
     */
    const struct MetisFutureVtable *vtable;
} MetisFutureHandle;

/**
 * Appended (lane 3): starts a host-polled future for `capability`.
 *
 * On success `*out` holds the plugin-allocated future handle. The host is the
 * only poller, and it must call the handle's `drop` exactly once.
 */
typedef enum MetisStatus (*MetisInvokeFutureFn)(void *instance,
                                                const uint8_t *capability,
                                                size_t capability_len,
                                                uint64_t request_id,
                                                struct MetisBuffer payload,
                                                const struct MetisHostApiV1 *host,
                                                struct MetisFutureHandle *out);

/**
 * Appended (lane 3): delivers a host-owned service the plugin awaited.
 *
 * The host calls it after answering a `service_request`, carrying the answer's
 * status and payload. The plugin wakes the future's waker from this entry, so
 * the host never blocks on the service.
 */
typedef enum MetisStatus (*MetisServiceCompleteFn)(void *instance,
                                                   uint64_t request_id,
                                                   enum MetisStatus status,
                                                   struct MetisBuffer payload);

/**
 * The version 1 function table: `size` first, then seven function pointers,
 * then the two appended host-polled future entries (lane 3).
 *
 * The first seven entries and the prefix are byte-for-byte the landed table;
 * the two appended entries land exactly after `shutdown`, so a reader that
 * stops at the prefix it knows reads a valid table and only needs its `size`
 * check to decide whether the appended slots may be touched.
 */
typedef struct MetisPluginTableV1 {
    size_t size;
    uint32_t abi_major;
    uint32_t abi_minor;
    MetisCreateInstanceFn create_instance;
    MetisDestroyInstanceFn destroy_instance;
    MetisListCapabilitiesFn list_capabilities;
    MetisInvokeFn invoke;
    MetisCancelFn cancel;
    MetisReleaseBufferFn release_buffer;
    MetisShutdownFn shutdown;
    /**
     * Appended (lane 3): starts a host-polled future instead of a callback.
     * Present only when `size` covers it.
     */
    MetisInvokeFutureFn invoke_future;
    /**
     * Appended (lane 3): delivers a host-owned service the plugin awaited.
     * Present only when `size` covers it.
     */
    MetisServiceCompleteFn service_complete;
} MetisPluginTableV1;

typedef const struct MetisPluginTableV1 *(*MetisPluginEntryV1Fn)(void);





#endif  /* METIS_PLUGIN_ABI_H */
