/* Which points a pass of the main loop visits, and how long it waits
 * (schedule.h): the lists must name exactly the points a full scan would act
 * on, through every change of push state the runtime makes (subscribe,
 * transport drop, reload). */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "tedge_dot/config.h"
#include "tedge_dot/report.h"
#include "tedge_dot/schedule.h"

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

/* A modbus device with four points:
 *   0 plain           1 report = { min_interval = "5s" }
 *   2 write-only      3 sampling_interval = `fast` */
static tdot_config_t *load(const char *fast) {
    char tmpl[] = "/tmp/tdot-schedule-XXXXXX";
    char *dir = mkdtemp(tmpl);
    if (!dir) {
        perror("mkdtemp");
        exit(2);
    }
    char path[256];
    snprintf(path, sizeof path, "%s/modbus.toml", dir);
    FILE *fp = fopen(path, "w");
    fprintf(fp,
            "[connector]\n"
            "protocol = \"modbus\"\n"
            "poll_interval = \"2s\"\n"
            "\n"
            "[[device]]\n"
            "name = \"plc1\"\n"
            "protocol_address = { transport = \"tcp\", host = \"127.0.0.1\", "
            "port = 502, unit_id = 1 }\n"
            "\n"
            "  [[device.point]]\n"
            "  id = \"plain\"\n"
            "  datatype = \"uint16\"\n"
            "  address = { table = \"holding\", address = 0, count = 1 }\n"
            "  [[device.point]]\n"
            "  id = \"policy\"\n"
            "  datatype = \"uint16\"\n"
            "  address = { table = \"holding\", address = 1, count = 1 }\n"
            "  report = { min_interval = \"5s\" }\n"
            "  [[device.point]]\n"
            "  id = \"setpoint\"\n"
            "  datatype = \"uint16\"\n"
            "  access = \"write\"\n"
            "  address = { table = \"holding\", address = 2, count = 1 }\n"
            "  [[device.point]]\n"
            "  id = \"fast\"\n"
            "  datatype = \"uint16\"\n"
            "  sampling_interval = \"%s\"\n"
            "  address = { table = \"holding\", address = 3, count = 1 }\n",
            fast);
    fclose(fp);
    char err[256];
    tdot_config_t *cfg = tdot_config_load(path, err, sizeof err);
    unlink(path);
    rmdir(dir);
    if (!cfg) {
        printf("FAIL config did not load: %s\n", err);
        exit(1);
    }
    return cfg;
}

/* The lists against a full scan of the points. */
static void check_matches_scan(const tdot_device_t *dev, const char *when) {
    size_t polled = 0, reported = 0, pushed = 0;
    for (size_t j = 0; j < dev->npoints; j++) {
        const tdot_point_t *pt = &dev->points[j];
        if (pt->subscribed) {
            pushed++;
        } else if (pt->access & TDOT_ACCESS_READ) {
            CHECK(polled < dev->npolled && dev->polled[polled] == j,
                  "%s: point %s missing from the polled list", when, pt->id);
            polled++;
        }
        if (!tdot_report_is_passthrough(&pt->report)) {
            CHECK(reported < dev->nreported && dev->reported[reported] == j,
                  "%s: point %s missing from the reported list", when, pt->id);
            reported++;
        }
    }
    CHECK(dev->npolled == polled, "%s: %zu polled, a scan finds %zu", when,
          dev->npolled, polled);
    CHECK(dev->nreported == reported, "%s: %zu reported, a scan finds %zu",
          when, dev->nreported, reported);
    CHECK(dev->npushed == pushed, "%s: %zu pushed, a scan finds %zu", when,
          dev->npushed, pushed);
}

static void set_pushed(tdot_device_t *dev, bool on) {
    for (size_t j = 0; j < dev->npoints; j++)
        dev->points[j].subscribed =
            on && (dev->points[j].access & TDOT_ACCESS_READ);
}

static void check_lists_follow_push_state(void) {
    tdot_config_t *cfg = load("50ms");
    tdot_device_t *dev = &cfg->devices[0];

    /* Before the first connect: nothing pushed, every readable point polled. */
    tdot_schedule_rebuild(dev);
    check_matches_scan(dev, "start");
    CHECK(dev->npolled == 3, "start: 3 readable points polled, got %zu",
          dev->npolled);
    CHECK(dev->nreported == 1, "only the policy point is reported, got %zu",
          dev->nreported);
    CHECK(tdot_schedule_tick_ms(cfg) == TDOT_TICK_MAX_MS,
          "nothing pushed: tick %d", tdot_schedule_tick_ms(cfg));

    /* Subscribed: the pushed points leave the polling list. */
    set_pushed(dev, true);
    tdot_schedule_rebuild(dev);
    check_matches_scan(dev, "subscribed");
    CHECK(dev->npolled == 0, "subscribed: nothing polled, got %zu",
          dev->npolled);
    CHECK(tdot_schedule_tick_ms(cfg) == 50,
          "fastest pushed 50ms: tick %d", tdot_schedule_tick_ms(cfg));

    /* The transport drops: back on the polling list, wait back to 200 ms. */
    set_pushed(dev, false);
    tdot_schedule_rebuild(dev);
    check_matches_scan(dev, "transport down");
    CHECK(tdot_schedule_tick_ms(cfg) == TDOT_TICK_MAX_MS,
          "outage: tick %d", tdot_schedule_tick_ms(cfg));

    /* Only one point pushed. */
    dev->points[3].subscribed = true;
    tdot_schedule_rebuild(dev);
    check_matches_scan(dev, "partly subscribed");
    CHECK(dev->npolled == 2, "partly subscribed: 2 polled, got %zu",
          dev->npolled);

    /* A reload swaps the configuration in: the new devices start empty. */
    tdot_config_t *next = load("50ms");
    tdot_config_replace(cfg, next);
    dev = &cfg->devices[0];
    CHECK(dev->polled == NULL && dev->npushed == 0,
          "a reloaded device starts without lists");
    tdot_schedule_rebuild(dev);
    check_matches_scan(dev, "reloaded");
    tdot_config_free(cfg);
}

static void check_tick_bounds(void) {
    struct {
        const char *interval;
        int tick_ms;
    } cases[] = {
        {"0", TDOT_TICK_MIN_MS},   /* fastest-rate request */
        {"5ms", TDOT_TICK_MIN_MS}, /* below the floor */
        {"100ms", 100},
        {"5s", TDOT_TICK_MAX_MS}, /* slower than the default tick */
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        tdot_config_t *cfg = load(cases[i].interval);
        tdot_device_t *dev = &cfg->devices[0];
        dev->points[3].subscribed = true;
        tdot_schedule_rebuild(dev);
        CHECK(tdot_schedule_tick_ms(cfg) == cases[i].tick_ms,
              "sampling_interval %s: tick %d, want %d", cases[i].interval,
              tdot_schedule_tick_ms(cfg), cases[i].tick_ms);
        tdot_config_free(cfg);
    }
}

int main(void) {
    check_lists_follow_push_state();
    check_tick_bounds();
    if (failures) {
        printf("%d check(s) failed\n", failures);
        return 1;
    }
    printf("schedule: all checks passed\n");
    return 0;
}
