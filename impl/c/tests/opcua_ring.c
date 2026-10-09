/* The ring of pushed OPC UA values (connectors/opcua/push_ring.h): sized from
 * the device's points, grown in order when a burst fills it, bounded, and
 * dropping only the newest value past the bound. */
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* An allocator that fails while `fail_alloc` is set. */
static bool fail_alloc = false;
static void *test_alloc(size_t n) { return fail_alloc ? NULL : malloc(n); }
#define UA_RING_ALLOC test_alloc

#include "push_ring.h"

static int failures = 0;

#define CHECK(cond, ...)                                                       \
    do {                                                                       \
        if (!(cond)) {                                                         \
            failures++;                                                        \
            printf("FAIL %s:%d: ", __FILE__, __LINE__);                        \
            printf(__VA_ARGS__);                                               \
            printf("\n");                                                      \
        }                                                                      \
    } while (0)

/* Queue the value `v` (kept in sample.value.num); false when dropped. */
static bool push(ua_ring_t *r, double v) {
    ua_pending_t *slot = ua_ring_slot(r);
    if (!slot)
        return false;
    memset(slot, 0, sizeof *slot);
    slot->sample.value.kind = TDOT_VAL_NUM;
    slot->sample.value.num = v;
    ua_ring_commit(r);
    return true;
}

/* Drain everything; the values must be first, first + 1, ... */
static size_t drain_in_order(ua_ring_t *r, double first, const char *when) {
    size_t n = 0;
    ua_pending_t *slot;
    while ((slot = ua_ring_peek(r))) {
        CHECK(slot->sample.value.num == first + (double)n,
              "%s: value %zu is %g, want %g", when, n, slot->sample.value.num,
              first + (double)n);
        ua_ring_pop(r);
        n++;
    }
    return n;
}

static void check_sizing(void) {
    ua_ring_t r = {0};
    ua_ring_reserve(&r, 100);
    CHECK(r.cap == UA_RING_MIN, "100 points: %zu slots, want %d", r.cap,
          UA_RING_MIN);
    CHECK(r.bound == UA_RING_BOUND_MIN, "100 points: bound %zu", r.bound);

    ua_ring_reserve(&r, 20000);
    CHECK(r.cap >= 2 * 20000 + 1, "20000 points: %zu slots", r.cap);
    CHECK(r.bound == 80000, "20000 points: bound %zu, want 80000", r.bound);
    /* Every start value fits without a grow or a drop. */
    size_t cap = r.cap;
    for (int i = 0; i < 20000; i++)
        CHECK(push(&r, i), "start value %d dropped", i);
    CHECK(r.cap == cap && r.dropped == 0, "start values grew or dropped");
    CHECK(drain_in_order(&r, 0, "start values") == 20000, "not all drained");
    ua_ring_free(&r);
}

static void check_grows_in_order_across_a_wrap(void) {
    ua_ring_t r = {0};
    ua_ring_reserve(&r, 10); /* 256 slots */
    /* Move the read position into the middle so the grow has to unwrap. */
    for (int i = 0; i < 200; i++)
        push(&r, -1);
    while (ua_ring_peek(&r))
        ua_ring_pop(&r);
    for (int i = 0; i < 1000; i++)
        CHECK(push(&r, i), "burst value %d dropped below the bound", i);
    CHECK(r.cap > UA_RING_MIN, "the ring did not grow: %zu", r.cap);
    CHECK(r.dropped == 0, "%lu dropped below the bound", r.dropped);
    CHECK(drain_in_order(&r, 0, "burst") == 1000, "burst not all drained");
    ua_ring_free(&r);
}

static void check_bound_drops_newest(void) {
    ua_ring_t r = {0};
    ua_ring_reserve(&r, 1);
    size_t room = r.bound - 1;
    for (size_t i = 0; i < room; i++)
        push(&r, (double)i);
    CHECK(r.cap == r.bound, "full ring not at its bound: %zu of %zu", r.cap,
          r.bound);
    CHECK(!push(&r, -1) && !push(&r, -2), "pushed past the bound");
    CHECK(r.dropped == 2, "%lu dropped, want 2", r.dropped);
    CHECK(drain_in_order(&r, 0, "bounded") == room, "queued values lost");
    ua_ring_free(&r);
}

static void check_clear_and_unsized(void) {
    ua_ring_t r = {0};
    /* Never sized (a value before the subscribe sized it): grows from nothing. */
    CHECK(push(&r, 7), "unsized ring dropped");
    CHECK(ua_ring_len(&r) == 1, "unsized ring holds %zu", ua_ring_len(&r));
    ua_ring_clear(&r);
    CHECK(ua_ring_peek(&r) == NULL, "cleared ring still delivers");
    ua_ring_free(&r);
    CHECK(r.slots == NULL && r.cap == 0, "free left memory behind");
}

/* Out of memory while full: the newest value is dropped and counted as such,
 * what is queued stays intact and in order, and the ring grows again once
 * memory is back. */
static void check_grow_failure_keeps_the_queue(void) {
    ua_ring_t r = {0};
    ua_ring_reserve(&r, 10); /* 256 slots */
    size_t room = r.cap - 1;
    for (size_t i = 0; i < room; i++)
        push(&r, (double)i);
    fail_alloc = true;
    CHECK(!push(&r, -1), "pushed into a full ring that could not grow");
    fail_alloc = false;
    CHECK(r.dropped == 1 && r.nomem == 1, "dropped %lu, nomem %lu, want 1 and 1",
          r.dropped, r.nomem);
    CHECK(r.cap == UA_RING_MIN, "a failed grow changed the ring: %zu", r.cap);
    CHECK(push(&r, (double)room), "no grow once memory is back");
    CHECK(r.cap > UA_RING_MIN, "the ring did not grow after recovering");
    CHECK(drain_in_order(&r, 0, "after a failed grow") == room + 1,
          "queued values lost across a failed grow");
    ua_ring_free(&r);
}

/* At the bound the drop is not an out-of-memory drop. */
static void check_bound_drop_is_not_nomem(void) {
    ua_ring_t r = {0};
    ua_ring_reserve(&r, 1);
    for (size_t i = 0; i < r.bound - 1; i++)
        push(&r, (double)i);
    CHECK(!push(&r, -1), "pushed past the bound");
    CHECK(r.dropped == 1 && r.nomem == 0, "dropped %lu, nomem %lu, want 1 and 0",
          r.dropped, r.nomem);
    ua_ring_free(&r);
}

/* A reserve that cannot allocate keeps the ring it has; one with no ring at
 * all grows from nothing once memory is back. */
static void check_reserve_failure(void) {
    ua_ring_t r = {0};
    ua_ring_reserve(&r, 10);
    ua_pending_t *before = r.slots;
    size_t cap = r.cap;
    fail_alloc = true;
    ua_ring_reserve(&r, 20000); /* wants a bigger ring */
    CHECK(r.slots == before && r.cap == cap,
          "a failed reserve dropped the working ring (%zu slots)", r.cap);
    CHECK(push(&r, 1), "the kept ring does not take values");
    ua_ring_free(&r);

    ua_ring_reserve(&r, 10); /* still failing: no ring */
    CHECK(r.slots == NULL && r.cap == 0, "a ring appeared without memory");
    CHECK(!push(&r, 1) && r.nomem == 1, "an unsized ring took a value without memory");
    fail_alloc = false;
    CHECK(push(&r, 2), "the unsized ring did not grow once memory was back");
    CHECK(drain_in_order(&r, 2, "unsized") == 1, "the value was not queued");
    ua_ring_free(&r);
}

/* A size whose byte count does not fit size_t is refused, not wrapped. */
static void check_size_overflow(void) {
    ua_ring_t r = {0};
    CHECK(!ua_ring_resize(&r, SIZE_MAX / sizeof(ua_pending_t) + 1),
          "an overflowing size was allocated");
    CHECK(r.slots == NULL && r.cap == 0, "a refused resize changed the ring");
}

int main(void) {
    check_sizing();
    check_grows_in_order_across_a_wrap();
    check_bound_drops_newest();
    check_clear_and_unsized();
    check_grow_failure_keeps_the_queue();
    check_bound_drop_is_not_nomem();
    check_reserve_failure();
    check_size_overflow();
    if (failures) {
        printf("%d check(s) failed\n", failures);
        return 1;
    }
    printf("opcua-ring: all checks passed\n");
    return 0;
}
