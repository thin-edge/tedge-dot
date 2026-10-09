/* The ring that holds pushed OPC UA values of one device between two runtime
 * ticks (connector_opcua.c). Header-only so that impl/c/tests/opcua_ring.c can
 * test it without a server.
 *
 * The callback that fills it and the drain that empties it run on the same
 * thread, so it needs no locking. Every monitored item reports its current
 * value when it is created, so the ring is sized from the device's point count
 * when it subscribes. It grows by doubling when a burst fills it, up to a
 * bound. Only past the bound, or when memory runs out, is a value dropped. The
 * newest one is dropped, so that the queued values keep their order.
 */
#ifndef TDOT_UA_PUSH_RING_H
#define TDOT_UA_PUSH_RING_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

#include "tedge_dot/config.h"
#include "tedge_dot/model.h"

#define UA_RING_MIN 256
#define UA_RING_BOUND_MIN 65536

/* The allocator, replaceable so impl/c/tests/opcua_ring.c can make it fail. */
#ifndef UA_RING_ALLOC
#define UA_RING_ALLOC malloc
#endif

typedef struct {
    tdot_point_t *pt;
    tdot_sample_t sample;
} ua_pending_t;

typedef struct {
    ua_pending_t *slots; /* NULL until sized */
    size_t cap;          /* slots; one stays free to tell full from empty */
    size_t bound;        /* the most slots it may grow to */
    size_t head;         /* next slot to write */
    size_t tail;         /* next slot to read */
    unsigned long dropped; /* values dropped since the last warning */
    unsigned long nomem;   /* of those, dropped because the ring could not grow */
} ua_ring_t;

static inline size_t ua_ring_len(const ua_ring_t *r) {
    return r->cap ? (r->head + r->cap - r->tail) % r->cap : 0;
}

/* Move the queued values, in order, into a ring of `cap` slots. On failure
 * the ring is left exactly as it was. */
static inline bool ua_ring_resize(ua_ring_t *r, size_t cap) {
    if (cap == 0 || cap > SIZE_MAX / sizeof(ua_pending_t))
        return false;
    ua_pending_t *slots = UA_RING_ALLOC(cap * sizeof *slots);
    if (!slots)
        return false;
    size_t n = ua_ring_len(r);
    for (size_t i = 0; i < n; i++)
        slots[i] = r->slots[(r->tail + i) % r->cap];
    free(r->slots);
    r->slots = slots;
    r->cap = cap;
    r->tail = 0;
    r->head = n;
    return true;
}

/* Size an empty ring for a device that subscribes `points` monitored items:
 * room for all their start values twice over, at least UA_RING_MIN slots, and
 * a bound of the larger of UA_RING_BOUND_MIN and four times the points. */
static inline void ua_ring_reserve(ua_ring_t *r, size_t points) {
    size_t want = UA_RING_MIN;
    while (want < 2 * points + 1)
        want *= 2;
    r->bound = 4 * points > UA_RING_BOUND_MIN ? 4 * points : UA_RING_BOUND_MIN;
    if (want > r->bound)
        want = r->bound;
    r->head = r->tail = 0;
    r->dropped = r->nomem = 0;
    /* The ring is empty, so a resize only swaps the buffer. When it cannot be
     * allocated, the ring keeps the one it has (or none: ua_ring_slot grows
     * from nothing), so a failed reserve never costs a working ring. */
    if (r->cap != want)
        (void)ua_ring_resize(r, want);
}

/* A slot for the next value, growing the ring when it is full. NULL: the
 * value is dropped (and counted). Commit the filled slot with ua_ring_commit. */
static inline ua_pending_t *ua_ring_slot(ua_ring_t *r) {
    if (!r->cap || (r->head + 1) % r->cap == r->tail) {
        size_t bound = r->bound ? r->bound : UA_RING_BOUND_MIN;
        size_t cap = r->cap ? 2 * r->cap : UA_RING_MIN;
        if (cap > bound)
            cap = bound;
        if (cap <= r->cap) {
            r->dropped++; /* at the bound */
            return NULL;
        }
        if (!ua_ring_resize(r, cap)) {
            r->dropped++;
            r->nomem++;
            return NULL;
        }
    }
    return &r->slots[r->head];
}

static inline void ua_ring_commit(ua_ring_t *r) {
    r->head = (r->head + 1) % r->cap;
}

/* The oldest queued value, or NULL; release it with ua_ring_pop. */
static inline ua_pending_t *ua_ring_peek(ua_ring_t *r) {
    return r->tail != r->head ? &r->slots[r->tail] : NULL;
}

static inline void ua_ring_pop(ua_ring_t *r) {
    r->tail = (r->tail + 1) % r->cap;
}

/* Forget what is queued (a new session must not deliver the old one's). */
static inline void ua_ring_clear(ua_ring_t *r) {
    r->head = r->tail = 0;
    r->dropped = r->nomem = 0;
}

static inline void ua_ring_free(ua_ring_t *r) {
    free(r->slots);
    r->slots = NULL;
    r->cap = r->bound = r->head = r->tail = 0;
    r->dropped = r->nomem = 0;
}

#endif
