/* The write direction of the per-point transform (contract §4.2, §6.2).
 *
 * A write request carries engineering units -- what the samples show -- so the
 * SDK maps it back to the raw value through the inverse transform before the
 * connector encodes it. Checks tdot_transform_invert itself, and that
 * tdot_connector_write (the one path the `write`/`write-batch` verbs and the
 * CLI `write` take) hands write_point the raw value. Mirrors the Rust SDK's
 * Transform tests (impl/rust/crates/sdk/src/model.rs).
 */
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "tedge_dot/config.h"
#include "tedge_dot/connector.h"

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

static tdot_transform_t tr(double m, double d, int shift, double o) {
    tdot_transform_t t = {.multiplier = m, .divisor = d, .decimal_shift = shift,
                          .offset = o};
    return t;
}

static bool close_to(double a, double b) {
    return fabs(a - b) <= 1e-9 * fmax(1.0, fmax(fabs(a), fabs(b)));
}

static void check_invert_round_trips(void) {
    const tdot_transform_t ts[] = {
        tr(0.1, 1, 0, 0),   tr(1, 10, 0, 0),    tr(1, 1, -3, 0),
        tr(1, 1, 2, 0),     tr(2.5, 4, -1, -40), tr(1, 0, 0, 273.15),
        tr(-0.5, 1, 0, 10), tr(3, 7, 1, 0.25),
    };
    const double xs[] = {0, 1, -1, 21.5, 1234.5678, -40, 1e6, 0.001};
    for (size_t i = 0; i < sizeof ts / sizeof ts[0]; i++)
        for (size_t j = 0; j < sizeof xs / sizeof xs[0]; j++) {
            double raw = NAN;
            int rc = tdot_transform_invert(&ts[i], xs[j], &raw);
            CHECK(rc == 0, "transform %zu: invert(%g) failed: %d", i, xs[j], rc);
            double back = tdot_transform_apply(&ts[i], raw);
            CHECK(close_to(back, xs[j]),
                  "transform %zu: apply(invert(%g)) = %.17g", i, xs[j], back);
        }
}

static void check_invert_values(void) {
    double raw = 0;
    tdot_transform_t t = tr(0.1, 1, 0, 0);
    CHECK(tdot_transform_invert(&t, 21.5, &raw) == 0 && close_to(raw, 215),
          "multiplier 0.1: 21.5 -> %.17g", raw);
    t = tr(1, 1, -3, 0); /* decimal_shift -3 divides by 1000 on read */
    CHECK(tdot_transform_invert(&t, 1.5, &raw) == 0 && close_to(raw, 1500),
          "decimal_shift -3: 1.5 -> %.17g", raw);
    t = tr(1, 10, 0, -40); /* read: raw / 10 - 40 */
    CHECK(tdot_transform_invert(&t, 21.5, &raw) == 0 && close_to(raw, 615),
          "divisor 10 offset -40: 21.5 -> %.17g", raw);
    t = tr(2, 0, 0, 0); /* divisor 0 is 1, as on read */
    CHECK(tdot_transform_invert(&t, 8, &raw) == 0 && raw == 4,
          "divisor 0: 8 -> %.17g", raw);

    raw = 42;
    t = tr(0, 1, 0, 5);
    CHECK(tdot_transform_invert(&t, 5, &raw) == TDOT_TRANSFORM_NOT_INVERTIBLE &&
              raw == 42,
          "multiplier 0 must not be invertible (raw %g)", raw);
    t = tr(1, 1, -400, 0); /* 10^-400 underflows to 0 */
    CHECK(tdot_transform_invert(&t, 5, &raw) == TDOT_TRANSFORM_NOT_INVERTIBLE,
          "a scale that underflows to 0 must not be invertible");
    t = tr(1e-300, 1e300, 0, 0);
    CHECK(tdot_transform_invert(&t, 1e300, &raw) == TDOT_TRANSFORM_NOT_FINITE,
          "an infinite raw value must be refused");
}

/* ---- the write path, through a connector that records what it got -------- */

static tdot_value_t g_written;
static int g_writes;

static int rec_write(tdot_connector_t *self, tdot_device_t *dev,
                     tdot_point_t *pt, const tdot_value_t *value, char *err,
                     size_t errlen) {
    (void)self, (void)dev, (void)pt, (void)err, (void)errlen;
    g_written = *value;
    g_writes++;
    return 0;
}

static tdot_config_t *load(const char *points) {
    char tmpl[] = "/tmp/tdot-write-XXXXXX";
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
            "\n"
            "[[device]]\n"
            "name = \"plc1\"\n"
            "protocol_address = { transport = \"tcp\", host = \"127.0.0.1\", "
            "port = 502, unit_id = 1 }\n"
            "%s",
            points);
    fclose(fp);
    char err[256];
    tdot_config_t *cfg = tdot_config_load(path, err, sizeof err);
    if (!cfg) {
        printf("FAIL config did not load: %s\n", err);
        exit(1);
    }
    unlink(path);
    rmdir(dir);
    return cfg;
}

#define POINT(id, dt, extra)                                                   \
    "\n  [[device.point]]\n  id = \"" id "\"\n  datatype = \"" dt "\"\n"       \
    "  access = \"read_write\"\n"                                               \
    "  address = { table = \"holding\", address = 0, count = 1 }\n" extra

static tdot_value_t num(double v) {
    tdot_value_t x = {.kind = TDOT_VAL_NUM, .num = v};
    return x;
}

/* Write `in` to point `id`; returns write_point's value (kind NONE when the
 * write failed before reaching it, with `err` filled). */
static tdot_value_t write_to(tdot_config_t *cfg, const char *id, tdot_value_t in,
                          char *err, size_t errlen) {
    tdot_connector_t conn = {.protocol = "rec", .write_point = rec_write};
    tdot_device_t *dev = &cfg->devices[0];
    tdot_point_t *pt = tdot_device_point(dev, id);
    memset(&g_written, 0, sizeof g_written);
    err[0] = '\0';
    tdot_connector_write(&conn, dev, pt, &in, err, errlen);
    return g_written;
}

static void check_write_path(void) {
    tdot_config_t *cfg = load(
        POINT("sp_i16", "int16", "  transform = { multiplier = 0.1 }\n")
        POINT("sp_u32", "uint32", "  transform = { decimal_shift = -1, offset = -40 }\n")
        POINT("sp_i64", "int64", "  transform = { multiplier = 0.1 }\n")
        POINT("sp_f32", "float32", "  transform = { divisor = 4 }\n")
        POINT("sp_f64", "float64", "  transform = { multiplier = 0.1 }\n")
        POINT("sp_half", "int16", "  transform = { divisor = 2 }\n")
        POINT("plain", "int16", "")
        POINT("identity", "float64", "  transform = { multiplier = 1 }\n")
        POINT("dead", "int16", "  transform = { multiplier = 0 }\n")
        POINT("flag", "bool", "  transform = { multiplier = 10 }\n")
        POINT("label", "string", "  transform = { multiplier = 10 }\n")
        POINT("raw_pt", "uint16", "  mode = \"raw\"\n  transform = { multiplier = 0.1 }\n"));
    char err[256];
    tdot_value_t out;

    /* integer datatypes: inverted, then rounded -- 0.7 / 0.1 is
     * 6.999999999999999 in floating point, which must be 7, not truncated */
    out = write_to(cfg, "sp_i16", num(21.5), err, sizeof err);
    CHECK(out.kind == TDOT_VAL_NUM && out.num == 215, "int16 x0.1: got %.17g", out.num);
    out = write_to(cfg, "sp_i16", num(0.7), err, sizeof err);
    CHECK(out.num == 7, "int16 x0.1 inexact: got %.17g", out.num);
    out = write_to(cfg, "sp_i16", num(-21.5), err, sizeof err);
    CHECK(out.num == -215, "int16 x0.1 negative: got %.17g", out.num);
    out = write_to(cfg, "sp_u32", num(21.5), err, sizeof err);
    CHECK(out.num == 615, "uint32 shift -1 offset -40: got %.17g", out.num);
    out = write_to(cfg, "sp_i64", num(0.3), err, sizeof err);
    CHECK(out.num == 3, "int64 x0.1: got %.17g", out.num);
    /* ties round away from zero */
    out = write_to(cfg, "sp_half", num(1.25), err, sizeof err);
    CHECK(out.num == 3, "int16 /2 tie: got %.17g", out.num);
    out = write_to(cfg, "sp_half", num(-1.25), err, sizeof err);
    CHECK(out.num == -3, "int16 /2 negative tie: got %.17g", out.num);

    /* float datatypes: inverted, not rounded */
    out = write_to(cfg, "sp_f32", num(1.3), err, sizeof err);
    CHECK(close_to(out.num, 5.2), "float32 /4: got %.17g", out.num);
    out = write_to(cfg, "sp_f64", num(0.25), err, sizeof err);
    CHECK(close_to(out.num, 2.5), "float64 x0.1: got %.17g", out.num);

    /* no transform / identity: bit-exact passthrough */
    out = write_to(cfg, "plain", num(21.5), err, sizeof err);
    CHECK(out.num == 21.5, "no transform: got %.17g", out.num);
    out = write_to(cfg, "identity", num(0.1), err, sizeof err);
    CHECK(out.num == 0.1, "identity: got %.17g", out.num);

    /* bool/string values and raw points are never touched */
    tdot_value_t b = {.kind = TDOT_VAL_BOOL, .b = true};
    out = write_to(cfg, "flag", b, err, sizeof err);
    CHECK(out.kind == TDOT_VAL_BOOL && out.b, "bool passthrough");
    tdot_value_t s = {.kind = TDOT_VAL_STR};
    snprintf(s.str, sizeof s.str, "abc");
    out = write_to(cfg, "label", s, err, sizeof err);
    CHECK(out.kind == TDOT_VAL_STR && !strcmp(out.str, "abc"), "string passthrough");
    snprintf(s.str, sizeof s.str, "00d7");
    out = write_to(cfg, "raw_pt", s, err, sizeof err);
    CHECK(out.kind == TDOT_VAL_STR && !strcmp(out.str, "00d7"), "raw passthrough");
    out = write_to(cfg, "raw_pt", num(21.5), err, sizeof err);
    CHECK(out.num == 21.5, "raw-mode number passthrough: got %.17g", out.num);

    /* a transform without an inverse fails the write, and writes nothing */
    int before = g_writes;
    out = write_to(cfg, "dead", num(5), err, sizeof err);
    CHECK(g_writes == before && out.kind == TDOT_VAL_NONE,
          "non-invertible transform must not reach write_point");
    CHECK(!strcmp(err, "point dead transform is not invertible (multiplier 0)"),
          "non-invertible reason: %s", err);

    tdot_config_free(cfg);
}

int main(void) {
    check_invert_round_trips();
    check_invert_values();
    check_write_path();
    if (failures) {
        printf("%d failure(s)\n", failures);
        return 1;
    }
    printf("write: all checks passed\n");
    return 0;
}
