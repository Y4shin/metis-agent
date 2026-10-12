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
 * Mask for the low four bits: payload format version.
 */
#define METIS_PAYLOAD_VERSION_MASK 15

/**
 * Bit 4: codec selector.
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
 * Codec 0: UTF-8 JSON.
 */
#define METIS_CODEC_JSON 0

/**
 * Codec 1: MessagePack, defined but unused (D4).
 */
#define METIS_CODEC_MESSAGEPACK 1

/**
 * No owner: an unset buffer.
 */
#define METIS_BUFFER_OWNER_NONE 0

/**
 * Allocated by the host, freed by the host.
 */
#define METIS_BUFFER_OWNER_HOST 1

/**
 * Allocated by the plugin, released through `release_buffer`.
 */
#define METIS_BUFFER_OWNER_PLUGIN 2

#define METIS_LOG_DEBUG 0

#define METIS_LOG_INFO 1

#define METIS_LOG_WARN 2

#define METIS_LOG_ERROR 3

/**
 * Frozen status enum (D7).
 *
 * Spike revision: `Pending` is appended. The D7 list of seven members has no
 * way to express "the result arrives through `complete`", which D11 requires.
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
     * Spike addition, see the module docs and the findings.
     */
    METIS_STATUS_PENDING = 7,
} MetisStatus;

/**
 * A borrowed or owned byte buffer crossing the boundary (D6).
 *
 * The host-allocated variant is only valid for the duration of the call that
 * received it. The plugin-allocated variant is released by the plugin table's
 * `release_buffer`.
 */
typedef struct MetisBuffer {
    uint8_t *ptr;
    size_t len;
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
 * The smallest prefix of any table: enough to read the size and the version
 * fields without assuming anything about the rest (D8).
 */
typedef struct MetisTablePrefix {
    size_t size;
    uint32_t abi_major;
    uint32_t abi_minor;
} MetisTablePrefix;

/**
 * A completion arrives with a plugin-allocated payload that the receiver of
 * the callback still has to release through `release_buffer`, even when the
 * receiver discards the payload.
 */
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
 * The version 1 host table (D1).
 *
 * Same rule as the plugin table: entries are plain function pointers and a
 * host that does not support a callback installs a no-op stub. A nullable
 * entry would have to be written inline as
 * `Option<unsafe extern "C" fn(...)>`, which cbindgen renders correctly but
 * without the named typedef.
 */
typedef struct MetisHostApiV1 {
    /**
     * Size in bytes of the host table.
     */
    size_t size;
    void *user_data;
    MetisCompleteFn complete;
    MetisProgressFn progress;
    MetisEmitEventFn emit_event;
    MetisLogFn log;
} MetisHostApiV1;

typedef enum MetisStatus (*MetisCreateInstanceFn)(const struct MetisHostApiV1 *host,
                                                  void **out_instance);

typedef enum MetisStatus (*MetisDestroyInstanceFn)(void *instance);

/**
 * Returns a plugin-allocated payload holding the capability descriptors.
 */
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
 * The version 1 function table (D1, D8).
 *
 * Append-only evolution: new entries are appended after `shutdown` and the
 * `size` field grows. A reader must compare `size` against
 * `size_of::<MetisPluginTableV1>()` (or the prefix it knows) before touching a
 * field.
 *
 * Presence of an entry is decided by `size`, not by a null pointer. The fields
 * are plain function pointers on purpose: cbindgen 0.29 renders
 * `Option<NamedFnAlias>` as an opaque struct, which produces a header that a C
 * compiler rejects. See the findings for the measured behaviour.
 */
typedef struct MetisPluginTableV1 {
    /**
     * Size in bytes of the table the plugin returned.
     */
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
} MetisPluginTableV1;

/**
 * Signature of the exported entry point, `metis_plugin_entry_v1`.
 *
 * No entry point mentions `MetisPluginTableV1` in its parameters: letting one
 * do so makes cbindgen emit the function typedef before the struct, and C then
 * scopes the struct tag to the prototype. See the findings.
 *
 * The entry point takes no arguments on purpose: the symbol name already
 * carries the ABI major version, and a signature that changed within a major
 * version could not be detected before the call.
 */
typedef const struct MetisPluginTableV1 *(*MetisPluginEntryV1Fn)(void);

#endif  /* METIS_PLUGIN_ABI_H */
