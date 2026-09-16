/* tedge-dot — SNMP value handling (see snmp_value.h). */
#include "snmp_value.h"

#include <inttypes.h>
#include <stdio.h>
#include <string.h>

static const char *const TYPE_NAMES[] = {
    [TSNMP_TYPE_INTEGER] = "integer",
    [TSNMP_TYPE_OCTET_STRING] = "octet_string",
    [TSNMP_TYPE_NULL] = "null",
    [TSNMP_TYPE_OID] = "oid",
    [TSNMP_TYPE_IP_ADDRESS] = "ip_address",
    [TSNMP_TYPE_COUNTER32] = "counter32",
    [TSNMP_TYPE_GAUGE32] = "gauge32",
    [TSNMP_TYPE_TIMETICKS] = "timeticks",
    [TSNMP_TYPE_OPAQUE] = "opaque",
    [TSNMP_TYPE_COUNTER64] = "counter64",
    [TSNMP_TYPE_NO_SUCH_OBJECT] = "no_such_object",
    [TSNMP_TYPE_NO_SUCH_INSTANCE] = "no_such_instance",
    [TSNMP_TYPE_END_OF_MIB_VIEW] = "end_of_mib_view",
    [TSNMP_TYPE_UNKNOWN] = "unknown",
};

const char *tsnmp_type_name(tsnmp_type_t type) {
    if ((size_t)type < sizeof TYPE_NAMES / sizeof TYPE_NAMES[0])
        return TYPE_NAMES[type];
    return "unknown";
}

int tsnmp_type_parse(const char *name, tsnmp_type_t *type) {
    for (size_t i = 0; i < sizeof TYPE_NAMES / sizeof TYPE_NAMES[0]; i++)
        if (strcmp(TYPE_NAMES[i], name) == 0) {
            *type = (tsnmp_type_t)i;
            return 0;
        }
    return -1;
}

int tsnmp_set_type_parse(const char *name, tsnmp_type_t *type) {
    static const struct {
        const char *name;
        tsnmp_type_t type;
    } SET_TYPES[] = {
        {"integer", TSNMP_TYPE_INTEGER},     {"unsigned32", TSNMP_TYPE_GAUGE32},
        {"gauge32", TSNMP_TYPE_GAUGE32},     {"counter32", TSNMP_TYPE_COUNTER32},
        {"counter64", TSNMP_TYPE_COUNTER64}, {"timeticks", TSNMP_TYPE_TIMETICKS},
        {"octet_string", TSNMP_TYPE_OCTET_STRING},
        {"ip_address", TSNMP_TYPE_IP_ADDRESS}, {"oid", TSNMP_TYPE_OID},
    };
    for (size_t i = 0; i < sizeof SET_TYPES / sizeof SET_TYPES[0]; i++)
        if (strcmp(SET_TYPES[i].name, name) == 0) {
            *type = SET_TYPES[i].type;
            return 0;
        }
    return -1;
}

bool tsnmp_type_is_exception(tsnmp_type_t type) {
    return type == TSNMP_TYPE_NO_SUCH_OBJECT ||
           type == TSNMP_TYPE_NO_SUCH_INSTANCE ||
           type == TSNMP_TYPE_END_OF_MIB_VIEW;
}

const char *tsnmp_exception_name(tsnmp_type_t type) {
    switch (type) {
    case TSNMP_TYPE_NO_SUCH_OBJECT:
        return "noSuchObject";
    case TSNMP_TYPE_NO_SUCH_INSTANCE:
        return "noSuchInstance";
    case TSNMP_TYPE_END_OF_MIB_VIEW:
        return "endOfMibView";
    default:
        return tsnmp_type_name(type);
    }
}

const char *tsnmp_version_name(tsnmp_version_t v) {
    return v == TSNMP_V1 ? "v1" : v == TSNMP_V2C ? "v2c" : "v3";
}

/* ---- content rules ------------------------------------------------------- */

int tsnmp_integer_decode(const uint8_t *content, size_t len, int64_t *out) {
    if (len < 1 || len > 8)
        return -1;
    uint64_t u = 0;
    for (size_t i = 0; i < len; i++)
        u = (u << 8) | content[i];
    if (len < 8 && (content[0] & 0x80))
        u |= UINT64_MAX << (8 * len); /* sign-extend */
    *out = (int64_t)u;
    return 0;
}

int tsnmp_unsigned_decode(const uint8_t *content, size_t len,
                          size_t max_octets, uint64_t max, uint64_t *out) {
    if (len < 1 || len > max_octets)
        return -1;
    uint64_t u = 0;
    for (size_t i = 0; i < len; i++) {
        if (u > (UINT64_MAX >> 8))
            return -1; /* a ninth significant octet */
        u = (u << 8) | content[i]; /* a sign bit is not a sign */
    }
    if (u > max)
        return -1;
    *out = u;
    return 0;
}

size_t tsnmp_encode_integer(int64_t v, uint8_t *dst) {
    uint64_t u = (uint64_t)v;
    size_t n = 8;
    while (n > 1) {
        uint8_t top = (uint8_t)(u >> (8 * (n - 1)));
        uint8_t next = (uint8_t)(u >> (8 * (n - 2)));
        if ((top == 0x00 && !(next & 0x80)) || (top == 0xff && (next & 0x80)))
            n--;
        else
            break;
    }
    for (size_t i = 0; i < n; i++)
        dst[i] = (uint8_t)(u >> (8 * (n - 1 - i)));
    return n;
}

size_t tsnmp_encode_unsigned(uint64_t v, uint8_t *dst) {
    size_t n = 1;
    for (uint64_t x = v >> 8; x; x >>= 8)
        n++;
    size_t at = 0;
    if ((v >> (8 * (n - 1))) & 0x80)
        dst[at++] = 0x00; /* not a sign */
    for (size_t i = 0; i < n; i++)
        dst[at + i] = (uint8_t)(v >> (8 * (n - 1 - i)));
    return at + n;
}

/* ---- OIDs --------------------------------------------------------------- */

static int oid_push(tsnmp_oid_t *out, uint32_t arc, char *err, size_t errlen) {
    if (out->n >= TSNMP_MAX_ARCS) {
        snprintf(err, errlen, "oid longer than %d arcs", TSNMP_MAX_ARCS);
        return -1;
    }
    out->arcs[out->n++] = arc;
    return 0;
}

int tsnmp_oid_decode(const uint8_t *content, size_t len, tsnmp_oid_t *out,
                     char *err, size_t errlen) {
    out->n = 0;
    if (len == 0) {
        snprintf(err, errlen, "empty oid");
        return -1;
    }
    uint64_t v = 0;
    bool first = true;
    for (size_t i = 0; i < len; i++) {
        v = (v << 7) | (content[i] & 0x7f);
        if (v > UINT32_MAX) {
            snprintf(err, errlen, "oid arc overflow");
            return -1;
        }
        if (content[i] & 0x80)
            continue;
        if (first) {
            uint32_t s = (uint32_t)v;
            uint32_t a0 = s < 40 ? 0 : s < 80 ? 1 : 2;
            if (oid_push(out, a0, err, errlen) != 0 ||
                oid_push(out, s - a0 * 40, err, errlen) != 0)
                return -1;
            first = false;
        } else if (oid_push(out, (uint32_t)v, err, errlen) != 0) {
            return -1;
        }
        v = 0;
    }
    if (content[len - 1] & 0x80) {
        snprintf(err, errlen, "truncated oid");
        return -1;
    }
    return 0;
}

/* Base-128 subidentifier, minimal. Returns its length; writes what fits. */
static size_t subid_encode(uint64_t v, uint8_t *dst, size_t at, size_t cap) {
    size_t n = 1;
    for (uint64_t x = v >> 7; x; x >>= 7)
        n++;
    for (size_t i = 0; i < n; i++) {
        uint8_t septet = (uint8_t)((v >> (7 * (n - 1 - i))) & 0x7f);
        if (i + 1 < n)
            septet |= 0x80;
        if (at + i < cap)
            dst[at + i] = septet;
    }
    return n;
}

size_t tsnmp_oid_encode(const tsnmp_oid_t *oid, uint8_t *dst, size_t cap) {
    if (oid->n == 0)
        return 0;
    uint64_t first = (uint64_t)oid->arcs[0] * 40 +
                     (oid->n > 1 ? (uint64_t)oid->arcs[1] : 0);
    size_t at = subid_encode(first, dst, 0, cap);
    for (size_t i = 2; i < oid->n; i++)
        at += subid_encode(oid->arcs[i], dst, at, cap);
    return at;
}

int tsnmp_oid_parse(const char *text, tsnmp_oid_t *out, char *err,
                    size_t errlen) {
    const char *p = text;
    out->n = 0;
    if (*p == '.')
        p++;
    for (;;) {
        if (*p < '0' || *p > '9') {
            snprintf(err, errlen, "invalid OID '%s': expected dotted decimal",
                     text);
            return -1;
        }
        uint64_t v = 0;
        for (; *p >= '0' && *p <= '9'; p++) {
            v = v * 10 + (uint64_t)(*p - '0');
            if (v > UINT32_MAX) {
                snprintf(err, errlen,
                         "invalid OID '%s': an arc is larger than 32 bits", text);
                return -1;
            }
        }
        if (out->n >= TSNMP_MAX_ARCS) {
            snprintf(err, errlen, "invalid OID '%s': more than %d arcs", text,
                     TSNMP_MAX_ARCS);
            return -1;
        }
        out->arcs[out->n++] = (uint32_t)v;
        if (*p == '\0')
            break;
        if (*p != '.') {
            snprintf(err, errlen, "invalid OID '%s': expected dotted decimal",
                     text);
            return -1;
        }
        p++;
    }
    if (out->n < 2) {
        snprintf(err, errlen, "invalid OID '%s': at least two arcs required",
                 text);
        return -1;
    }
    if (out->arcs[0] > 2) {
        snprintf(err, errlen,
                 "invalid OID '%s': the first arc must be 0, 1 or 2", text);
        return -1;
    }
    if (out->arcs[0] < 2 && out->arcs[1] >= 40) {
        snprintf(err, errlen,
                 "invalid OID '%s': the second arc must be below 40 unless the "
                 "first is 2",
                 text);
        return -1;
    }
    /* 2.x: the first subidentifier (80 + x) must still fit in 32 bits */
    if (out->arcs[0] == 2 && out->arcs[1] > UINT32_MAX - 80) {
        snprintf(err, errlen,
                 "invalid OID '%s': under 2 the second arc must be at most "
                 "4294967215",
                 text);
        return -1;
    }
    return 0;
}

size_t tsnmp_oid_format(const tsnmp_oid_t *oid, char *dst, size_t cap) {
    size_t used = 0;
    if (cap)
        dst[0] = '\0';
    for (size_t i = 0; i < oid->n; i++) {
        int n = snprintf(used < cap ? dst + used : NULL,
                         used < cap ? cap - used : 0, "%s%" PRIu32,
                         i ? "." : "", oid->arcs[i]);
        if (n > 0)
            used += (size_t)n;
    }
    return used;
}

bool tsnmp_oid_equal(const tsnmp_oid_t *a, const tsnmp_oid_t *b) {
    return a->n == b->n &&
           memcmp(a->arcs, b->arcs, a->n * sizeof a->arcs[0]) == 0;
}

bool tsnmp_oid_under(const tsnmp_oid_t *oid, const tsnmp_oid_t *prefix) {
    return oid->n >= prefix->n &&
           memcmp(oid->arcs, prefix->arcs, prefix->n * sizeof oid->arcs[0]) == 0;
}

/* ---- conversion (§6) ---------------------------------------------------- */

bool tsnmp_utf8_valid(const uint8_t *s, size_t len) {
    size_t i = 0;
    while (i < len) {
        uint8_t b = s[i];
        size_t n;
        uint8_t lo = 0x80, hi = 0xBF; /* range of the second octet */
        if (b < 0x80) {
            i++;
            continue;
        } else if (b >= 0xC2 && b <= 0xDF) {
            n = 2;
        } else if (b >= 0xE0 && b <= 0xEF) {
            n = 3;
            if (b == 0xE0)
                lo = 0xA0; /* overlong */
            else if (b == 0xED)
                hi = 0x9F; /* surrogates */
        } else if (b >= 0xF0 && b <= 0xF4) {
            n = 4;
            if (b == 0xF0)
                lo = 0x90; /* overlong */
            else if (b == 0xF4)
                hi = 0x8F; /* above U+10FFFF */
        } else {
            return false; /* continuation, overlong C0/C1, or F5..FF */
        }
        if (len - i < n)
            return false;
        if (s[i + 1] < lo || s[i + 1] > hi)
            return false;
        for (size_t k = 2; k < n; k++)
            if ((s[i + k] & 0xC0) != 0x80)
                return false;
        i += n;
    }
    return true;
}

/* Copy a valid UTF-8 string into dst, cutting at a character boundary when
 * it does not fit. */
static void utf8_copy(const uint8_t *s, size_t len, char *dst, size_t cap) {
    size_t n = len < cap - 1 ? len : cap - 1;
    while (n > 0 && n < len && (s[n] & 0xC0) == 0x80)
        n--;
    memcpy(dst, s, n);
    dst[n] = '\0';
}

/* Inclusive range of an integer datatype. */
static void datatype_range(tdot_datatype_t dt, int64_t *lo, uint64_t *hi) {
    switch (dt) {
    case TDOT_DT_INT8: *lo = INT8_MIN; *hi = INT8_MAX; break;
    case TDOT_DT_UINT8: *lo = 0; *hi = UINT8_MAX; break;
    case TDOT_DT_INT16: *lo = INT16_MIN; *hi = INT16_MAX; break;
    case TDOT_DT_UINT16: *lo = 0; *hi = UINT16_MAX; break;
    case TDOT_DT_INT32: *lo = INT32_MIN; *hi = INT32_MAX; break;
    case TDOT_DT_UINT32: *lo = 0; *hi = UINT32_MAX; break;
    case TDOT_DT_INT64: *lo = INT64_MIN; *hi = INT64_MAX; break;
    default: *lo = 0; *hi = UINT64_MAX; break; /* uint64 */
    }
}

static int convert_number(tsnmp_type_t type, bool is_signed, int64_t iv,
                          uint64_t uv, tdot_datatype_t dt, tdot_value_t *out,
                          char *err, size_t errlen) {
    switch (dt) {
    case TDOT_DT_BOOL:
        out->kind = TDOT_VAL_BOOL;
        out->b = is_signed ? iv != 0 : uv != 0;
        return 0;
    case TDOT_DT_STRING:
        out->kind = TDOT_VAL_STR;
        if (is_signed)
            snprintf(out->str, sizeof out->str, "%" PRId64, iv);
        else
            snprintf(out->str, sizeof out->str, "%" PRIu64, uv);
        return 0;
    case TDOT_DT_FLOAT32:
        /* the nearest float32, published as the double it is */
        out->kind = TDOT_VAL_NUM;
        out->num = is_signed ? (double)(float)iv : (double)(float)uv;
        return 0;
    case TDOT_DT_FLOAT64:
        out->kind = TDOT_VAL_NUM;
        out->num = is_signed ? (double)iv : (double)uv;
        return 0;
    case TDOT_DT_INT8:
    case TDOT_DT_UINT8:
    case TDOT_DT_INT16:
    case TDOT_DT_UINT16:
    case TDOT_DT_INT32:
    case TDOT_DT_UINT32:
    case TDOT_DT_INT64:
    case TDOT_DT_UINT64: {
        int64_t lo;
        uint64_t hi;
        datatype_range(dt, &lo, &hi);
        bool in_range = is_signed ? (iv >= lo && (iv < 0 || (uint64_t)iv <= hi))
                                  : uv <= hi;
        if (!in_range) {
            char text[24];
            if (is_signed)
                snprintf(text, sizeof text, "%" PRId64, iv);
            else
                snprintf(text, sizeof text, "%" PRIu64, uv);
            snprintf(err, errlen, "%s value %s is out of range for %s",
                     tsnmp_type_name(type), text, tdot_datatype_str(dt));
            return -1;
        }
        /* beyond the JS safe-integer range a decimal string (contract §4.1) */
        if (is_signed ? (iv > TDOT_JS_SAFE_MAX || iv < -TDOT_JS_SAFE_MAX)
                      : uv > (uint64_t)TDOT_JS_SAFE_MAX) {
            out->kind = TDOT_VAL_STR;
            if (is_signed)
                snprintf(out->str, sizeof out->str, "%" PRId64, iv);
            else
                snprintf(out->str, sizeof out->str, "%" PRIu64, uv);
        } else {
            out->kind = TDOT_VAL_NUM;
            out->num = is_signed ? (double)iv : (double)uv;
        }
        return 0;
    }
    default:
        snprintf(err, errlen, "unsupported datatype");
        return -1;
    }
}

int tsnmp_convert(tsnmp_type_t type, const uint8_t *content, size_t len,
                  tdot_datatype_t dt, tdot_value_t *out, char *err,
                  size_t errlen) {
    memset(out, 0, sizeof *out);
    int64_t iv = 0;
    uint64_t uv = 0;
    const char *dt_name = tdot_datatype_str(dt) ? tdot_datatype_str(dt) : "none";
    switch (type) {
    case TSNMP_TYPE_INTEGER:
        if (tsnmp_integer_decode(content, len, &iv) != 0) {
            snprintf(err, errlen, "invalid integer");
            return -1;
        }
        return convert_number(type, true, iv, 0, dt, out, err, errlen);
    case TSNMP_TYPE_COUNTER32:
    case TSNMP_TYPE_GAUGE32:
    case TSNMP_TYPE_TIMETICKS:
    case TSNMP_TYPE_COUNTER64: {
        bool wide = type == TSNMP_TYPE_COUNTER64;
        if (tsnmp_unsigned_decode(content, len, wide ? 9 : 5,
                                  wide ? UINT64_MAX : UINT32_MAX, &uv) != 0) {
            snprintf(err, errlen, "invalid %s", tsnmp_type_name(type));
            return -1;
        }
        return convert_number(type, false, 0, uv, dt, out, err, errlen);
    }
    case TSNMP_TYPE_OCTET_STRING:
        if (dt != TDOT_DT_STRING)
            break;
        if (!tsnmp_utf8_valid(content, len)) {
            snprintf(err, errlen,
                     "octet_string is not valid UTF-8 (read it with mode = \"raw\")");
            return -1;
        }
        out->kind = TDOT_VAL_STR;
        utf8_copy(content, len, out->str, sizeof out->str);
        return 0;
    case TSNMP_TYPE_OID: {
        if (dt != TDOT_DT_STRING)
            break;
        tsnmp_oid_t oid;
        if (tsnmp_oid_decode(content, len, &oid, err, errlen) != 0)
            return -1;
        out->kind = TDOT_VAL_STR;
        tsnmp_oid_format(&oid, out->str, sizeof out->str);
        return 0;
    }
    case TSNMP_TYPE_IP_ADDRESS:
        if (dt != TDOT_DT_STRING)
            break;
        if (len != 4) {
            snprintf(err, errlen, "invalid ip_address");
            return -1;
        }
        out->kind = TDOT_VAL_STR;
        snprintf(out->str, sizeof out->str, "%u.%u.%u.%u", content[0],
                 content[1], content[2], content[3]);
        return 0;
    default:
        break;
    }
    snprintf(err, errlen, "a value of type %s has no %s value",
             tsnmp_type_name(type), dt_name);
    return -1;
}

const char *tsnmp_error_status_name(long status) {
    static const char *const NAMES[] = {
        "noError",      "tooBig",        "noSuchName",        "badValue",
        "readOnly",     "genErr",        "noAccess",          "wrongType",
        "wrongLength",  "wrongEncoding", "wrongValue",        "noCreation",
        "inconsistentValue", "resourceUnavailable", "commitFailed",
        "undoFailed",   "authorizationError", "notWritable",
        "inconsistentName"};
    if (status >= 0 && (size_t)status < sizeof NAMES / sizeof NAMES[0])
        return NAMES[status];
    return "unknownError";
}
