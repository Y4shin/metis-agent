/*
 * A C implementer of the revised v1 plugin table.
 *
 * This file exists to answer one question the Rust fixture cannot: is the
 * committed header enough for a non-Rust author, or does the ABI secretly
 * depend on something only a Rust implementer has? Everything it needs comes
 * from `metis_plugin_abi.h`: the tables, the status space, the buffer owner
 * tags, the payload envelope constants, and the presence rule (which is why the
 * constants are in the header rather than in a plugin author's head).
 *
 * It is compiled by `scripts/build-c-plugin.sh` with `-std=c11 -Wall -Wextra
 * -Werror` under both gcc and clang, and the ABI harness drives it end to end.
 *
 * What it keeps, and why the harness reads it:
 *
 * - an allocation ledger: every buffer it hands to the host is counted, every
 *   `release_buffer` is counted, and a second release of the same pointer is
 *   detected rather than being a second `free`;
 * - a waker ledger: every clone it takes of the host's waker is counted and
 *   released exactly once, so the host's own clone count balances;
 * - a reply-after-drop count: a service the host answers for a future that is
 *   already gone is reported, not dereferenced.
 *
 * Its four capabilities are `c.ready` (ready on the first poll, and it emits one
 * event through the host's `emit_event` entry), `c.timer` (asks the host for a
 * timer, then answers), `c.cancel` (parks until cancelled), and
 * `c.drop_in_flight` (asks for a timer and is dropped before the answer).
 *
 * Initialisation is the `init_plugin` entry: it takes the one payload holding
 * the plugin's configuration and the host context, hands out the instance and a
 * future, and the future resolves `Ok` only when the configuration is the one
 * this plugin declares. `declared_config_matches` is that declaration, and it
 * is written as the bytes of the configuration object the plugin accepts, which
 * is what a hand-written C plugin can check without a JSON parser.
 */

/* The configuration this plugin declares: one object naming the instance it was
 * resolved for. A payload whose `config` part is not these exact bytes is
 * refused with METIS_STATUS_INVALID_ARGUMENT, which is the "the configuration
 * does not match what the plugin declared" class. */
#define CPLUGIN_CONFIG_OBJECT "{\"instance\":\"cplugin\"}"
#define CPLUGIN_CONFIG_MARKER "\"config\":"

#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "metis_plugin_abi.h"

/* --- the ledger ---------------------------------------------------------- */

typedef struct {
    uint64_t allocs;
    uint64_t frees;
    uint64_t live;
    uint64_t double_frees;
    uint64_t bad_frees;
    uint64_t waker_clones;
    uint64_t waker_releases;
    uint64_t replies_after_drop;
    uint64_t calls;
} ledger_t;

static ledger_t g_ledger;

#define BUF_MAGIC 0x4d455449u /* "METI" */

/* Every plugin-owned buffer is this header followed by the payload. The magic
 * is cleared on release, so a second release of the same pointer is detectable
 * instead of being a double free. */
typedef struct {
    uint32_t magic;
    size_t cap;
} buf_hdr_t;

static MetisBuffer buf_from_body(const char *body) {
    size_t body_len = strlen(body);
    size_t total = 1 + body_len;
    buf_hdr_t *hdr = malloc(sizeof(buf_hdr_t) + total);
    if (hdr == NULL) {
        return (MetisBuffer){0, 0, 0, METIS_BUFFER_OWNER_NONE, 0};
    }
    hdr->magic = BUF_MAGIC;
    hdr->cap = total;
    uint8_t *bytes = (uint8_t *)(hdr + 1);
    /* The envelope: the format version in the low nibble, the codec bit above
     * it, reserved bits zero. The constants are the header's. */
    bytes[0] = (uint8_t)(METIS_PAYLOAD_FORMAT_VERSION | (METIS_CODEC_JSON << 4));
    memcpy(bytes + 1, body, body_len);
    g_ledger.allocs++;
    g_ledger.live++;
    MetisBuffer buffer;
    buffer.ptr = bytes;
    buffer.len = total;
    buffer.cap = total;
    buffer.owner = METIS_BUFFER_OWNER_PLUGIN;
    buffer.reserved = 0;
    return buffer;
}

/* --- capabilities -------------------------------------------------------- */

typedef enum {
    MODE_READY = 0,
    MODE_TIMER = 1,
    MODE_CANCEL = 2,
    MODE_DROP_IN_FLIGHT = 3,
    MODE_INITIALISING = 4
} mode_t;

#define MAX_CTX 32

typedef struct {
    uint64_t request_id;
    mode_t mode;
    int in_use;
    int issued;
    int answered;
    int cancelled;
    int init_ok;
    MetisStatus answer_status;
    MetisWaker waker;
    int has_waker;
    const struct MetisHostApiV1 *host;
} ctx_t;

typedef struct {
    const struct MetisHostApiV1 *host;
    ctx_t ctxs[MAX_CTX];
} instance_t;

static const char *CAPABILITY_NAMES[4] = {"c.ready", "c.timer", "c.cancel", "c.drop_in_flight"};

static const char *CAPABILITY_LIST =
    "{\"capabilities\":["
    "{\"name\":\"c.ready\",\"kind\":\"tool\",\"version\":1,"
    "\"request_schema\":{\"type\":\"object\"},\"response_schema\":{\"type\":\"object\"}},"
    "{\"name\":\"c.timer\",\"kind\":\"tool\",\"version\":1,"
    "\"request_schema\":{\"type\":\"object\"},\"response_schema\":{\"type\":\"object\"}},"
    "{\"name\":\"c.cancel\",\"kind\":\"tool\",\"version\":1,"
    "\"request_schema\":{\"type\":\"object\"},\"response_schema\":{\"type\":\"object\"}},"
    "{\"name\":\"c.drop_in_flight\",\"kind\":\"tool\",\"version\":1,"
    "\"request_schema\":{\"type\":\"object\"},\"response_schema\":{\"type\":\"object\"}}"
    "]}";

static ctx_t *ctx_find(instance_t *instance, uint64_t request_id) {
    for (size_t index = 0; index < MAX_CTX; index++) {
        ctx_t *ctx = &instance->ctxs[index];
        if (ctx->in_use && ctx->request_id == request_id) {
            return ctx;
        }
    }
    return NULL;
}

/* Declared here so the vtable exists before the entry points that hand it out.
 * The definitions follow the poll body below. */
static MetisStatus c_future_poll(void *ctx, MetisWaker waker, MetisBuffer *out_value);
static MetisStatus c_future_drop(void *ctx);
static const MetisFutureVtable FUTURE_VTABLE = {c_future_poll, c_future_drop};

/* Declared here because the initialisation entry, which is defined before the
 * envelope check and the configuration declaration, uses both. */
static MetisStatus request_is_valid(MetisBuffer payload);
static int declared_config_matches(const uint8_t *body, size_t len);

static ctx_t *ctx_take(instance_t *instance) {
    for (size_t index = 0; index < MAX_CTX; index++) {
        if (!instance->ctxs[index].in_use) {
            return &instance->ctxs[index];
        }
    }
    return NULL;
}

/* --- the waker ----------------------------------------------------------- */

static void waker_release(MetisWaker waker) {
    if (waker.vtable == NULL) {
        return;
    }
    /* `drop` consumes an owning waker without waking it. */
    waker.vtable->drop(waker);
    g_ledger.waker_releases++;
}

static void waker_wake_owned(MetisWaker waker) {
    if (waker.vtable == NULL) {
        return;
    }
    /* `wake` consumes an owning waker and schedules the task. */
    waker.vtable->wake(waker);
    g_ledger.waker_releases++;
}

static void ctx_park(ctx_t *ctx, MetisWaker borrowed) {
    if (ctx->has_waker || borrowed.vtable == NULL) {
        return;
    }
    /* The waker's liveness rule is written once, in docs/plugin-contract.md
     * section 8, and is read there rather than restated here. What this plugin
     * does with it: it keeps a clone of the waker the poll was handed and never
     * releases the borrowed base. */
    ctx->waker = borrowed.vtable->clone(&borrowed);
    ctx->has_waker = 1;
    g_ledger.waker_clones++;
}

static void ctx_release_waker(ctx_t *ctx, int wake) {
    if (!ctx->has_waker) {
        return;
    }
    MetisWaker waker = ctx->waker;
    ctx->has_waker = 0;
    if (wake) {
        waker_wake_owned(waker);
    } else {
        waker_release(waker);
    }
}

/* --- the table entries --------------------------------------------------- */

static MetisStatus c_release_buffer(MetisBuffer buffer) {
    if (buffer.owner != METIS_BUFFER_OWNER_PLUGIN) {
        g_ledger.bad_frees++;
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    if (buffer.ptr == NULL) {
        return METIS_STATUS_OK;
    }
    buf_hdr_t *hdr = ((buf_hdr_t *)buffer.ptr) - 1;
    if (hdr->magic != BUF_MAGIC) {
        /* Already released, or never ours. Either way: not a second free. */
        g_ledger.double_frees++;
        return METIS_STATUS_NOT_FOUND;
    }
    hdr->magic = 0;
    if (g_ledger.live > 0) {
        g_ledger.live--;
    }
    g_ledger.frees++;
    free(hdr);
    return METIS_STATUS_OK;
}

/* Constructs and initialises one instance, and hands out the future that
 * completes the initialisation. The instance pointer is valid immediately and
 * its initialisation is valid exactly when that future resolved `Ok`; on any
 * other resolution the host drops the future and then destroys the pointer,
 * which this function tolerates because there is nothing to unwind. */
static MetisStatus c_init_plugin(const struct MetisHostApiV1 *host, MetisBuffer payload, void **out_instance,
                                 MetisFutureHandle *out_future) {
    if (host == NULL || out_instance == NULL || out_future == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    MetisStatus status = request_is_valid(payload);
    if (status != METIS_STATUS_OK) {
        return status;
    }
    instance_t *instance = calloc(1, sizeof(instance_t));
    if (instance == NULL) {
        return METIS_STATUS_INTERNAL;
    }
    instance->host = host;
    /* The payload is host-owned and borrowed for this call, so what is read
     * here is copied into the future's context rather than kept. */
    int matches = declared_config_matches(payload.ptr + 1, payload.len - 1);
    ctx_t *ctx = ctx_take(instance);
    if (ctx == NULL) {
        free(instance);
        return METIS_STATUS_INTERNAL;
    }
    memset(ctx, 0, sizeof(*ctx));
    ctx->in_use = 1;
    ctx->mode = MODE_INITIALISING;
    ctx->init_ok = matches;
    ctx->host = host;
    *out_instance = instance;
    out_future->ctx = ctx;
    out_future->vtable = &FUTURE_VTABLE;
    return METIS_STATUS_OK;
}

static MetisStatus c_destroy_instance(void *instance) {
    if (instance == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    instance_t *state = (instance_t *)instance;
    for (size_t index = 0; index < MAX_CTX; index++) {
        if (state->ctxs[index].in_use) {
            ctx_release_waker(&state->ctxs[index], 0);
        }
    }
    free(state);
    return METIS_STATUS_OK;
}

static MetisStatus c_list_capabilities(void *instance, MetisBuffer *out_payload) {
    if (instance == NULL || out_payload == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    *out_payload = buf_from_body(CAPABILITY_LIST);
    if (out_payload->ptr == NULL) {
        return METIS_STATUS_INTERNAL;
    }
    return METIS_STATUS_OK;
}

/* Validates the envelope and copies nothing: the request payload is host-owned
 * and valid for the call only, which is the rule the contract states. */
static MetisStatus request_is_valid(MetisBuffer payload) {
    if (payload.owner != METIS_BUFFER_OWNER_HOST || payload.ptr == NULL || payload.len < 1) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    uint8_t header = payload.ptr[0];
    if ((header & METIS_PAYLOAD_RESERVED_MASK) != 0) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    if ((header & METIS_PAYLOAD_VERSION_MASK) != METIS_PAYLOAD_FORMAT_VERSION) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    if (((header & METIS_PAYLOAD_CODEC_BIT) >> 4) != METIS_CODEC_JSON) {
        return METIS_STATUS_UNSUPPORTED;
    }
    return METIS_STATUS_OK;
}

/* Whether the payload's `config` part is the configuration object this plugin
 * declares. The body is the JSON after the envelope byte, and the marker is
 * searched for rather than parsed: a hand-written C plugin has no JSON reader,
 * and the declaration it checks against is a fixed byte string. */
static int declared_config_matches(const uint8_t *body, size_t len) {
    static const char marker[] = CPLUGIN_CONFIG_MARKER;
    static const char declared[] = CPLUGIN_CONFIG_OBJECT;
    const size_t marker_len = sizeof(marker) - 1;
    const size_t declared_len = sizeof(declared) - 1;
    if (len < marker_len + declared_len) {
        return 0;
    }
    for (size_t at = 0; at + marker_len + declared_len <= len; at++) {
        if (memcmp(body + at, marker, marker_len) != 0) {
            continue;
        }
        return memcmp(body + at + marker_len, declared, declared_len) == 0;
    }
    return 0;
}

static MetisStatus mode_for(const uint8_t *capability, size_t capability_len, mode_t *mode) {
    for (int index = 0; index < 4; index++) {
        const char *name = CAPABILITY_NAMES[index];
        if (strlen(name) == capability_len && memcmp(name, capability, capability_len) == 0) {
            *mode = (mode_t)index;
            return METIS_STATUS_OK;
        }
    }
    return METIS_STATUS_NOT_FOUND;
}

/* The one event this plugin emits, from its ready capability.
 *
 * A plugin-originated event is the plugin's own buffer, and the host releases
 * it through this plugin's `release_buffer` entry once it has copied it, which
 * is why the ledger below balances. The `emit_event` entry is present only when
 * the host table's `size` covers it, the same rule `issue_timer` applies to
 * `service_request`. */
static void emit_ready_event(const struct MetisHostApiV1 *host) {
    static const char topic[] = "cplugin.ready";
    if (host == NULL ||
        host->size < offsetof(struct MetisHostApiV1, emit_event) + sizeof(void *)) {
        return;
    }
    MetisBuffer payload = buf_from_body("{\"ready\":true}");
    if (payload.ptr == NULL) {
        return;
    }
    host->emit_event(host->user_data, (const uint8_t *)topic, sizeof(topic) - 1, payload);
}

static MetisStatus issue_timer(ctx_t *ctx) {
    const char *body = "{\"delay_ms\":20}";
    MetisBuffer payload = buf_from_body(body);
    if (payload.ptr == NULL) {
        return METIS_STATUS_INTERNAL;
    }
    const char *service = "timer";
    /* The `service_request` entry is present only when the host table's `size`
     * covers it: presence is decided by size, never by a pointer. The header's
     * slot constant is what makes the same rule expressible here. */
    if (ctx->host == NULL ||
        ctx->host->size < offsetof(struct MetisHostApiV1, service_request) + sizeof(void *)) {
        /* The host built without the route, so release the buffer we just made
         * and refuse rather than calling through a table that does not have it. */
        c_release_buffer(payload);
        return METIS_STATUS_UNSUPPORTED;
    }
    ctx->issued = 1;
    return ctx->host->service_request(ctx->host->user_data, (const uint8_t *)service, strlen(service),
                                      ctx->request_id, payload);
}

static MetisStatus c_invoke_future(void *instance, const uint8_t *capability, size_t capability_len,
                                   uint64_t request_id, MetisBuffer payload,
                                   const struct MetisHostApiV1 *host, MetisFutureHandle *out) {
    g_ledger.calls++;
    if (instance == NULL || capability == NULL || out == NULL || host == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    mode_t mode = MODE_READY;
    MetisStatus status = mode_for(capability, capability_len, &mode);
    if (status != METIS_STATUS_OK) {
        return status;
    }
    status = request_is_valid(payload);
    if (status != METIS_STATUS_OK) {
        return status;
    }
    instance_t *state = (instance_t *)instance;
    ctx_t *ctx = ctx_take(state);
    if (ctx == NULL) {
        return METIS_STATUS_INTERNAL;
    }
    memset(ctx, 0, sizeof(*ctx));
    ctx->in_use = 1;
    ctx->request_id = request_id;
    ctx->mode = mode;
    ctx->host = host;
    out->ctx = ctx;
    out->vtable = &FUTURE_VTABLE;
    return METIS_STATUS_OK;
}

static MetisStatus poll_body(ctx_t *ctx, MetisWaker waker, MetisBuffer *out_value) {
    switch (ctx->mode) {
    case MODE_READY: {
        /* The event goes out before the response, the way a capability that has
         * something to say says it while it is still running. */
        emit_ready_event(ctx->host);
        *out_value = buf_from_body("{\"echo\":\"c.ready\",\"bytes\":5}");
        return out_value->ptr == NULL ? METIS_STATUS_INTERNAL : METIS_STATUS_OK;
    }
    case MODE_TIMER: {
        if (!ctx->issued) {
            MetisStatus status = issue_timer(ctx);
            if (status != METIS_STATUS_OK) {
                return status;
            }
            ctx_park(ctx, waker);
            return METIS_STATUS_PENDING;
        }
        if (ctx->answered) {
            *out_value = buf_from_body("{\"echo\":\"c.timer\",\"bytes\":5}");
            return out_value->ptr == NULL ? METIS_STATUS_INTERNAL : METIS_STATUS_OK;
        }
        return METIS_STATUS_PENDING;
    }
    case MODE_CANCEL: {
        if (ctx->cancelled) {
            return METIS_STATUS_CANCELLED;
        }
        ctx_park(ctx, waker);
        return METIS_STATUS_PENDING;
    }
    case MODE_INITIALISING: {
        /* The one poll that completes initialisation. Nothing here blocks or
         * waits: the configuration is already checked, and this is where the
         * verdict is handed over. */
        ctx->answered = 1;
        return ctx->init_ok ? METIS_STATUS_OK : METIS_STATUS_INVALID_ARGUMENT;
    }
    case MODE_DROP_IN_FLIGHT: {
        if (!ctx->issued) {
            MetisStatus status = issue_timer(ctx);
            if (status != METIS_STATUS_OK) {
                return status;
            }
            ctx_park(ctx, waker);
            return METIS_STATUS_PENDING;
        }
        *out_value = buf_from_body("{\"echo\":\"c.drop_in_flight\",\"bytes\":5}");
        return out_value->ptr == NULL ? METIS_STATUS_INTERNAL : METIS_STATUS_OK;
    }
    }
    return METIS_STATUS_INTERNAL;
}

static MetisStatus c_future_poll(void *ctx, MetisWaker waker, MetisBuffer *out_value) {
    if (ctx == NULL || out_value == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    return poll_body((ctx_t *)ctx, waker, out_value);
}

static MetisStatus c_future_drop(void *ctx) {
    if (ctx == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    ctx_t *state = (ctx_t *)ctx;
    ctx_release_waker(state, 0);
    state->in_use = 0;
    return METIS_STATUS_OK;
}

static MetisStatus c_cancel(void *instance, uint64_t request_id) {
    if (instance == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    ctx_t *ctx = ctx_find((instance_t *)instance, request_id);
    if (ctx == NULL) {
        return METIS_STATUS_NOT_FOUND;
    }
    ctx->cancelled = 1;
    ctx_release_waker(ctx, 1);
    return METIS_STATUS_OK;
}

static MetisStatus c_service_complete(void *instance, uint64_t request_id, MetisStatus status,
                                      MetisBuffer payload) {
    if (instance == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    (void)payload; /* Host-owned and borrowed: the plugin never frees it. */
    ctx_t *ctx = ctx_find((instance_t *)instance, request_id);
    if (ctx == NULL) {
        /* The host answered a service for a future it already dropped. That is
         * a recorded event, not a dangling dereference. */
        g_ledger.replies_after_drop++;
        return METIS_STATUS_NOT_FOUND;
    }
    ctx->answered = 1;
    ctx->answer_status = status;
    ctx_release_waker(ctx, 1);
    return METIS_STATUS_OK;
}

static MetisStatus c_shutdown(void *instance) {
    if (instance == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    instance_t *state = (instance_t *)instance;
    if (state->host != NULL) {
        char line[256];
        int written = snprintf(line, sizeof(line),
                               "CPLUGIN_ACCOUNT allocs=%llu frees=%llu live=%llu double_frees=%llu "
                               "bad_frees=%llu waker_clones=%llu waker_releases=%llu replies_after_drop=%llu",
                               (unsigned long long)g_ledger.allocs, (unsigned long long)g_ledger.frees,
                               (unsigned long long)g_ledger.live, (unsigned long long)g_ledger.double_frees,
                               (unsigned long long)g_ledger.bad_frees,
                               (unsigned long long)g_ledger.waker_clones,
                               (unsigned long long)g_ledger.waker_releases,
                               (unsigned long long)g_ledger.replies_after_drop);
        if (written > 0) {
            state->host->log(state->host->user_data, METIS_LOG_INFO, (const uint8_t *)line,
                             (size_t)written);
        }
    }
    return METIS_STATUS_OK;
}

/* --- the exported table -------------------------------------------------- */

static const MetisPluginTableV1 TABLE = {
    sizeof(MetisPluginTableV1),
    METIS_ABI_MAJOR,
    METIS_ABI_MINOR,
    c_init_plugin,
    c_destroy_instance,
    c_list_capabilities,
    c_invoke_future,
    c_cancel,
    c_release_buffer,
    c_shutdown,
    c_service_complete,
};

/* The one exported symbol, and the table it hands out. */
const MetisPluginTableV1 *metis_plugin_entry_v1(void) {
    return &TABLE;
}
