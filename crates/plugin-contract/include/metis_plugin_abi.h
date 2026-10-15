#ifndef METIS_PLUGIN_ABI_H
#define METIS_PLUGIN_ABI_H

#pragma once

#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * ABI major version. It is part of the exported symbol name.
 */
#define METIS_ABI_MAJOR 1

/**
 * ABI minor version. It travels in the function table.
 *
 * It is 2 because this revision replaced the instance-creation entry with the
 * asynchronous initialisation entry under an unchanged symbol and major. The
 * minor is compared additively: a reader accepts the minor it knows and any
 * newer one, because a newer table's extra entries are appended after the last
 * entry this build reads, and it refuses an older minor, because that table's
 * entries are in a superseded order. A plugin built against a superseded order
 * is therefore refused instead of misread.
 */
#define METIS_ABI_MINOR 2

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
 * The `init_plugin` pointer slot of the plugin table, after its prefix.
 */
#define METIS_PLUGIN_TABLE_V1_INIT_PLUGIN_SLOT 0

/**
 * The `destroy_instance` pointer slot of the plugin table, after its prefix.
 */
#define METIS_PLUGIN_TABLE_V1_DESTROY_INSTANCE_SLOT 1

/**
 * The `list_capabilities` pointer slot of the plugin table, after its prefix.
 */
#define METIS_PLUGIN_TABLE_V1_LIST_CAPABILITIES_SLOT 2

/**
 * The `invoke_future` pointer slot of the plugin table, after its prefix.
 */
#define METIS_PLUGIN_TABLE_V1_INVOKE_FUTURE_SLOT 3

/**
 * The `cancel` pointer slot of the plugin table, after its prefix.
 */
#define METIS_PLUGIN_TABLE_V1_CANCEL_SLOT 4

/**
 * The `release_buffer` pointer slot of the plugin table, after its prefix.
 */
#define METIS_PLUGIN_TABLE_V1_RELEASE_BUFFER_SLOT 5

/**
 * The `shutdown` pointer slot of the plugin table, after its prefix.
 */
#define METIS_PLUGIN_TABLE_V1_SHUTDOWN_SLOT 6

/**
 * The `service_complete` pointer slot of the plugin table, after its prefix.
 */
#define METIS_PLUGIN_TABLE_V1_SERVICE_COMPLETE_SLOT 7

/**
 * The number of pointer slots in the plugin table, after its prefix.
 */
#define METIS_PLUGIN_TABLE_V1_SLOTS 8

/**
 * The `user_data` pointer slot of the host table, after its `size` field.
 */
#define METIS_HOST_API_V1_USER_DATA_SLOT 0

/**
 * The `progress` pointer slot of the host table, after its `size` field.
 */
#define METIS_HOST_API_V1_PROGRESS_SLOT 1

/**
 * The `emit_event` pointer slot of the host table, after its `size` field.
 */
#define METIS_HOST_API_V1_EMIT_EVENT_SLOT 2

/**
 * The `log` pointer slot of the host table, after its `size` field.
 */
#define METIS_HOST_API_V1_LOG_SLOT 3

/**
 * The `service_request` pointer slot of the host table, after its `size`
 * field.
 */
#define METIS_HOST_API_V1_SERVICE_REQUEST_SLOT 4

/**
 * The number of pointer slots in the host table, after its `size` field.
 */
#define METIS_HOST_API_V1_SLOTS 5

/**
 * Mask for the low four bits: the payload format version.
 */
#define METIS_PAYLOAD_VERSION_MASK 15

/**
 * Bit 4: the codec selector.
 */
#define METIS_PAYLOAD_CODEC_BIT 16

/**
 * Bits 5 to 7: reserved, must be zero.
 */
#define METIS_PAYLOAD_RESERVED_MASK 224

/**
 * The only payload format version defined at first.
 */
#define METIS_PAYLOAD_FORMAT_VERSION 1

/**
 * Codec 0: UTF-8 JSON. The only codec this build implements.
 */
#define METIS_CODEC_JSON 0

/**
 * Codec 1: MessagePack. Defined by D4 so it can arrive without a format
 * break, not implemented by this build, and therefore refused as unsupported.
 */
#define METIS_CODEC_MESSAGEPACK 1

/**
 * The status space, frozen within ABI major 1.
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
     * Not finished yet. A poll returns it, the plugin wakes the host waker
     * when the answer arrives, and the host polls again.
     */
    METIS_STATUS_PENDING = 7,
} MetisStatus;

/**
 * A byte buffer crossing the boundary.
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
 * Progress for an in-flight call, carrying a plugin-allocated payload.
 *
 * The receiver releases the payload through `release_buffer`.
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
 * The plugin asks the host for a service it can await: today a timer, later
 * IO.
 *
 * The host answers later through the plugin's `service_complete`, with the
 * same `request_id`, and the plugin wakes its future's waker when the answer
 * arrives. The payload is plugin-owned and the host releases it, including
 * from inside this callback, where `release_buffer` is reentrant.
 */
typedef enum MetisStatus (*MetisServiceRequestFn)(void *user_data,
                                                  const uint8_t *service,
                                                  size_t service_len,
                                                  uint64_t request_id,
                                                  struct MetisBuffer payload);

/**
 * The version 1 host callback table, revision 1 of the contract.
 *
 * Same rules as the plugin table: append-only, presence decided by `size`, and
 * entries are plain function pointers. `user_data` is what makes the callbacks
 * usable: without it a host has nowhere to keep the state the callback needs.
 * A host that does not support a callback installs a no-op stub rather than a
 * null pointer.
 *
 * There is deliberately no `complete` entry. A call completes when a poll of
 * its future returns `Ok`, and a callback that duplicated that would be a
 * second protocol for the same event.
 *
 * Each callback is an `extern "C"` frame reached from plugin code, so a panic
 * inside one unwinds into plugin frames and aborts across the boundary. The
 * host must wrap every callback body in `catch_unwind` and turn a panic into a
 * status the plugin sees, exactly as the plugin side wraps its exported
 * adapters; only the callback's own frame can do this, because the plugin that
 * called it catches nothing.
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
     * Progress of an in-flight call.
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
    /**
     * A host service the plugin awaits.
     */
    MetisServiceRequestFn service_request;
} MetisHostApiV1;

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
 * The host's waker vtable.
 *
 * There is deliberately no `size` field, for the same reason as
 * [`MetisFutureVtable`]: the handle is the only route to it, the four entries
 * below are the whole surface, and a wrong-shape vtable is its author's bug.
 */
typedef struct MetisWakerVtable {
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
 * The host's waker: how a plugin asks to be polled again.
 *
 * The waker's liveness rule, who owns each reference and when, is written once
 * in `docs/plugin-contract.md` section 8. This type carries that rule and
 * points at its one statement rather than repeating it, so there is a single
 * wording to keep in step; the generated header carries this pointer with it.
 *
 * There is deliberately no `Copy`: duplicating this value in Rust would make a
 * second owning handle without a `clone` call, and every clone has to be
 * counted and released through the vtable.
 */
typedef struct MetisWaker {
    /**
     * Host-owned state behind the waker, opaque to the plugin.
     */
    void *data;
    /**
     * The host's waker vtable.
     */
    const struct MetisWakerVtable *vtable;
} MetisWaker;

/**
 * Polls a future's context.
 *
 * Returns `Ok` with a plugin-allocated buffer in `out_value` when the call is
 * done, `Pending` when it is not (the plugin wakes `waker` when it is), and
 * any other status when the call failed and the future is terminal. A poll
 * must not block.
 */
typedef enum MetisStatus (*MetisFuturePollFn)(void *ctx,
                                              struct MetisWaker waker,
                                              struct MetisBuffer *out_value);

/**
 * Releases a future's context. Called by the host exactly once, and never
 * while a poll is in progress.
 */
typedef enum MetisStatus (*MetisFutureDropFn)(void *ctx);

/**
 * The plugin's future vtable: how the host polls a plugin-owned context.
 *
 * There is deliberately no `size` field. A vtable is reached through the
 * handle that carries it, one poll and one drop are the whole surface, and a
 * plugin that publishes a vtable of the wrong shape is that plugin's own bug:
 * it is not detectable at load, and a size field nobody reads would not make
 * it detectable. The residual is recorded here rather than paid for in every
 * poll.
 */
typedef struct MetisFutureVtable {
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
 * A plugin-owned future: an opaque context plus the vtable it is polled
 * through.
 *
 * The host never looks inside `ctx`; it only passes it back to `poll` and to
 * `drop`. The value is copyable because that is what a two-field C struct is,
 * and copying it does not create a second future: the host still drops the one
 * handle it was given, exactly once.
 */
typedef struct MetisFutureHandle {
    /**
     * Plugin-owned state for one in-flight future.
     */
    void *ctx;
    /**
     * The plugin's future vtable.
     */
    const struct MetisFutureVtable *vtable;
} MetisFutureHandle;

/**
 * Initialises an instance and starts the future that completes it.
 *
 * The one payload is host-owned, borrowed for this call, and carries the
 * plugin's own configuration and the host context as the two named parts of
 * one body; a plugin that needs it after this call returns copies it. On
 * `Ok` both out parameters are filled: the instance, and a plugin-owned future
 * that must resolve `Ok` before the instance is valid. The host drives that
 * future on its own runtime, so initialisation may await a host service and
 * return `Pending` rather than block. On any other resolution the host drops
 * the future and then calls `destroy_instance` on the instance pointer, which
 * must tolerate a failed initialisation.
 */
typedef enum MetisStatus (*MetisInitPluginFn)(const struct MetisHostApiV1 *host,
                                              struct MetisBuffer payload,
                                              void **out_instance,
                                              struct MetisFutureHandle *out_future);

/**
 * Destroys an instance. A destroyed handle is not used again.
 */
typedef enum MetisStatus (*MetisDestroyInstanceFn)(void *instance);

/**
 * Returns a plugin-allocated payload holding the capability descriptors.
 */
typedef enum MetisStatus (*MetisListCapabilitiesFn)(void *instance, struct MetisBuffer *out_payload);

/**
 * The only call entry: starts a host-polled future for a capability.
 *
 * On success `*out` holds a plugin-owned future. Nothing is called back for
 * the call's result: the host polls the future it was handed. The request
 * payload is host-owned and valid for the call only.
 */
typedef enum MetisStatus (*MetisInvokeFutureFn)(void *instance,
                                                const uint8_t *capability,
                                                size_t capability_len,
                                                uint64_t request_id,
                                                struct MetisBuffer payload,
                                                const struct MetisHostApiV1 *host,
                                                struct MetisFutureHandle *out);

/**
 * Marks a call's cancellation token.
 *
 * The plugin's future for that request is expected to resolve as `Cancelled`
 * rather than be dropped mid-flight.
 */
typedef enum MetisStatus (*MetisCancelFn)(void *instance, uint64_t request_id);

/**
 * Releases a buffer this plugin allocated.
 *
 * The host calls it for every plugin-owned buffer it received, including a
 * payload it decided to drop and one it is handed from inside
 * `service_request`, where this entry is reentrant.
 */
typedef enum MetisStatus (*MetisReleaseBufferFn)(struct MetisBuffer buffer);

/**
 * Quiesces an instance.
 *
 * The host drops every future before calling it, so nothing may arrive after
 * it returns and the instance may then be destroyed.
 */
typedef enum MetisStatus (*MetisShutdownFn)(void *instance);

/**
 * Delivers the host's answer to an earlier `service_request`.
 *
 * The plugin wakes its future's waker from here. The payload is host-owned and
 * borrowed for the call.
 */
typedef enum MetisStatus (*MetisServiceCompleteFn)(void *instance,
                                                   uint64_t request_id,
                                                   enum MetisStatus status,
                                                   struct MetisBuffer payload);

/**
 * The version 1 function table, revision 2 of the contract.
 *
 * Eight entries and no more: there is no counters entry and no counter of any
 * kind is reachable through the table. Append-only evolution from here: a new
 * entry is appended after `service_complete` and `size` grows to cover it. A
 * reader compares `size` against the prefix it knows before touching a field,
 * and never decides presence from a null pointer: the fields are plain
 * function pointers, because `Option<NamedFnAlias>` has no C rendering
 * cbindgen can emit (measured).
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
     * ABI minor version. A reader accepts the one it speaks and any newer one,
     * and refuses an older one.
     */
    uint32_t abi_minor;
    /**
     * Instance lifecycle: construct and initialise, then hand out the future
     * that completes the initialisation.
     */
    MetisInitPluginFn init_plugin;
    /**
     * Instance lifecycle.
     */
    MetisDestroyInstanceFn destroy_instance;
    /**
     * Capability discovery.
     */
    MetisListCapabilitiesFn list_capabilities;
    /**
     * The only call entry.
     */
    MetisInvokeFutureFn invoke_future;
    /**
     * Cancellation.
     */
    MetisCancelFn cancel;
    /**
     * Buffer release.
     */
    MetisReleaseBufferFn release_buffer;
    /**
     * Quiescence.
     */
    MetisShutdownFn shutdown;
    /**
     * Service completion.
     */
    MetisServiceCompleteFn service_complete;
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
