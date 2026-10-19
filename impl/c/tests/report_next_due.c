/* The reporting pass may skip a device until its bound (report.h:
 * tdot_report_next_due / tdot_report_lower_bound). The skip must be invisible:
 * the same random script of readings, resets, push-state changes and ticks is
 * run over a device of a few points with mixed policies, once as the runtime
 * used to (every tick looks at every point) and once with the bound, driven as
 * runtime.c drives it. Both must publish the same readings at the same times and
 * ask for the same heartbeat reads. The due times themselves are pinned exactly. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "tedge_dot/report.h"

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

#define MS 1000000LL
#define NPOINTS 4
#define LOGMAX (1 << 20)

static uint64_t rng_state;

static int rnd(int n) {
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 7;
    rng_state ^= rng_state << 17;
    return (int)(rng_state % (uint64_t)n);
}

/* A decision both runs must take alike: a function of the step and point only. */
static bool decide(int step, int point, int one_in, int salt) {
    uint64_t h = (uint64_t)step * 2654435761u + (uint64_t)point * 40503u + (uint64_t)salt * 97u;
    h ^= h >> 15;
    h *= 2246822519u;
    h ^= h >> 13;
    return (int)(h % (uint64_t)one_in) == 0;
}

static tdot_report_policy_t random_policy(void) {
    tdot_report_policy_t p;
    memset(&p, 0, sizeof p);
    p.on_change = rnd(3) == 0;
    if (rnd(4) == 0) {
        p.deadband_kind = TDOT_DEADBAND_ABSOLUTE;
        p.deadband = 1.5;
    }
    if (rnd(2) == 0)
        p.min_interval = (200 + rnd(6) * 300) * MS;
    if (rnd(2) == 0)
        p.max_interval = (1000 + rnd(6) * 700) * MS;
    if (rnd(3) == 0)
        p.debounce = (100 + rnd(4) * 200) * MS;
    if (tdot_report_is_passthrough(&p))
        p.max_interval = 2000 * MS;
    return p;
}

/* ---- one run over the device --------------------------------------------------- */

typedef struct {
    tdot_report_state_t *st[NPOINTS];
    bool pushed[NPOINTS];
    bool optimized;
    int64_t bound; /* the device's report_next */
    long scans, ticks;
    char *log; /* what was published or asked for, one line each */
    size_t n;
} run_t;

static void event(run_t *r, int kind, int point, long long ts) {
    if (r->n + 64 < LOGMAX)
        r->n += (size_t)snprintf(r->log + r->n, LOGMAX - r->n, "%d %d %lld\n", kind, point, ts);
}

/* emit_sample: the reading is offered to the point's policy. */
static void offer(run_t *r, int point, int value, tdot_quality_t quality, int64_t now) {
    tdot_report_item_t item;
    memset(&item, 0, sizeof item);
    item.ts_ms = (double)now / 1e6;
    tdot_report_obs_t obs;
    memset(&obs, 0, sizeof obs);
    obs.is_num = true;
    obs.num = value;
    bool publish_now = tdot_report_offer(r->st[point], &item, &obs, quality, now);
    if (r->optimized)
        tdot_report_lower_bound(&r->bound, r->st[point], r->pushed[point]);
    if (publish_now)
        event(r, 1, point, (long long)item.ts_ms);
}

/* report_pass over the device. */
static void tick(run_t *r, int64_t now, int step) {
    r->ticks++;
    if (r->optimized && now < r->bound)
        return;
    r->scans++;
    bool transport_down = false;
    int64_t next = INT64_MAX;
    int k;
    for (k = 0; k < NPOINTS && !transport_down; k++) {
        tdot_report_item_t held;
        bool due_read = false;
        if (tdot_report_due(r->st[k], now, r->pushed[k], &held, &due_read)) {
            event(r, 2, k, (long long)held.ts_ms);
            tdot_report_item_release(&held);
        }
        if (due_read) {
            event(r, 3, k, now);
            if (decide(step, k, 6, 1)) { /* TDOT_READ_NO_DATA */
                tdot_report_no_data(r->st[k]);
            } else {
                offer(r, k, (step / 5 + k * 3) % 4, decide(step, k, 9, 2) ? TDOT_Q_BAD : TDOT_Q_GOOD, now);
                transport_down = decide(step, k, 23, 3);
            }
        }
        tdot_report_lower_bound(&next, r->st[k], r->pushed[k]);
    }
    if (r->optimized)
        r->bound = k < NPOINTS ? 0 : next;
}

typedef struct {
    int64_t dt;
    int offer_point; /* -1: none */
    int offer_value;
    bool offer_bad, reset;
    int toggle_point; /* -1: none */
} step_t;

static void run(run_t *r, const tdot_report_policy_t *pol, const step_t *script, int nsteps,
                bool optimized) {
    int64_t now = 1000 * MS;
    memset(r, 0, sizeof *r);
    r->optimized = optimized;
    r->log = calloc(1, LOGMAX);
    for (int i = 0; i < NPOINTS; i++) {
        r->st[i] = malloc(sizeof *r->st[i]);
        tdot_report_init(r->st[i], &pol[i], now);
        r->pushed[i] = true;
    }
    for (int i = 0; i < nsteps; i++) {
        const step_t *s = &script[i];
        now += s->dt;
        if (s->reset) { /* reset_device_reports */
            for (int p = 0; p < NPOINTS; p++)
                tdot_report_reset(r->st[p], now);
            r->bound = 0;
        }
        if (s->toggle_point >= 0) { /* a change of push state rebuilds the schedule */
            r->pushed[s->toggle_point] = !r->pushed[s->toggle_point];
            r->bound = 0;
        }
        if (s->offer_point >= 0)
            offer(r, s->offer_point, s->offer_value, s->offer_bad ? TDOT_Q_BAD : TDOT_Q_GOOD, now);
        tick(r, now, i);
    }
}

static void run_free(run_t *r) {
    for (int i = 0; i < NPOINTS; i++) {
        tdot_report_free(r->st[i]);
        free(r->st[i]);
    }
    free(r->log);
}

static void check_skipping_is_invisible(void) {
    long ticks = 0, scans = 0, events = 0;
    for (int seed = 1; seed <= 400 && failures == 0; seed++) {
        rng_state = 0x9E3779B97F4A7C15ULL * (uint64_t)seed + 1;
        tdot_report_policy_t pol[NPOINTS];
        for (int i = 0; i < NPOINTS; i++)
            pol[i] = random_policy();
        enum { NSTEPS = 600 };
        step_t script[NSTEPS];
        for (int i = 0; i < NSTEPS; i++) {
            step_t *s = &script[i];
            s->dt = (rnd(10) == 0 ? 800 + rnd(2500) : 1 + rnd(120)) * MS;
            s->offer_point = rnd(100) < 55 ? rnd(NPOINTS) : -1;
            s->offer_value = rnd(4);
            s->offer_bad = rnd(20) == 0;
            s->reset = rnd(80) == 0;
            s->toggle_point = rnd(60) == 0 ? rnd(NPOINTS) : -1;
        }
        run_t full, skip;
        run(&full, pol, script, NSTEPS, false);
        run(&skip, pol, script, NSTEPS, true);
        CHECK(full.n == skip.n && memcmp(full.log, skip.log, full.n) == 0,
              "seed %d: the skipping pass differs from the full one (%zu vs %zu bytes of log)", seed,
              full.n, skip.n);
        ticks += skip.ticks;
        scans += skip.scans;
        for (size_t i = 0; i < full.n; i++)
            events += full.log[i] == '\n';
        run_free(&full);
        run_free(&skip);
    }
    /* Not vacuous: there is something to compare, and the bound lets ticks go by. */
    CHECK(events > 20000, "only %ld events compared", events);
    CHECK(scans < ticks / 2, "the bound skipped too little: %ld scans in %ld ticks", scans, ticks);
}

/* ---- the due times, exactly ----------------------------------------------------- */

static tdot_report_state_t *make(const tdot_report_policy_t *p) {
    tdot_report_state_t *st = malloc(sizeof *st);
    tdot_report_init(st, p, 0);
    return st;
}

/* Offer a reading of `value` at `at` (milliseconds). The states here hold no heap
 * items (addr_json is NULL), so a struct copy is a deep copy. */
static bool offer_at(tdot_report_state_t *st, int value, int64_t at) {
    tdot_report_item_t item;
    memset(&item, 0, sizeof item);
    tdot_report_obs_t obs;
    memset(&obs, 0, sizeof obs);
    obs.is_num = true;
    obs.num = value;
    return tdot_report_offer(st, &item, &obs, TDOT_Q_GOOD, at * MS);
}

static bool due_at(const tdot_report_state_t *st, int64_t at, bool pushed, bool *read) {
    tdot_report_state_t *probe = malloc(sizeof *probe);
    *probe = *st;
    tdot_report_item_t out;
    bool published = tdot_report_due(probe, at, pushed, &out, read);
    if (published)
        tdot_report_item_release(&out);
    tdot_report_free(probe);
    free(probe);
    return published;
}

/* next_due is `want`, tdot_report_due does nothing one nanosecond before it and
 * acts at it (by publishing, or for a heartbeat by asking for a read). The random
 * script meets these instants only by chance. */
static void check_exact(const char *what, const tdot_report_state_t *st, int64_t want, bool by_read) {
    int64_t t = tdot_report_next_due(st, true);
    bool read = false;
    CHECK(t == want, "%s: due at %lld, expected %lld", what, (long long)t, (long long)want);
    bool early = due_at(st, t - 1, true, &read);
    CHECK(!early && !read, "%s: nothing one nanosecond before", what);
    bool at = due_at(st, t, true, &read);
    CHECK(by_read ? read : at, "%s: acts at the due time", what);
}

static void check_next_due_is_exact(void) {
    tdot_report_policy_t p;
    memset(&p, 0, sizeof p);
    p.min_interval = 500 * MS; /* a held reading, min_interval after the last publish */
    tdot_report_state_t *st = make(&p);
    CHECK(tdot_report_next_due(st, true) == INT64_MAX, "min_interval: nothing held, nothing due");
    CHECK(offer_at(st, 1, 1000), "the first reading is published");
    CHECK(!offer_at(st, 2, 1100), "a change within min_interval is held");
    check_exact("min_interval", st, 1500 * MS, false);
    free(st);

    memset(&p, 0, sizeof p);
    p.debounce = 800 * MS; /* a change, debounce after it first appeared */
    st = make(&p);
    offer_at(st, 1, 1000);
    CHECK(!offer_at(st, 2, 2000), "a change becomes a debounce candidate");
    check_exact("debounce", st, 2800 * MS, false);
    free(st);

    memset(&p, 0, sizeof p);
    p.max_interval = 4000 * MS; /* a heartbeat read, max_interval after the last publish */
    st = make(&p);
    offer_at(st, 1, 1000);
    check_exact("max_interval", st, 5000 * MS, true);
    bool read = true;
    CHECK(tdot_report_next_due(st, false) == INT64_MAX, "a polled point has no heartbeat read of its own");
    due_at(st, 5000 * MS, false, &read);
    CHECK(!read, "a polled point is never asked for a heartbeat read");
    st->unreadable = true;
    CHECK(tdot_report_next_due(st, true) == INT64_MAX, "an unreadable point is not asked again");
    free(st);

    memset(&p, 0, sizeof p);
    p.on_change = true;
    st = make(&p);
    offer_at(st, 1, 1000);
    offer_at(st, 2, 1100);
    CHECK(tdot_report_next_due(st, true) == INT64_MAX, "on_change alone never has anything due");
    int64_t bound = 100;
    tdot_report_lower_bound(&bound, st, true);
    CHECK(bound == 100, "an unarmed point leaves the bound alone");
    st->policy.max_interval = 1000 * MS; /* now due at 2100 ms: later than the bound */
    tdot_report_lower_bound(&bound, st, true);
    CHECK(bound == 100, "the bound is lowered, never raised");
    bound = INT64_MAX;
    tdot_report_lower_bound(&bound, st, true);
    CHECK(bound == 2100 * MS, "the bound is lowered to the point's due time, got %lld", (long long)bound);
    free(st);
}

int main(void) {
    check_next_due_is_exact();
    check_skipping_is_invisible();
    if (failures) {
        printf("report next-due: %d check(s) failed\n", failures);
        return 1;
    }
    printf("report next-due: all checks passed\n");
    return 0;
}
