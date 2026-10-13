/*
 * PROTOTYPE (throwaway). Not product code.
 *
 * Lane "ownership": a plugin implemented by hand in C, against the committed
 * cbindgen header `crates/proto-abi/include/metis_plugin_abi.h`. No Rust, no
 * Rust glue, no Rust async machinery. It exists to answer one question: is the
 * written ownership rule enough for a non-Rust implementer, and can such an
 * implementer drive the seam end to end?
 *
 * What it implements, all of it by hand:
 *
 *   - the exported entry point `metis_plugin_entry_v1`;
 *   - the plugin table, every landed entry plus the two appended ones;
 *   - `invoke_future` for three capabilities:
 *       `c.echo`        ready on the first poll, one plugin-allocated buffer;
 *       `c.timer`       issues `service_request("timer", ...)`, keeps a cloned
 *                       host waker, and answers when the host calls
 *                       `service_complete`;
 *       `c.accounting`  ready, returns this file's own counters as JSON;
 *   - the future `poll` function, the future `drop` function, and the use of
 *     the host's waker vtable (`clone`, `wake`, `drop`);
 *   - `service_complete`, called by the host's timer thread.
 *
 * It counts every plugin allocation and every free, distinguishes them by the
 * category the buffer was handed out for, detects double frees and releases of
 * pointers it never allocated, and reports the totals through the seam and
 * through a ledger file at shutdown.
 *
 * Build (inside `devenv shell`):
 *
 *   gcc -std=c11 -Wall -Wextra -Werror -O2 -fPIC -shared -pthread \
 *       -I crates/proto-abi/include \
 *       -o c-ownership/libmetis_c_plugin.so c-ownership/c_plugin.c
 */

#include "metis_plugin_abi.h"

#include <inttypes.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* --- configuration the implementer had to invent ------------------------- */

/* The payload envelope byte: version 1 in the low nibble, JSON codec bit clear.
 * The committed header has no constant, type or comment for it, so the C
 * implementer hard-codes 0x01 here and learned it from the host's reader. */
#define C_ENVELOPE_BYTE 0x01u

/* Slot count for in-flight requests. The ABI states no maximum, so the
 * implementer picked one. */
#define C_MAX_SLOTS 8
/* Live-allocation table size, for the accounting. */
#define C_MAX_LIVE 256
/* Recently freed pointers, so a double free is reported as a double free and
 * not as an unknown pointer. */
#define C_FREED_RING 16

/* --- the plugin's own allocator and its counters -------------------------- */

enum c_buf_tag {
    C_TAG_READY = 1,   /* handed out through `out_value` */
    C_TAG_SERVICE = 2, /* handed to `service_request` */
    C_TAG_LEGACY = 3,  /* handed out by `list_capabilities` */
};

struct c_hdr {
    uint64_t magic;
    uint64_t tag;
    uint64_t len;
};

#define C_HDR_MAGIC 0x6d657469735f6331ULL /* "metis_c1" */

static struct c_hdr *c_hdr_of(uint8_t *ptr) { return ((struct c_hdr *)ptr) - 1; }

static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;

static _Atomic uint64_t g_allocs = 0;
static _Atomic uint64_t g_frees = 0;
static _Atomic uint64_t g_alloc_failures = 0;
static _Atomic uint64_t g_double_frees = 0;
static _Atomic uint64_t g_foreign_releases = 0;
static _Atomic uint64_t g_unknown_releases = 0;
static _Atomic uint64_t g_live_overflow = 0;
static _Atomic uint64_t g_ready_buffers = 0;
static _Atomic uint64_t g_service_payloads = 0;
static _Atomic uint64_t g_legacy_buffers = 0;
static _Atomic uint64_t g_frees_ready = 0;
static _Atomic uint64_t g_frees_service = 0;
static _Atomic uint64_t g_frees_legacy = 0;
static _Atomic uint64_t g_late_completions = 0;
static _Atomic uint64_t g_service_roundtrips = 0;
static _Atomic uint64_t g_service_request_failures = 0;
static _Atomic uint64_t g_wake_calls = 0;
static _Atomic uint64_t g_waker_clones = 0;
static _Atomic uint64_t g_waker_releases = 0;
static _Atomic uint64_t g_polls = 0;
static _Atomic uint64_t g_polls_ready = 0;
static _Atomic uint64_t g_futures_created = 0;
static _Atomic uint64_t g_futures_freed = 0;
static _Atomic uint64_t g_futures_dropped = 0;
static _Atomic uint64_t g_double_drops = 0;
static _Atomic uint64_t g_cancels = 0;
static _Atomic uint64_t g_cancel_resolutions = 0;
static _Atomic uint64_t g_host_owned_service_payloads = 0;
static _Atomic uint64_t g_plugin_owned_service_payloads = 0;
static _Atomic uint64_t g_cross_thread_completions = 0;
static _Atomic uint64_t g_bad_envelope_in = 0;
static _Atomic uint64_t g_legacy_invoke_calls = 0;
static _Atomic uint64_t g_instances_created = 0;
static _Atomic uint64_t g_guard_tests = 0;

static uint64_t g_create_host_size = 0;
static uint64_t g_service_host_size = 0;
static uint64_t g_instance_serial = 0;
static pthread_t g_poll_thread;
static int g_poll_thread_set = 0;

static void c_lock(void) {
    int rc = pthread_mutex_lock(&g_lock);
    (void)rc;
}

static void c_unlock(void) {
    int rc = pthread_mutex_unlock(&g_lock);
    (void)rc;
}

/* The live set and the freed ring are only touched under `g_lock`. */
static uint8_t *g_live[C_MAX_LIVE];
static size_t g_live_count = 0;
static uint8_t *g_freed[C_FREED_RING];
static size_t g_freed_next = 0;

static int live_insert(uint8_t *ptr) {
    if (g_live_count == C_MAX_LIVE) {
        atomic_fetch_add(&g_live_overflow, 1);
        return 0;
    }
    g_live[g_live_count++] = ptr;
    return 1;
}

static int live_remove(uint8_t *ptr) {
    for (size_t i = 0; i < g_live_count; i++) {
        if (g_live[i] == ptr) {
            g_live[i] = g_live[g_live_count - 1];
            g_live_count--;
            return 1;
        }
    }
    return 0;
}

static int freed_contains(uint8_t *ptr) {
    for (size_t i = 0; i < C_FREED_RING; i++) {
        if (g_freed[i] == ptr) {
            return 1;
        }
    }
    return 0;
}

static void freed_push(uint8_t *ptr) {
    g_freed[g_freed_next] = ptr;
    g_freed_next = (g_freed_next + 1) % C_FREED_RING;
}

static struct MetisBuffer c_empty(void) {
    struct MetisBuffer buffer;
    buffer.ptr = NULL;
    buffer.len = 0;
    buffer.cap = 0;
    buffer.owner = METIS_BUFFER_OWNER_NONE;
    buffer.reserved = 0;
    return buffer;
}

/* The plugin's only allocation entry. The returned pointer is owned by whoever
 * the caller hands the buffer to, per the ownership clause. */
static struct MetisBuffer c_alloc(const void *bytes, size_t len, enum c_buf_tag tag) {
    struct MetisBuffer buffer = c_empty();
    if (len == 0) {
        return buffer;
    }
    struct c_hdr *hdr = malloc(sizeof *hdr + len);
    if (hdr == NULL) {
        atomic_fetch_add(&g_alloc_failures, 1);
        return buffer;
    }
    uint8_t *data = (uint8_t *)(hdr + 1);
    hdr->magic = C_HDR_MAGIC;
    hdr->tag = (uint64_t)tag;
    hdr->len = (uint64_t)len;
    memcpy(data, bytes, len);
    buffer.ptr = data;
    buffer.len = len;
    buffer.cap = len;
    buffer.owner = METIS_BUFFER_OWNER_PLUGIN;
    c_lock();
    live_insert(data);
    c_unlock();
    atomic_fetch_add(&g_allocs, 1);
    switch (tag) {
    case C_TAG_READY: atomic_fetch_add(&g_ready_buffers, 1); break;
    case C_TAG_SERVICE: atomic_fetch_add(&g_service_payloads, 1); break;
    case C_TAG_LEGACY: atomic_fetch_add(&g_legacy_buffers, 1); break;
    }
    return buffer;
}

/* A payload body wrapped in the envelope the host's reader expects. */
static struct MetisBuffer c_reply(const char *body, size_t body_len, enum c_buf_tag tag) {
    unsigned char scratch[1024];
    if (body_len + 1 > sizeof scratch) {
        atomic_fetch_add(&g_alloc_failures, 1);
        return c_empty();
    }
    scratch[0] = C_ENVELOPE_BYTE;
    memcpy(scratch + 1, body, body_len);
    return c_alloc(scratch, body_len + 1, tag);
}

/* --- the plugin's `release_buffer`, which is the whole ownership proof ----- */

static enum MetisStatus c_release(struct MetisBuffer buffer) {
    if (buffer.owner != METIS_BUFFER_OWNER_PLUGIN) {
        /* A host-owned or empty buffer must never reach this parser. */
        atomic_fetch_add(&g_foreign_releases, 1);
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    if (buffer.ptr == NULL) {
        return METIS_STATUS_OK;
    }
    c_lock();
    int found = live_remove(buffer.ptr);
    int was_freed = found ? 0 : freed_contains(buffer.ptr);
    if (!found) {
        freed_push(buffer.ptr);
    }
    c_unlock();
    if (!found) {
        if (was_freed) {
            atomic_fetch_add(&g_double_frees, 1);
        } else {
            atomic_fetch_add(&g_unknown_releases, 1);
        }
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    struct c_hdr *hdr = c_hdr_of(buffer.ptr);
    uint64_t tag = hdr->tag;
    hdr->magic = 0;
    free(hdr);
    atomic_fetch_add(&g_frees, 1);
    switch (tag) {
    case C_TAG_READY: atomic_fetch_add(&g_frees_ready, 1); break;
    case C_TAG_SERVICE: atomic_fetch_add(&g_frees_service, 1); break;
    case C_TAG_LEGACY: atomic_fetch_add(&g_frees_legacy, 1); break;
    default: break;
    }
    return METIS_STATUS_OK;
}

/* --- instances, slots and futures ---------------------------------------- */

struct c_future;

struct c_slot {
    uint64_t request_id;
    struct c_future *ctx;
};

struct c_instance {
    const struct MetisHostApiV1 *create_host;
    size_t create_host_size;
    uint64_t serial;
    int alive;
    struct c_slot slots[C_MAX_SLOTS];
};

enum c_kind { C_READY = 0, C_TIMER = 1, C_ACCOUNTING = 2 };
enum c_state { C_ARMED = 0, C_DONE = 1, C_CANCELLED = 2, C_DROPPED = 3 };

struct c_future {
    struct c_instance *instance;
    uint64_t request_id;
    enum c_kind kind;
    const struct MetisHostApiV1 *service_host;
    size_t service_host_size;
    long long x;
    uint64_t delay_ms;
    int refs;
    enum c_state state;
    int waker_present;
    int guard_status;
    struct MetisWaker waker;
    uint64_t polls;
    int issued;
};

static struct MetisWaker c_null_waker(void) {
    struct MetisWaker waker;
    waker.data = NULL;
    waker.vtable = NULL;
    return waker;
}

/* --- tiny JSON field reader (the request body is not NUL terminated) ------ */

static long long json_int_field(const char *body, size_t len, const char *key, long long fallback) {
    size_t key_len = strlen(key);
    for (size_t i = 0; i + key_len + 2 <= len; i++) {
        if (body[i] != '"') {
            continue;
        }
        if (memcmp(body + i + 1, key, key_len) != 0) {
            continue;
        }
        size_t j = i + 1 + key_len;
        if (j >= len || body[j] != '"') {
            continue;
        }
        j++;
        while (j < len && (body[j] == ' ' || body[j] == ':')) {
            j++;
        }
        long long value = 0;
        int any = 0;
        while (j < len && body[j] >= '0' && body[j] <= '9') {
            value = value * 10 + (body[j] - '0');
            j++;
            any = 1;
        }
        if (any) {
            return value;
        }
    }
    return fallback;
}

static int name_is(const uint8_t *name, size_t len, const char *literal) {
    size_t literal_len = strlen(literal);
    return len == literal_len && memcmp(name, literal, literal_len) == 0;
}

/* --- the ledger file, which survives the library being unloaded ----------- */

static void c_append(char *buf, size_t cap, size_t *used, const char *fmt, ...)
#if defined(__GNUC__)
    __attribute__((format(printf, 4, 5)))
#endif
    ;

static void c_append(char *buf, size_t cap, size_t *used, const char *fmt, ...) {
    if (*used >= cap) {
        return;
    }
    va_list args;
    va_start(args, fmt);
    int written = vsnprintf(buf + *used, cap - *used, fmt, args);
    va_end(args);
    if (written < 0) {
        return;
    }
    size_t room = cap - *used;
    if ((size_t)written >= room) {
        *used = cap;
    } else {
        *used += (size_t)written;
    }
}

/* The line the driver reads back after unload. `<64>` is a fixed width so the
 * numbers line up; not all fields matter to every check. */
static void c_write_ledger(const char *event) {
    char line[2048];
    size_t used = 0;
    const char *nonce = getenv("C_PLUGIN_RUN_NONCE");
    c_append(line, sizeof line, &used, "run=%s event=%s", nonce ? nonce : "(none)", event);
    c_append(line, sizeof line, &used, " allocs=%" PRIu64, atomic_load(&g_allocs));
    c_append(line, sizeof line, &used, " frees=%" PRIu64, atomic_load(&g_frees));
    c_append(line, sizeof line, &used, " live=%zu", g_live_count);
    c_append(line, sizeof line, &used, " double_frees=%" PRIu64, atomic_load(&g_double_frees));
    c_append(line, sizeof line, &used, " foreign_releases=%" PRIu64,
             atomic_load(&g_foreign_releases));
    c_append(line, sizeof line, &used, " unknown_releases=%" PRIu64,
             atomic_load(&g_unknown_releases));
    c_append(line, sizeof line, &used, " ready=%" PRIu64, atomic_load(&g_ready_buffers));
    c_append(line, sizeof line, &used, " service_payloads=%" PRIu64,
             atomic_load(&g_service_payloads));
    c_append(line, sizeof line, &used, " frees_ready=%" PRIu64, atomic_load(&g_frees_ready));
    c_append(line, sizeof line, &used, " frees_service=%" PRIu64, atomic_load(&g_frees_service));
    c_append(line, sizeof line, &used, " frees_legacy=%" PRIu64, atomic_load(&g_frees_legacy));
    c_append(line, sizeof line, &used, " late_completions=%" PRIu64,
             atomic_load(&g_late_completions));
    c_append(line, sizeof line, &used, " service_roundtrips=%" PRIu64,
             atomic_load(&g_service_roundtrips));
    c_append(line, sizeof line, &used, " wake_calls=%" PRIu64, atomic_load(&g_wake_calls));
    c_append(line, sizeof line, &used, " waker_clones=%" PRIu64, atomic_load(&g_waker_clones));
    c_append(line, sizeof line, &used, " waker_releases=%" PRIu64,
             atomic_load(&g_waker_releases));
    c_append(line, sizeof line, &used, " futures_created=%" PRIu64,
             atomic_load(&g_futures_created));
    c_append(line, sizeof line, &used, " futures_freed=%" PRIu64, atomic_load(&g_futures_freed));
    c_append(line, sizeof line, &used, " futures_dropped=%" PRIu64,
             atomic_load(&g_futures_dropped));
    c_append(line, sizeof line, &used, " double_drops=%" PRIu64, atomic_load(&g_double_drops));
    c_append(line, sizeof line, &used, " polls=%" PRIu64, atomic_load(&g_polls));
    c_append(line, sizeof line, &used, " cross_thread_completions=%" PRIu64,
             atomic_load(&g_cross_thread_completions));
    c_append(line, sizeof line, &used, " instances=%" PRIu64, atomic_load(&g_instances_created));
    c_append(line, sizeof line, &used, " guard_tests=%" PRIu64, atomic_load(&g_guard_tests));
    line[sizeof line - 1] = '\0';

    fprintf(stderr, "c-plugin: %s\n", line);
    fflush(stderr);

    const char *path = getenv("C_PLUGIN_LEDGER");
    if (path == NULL) {
        return;
    }
    FILE *file = fopen(path, "a");
    if (file == NULL) {
        return;
    }
    fputs(line, file);
    fputc('\n', file);
    fflush(file);
    fclose(file);
}

/* --- the counters as a payload ------------------------------------------- */

static struct MetisBuffer c_accounting_buffer(void) {
    /* Snapshot first: the reply this function allocates must not be part of the
     * numbers it reports. */
    uint64_t allocs = atomic_load(&g_allocs);
    uint64_t frees = atomic_load(&g_frees);
    char body[1400];
    size_t used = 0;
    c_lock();
    size_t live = g_live_count;
    c_unlock();
    c_append(body, sizeof body, &used, "{\"impl\":\"c\"");
    c_append(body, sizeof body, &used, ",\"allocs\":%" PRIu64, allocs);
    c_append(body, sizeof body, &used, ",\"frees\":%" PRIu64, frees);
    c_append(body, sizeof body, &used, ",\"live\":%zu", live);
    c_append(body, sizeof body, &used, ",\"double_frees\":%" PRIu64,
             atomic_load(&g_double_frees));
    c_append(body, sizeof body, &used, ",\"foreign_releases\":%" PRIu64,
             atomic_load(&g_foreign_releases));
    c_append(body, sizeof body, &used, ",\"unknown_releases\":%" PRIu64,
             atomic_load(&g_unknown_releases));
    c_append(body, sizeof body, &used, ",\"alloc_failures\":%" PRIu64,
             atomic_load(&g_alloc_failures));
    c_append(body, sizeof body, &used, ",\"live_overflow\":%" PRIu64,
             atomic_load(&g_live_overflow));
    c_append(body, sizeof body, &used, ",\"ready_buffers\":%" PRIu64,
             atomic_load(&g_ready_buffers));
    c_append(body, sizeof body, &used, ",\"service_payloads\":%" PRIu64,
             atomic_load(&g_service_payloads));
    c_append(body, sizeof body, &used, ",\"legacy_buffers\":%" PRIu64,
             atomic_load(&g_legacy_buffers));
    c_append(body, sizeof body, &used, ",\"frees_ready\":%" PRIu64,
             atomic_load(&g_frees_ready));
    c_append(body, sizeof body, &used, ",\"frees_service\":%" PRIu64,
             atomic_load(&g_frees_service));
    c_append(body, sizeof body, &used, ",\"frees_legacy\":%" PRIu64,
             atomic_load(&g_frees_legacy));
    c_append(body, sizeof body, &used, ",\"late_completions\":%" PRIu64,
             atomic_load(&g_late_completions));
    c_append(body, sizeof body, &used, ",\"service_roundtrips\":%" PRIu64,
             atomic_load(&g_service_roundtrips));
    c_append(body, sizeof body, &used, ",\"service_request_failures\":%" PRIu64,
             atomic_load(&g_service_request_failures));
    c_append(body, sizeof body, &used, ",\"wake_calls\":%" PRIu64, atomic_load(&g_wake_calls));
    c_append(body, sizeof body, &used, ",\"waker_clones\":%" PRIu64,
             atomic_load(&g_waker_clones));
    c_append(body, sizeof body, &used, ",\"waker_releases\":%" PRIu64,
             atomic_load(&g_waker_releases));
    c_append(body, sizeof body, &used, ",\"polls\":%" PRIu64, atomic_load(&g_polls));
    c_append(body, sizeof body, &used, ",\"polls_ready\":%" PRIu64,
             atomic_load(&g_polls_ready));
    c_append(body, sizeof body, &used, ",\"futures_created\":%" PRIu64,
             atomic_load(&g_futures_created));
    c_append(body, sizeof body, &used, ",\"futures_freed\":%" PRIu64,
             atomic_load(&g_futures_freed));
    c_append(body, sizeof body, &used, ",\"futures_dropped\":%" PRIu64,
             atomic_load(&g_futures_dropped));
    c_append(body, sizeof body, &used, ",\"double_drops\":%" PRIu64,
             atomic_load(&g_double_drops));
    c_append(body, sizeof body, &used, ",\"cancels\":%" PRIu64, atomic_load(&g_cancels));
    c_append(body, sizeof body, &used, ",\"cancel_resolutions\":%" PRIu64,
             atomic_load(&g_cancel_resolutions));
    c_append(body, sizeof body, &used, ",\"host_owned_service_payloads\":%" PRIu64,
             atomic_load(&g_host_owned_service_payloads));
    c_append(body, sizeof body, &used, ",\"plugin_owned_service_payloads\":%" PRIu64,
             atomic_load(&g_plugin_owned_service_payloads));
    c_append(body, sizeof body, &used, ",\"cross_thread_completions\":%" PRIu64,
             atomic_load(&g_cross_thread_completions));
    c_append(body, sizeof body, &used, ",\"bad_envelope_in\":%" PRIu64,
             atomic_load(&g_bad_envelope_in));
    c_append(body, sizeof body, &used, ",\"legacy_invoke_calls\":%" PRIu64,
             atomic_load(&g_legacy_invoke_calls));
    c_append(body, sizeof body, &used, ",\"instances\":%" PRIu64,
             atomic_load(&g_instances_created));
    c_append(body, sizeof body, &used, ",\"guard_tests\":%" PRIu64,
             atomic_load(&g_guard_tests));
    c_append(body, sizeof body, &used, ",\"create_host_size\":%" PRIu64, g_create_host_size);
    c_append(body, sizeof body, &used, ",\"service_host_size\":%" PRIu64, g_service_host_size);
    c_append(body, sizeof body, &used, ",\"table_size\":%zu",
             sizeof(struct MetisPluginTableV1));
    c_append(body, sizeof body, &used, "}");
    if (used >= sizeof body) {
        const char *truncated = "{\"impl\":\"c\",\"truncated\":true}";
        return c_reply(truncated, strlen(truncated), C_TAG_READY);
    }
    return c_reply(body, used, C_TAG_READY);
}

/* --- slots ---------------------------------------------------------------- */

/* Callers hold `g_lock`. */
static struct c_future *slot_find(struct c_instance *instance, uint64_t request_id) {
    for (size_t i = 0; i < C_MAX_SLOTS; i++) {
        if (instance->slots[i].ctx != NULL && instance->slots[i].request_id == request_id) {
            return instance->slots[i].ctx;
        }
    }
    return NULL;
}

/* Takes the slot out and hands its reference to the caller. */
static struct c_future *slot_take(struct c_instance *instance, uint64_t request_id) {
    for (size_t i = 0; i < C_MAX_SLOTS; i++) {
        if (instance->slots[i].ctx != NULL && instance->slots[i].request_id == request_id) {
            struct c_future *ctx = instance->slots[i].ctx;
            instance->slots[i].ctx = NULL;
            instance->slots[i].request_id = 0;
            return ctx;
        }
    }
    return NULL;
}

/* Callers hold `g_lock`. Returns 1 when the slot's reference was released. */
static int slot_remove_ctx(struct c_instance *instance, struct c_future *ctx) {
    for (size_t i = 0; i < C_MAX_SLOTS; i++) {
        if (instance->slots[i].ctx == ctx) {
            instance->slots[i].ctx = NULL;
            instance->slots[i].request_id = 0;
            return 1;
        }
    }
    return 0;
}

/* Callers hold `g_lock`. Takes a reference into the slot. */
static int slot_insert(struct c_instance *instance, uint64_t request_id, struct c_future *ctx) {
    for (size_t i = 0; i < C_MAX_SLOTS; i++) {
        if (instance->slots[i].ctx == NULL) {
            instance->slots[i].ctx = ctx;
            instance->slots[i].request_id = request_id;
            return 1;
        }
    }
    return 0;
}

static void c_ctx_unref(struct c_future *ctx) {
    int free_it = 0;
    c_lock();
    ctx->refs -= 1;
    if (ctx->refs == 0) {
        free_it = 1;
    }
    c_unlock();
    if (free_it) {
        atomic_fetch_add(&g_futures_freed, 1);
        free(ctx);
    }
}

/* --- the future vtable ---------------------------------------------------- */

static void c_record_poll_thread(void) {
    c_lock();
    if (!g_poll_thread_set) {
        g_poll_thread = pthread_self();
        g_poll_thread_set = 1;
    }
    c_unlock();
}

static void c_record_completion_thread(void) {
    c_lock();
    int set = g_poll_thread_set;
    pthread_t poll_thread = g_poll_thread;
    c_unlock();
    if (set && !pthread_equal(poll_thread, pthread_self())) {
        atomic_fetch_add(&g_cross_thread_completions, 1);
    }
}

static enum MetisStatus c_poll(void *ctx_ptr, struct MetisWaker waker, struct MetisBuffer *out_value) {
    if (ctx_ptr == NULL || out_value == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    struct c_future *ctx = (struct c_future *)ctx_ptr;
    c_record_poll_thread();
    atomic_fetch_add(&g_polls, 1);
    c_lock();
    ctx->polls += 1;
    enum c_state state = ctx->state;
    enum c_kind kind = ctx->kind;
    int issued = ctx->issued;
    int service_covered =
        ctx->service_host != NULL &&
        ctx->service_host_size >=
            offsetof(struct MetisHostApiV1, service_request) + sizeof(MetisServiceRequestFn);
    c_unlock();

    if (state == C_DROPPED) {
        return METIS_STATUS_INTERNAL;
    }
    if (state == C_CANCELLED) {
        atomic_fetch_add(&g_cancel_resolutions, 1);
        return METIS_STATUS_CANCELLED;
    }

    if (kind == C_READY || kind == C_ACCOUNTING) {
        struct MetisBuffer buffer;
        if (kind == C_READY) {
            char body[128];
            int written = snprintf(body, sizeof body,
                                   "{\"impl\":\"c\",\"kind\":\"ready\",\"x\":%lld,\"guard\":%d}",
                                   ctx->x, ctx->guard_status);
            if (written < 0 || (size_t)written >= sizeof body) {
                return METIS_STATUS_INTERNAL;
            }
            buffer = c_reply(body, (size_t)written, C_TAG_READY);
        } else {
            buffer = c_accounting_buffer();
        }
        if (buffer.ptr == NULL) {
            return METIS_STATUS_INTERNAL;
        }
        c_lock();
        ctx->state = C_DONE;
        c_unlock();
        *out_value = buffer;
        atomic_fetch_add(&g_polls_ready, 1);
        return METIS_STATUS_OK;
    }

    /* C_TIMER */
    if (state == C_DONE) {
        char body[160];
        int written = snprintf(body, sizeof body,
                               "{\"impl\":\"c\",\"kind\":\"timer\",\"x\":%lld,\"polls\":%" PRIu64 "}",
                               ctx->x, ctx->polls);
        if (written < 0 || (size_t)written >= sizeof body) {
            return METIS_STATUS_INTERNAL;
        }
        struct MetisBuffer buffer = c_reply(body, (size_t)written, C_TAG_READY);
        if (buffer.ptr == NULL) {
            return METIS_STATUS_INTERNAL;
        }
        *out_value = buffer;
        atomic_fetch_add(&g_polls_ready, 1);
        return METIS_STATUS_OK;
    }
    if (issued) {
        return METIS_STATUS_PENDING;
    }
    if (!service_covered) {
        /* The host's table does not cover `service_request`, so this plugin
         * must not call it. That is the append-only presence rule. */
        atomic_fetch_add(&g_service_request_failures, 1);
        return METIS_STATUS_UNSUPPORTED;
    }

    c_lock();
    ctx->issued = 1;
    c_unlock();

    /* Clone the borrowed waker: the host still owns the handle it passed, so
     * the plugin may only keep a clone, and it must release every clone. */
    struct MetisWaker clone = c_null_waker();
    if (waker.vtable != NULL && waker.data != NULL) {
        clone = waker.vtable->clone(&waker);
        atomic_fetch_add(&g_waker_clones, 1);
    }

    int armed = 0;
    c_lock();
    if (ctx->state == C_ARMED) {
        ctx->waker = clone;
        ctx->waker_present = 1;
        armed = 1;
    }
    c_unlock();
    if (!armed) {
        if (clone.vtable != NULL && clone.data != NULL) {
            clone.vtable->drop(clone);
            atomic_fetch_add(&g_waker_releases, 1);
        }
        return METIS_STATUS_PENDING;
    }

    /* The request payload is plugin-owned and released by the host *inside*
     * `service_request`, which re-enters this file's `release_buffer`. The lock
     * is therefore released before the call. */
    char body[96];
    int written = snprintf(body, sizeof body, "{\"delay_ms\":%" PRIu64 "}", ctx->delay_ms);
    if (written < 0 || (size_t)written >= sizeof body) {
        return METIS_STATUS_INTERNAL;
    }
    unsigned char scratch[128];
    if ((size_t)written + 1 > sizeof scratch) {
        return METIS_STATUS_INTERNAL;
    }
    scratch[0] = C_ENVELOPE_BYTE;
    memcpy(scratch + 1, body, (size_t)written);
    struct MetisBuffer request = c_alloc(scratch, (size_t)written + 1, C_TAG_SERVICE);
    if (request.ptr == NULL) {
        return METIS_STATUS_INTERNAL;
    }
    const char *service = "timer";
    enum MetisStatus status = ctx->service_host->service_request(
        ctx->service_host->user_data, (const uint8_t *)service, strlen(service), ctx->request_id,
        request);
    if (status != METIS_STATUS_OK) {
        atomic_fetch_add(&g_service_request_failures, 1);
    } else {
        atomic_fetch_add(&g_service_roundtrips, 1);
    }
    return METIS_STATUS_PENDING;
}

/* `drop` runs exactly once per handle. It releases the plugin's buffers, its
 * waker clone, and its reference to the context, and it marks the request so a
 * service reply that arrives afterwards does not wake a dropped future. */
static enum MetisStatus c_drop(void *ctx_ptr) {
    if (ctx_ptr == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    struct c_future *ctx = (struct c_future *)ctx_ptr;
    struct MetisWaker waker = c_null_waker();
    int have_waker = 0;
    int slot_ref = 0;
    c_lock();
    if (ctx->state == C_DROPPED) {
        c_unlock();
        atomic_fetch_add(&g_double_drops, 1);
        return METIS_STATUS_OK;
    }
    if (slot_remove_ctx(ctx->instance, ctx)) {
        slot_ref = 1;
    }
    ctx->state = C_DROPPED;
    if (ctx->waker_present) {
        waker = ctx->waker;
        ctx->waker_present = 0;
        have_waker = 1;
    }
    c_unlock();
    if (have_waker) {
        /* `drop` on the waker vtable consumes the clone without waking, which
         * is what the clause requires when the future is going away. */
        waker.vtable->drop(waker);
        atomic_fetch_add(&g_waker_releases, 1);
    }
    atomic_fetch_add(&g_futures_dropped, 1);
    if (slot_ref) {
        c_ctx_unref(ctx);
    }
    c_ctx_unref(ctx);
    return METIS_STATUS_OK;
}

static const struct MetisFutureVtable C_FUTURE_VTABLE = {
    .size = sizeof(struct MetisFutureVtable),
    .poll = c_poll,
    .drop = c_drop,
};

/* --- the table entries ---------------------------------------------------- */

static enum MetisStatus c_create_instance(const struct MetisHostApiV1 *host, void **out_instance) {
    if (out_instance == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    struct c_instance *instance = calloc(1, sizeof *instance);
    if (instance == NULL) {
        return METIS_STATUS_INTERNAL;
    }
    instance->create_host = host;
    instance->create_host_size = host != NULL ? host->size : 0;
    instance->alive = 1;
    c_lock();
    g_instance_serial += 1;
    instance->serial = g_instance_serial;
    g_create_host_size = instance->create_host_size;
    c_unlock();
    *out_instance = instance;
    atomic_fetch_add(&g_instances_created, 1);
    c_write_ledger("create_instance");
    return METIS_STATUS_OK;
}

static enum MetisStatus c_destroy_instance(void *instance_ptr) {
    if (instance_ptr == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    struct c_instance *instance = (struct c_instance *)instance_ptr;
    c_lock();
    if (!instance->alive) {
        c_unlock();
        return METIS_STATUS_NOT_FOUND;
    }
    instance->alive = 0;
    struct c_future *orphans[C_MAX_SLOTS];
    size_t orphan_count = 0;
    for (size_t i = 0; i < C_MAX_SLOTS; i++) {
        if (instance->slots[i].ctx != NULL) {
            orphans[orphan_count++] = instance->slots[i].ctx;
            instance->slots[i].ctx = NULL;
            instance->slots[i].request_id = 0;
        }
    }
    c_unlock();
    for (size_t i = 0; i < orphan_count; i++) {
        c_ctx_unref(orphans[i]);
    }
    free(instance);
    return METIS_STATUS_OK;
}

static enum MetisStatus c_list_capabilities(void *instance_ptr, struct MetisBuffer *out_payload) {
    if (instance_ptr == NULL || out_payload == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    static const char *BODY =
        "{\"capabilities\":["
        "{\"name\":\"c.echo\",\"kind\":\"tool\",\"version\":1,"
        "\"request_schema_id\":\"c.echo.request\",\"response_schema_id\":\"c.echo.response\"},"
        "{\"name\":\"c.timer\",\"kind\":\"tool\",\"version\":1,"
        "\"request_schema_id\":\"c.timer.request\",\"response_schema_id\":\"c.timer.response\"},"
        "{\"name\":\"c.accounting\",\"kind\":\"tool\",\"version\":1,"
        "\"request_schema_id\":\"c.accounting.request\",\"response_schema_id\":\"c.accounting.response\"}"
        "]}";
    struct MetisBuffer buffer = c_reply(BODY, strlen(BODY), C_TAG_LEGACY);
    if (buffer.ptr == NULL) {
        return METIS_STATUS_INTERNAL;
    }
    *out_payload = buffer;
    return METIS_STATUS_OK;
}

/* The legacy callback path: this implementer uses the future seam instead, so
 * `invoke` refuses rather than pretending. */
static enum MetisStatus c_invoke(void *instance_ptr, const uint8_t *capability,
                                size_t capability_len, uint64_t request_id,
                                struct MetisBuffer payload, const struct MetisHostApiV1 *host,
                                struct MetisBuffer *out_payload) {
    (void)instance_ptr;
    (void)capability;
    (void)capability_len;
    (void)request_id;
    (void)payload;
    (void)host;
    (void)out_payload;
    atomic_fetch_add(&g_legacy_invoke_calls, 1);
    return METIS_STATUS_UNSUPPORTED;
}

static enum MetisStatus c_cancel(void *instance_ptr, uint64_t request_id) {
    if (instance_ptr == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    struct c_instance *instance = (struct c_instance *)instance_ptr;
    struct MetisWaker waker = c_null_waker();
    int have_waker = 0;
    c_lock();
    struct c_future *ctx = slot_find(instance, request_id);
    if (ctx == NULL) {
        c_unlock();
        return METIS_STATUS_NOT_FOUND;
    }
    ctx->state = C_CANCELLED;
    if (ctx->waker_present) {
        waker = ctx->waker;
        ctx->waker_present = 0;
        have_waker = 1;
    }
    c_unlock();
    if (have_waker) {
        waker.vtable->wake(waker);
        atomic_fetch_add(&g_wake_calls, 1);
        atomic_fetch_add(&g_waker_releases, 1);
    }
    atomic_fetch_add(&g_cancels, 1);
    return METIS_STATUS_OK;
}

static enum MetisStatus c_release_buffer(struct MetisBuffer buffer) { return c_release(buffer); }

static enum MetisStatus c_shutdown(void *instance_ptr) {
    if (instance_ptr != NULL) {
        struct c_instance *instance = (struct c_instance *)instance_ptr;
        c_lock();
        instance->alive = 0;
        c_unlock();
    }
    c_write_ledger("shutdown");
    return METIS_STATUS_OK;
}

static enum MetisStatus c_invoke_future(void *instance_ptr, const uint8_t *capability,
                                       size_t capability_len, uint64_t request_id,
                                       struct MetisBuffer payload,
                                       const struct MetisHostApiV1 *host,
                                       struct MetisFutureHandle *out) {
    if (instance_ptr == NULL || capability == NULL || host == NULL || out == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    /* The clause: the request buffer is the host's, borrowed for this call, and
     * the host frees it when this function returns. A plugin must copy what it
     * needs now and must never release it. */
    if (payload.owner != METIS_BUFFER_OWNER_HOST || payload.ptr == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    if (payload.len == 0 || payload.ptr[0] != C_ENVELOPE_BYTE) {
        atomic_fetch_add(&g_bad_envelope_in, 1);
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    const char *body = (const char *)(payload.ptr + 1);
    size_t body_len = payload.len - 1;
    /* Copied here, because the pointer dies with the call. */
    long long x = json_int_field(body, body_len, "x", 0);
    long long delay = json_int_field(body, body_len, "delay_ms", 0);

    enum c_kind kind;
    if (name_is(capability, capability_len, "c.echo")) {
        kind = C_READY;
    } else if (name_is(capability, capability_len, "c.timer")) {
        kind = C_TIMER;
    } else if (name_is(capability, capability_len, "c.accounting")) {
        kind = C_ACCOUNTING;
    } else {
        return METIS_STATUS_NOT_FOUND;
    }

    struct c_future *ctx = calloc(1, sizeof *ctx);
    if (ctx == NULL) {
        return METIS_STATUS_INTERNAL;
    }
    ctx->instance = (struct c_instance *)instance_ptr;
    ctx->request_id = request_id;
    ctx->kind = kind;
    ctx->service_host = host;
    ctx->service_host_size = host->size;
    ctx->x = x;
    ctx->delay_ms = delay < 0 ? 0u : (uint64_t)delay;
    ctx->refs = 1; /* the handle's reference */
    ctx->state = C_ARMED;
    ctx->waker_present = 0;
    ctx->waker = c_null_waker();
    ctx->issued = 0;

    if (kind == C_READY) {
        /* The owned-buffer guard, driven on every ready call: hand the
         * host-owned request buffer to this plugin's own `release_buffer` and
         * record that it is refused. A plugin that freed it would be freeing
         * the host's memory. */
        (void)c_release(payload);
        atomic_fetch_add(&g_guard_tests, 1);
        ctx->guard_status = (int)METIS_STATUS_INVALID_ARGUMENT;
    }

    c_lock();
    g_service_host_size = ctx->service_host_size;
    int inserted = slot_insert(ctx->instance, request_id, ctx);
    if (inserted) {
        ctx->refs += 1; /* the slot's reference */
    }
    c_unlock();
    if (!inserted) {
        /* No free slot: the previous request was never resolved and never
         * dropped. The ABI states no rule, so this implementer refuses. */
        free(ctx);
        return METIS_STATUS_INTERNAL;
    }
    atomic_fetch_add(&g_futures_created, 1);
    out->ctx = ctx;
    out->vtable = &C_FUTURE_VTABLE;
    return METIS_STATUS_OK;
}

static enum MetisStatus c_service_complete(void *instance_ptr, uint64_t request_id,
                                          enum MetisStatus status, struct MetisBuffer payload) {
    /* The clause: the answer payload is host-owned and borrowed for this call.
     * This implementer reads nothing from it and never releases it. */
    if (payload.owner == METIS_BUFFER_OWNER_PLUGIN) {
        /* The landed Rust plugin frees a plugin-owned answer here. Per the
         * clause that would be freeing the host's buffer, so this implementer
         * only counts it. */
        atomic_fetch_add(&g_plugin_owned_service_payloads, 1);
    } else {
        atomic_fetch_add(&g_host_owned_service_payloads, 1);
    }
    /* Every answer is delivered on the host's timer thread, never on the poll
     * thread, which is what makes the C implementer's state thread safe. */
    c_record_completion_thread();
    if (instance_ptr == NULL) {
        return METIS_STATUS_INVALID_ARGUMENT;
    }
    struct c_instance *instance = (struct c_instance *)instance_ptr;
    struct MetisWaker waker = c_null_waker();
    int have_waker = 0;
    c_lock();
    struct c_future *ctx = slot_take(instance, request_id); /* takes the slot ref */
    if (ctx == NULL) {
        c_unlock();
        /* The future was dropped (or cancelled and dropped) before the answer
         * arrived. The conforming behaviour is to wake nobody and say so. */
        atomic_fetch_add(&g_late_completions, 1);
        return METIS_STATUS_OK;
    }
    if (ctx->state == C_ARMED) {
        ctx->state = C_DONE;
        if (ctx->waker_present) {
            waker = ctx->waker;
            ctx->waker_present = 0;
            have_waker = 1;
        }
    }
    c_unlock();
    if (have_waker) {
        /* `wake` consumes the clone, so this is also its release. */
        waker.vtable->wake(waker);
        atomic_fetch_add(&g_wake_calls, 1);
        atomic_fetch_add(&g_waker_releases, 1);
    }
    if (status != METIS_STATUS_OK) {
        atomic_fetch_add(&g_service_request_failures, 1);
    }
    c_ctx_unref(ctx);
    return METIS_STATUS_OK;
}

static const struct MetisPluginTableV1 PLUGIN_TABLE_V1 = {
    .size = sizeof(struct MetisPluginTableV1),
    .abi_major = METIS_ABI_MAJOR,
    .abi_minor = METIS_ABI_MINOR,
    .create_instance = c_create_instance,
    .destroy_instance = c_destroy_instance,
    .list_capabilities = c_list_capabilities,
    .invoke = c_invoke,
    .cancel = c_cancel,
    .release_buffer = c_release_buffer,
    .shutdown = c_shutdown,
    .invoke_future = c_invoke_future,
    .service_complete = c_service_complete,
};

const struct MetisPluginTableV1 *metis_plugin_entry_v1(void) { return &PLUGIN_TABLE_V1; }
