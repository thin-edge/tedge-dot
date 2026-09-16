/* SNMP connector checks (doc/connectors/snmp-connector-spec.md).
 *
 * Everything goes through the same library path the connector uses (net-snmp
 * via snmp_netsnmp.c), so the vectors hold the decoder the product ships:
 *
 *   - the shared golden vectors (connectors/snmp/conformance/trap-vectors.json,
 *     read by the Rust crate too): every `messages[*]` decodes to its
 *     expectation, every `malformed[*]` is rejected, every `conversions[*]`
 *     matches (§4, §6). A `v3` section is used when the file has one;
 *   - the configuration rules of §3;
 *   - notifications end to end over the loopback: routing, trap and varbind
 *     points, a bad sample for a missing varbind, community rejection, inform
 *     acknowledgement, forwarder routing, and v3 authPriv traps (§4);
 *   - polling, batching and SET against an SNMP responder in a child process,
 *     v1/v2c/v3 (§5).
 *
 *   tedge-dot-snmp <trap-vectors.json>
 */
#include <arpa/inet.h>
#include <inttypes.h>
#include <netinet/in.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#include "cjson/cJSON.h"
#include "snmp_netsnmp.h"
#include "tedge_dot/config.h"
#include "tedge_dot/connector.h"

static int failures = 0;
static int tolerated_raw = 0; /* vector raw octets kept as received (§6) */

#define CHECK(cond, ...)                                                       \
    do {                                                                       \
        if (!(cond)) {                                                         \
            failures++;                                                        \
            printf("FAIL %s:%d: ", __FILE__, __LINE__);                        \
            printf(__VA_ARGS__);                                               \
            printf("\n");                                                      \
        }                                                                      \
    } while (0)

/* ---- small helpers ------------------------------------------------------- */

static char *slurp(const char *path) {
    FILE *fp = fopen(path, "rb");
    if (!fp)
        return NULL;
    fseek(fp, 0, SEEK_END);
    long n = ftell(fp);
    fseek(fp, 0, SEEK_SET);
    char *buf = malloc((size_t)n + 1);
    size_t got = fread(buf, 1, (size_t)n, fp);
    buf[got] = '\0';
    fclose(fp);
    return buf;
}

static long unhex(const char *hex, uint8_t **out) {
    size_t n = strlen(hex);
    *out = malloc(n / 2 + 1);
    if (n % 2)
        return -1;
    for (size_t i = 0; i < n / 2; i++) {
        unsigned v;
        if (sscanf(hex + 2 * i, "%2x", &v) != 1)
            return -1;
        (*out)[i] = (uint8_t)v;
    }
    return (long)(n / 2);
}

static void tohex(const uint8_t *b, size_t len, char *dst) {
    for (size_t i = 0; i < len; i++)
        sprintf(dst + 2 * i, "%02x", b[i]);
    dst[2 * len] = '\0';
}

static const char *jstr(const cJSON *obj, const char *key) {
    const cJSON *v = cJSON_GetObjectItem(obj, key);
    return cJSON_IsString(v) ? v->valuestring : NULL;
}

static bool field_is(const cJSON *obj, const char *key, const char *got) {
    const cJSON *v = cJSON_GetObjectItem(obj, key);
    if (!got)
        return cJSON_IsNull(v);
    return cJSON_IsString(v) && strcmp(v->valuestring, got) == 0;
}

static void pause_ms(int ms) {
    struct timespec ts = {.tv_sec = ms / 1000, .tv_nsec = (ms % 1000) * 1000000L};
    nanosleep(&ts, NULL);
}

static int free_udp_port(void) {
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in a = {.sin_family = AF_INET};
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    bind(fd, (struct sockaddr *)&a, sizeof a);
    socklen_t len = sizeof a;
    getsockname(fd, (struct sockaddr *)&a, &len);
    close(fd);
    return ntohs(a.sin_port);
}

/* ---- golden vectors ------------------------------------------------------ */

/* The vectors' `value` of one varbind, from its canonical content octets. */
static const char *value_text(tsnmp_type_t type, const uint8_t *content,
                              size_t len, char *buf, size_t cap) {
    int64_t i;
    uint64_t u;
    tsnmp_oid_t oid;
    char err[80];
    switch (type) {
    case TSNMP_TYPE_INTEGER:
        tsnmp_integer_decode(content, len, &i);
        snprintf(buf, cap, "%" PRId64, i);
        return buf;
    case TSNMP_TYPE_COUNTER32:
    case TSNMP_TYPE_GAUGE32:
    case TSNMP_TYPE_TIMETICKS:
    case TSNMP_TYPE_COUNTER64:
        tsnmp_unsigned_decode(content, len, 9, UINT64_MAX, &u);
        snprintf(buf, cap, "%" PRIu64, u);
        return buf;
    case TSNMP_TYPE_OCTET_STRING:
    case TSNMP_TYPE_OPAQUE:
        tohex(content, len, buf);
        return buf;
    case TSNMP_TYPE_OID:
        if (tsnmp_oid_decode(content, len, &oid, err, sizeof err) != 0)
            return "<bad oid>";
        tsnmp_oid_format(&oid, buf, cap);
        return buf;
    case TSNMP_TYPE_IP_ADDRESS:
        snprintf(buf, cap, "%u.%u.%u.%u", content[0], content[1], content[2],
                 content[3]);
        return buf;
    default:
        return NULL; /* null, the exceptions and unknown have no value */
    }
}

/* `raw` is the canonical content octets (§6). The vectors were generated from
 * the octets AS RECEIVED, so a non-minimal encoding (a padded OID
 * subidentifier, a Counter32 without its leading zero) differs by design: it
 * is accepted when it decodes to the same value, and counted. */
static bool raw_ok(tsnmp_type_t type, const char *want_hex, const char *got_hex) {
    if (strcmp(want_hex, got_hex) == 0)
        return true;
    uint8_t *want = NULL;
    long n = unhex(want_hex, &want);
    uint8_t canon[TSNMP_CANON_MAX];
    char text[TSNMP_CANON_MAX * 2 + 1] = "";
    size_t len = 0;
    int64_t i;
    uint64_t u;
    tsnmp_oid_t oid;
    char err[80];
    if (n >= 0) {
        switch (type) {
        case TSNMP_TYPE_INTEGER:
            if (tsnmp_integer_decode(want, (size_t)n, &i) == 0)
                len = tsnmp_encode_integer(i, canon);
            break;
        case TSNMP_TYPE_COUNTER32:
        case TSNMP_TYPE_GAUGE32:
        case TSNMP_TYPE_TIMETICKS:
        case TSNMP_TYPE_COUNTER64:
            if (tsnmp_unsigned_decode(want, (size_t)n, 9, UINT64_MAX, &u) == 0)
                len = tsnmp_encode_unsigned(u, canon);
            break;
        case TSNMP_TYPE_OID:
            if (tsnmp_oid_decode(want, (size_t)n, &oid, err, sizeof err) == 0)
                len = tsnmp_oid_encode(&oid, canon, sizeof canon);
            break;
        default:
            break;
        }
    }
    free(want);
    if (len)
        tohex(canon, len, text);
    bool ok = len && strcmp(text, got_hex) == 0;
    if (ok)
        tolerated_raw++;
    return ok;
}

static void check_message(const cJSON *v) {
    const char *name = jstr(v, "name");
    const cJSON *e = cJSON_GetObjectItem(v, "expect");
    uint8_t *bytes;
    long len = unhex(jstr(v, "hex"), &bytes);
    netsnmp_pdu *pdu = NULL;
    int lib_errno = 0;
    tsnmp_notification_t n;
    char err[200];

    tsnmp_lock();
    int rc = len < 0 ? -1
                     : tsnmp_parse_datagram(bytes, (size_t)len, &pdu, &lib_errno);
    if (rc == 0)
        rc = tsnmp_notification_check(pdu, &n, err, sizeof err);
    else
        snprintf(err, sizeof err, "%s", snmp_api_errstring(lib_errno));
    if (rc != 0) {
        CHECK(0, "[%s] should decode: %s", name, len < 0 ? "bad hex" : err);
        goto out;
    }

    char big[TSNMP_CANON_MAX * 2 + TSNMP_OID_STR_MAX + 64];
    char small[64];
    CHECK(field_is(e, "version", tsnmp_version_name(n.version)), "[%s] version",
          name);
    tohex(pdu->community, pdu->community_len, big);
    CHECK(field_is(e, "community", big), "[%s] community %s", name, big);
    CHECK(field_is(e, "pdu", n.kind == TSNMP_PDU_INFORM ? "inform" : "trap"),
          "[%s] pdu", name);
    snprintf(small, sizeof small, "%" PRId64, n.request_id);
    CHECK(field_is(e, "request_id", n.has_request_id ? small : NULL),
          "[%s] request_id %s", name, small);
    snprintf(small, sizeof small, "%u.%u.%u.%u", n.agent_addr[0], n.agent_addr[1],
             n.agent_addr[2], n.agent_addr[3]);
    CHECK(field_is(e, "agent_addr", n.has_agent_addr ? small : NULL),
          "[%s] agent_addr %s", name, small);
    snprintf(small, sizeof small, "%" PRIu32, n.uptime);
    CHECK(field_is(e, "uptime", n.has_uptime ? small : NULL), "[%s] uptime %s",
          name, small);
    tsnmp_oid_format(&n.trap, big, sizeof big);
    CHECK(field_is(e, "trap", big), "[%s] trap %s", name, big);
    uint8_t enc[TSNMP_CANON_MAX];
    tohex(enc, tsnmp_oid_encode(&n.trap, enc, sizeof enc), big);
    CHECK(field_is(e, "trap_raw", big), "[%s] trap_raw %s", name, big);

    const cJSON *vbs = cJSON_GetObjectItem(e, "varbinds");
    int want_vbs = cJSON_GetArraySize(vbs);
    CHECK((size_t)want_vbs == n.nvarbinds, "[%s] %zu varbinds, expected %d", name,
          n.nvarbinds, want_vbs);
    const netsnmp_variable_list *var = pdu->variables;
    for (int i = 0; i < want_vbs && var; i++, var = var->next_variable) {
        const cJSON *w = cJSON_GetArrayItem(vbs, i);
        tsnmp_oid_t oid;
        if (tsnmp_var_name(var, &oid))
            tsnmp_oid_format(&oid, big, sizeof big);
        else
            snprintf(big, sizeof big, "<oversized oid>");
        CHECK(field_is(w, "oid", big), "[%s] varbind %d oid %s", name, i, big);

        uint8_t canon[TSNMP_CANON_MAX];
        const uint8_t *content;
        size_t clen;
        tsnmp_type_t type = tsnmp_var_content(var, canon, &content, &clen);
        CHECK(field_is(w, "type", tsnmp_type_name(type)), "[%s] varbind %d type %s",
              name, i, tsnmp_type_name(type));
        char vbuf[TSNMP_OID_STR_MAX + 64];
        const char *value = clen * 2 < sizeof big
                                ? value_text(type, content, clen, vbuf, sizeof vbuf)
                                : NULL;
        if (type == TSNMP_TYPE_OCTET_STRING || type == TSNMP_TYPE_OPAQUE) {
            /* long strings: compare the hex directly */
            char *hex = malloc(clen * 2 + 1);
            tohex(content, clen, hex);
            CHECK(field_is(w, "value", hex), "[%s] varbind %d value", name, i);
            CHECK(field_is(w, "raw", hex), "[%s] varbind %d raw", name, i);
            free(hex);
            continue;
        }
        CHECK(field_is(w, "value", value), "[%s] varbind %d value %s", name, i,
              value ? value : "null");
        tohex(content, clen, big);
        CHECK(raw_ok(type, jstr(w, "raw"), big), "[%s] varbind %d raw %s (want %s)",
              name, i, big, jstr(w, "raw"));
    }
out:
    if (pdu)
        snmp_free_pdu(pdu);
    tsnmp_unlock();
    free(bytes);
}

static void check_conversion(const cJSON *c, int index) {
    tsnmp_type_t type;
    const char *type_name = jstr(c, "type");
    const char *dt_name = jstr(c, "datatype");
    uint8_t *raw;
    long len = unhex(jstr(c, "raw"), &raw);
    if (len < 0 || tsnmp_type_parse(type_name, &type) != 0) {
        CHECK(0, "[conversion %d] unreadable vector", index);
        free(raw);
        return;
    }
    tdot_datatype_t dt = tdot_datatype_parse(dt_name);
    tdot_value_t val;
    char err[160];
    int rc = tsnmp_convert(type, raw, (size_t)len, dt, &val, err, sizeof err);
    free(raw);

    if (cJSON_IsTrue(cJSON_GetObjectItem(c, "bad"))) {
        CHECK(rc != 0, "[conversion %d] %s %s -> %s should be bad", index,
              type_name, jstr(c, "raw"), dt_name);
        return;
    }
    if (rc != 0) {
        CHECK(0, "[conversion %d] %s %s -> %s: unexpected bad: %s", index,
              type_name, jstr(c, "raw"), dt_name, err);
        return;
    }
    const char *repr = jstr(c, "repr");
    const cJSON *want = cJSON_GetObjectItem(c, "value");
    if (strcmp(repr, "number") == 0)
        CHECK(val.kind == TDOT_VAL_NUM && val.num == want->valuedouble,
              "[conversion %d] %s -> %s: want %.17g, got kind %d %.17g", index,
              type_name, dt_name, want->valuedouble, val.kind, val.num);
    else if (strcmp(repr, "boolean") == 0)
        CHECK(val.kind == TDOT_VAL_BOOL && val.b == cJSON_IsTrue(want),
              "[conversion %d] %s -> %s: boolean mismatch", index, type_name,
              dt_name);
    else
        CHECK(val.kind == TDOT_VAL_STR && strcmp(val.str, want->valuestring) == 0,
              "[conversion %d] %s -> %s: want \"%s\", got kind %d \"%s\"", index,
              type_name, dt_name, want->valuestring, val.kind, val.str);
}

/* A v3 vector: a datagram, the USM user it was built with, and whether it must
 * authenticate. The section is optional while the Rust side generates it. */
static void check_v3_vector(const cJSON *v) {
    const char *name = jstr(v, "name");
    const char *hex = jstr(v, "hex");
    const cJSON *user = cJSON_GetObjectItem(v, "user");
    bool expect_ok = !cJSON_IsFalse(cJSON_GetObjectItem(v, "accepted"));
    if (!hex || !cJSON_IsObject(user)) {
        printf("skip  [%s]: no hex/user in the vector\n", name ? name : "?");
        return;
    }
    tsnmp_v3_creds_t creds;
    memset(&creds, 0, sizeof creds);
    snprintf(creds.user, sizeof creds.user, "%s",
             jstr(user, "name") ? jstr(user, "name") : "");
    const char *auth = jstr(user, "auth_protocol");
    const char *priv = jstr(user, "priv_protocol");
    if (auth && strcmp(auth, "MD5") == 0)
        creds.auth = TSNMP_AUTH_MD5;
    else if (auth && strcmp(auth, "SHA") == 0)
        creds.auth = TSNMP_AUTH_SHA;
    else if (auth) {
        printf("skip  [%s]: auth_protocol %s needs snmpv3-sha2\n", name, auth);
        return;
    }
    if (priv && strcmp(priv, "DES") == 0)
        creds.priv = TSNMP_PRIV_DES;
    else if (priv && strcmp(priv, "AES") == 0)
        creds.priv = TSNMP_PRIV_AES;
    else if (priv) {
        printf("skip  [%s]: priv_protocol %s needs snmpv3-sha2\n", name, priv);
        return;
    }
    creds.level = creds.priv ? SNMP_SEC_LEVEL_AUTHPRIV
                  : creds.auth ? SNMP_SEC_LEVEL_AUTHNOPRIV
                               : SNMP_SEC_LEVEL_NOAUTH;
    uint8_t *bytes;
    long len = unhex(hex, &bytes);
    uint8_t *engine = NULL;
    long engine_len = jstr(v, "engine_id") ? unhex(jstr(v, "engine_id"), &engine)
                                           : 0;
    char err[200];
    tsnmp_lock();
    if (creds.auth &&
        tsnmp_v3_creds_derive(&creds, jstr(user, "auth_password"),
                              jstr(user, "priv_password") ? jstr(user, "priv_password")
                                                          : "",
                              err, sizeof err) != 0) {
        tsnmp_unlock();
        CHECK(0, "[%s] cannot derive the keys: %s", name, err);
        goto out;
    }
    if (engine_len > 0)
        tsnmp_usm_install(&creds, engine, (size_t)engine_len, true);
    netsnmp_pdu *pdu = NULL;
    int lib_errno = 0;
    int rc = tsnmp_parse_datagram(bytes, (size_t)len, &pdu, &lib_errno);
    tsnmp_notification_t n;
    if (rc == 0)
        rc = tsnmp_notification_check(pdu, &n, err, sizeof err);
    if (expect_ok)
        CHECK(rc == 0, "[%s] should decode: %s", name,
              rc ? snmp_api_errstring(lib_errno) : "");
    else
        CHECK(rc != 0, "[%s] must be rejected", name);
    if (pdu)
        snmp_free_pdu(pdu);
    tsnmp_unlock();
out:
    free(bytes);
    free(engine);
}

static void check_vectors(const char *path) {
    char *text = slurp(path);
    if (!text) {
        CHECK(0, "cannot read %s", path);
        return;
    }
    cJSON *doc = cJSON_Parse(text);
    const cJSON *v;
    int messages = 0, malformed = 0, conversions = 0, v3 = 0;

    cJSON_ArrayForEach(v, cJSON_GetObjectItem(doc, "messages")) {
        check_message(v);
        messages++;
    }
    CHECK(messages > 0, "no messages in the vectors");

    cJSON_ArrayForEach(v, cJSON_GetObjectItem(doc, "malformed")) {
        uint8_t *bytes;
        long len = unhex(jstr(v, "hex"), &bytes);
        netsnmp_pdu *pdu = NULL;
        int lib_errno = 0;
        tsnmp_notification_t n;
        char err[200];
        tsnmp_lock();
        int rc = len < 0 ? -1
                         : tsnmp_parse_datagram(bytes, (size_t)len, &pdu, &lib_errno);
        if (rc == 0)
            rc = tsnmp_notification_check(pdu, &n, err, sizeof err);
        if (pdu)
            snmp_free_pdu(pdu);
        tsnmp_unlock();
        CHECK(rc != 0, "[%s] must be rejected (%s)", jstr(v, "name"),
              jstr(v, "why"));
        free(bytes);
        malformed++;
    }
    CHECK(malformed > 0, "no malformed datagrams in the vectors");

    cJSON_ArrayForEach(v, cJSON_GetObjectItem(doc, "conversions"))
        check_conversion(v, conversions++);
    CHECK(conversions > 0, "no conversions in the vectors");

    const cJSON *v3s = cJSON_GetObjectItem(doc, "v3");
    if (cJSON_IsArray(v3s)) {
        cJSON_ArrayForEach(v, v3s) {
            check_v3_vector(v);
            v3++;
        }
    }

    printf("snmp vectors: %d messages, %d malformed, %d conversions, %d v3%s "
           "(%d raw octets canonical rather than as received)\n",
           messages, malformed, conversions, v3,
           cJSON_IsArray(v3s) ? "" : " (no v3 section yet)", tolerated_raw);
    cJSON_Delete(doc);
    free(text);
}

/* A message's bytes from the vectors, by name. */
static long vector_message(const char *path, const char *name, uint8_t **out) {
    char *text = slurp(path);
    cJSON *doc = cJSON_Parse(text);
    long len = -1;
    const cJSON *v;
    cJSON_ArrayForEach(v, cJSON_GetObjectItem(doc, "messages")) {
        if (strcmp(jstr(v, "name"), name) == 0) {
            len = unhex(jstr(v, "hex"), out);
            break;
        }
    }
    cJSON_Delete(doc);
    free(text);
    return len;
}

/* ---- OID syntax (§3.3) --------------------------------------------------- */

static void check_oid_syntax(void) {
    static const char *const good[] = {
        "1.3.6.1.4.1.99999.0.1", ".1.3.6", "0.39", "2.999.4294967295",
        "2.4294967215", "1.3.6.1.4294967295"};
    static const char *const bad[] = {
        "", ".", "1", "3.1", "1.40", "0.40", "1..3", "1.3.", "abc", "1.3.a",
        "1.3.6.1.4294967296", "2.4294967216", " 1.3", "-1.3", "1.3 "};
    tsnmp_oid_t oid;
    char err[160];
    for (size_t i = 0; i < sizeof good / sizeof *good; i++)
        CHECK(tsnmp_oid_parse(good[i], &oid, err, sizeof err) == 0,
              "OID '%s' should parse: %s", good[i], err);
    for (size_t i = 0; i < sizeof bad / sizeof *bad; i++)
        CHECK(tsnmp_oid_parse(bad[i], &oid, err, sizeof err) != 0,
              "OID '%s' should be rejected", bad[i]);

    char text[TSNMP_OID_STR_MAX + 8] = "1.3";
    for (int i = 2; i < 128; i++)
        strcat(text, ".1");
    CHECK(tsnmp_oid_parse(text, &oid, err, sizeof err) == 0 && oid.n == 128,
          "128 arcs should parse: %s", err);
    strcat(text, ".1");
    CHECK(tsnmp_oid_parse(text, &oid, err, sizeof err) != 0,
          "129 arcs should be rejected");

    tsnmp_oid_t prefix;
    tsnmp_oid_parse("1.3.6.1.6.3.1.1.5", &prefix, err, sizeof err);
    tsnmp_oid_parse("1.3.6.1.6.3.1.1.5.3", &oid, err, sizeof err);
    CHECK(tsnmp_oid_under(&oid, &prefix) && tsnmp_oid_under(&prefix, &prefix) &&
              !tsnmp_oid_under(&prefix, &oid),
          "subtree matching");
    tsnmp_oid_parse("1.3.6.1.6.3.1.1.50", &oid, err, sizeof err);
    CHECK(!tsnmp_oid_under(&oid, &prefix), "arc 50 is not beneath arc 5");
}

/* ---- configuration (§3) ------------------------------------------------- */

static tdot_config_t *load_body(const char *body, char *err, size_t errlen) {
    static int counter;
    char template[] = "/tmp/tdot-snmp-XXXXXX";
    char *dir = mkdtemp(template);
    if (!dir) {
        perror("mkdtemp");
        exit(2);
    }
    char path[256];
    snprintf(path, sizeof path, "%s/snmp-%d.toml", dir, counter++);
    FILE *fp = fopen(path, "w");
    fputs(body, fp);
    fclose(fp);
    tdot_config_t *cfg = tdot_config_load(path, err, errlen);
    unlink(path);
    rmdir(dir);
    return cfg;
}

static int try_config(const char *connection, const char *device,
                      const char *point, char *err, size_t errlen) {
    char body[4096];
    snprintf(body, sizeof body,
             "[connector]\nprotocol = \"snmp\"\n\n"
             "[connection]\n%s\n"
             "[[device]]\nname = \"sw1\"\n%s\n"
             "  [[device.point]]\n  id = \"p\"\n%s\n",
             connection, device, point);
    tdot_config_t *cfg = load_body(body, err, errlen);
    if (!cfg)
        return -2; /* the loader rejected it, with its message */
    tdot_connector_t *conn = tdot_connector_factory("snmp");
    int rc = conn->configure(conn, cfg, err, errlen);
    conn->destroy(conn);
    tdot_config_free(cfg);
    return rc;
}

#define GOOD_DEVICE "protocol_address = { host = \"192.0.2.1\" }\n"
#define GOOD_POINT "  datatype = \"string\"\n  address = { trap = \"1.3.6.1.6.3.1.1.5\" }\n"
#define OBJECT_POINT "  datatype = \"int32\"\n  address = { oid = \"1.3.6.1.4.1.99999.1.1.0\" }\n"

static void expect_config_ok(const char *what, const char *connection,
                             const char *device, const char *point) {
    char err[512] = "";
    int rc = try_config(connection, device, point, err, sizeof err);
    CHECK(rc == 0, "%s: should configure, got: %s", what, err);
}

static void expect_config_error(const char *what, const char *connection,
                                const char *device, const char *point,
                                const char *needle) {
    char err[512] = "";
    int rc = try_config(connection, device, point, err, sizeof err);
    CHECK(rc == -1, "%s: configure should fail (rc %d, err '%s')", what, rc, err);
    CHECK(rc == -1 && strstr(err, needle) != NULL,
          "%s: expected '%s' in the error, got: %s", what, needle, err);
}

static void check_configuration(void) {
    expect_config_ok("defaults", "", GOOD_DEVICE, GOOD_POINT);
    expect_config_ok("object point", "", GOOD_DEVICE, OBJECT_POINT);
    expect_config_ok("connection tuning + v6 listen",
                     "listen = \"[::]:1162\"\ncommunity = [\"public\", \"site-ro\"]\n"
                     "forwarders = [\"127.0.0.1\"]\nrequest_timeout = \"3s\"\n"
                     "retries = 2\nmax_varbinds = 5\nengine_id = \"80001f8880aabbccdd\"\n",
                     "protocol_address = { host = \"192.0.2.1\", port = 1161, "
                     "version = \"v1\", community = \"x\", write_community = \"y\", "
                     "request_timeout = \"1s\", retries = 0, max_varbinds = 2, "
                     "bulk = false }\n",
                     GOOD_POINT);
    expect_config_ok("varbind point with trap list", "", GOOD_DEVICE,
                     "  datatype = \"uint64\"\n"
                     "  address = { trap = [\"1.3.6.1.6.3.1.1.5.3\", \".1.3.6.1.6.3.1.1.5.4\"], "
                     "oid = \"1.3.6.1.2.1.2.2.1.1\" }\n");
    expect_config_ok("trap point in raw mode", "", GOOD_DEVICE,
                     "  mode = \"raw\"\n  address = { trap = \"1.3.6.1\" }\n");
    expect_config_ok("writable object with a default type", "", GOOD_DEVICE,
                     "  datatype = \"int32\"\n  access = \"read_write\"\n"
                     "  address = { oid = \"1.3.6.1.4.1.1.1.0\" }\n");
    expect_config_ok("writable object with an explicit type", "", GOOD_DEVICE,
                     "  datatype = \"uint32\"\n  access = \"read_write\"\n"
                     "  address = { oid = \"1.3.6.1.4.1.1.1.0\", type = \"gauge32\" }\n");
    expect_config_ok("v3 authPriv", "",
                     "protocol_address = { host = \"192.0.2.1\", version = \"v3\", "
                     "v3 = { user = \"tedge\", auth_protocol = \"SHA\", "
                     "auth_password = \"auth-pass-1\", priv_protocol = \"AES\", "
                     "priv_password = \"priv-pass-1\", context = \"ctx\", "
                     "engine_id = \"80001f8880e2e0000000000001\" } }\n",
                     OBJECT_POINT);
    /* §10: SHA-2 / AES-192/256 load, and fail at connect naming the capability */
    expect_config_ok("v3 SHA-256 + AES-256 loads", "",
                     "protocol_address = { host = \"192.0.2.1\", version = \"v3\", "
                     "v3 = { user = \"sha2\", auth_protocol = \"SHA256\", "
                     "auth_password = \"auth-pass-1\", priv_protocol = \"AES256\", "
                     "priv_password = \"priv-pass-1\" } }\n",
                     OBJECT_POINT);

    expect_config_error("unknown [connection] key", "timeout = 5\n", GOOD_DEVICE,
                        GOOD_POINT, "[connection]: unknown key 'timeout'");
    expect_config_error("unknown protocol_address key", "",
                        "protocol_address = { host = \"192.0.2.1\", ttl = 4 }\n",
                        GOOD_POINT, "device sw1: protocol_address: unknown key 'ttl'");
    expect_config_error("unknown v3 key", "",
                        "protocol_address = { host = \"192.0.2.1\", version = \"v3\", "
                        "v3 = { user = \"u\", realm = \"x\" } }\n",
                        GOOD_POINT, "unknown key 'realm'");
    expect_config_error("unknown address key", "", GOOD_DEVICE,
                        "  datatype = \"string\"\n"
                        "  address = { trap = \"1.3.6.1\", table = \"x\" }\n",
                        "point sw1/p: address: unknown key 'table'");
    expect_config_error("missing host", "", "protocol_address = { community = \"x\" }\n",
                        GOOD_POINT, "host required");
    expect_config_error("v3 without a v3 table", "",
                        "protocol_address = { host = \"192.0.2.1\", version = \"v3\" }\n",
                        OBJECT_POINT, "needs a v3 table");
    expect_config_error("v3 without a user", "",
                        "protocol_address = { host = \"192.0.2.1\", version = \"v3\", "
                        "v3 = { auth_protocol = \"SHA\", auth_password = \"auth-pass-1\" } }\n",
                        OBJECT_POINT, "user required");
    expect_config_error("v3 password without a protocol", "",
                        "protocol_address = { host = \"192.0.2.1\", version = \"v3\", "
                        "v3 = { user = \"u\", auth_password = \"auth-pass-1\" } }\n",
                        OBJECT_POINT, "auth_protocol required");
    expect_config_error("v3 short password", "",
                        "protocol_address = { host = \"192.0.2.1\", version = \"v3\", "
                        "v3 = { user = \"u\", auth_protocol = \"SHA\", auth_password = \"short\" } }\n",
                        OBJECT_POINT, "at least 8 characters");
    expect_config_error("v3 password twice", "",
                        "protocol_address = { host = \"192.0.2.1\", version = \"v3\", "
                        "v3 = { user = \"u\", auth_protocol = \"SHA\", "
                        "auth_password = \"auth-pass-1\", auth_password_file = \"/x\" } }\n",
                        OBJECT_POINT, "not both");
    expect_config_error("v3 privacy without auth", "",
                        "protocol_address = { host = \"192.0.2.1\", version = \"v3\", "
                        "v3 = { user = \"u\", priv_protocol = \"AES\", priv_password = \"priv-pass-1\" } }\n",
                        OBJECT_POINT, "privacy needs authentication");
    expect_config_error("v3 unreadable password file", "",
                        "protocol_address = { host = \"192.0.2.1\", version = \"v3\", "
                        "v3 = { user = \"u\", auth_protocol = \"SHA\", "
                        "auth_password_file = \"/nonexistent/secret\" } }\n",
                        OBJECT_POINT, "cannot read auth_password_file");
    expect_config_error("write access on a trap point", "", GOOD_DEVICE,
                        GOOD_POINT "  access = \"read_write\"\n",
                        "must have access = \"read\"");
    expect_config_error("subscribe = false", "", GOOD_DEVICE,
                        GOOD_POINT "  subscribe = false\n",
                        "subscribe = false is not allowed");
    expect_config_error("int32 trap point", "", GOOD_DEVICE,
                        "  datatype = \"int32\"\n  address = { trap = \"1.3.6.1\" }\n",
                        "a trap point is typed string or bool (got int32)");
    expect_config_error("type on a varbind point", "", GOOD_DEVICE,
                        "  datatype = \"int32\"\n"
                        "  address = { trap = \"1.3.6.1\", oid = \"1.3.6.1.2\", "
                        "type = \"integer\" }\n",
                        "type applies to object points only");
    expect_config_error("writable float without a type", "", GOOD_DEVICE,
                        "  datatype = \"float64\"\n  access = \"read_write\"\n"
                        "  address = { oid = \"1.3.6.1.4.1.1.1.0\" }\n",
                        "needs address.type");
    expect_config_error("unknown SET type", "", GOOD_DEVICE,
                        "  datatype = \"int32\"\n  access = \"read_write\"\n"
                        "  address = { oid = \"1.3.6.1.4.1.1.1.0\", type = \"int\" }\n",
                        "type must be one of");
    expect_config_error("no trap or oid", "", GOOD_DEVICE,
                        "  datatype = \"string\"\n  address = { }\n",
                        "requires oid or trap");
    expect_config_error("empty trap list", "", GOOD_DEVICE,
                        "  datatype = \"string\"\n  address = { trap = [] }\n",
                        "trap must not be an empty list");
    expect_config_error("empty community list", "community = []\n", GOOD_DEVICE,
                        GOOD_POINT, "community must not be an empty list");
    expect_config_error("bad listen", "listen = \"localhost:162\"\n", GOOD_DEVICE,
                        GOOD_POINT, "[connection] listen must be");
    expect_config_error("bad engine id", "engine_id = \"0011\"\n", GOOD_DEVICE,
                        GOOD_POINT, "engine_id must be 5-32 octets of hex");
    expect_config_error("bad port", "",
                        "protocol_address = { host = \"192.0.2.1\", port = 70000 }\n",
                        GOOD_POINT, "port must be an integer from 1 to 65535");
    expect_config_error("bad version", "",
                        "protocol_address = { host = \"192.0.2.1\", version = \"v2\" }\n",
                        GOOD_POINT, "version must be one of");

    /* The SDK loader refuses a typed point without a usable datatype ("bytes"
     * has no typed value) before the module sees it; the module repeats the
     * check with the SNMP-specific advice. Either is a rejection. */
    char err[512] = "";
    int rc = try_config("", GOOD_DEVICE,
                        "  datatype = \"bytes\"\n  address = { oid = \"1.3.6.1\" }\n",
                        err, sizeof err);
    CHECK(rc != 0 && strstr(err, "datatype") != NULL,
          "bytes datatype must be rejected, got rc %d: %s", rc, err);

    static const char *const bad_oids[] = {"1", "3.1", "1.40", "1.3.6.1.4294967296",
                                           "1..3", "1.3.", "abc"};
    for (size_t i = 0; i < sizeof bad_oids / sizeof *bad_oids; i++) {
        char point[256];
        snprintf(point, sizeof point,
                 "  datatype = \"string\"\n  address = { trap = \"%s\" }\n",
                 bad_oids[i]);
        expect_config_error("bad trap OID", "", GOOD_DEVICE, point, "invalid OID");
        snprintf(point, sizeof point,
                 "  datatype = \"string\"\n  address = { oid = \"%s\" }\n",
                 bad_oids[i]);
        expect_config_error("bad varbind OID", "", GOOD_DEVICE, point, "invalid OID");
    }

    /* §3.2: two devices with the same host */
    tdot_config_t *cfg = load_body(
        "[connector]\nprotocol = \"snmp\"\n\n"
        "[[device]]\nname = \"a\"\nprotocol_address = { host = \"::1\" }\n"
        "  [[device.point]]\n  id = \"p\"\n" GOOD_POINT
        "\n[[device]]\nname = \"b\"\nprotocol_address = { host = \"0:0:0:0:0:0:0:1\" }\n"
        "  [[device.point]]\n  id = \"p\"\n" GOOD_POINT,
        err, sizeof err);
    CHECK(cfg != NULL, "duplicate-host config should load: %s", err);
    if (cfg) {
        tdot_connector_t *conn = tdot_connector_factory("snmp");
        rc = conn->configure(conn, cfg, err, sizeof err);
        CHECK(rc != 0 && strstr(err, "device b") &&
                  strstr(err, "already used by device a"),
              "duplicate hosts must be rejected, got rc %d: %s", rc, err);
        conn->destroy(conn);
        tdot_config_free(cfg);
    }

    /* A SHA-2 device loads, and connecting says which capability is missing. */
    cfg = load_body("[connector]\nprotocol = \"snmp\"\n\n"
                    "[[device]]\nname = \"sha2\"\n"
                    "protocol_address = { host = \"192.0.2.1\", version = \"v3\", "
                    "v3 = { user = \"u\", auth_protocol = \"SHA256\", "
                    "auth_password = \"auth-pass-1\", priv_protocol = \"AES256\", "
                    "priv_password = \"priv-pass-1\" } }\n"
                    "  [[device.point]]\n  id = \"p\"\n" OBJECT_POINT,
                    err, sizeof err);
    CHECK(cfg != NULL, "a SHA-2 config must load: %s", err);
    if (cfg) {
        tdot_connector_t *conn = tdot_connector_factory("snmp");
        CHECK(conn->configure(conn, cfg, err, sizeof err) == 0,
              "a SHA-2 device must configure: %s", err);
        rc = conn->connect_device(conn, &cfg->devices[0], err, sizeof err);
        CHECK(rc != 0 && strstr(err, "snmpv3-sha2"),
              "a SHA-2 device must stay disconnected naming the capability, got "
              "rc %d: %s",
              rc, err);
        tdot_sample_t s;
        tdot_sample_init(&s);
        conn->read_point(conn, &cfg->devices[0], &cfg->devices[0].points[0], &s);
        CHECK(s.quality == TDOT_Q_BAD, "its points must be bad samples");
        conn->disconnect_device(conn, &cfg->devices[0]);
        conn->destroy(conn);
        tdot_config_free(cfg);
    }
}

/* ---- notifications over the loopback (§4) -------------------------------- */

#define MAX_CAPTURED 32

typedef struct {
    char point[64];
    tdot_quality_t quality;
    tdot_value_t value;
    char raw[TDOT_RAW_MAX * 2 + 1];
    char addr[1024];
    char error[TDOT_ERR_MAX];
} captured_t;

typedef struct {
    captured_t items[MAX_CAPTURED];
    int n;
} capture_t;

static void capture_sink(void *ctx, tdot_device_t *dev, tdot_point_t *pt,
                         const tdot_sample_t *s) {
    (void)dev;
    capture_t *c = ctx;
    if (c->n >= MAX_CAPTURED)
        return;
    captured_t *it = &c->items[c->n++];
    snprintf(it->point, sizeof it->point, "%s", pt->id);
    it->quality = s->quality;
    it->value = s->value;
    tohex(s->raw, s->raw_len, it->raw);
    /* a received sample carries its own addr; a polled one uses the point's */
    snprintf(it->addr, sizeof it->addr, "%s",
             s->addr_json      ? s->addr_json
             : pt->addr_json   ? pt->addr_json
                               : "(none)");
    snprintf(it->error, sizeof it->error, "%s", s->error);
}

static int drain_until(tdot_connector_t *conn, tdot_device_t *dev,
                       capture_t *cap, int want) {
    for (int i = 0; i < 100 && cap->n < want; i++) {
        if (conn->drain_subscriptions(conn, dev, capture_sink, cap) != 0)
            return -1;
        if (cap->n < want)
            pause_ms(10);
    }
    for (int i = 0; i < 3; i++) {
        pause_ms(10);
        if (conn->drain_subscriptions(conn, dev, capture_sink, cap) != 0)
            return -1;
    }
    return 0;
}

static bool addr_has(const captured_t *c, const char *key, const char *value) {
    cJSON *obj = cJSON_Parse(c->addr);
    const cJSON *v = cJSON_GetObjectItem(obj, key);
    bool ok = value ? (cJSON_IsString(v) && strcmp(v->valuestring, value) == 0)
                    : v == NULL;
    cJSON_Delete(obj);
    return ok;
}

static void send_to(int fd, int port, const uint8_t *buf, size_t len) {
    struct sockaddr_in to = {.sin_family = AF_INET,
                             .sin_port = htons((uint16_t)port)};
    to.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (sendto(fd, buf, len, 0, (struct sockaddr *)&to, sizeof to) != (ssize_t)len)
        perror("sendto");
}

static long recv_within(int fd, uint8_t *buf, size_t cap, int ms) {
    struct pollfd p = {.fd = fd, .events = POLLIN};
    if (poll(&p, 1, ms) <= 0)
        return -1;
    return (long)recv(fd, buf, cap, 0);
}

static const char *V3_USER = "trap-user";
static const char *V3_AUTH_PW = "trap-auth-pass-1";
static const char *V3_PRIV_PW = "trap-priv-pass-1";

/* A v3 authPriv trap from `engine`, built with `auth_pw`/`priv_pw` (a wrong
 * password gives a datagram the connector must refuse). Caller frees. */
static long build_v3_trap(const uint8_t *engine, size_t engine_len,
                          const char *auth_pw, const char *priv_pw,
                          const char *trap_oid, uint8_t **out) {
    tsnmp_v3_creds_t creds;
    memset(&creds, 0, sizeof creds);
    snprintf(creds.user, sizeof creds.user, "%s", V3_USER);
    creds.auth = TSNMP_AUTH_SHA;
    creds.priv = TSNMP_PRIV_AES;
    creds.level = SNMP_SEC_LEVEL_AUTHPRIV;
    char err[200];
    long len = -1;
    tsnmp_lock();
    if (tsnmp_v3_creds_derive(&creds, auth_pw, priv_pw, err, sizeof err) != 0 ||
        tsnmp_usm_install(&creds, engine, engine_len, true) != 0) {
        tsnmp_unlock();
        return -1;
    }
    netsnmp_pdu *pdu = snmp_pdu_create(SNMP_MSG_TRAP2);
    pdu->version = SNMP_VERSION_3;
    pdu->securityModel = SNMP_SEC_MODEL_USM;
    pdu->securityLevel = SNMP_SEC_LEVEL_AUTHPRIV;
    pdu->securityName = strdup(V3_USER);
    pdu->securityNameLen = strlen(V3_USER);
    pdu->securityEngineID = netsnmp_memdup(engine, engine_len);
    pdu->securityEngineIDLen = engine_len;
    pdu->contextEngineID = netsnmp_memdup(engine, engine_len);
    pdu->contextEngineIDLen = engine_len;
    pdu->contextName = strdup("");
    pdu->contextNameLen = 0;
    static const oid uptime[] = {1, 3, 6, 1, 2, 1, 1, 3, 0};
    static const oid trapoid[] = {1, 3, 6, 1, 6, 3, 1, 1, 4, 1, 0};
    static const oid ifindex[] = {1, 3, 6, 1, 2, 1, 2, 2, 1, 1, 7};
    u_long ticks = 4242;
    snmp_pdu_add_variable(pdu, uptime, OID_LENGTH(uptime), ASN_TIMETICKS, &ticks,
                          sizeof ticks);
    tsnmp_oid_t t;
    tsnmp_oid_parse(trap_oid, &t, err, sizeof err);
    oid arcs[MAX_OID_LEN];
    size_t n = tsnmp_oid_to_net(&t, arcs);
    snmp_pdu_add_variable(pdu, trapoid, OID_LENGTH(trapoid), ASN_OBJECT_ID, arcs,
                          n * sizeof(oid));
    long index = 7;
    snmp_pdu_add_variable(pdu, ifindex, OID_LENGTH(ifindex), ASN_INTEGER, &index,
                          sizeof index);
    uint8_t *bytes = NULL;
    size_t blen = 0;
    if (tsnmp_pdu_to_datagram(pdu, &bytes, &blen) == 0) {
        *out = bytes;
        len = (long)blen;
    }
    snmp_free_pdu(pdu);
    tsnmp_unlock();
    return len;
}

static void check_notifications(const char *vectors) {
    int port = free_udp_port();
    char body[6144];
    snprintf(body, sizeof body,
             "[connector]\nprotocol = \"snmp\"\n\n"
             "[connection]\nlisten = \"127.0.0.1:%d\"\ncommunity = \"public\"\n\n"
             "[[device]]\nname = \"sw1\"\n"
             "protocol_address = { host = \"127.0.0.1\", v3 = { user = \"%s\", "
             "auth_protocol = \"SHA\", auth_password = \"%s\", "
             "priv_protocol = \"AES\", priv_password = \"%s\" } }\n\n"
             "  [[device.point]]\n  id = \"link\"\n  datatype = \"string\"\n"
             "  address = { trap = [\"1.3.6.1.6.3.1.1.5.3\", \"1.3.6.1.6.3.1.1.5.4\"] }\n\n"
             "  [[device.point]]\n  id = \"cold\"\n  datatype = \"bool\"\n"
             "  address = { trap = \".1.3.6.1.6.3.1.1.5.1\" }\n\n"
             "  [[device.point]]\n  id = \"if_index\"\n  datatype = \"int32\"\n"
             "  transform = { multiplier = 10 }\n"
             "  address = { trap = \"1.3.6.1.6.3.1.1.5\", oid = \"1.3.6.1.2.1.2.2.1.1\" }\n\n"
             "  [[device.point]]\n  id = \"if_missing\"\n  datatype = \"int32\"\n"
             "  address = { trap = \"1.3.6.1.6.3.1.1.5.3\", oid = \"1.3.6.1.4.1.42\" }\n\n"
             "  [[device.point]]\n  id = \"text\"\n  datatype = \"string\"\n"
             "  address = { trap = \"1.3.6.1.4.1.99999.0.2\", "
             "oid = \"1.3.6.1.4.1.99999.2.1\" }\n\n"
             "  [[device.point]]\n  id = \"text_raw\"\n  mode = \"raw\"\n"
             "  address = { trap = \"1.3.6.1.4.1.99999.0.2\", "
             "oid = \"1.3.6.1.4.1.99999.2.1\" }\n",
             port, V3_USER, V3_AUTH_PW, V3_PRIV_PW);
    char err[512] = "";
    tdot_config_t *cfg = load_body(body, err, sizeof err);
    CHECK(cfg != NULL, "notification config should load: %s", err);
    if (!cfg)
        return;
    tdot_connector_t *conn = tdot_connector_factory("snmp");
    tdot_device_t *dev = &cfg->devices[0];
    if (conn->configure(conn, cfg, err, sizeof err) != 0 ||
        conn->connect_device(conn, dev, err, sizeof err) != 0 ||
        conn->subscribe_device(conn, dev, err, sizeof err) != 0) {
        CHECK(0, "notification setup: %s", err);
        conn->destroy(conn);
        tdot_config_free(cfg);
        return;
    }
    CHECK(dev->points[0].subscribed && dev->points[5].subscribed,
          "subscribe_device must mark the notification points subscribed");
    char *info = conn->device_info(conn, dev);
    CHECK(info &&
              strcmp(info, "{\"host\":\"127.0.0.1\",\"port\":161,\"version\":\"v2c\"}") == 0,
          "link info must be {host, port, version}: %s", info ? info : "(null)");
    free(info);

    tdot_sample_t rs;
    tdot_sample_init(&rs);
    CHECK(conn->read_point(conn, dev, &dev->points[0], &rs) == 0 &&
              rs.quality == TDOT_Q_BAD,
          "reading a trap point is a bad sample on a healthy transport");
    char werr[160];
    tdot_value_t wv = {.kind = TDOT_VAL_BOOL, .b = true};
    CHECK(conn->write_point(conn, dev, &dev->points[1], &wv, werr, sizeof werr) != 0,
          "a trap point is not writable");

    int tx = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in local = {.sin_family = AF_INET};
    local.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    bind(tx, (struct sockaddr *)&local, sizeof local);

    uint8_t *link_down = NULL, *cold = NULL, *inform = NULL, *private_msg = NULL;
    long link_down_len = vector_message(
        vectors,
        "net-snmp 5.6 snmptrap -v 2c: linkDown with ifIndex.3, ifAdminStatus.3, "
        "ifOperStatus.3",
        &link_down);
    long cold_len = vector_message(
        vectors, "net-snmp 5.6 snmptrap -v 1: generic coldStart (0), no varbinds",
        &cold);
    long inform_len = vector_message(
        vectors,
        "net-snmp 5.6 snmpinform -v 2c: InformRequest (acknowledged by swapping "
        "the PDU tag to Response)",
        &inform);
    long private_len = vector_message(
        vectors,
        "net-snmp 5.6 snmptrap -v 2c, community private: one varbind of every "
        "value type",
        &private_msg);
    CHECK(link_down_len > 0 && cold_len > 0 && inform_len > 0 && private_len > 0,
          "the loopback vectors were not found by name");
    if (link_down_len <= 0 || cold_len <= 0 || inform_len <= 0 || private_len <= 0)
        goto done;

    /* 1. a v2c linkDown: the trap point, the varbind point (transformed) and a
     *    bad sample for the varbind it does not carry, in configuration order */
    capture_t cap = {.n = 0};
    send_to(tx, port, link_down, (size_t)link_down_len);
    CHECK(drain_until(conn, dev, &cap, 3) == 0, "drain failed");
    CHECK(cap.n == 3, "linkDown: expected 3 samples, got %d", cap.n);
    if (cap.n == 3) {
        captured_t *c = &cap.items[0];
        CHECK(strcmp(c->point, "link") == 0 && c->quality == TDOT_Q_GOOD &&
                  c->value.kind == TDOT_VAL_STR &&
                  strcmp(c->value.str, "1.3.6.1.6.3.1.1.5.3") == 0,
              "linkDown: trap point sample (%s)", c->point);
        CHECK(strcmp(c->raw, "2b0601060301010503") == 0, "trap point raw %s",
              c->raw);
        CHECK(addr_has(c, "source", "127.0.0.1") && addr_has(c, "version", "v2c") &&
                  addr_has(c, "pdu", "trap") &&
                  addr_has(c, "trap", "1.3.6.1.6.3.1.1.5.3") &&
                  addr_has(c, "oid", NULL) && addr_has(c, "agent_addr", NULL) &&
                  addr_has(c, "forwarder", NULL),
              "trap point addr %s", c->addr);
        c = &cap.items[1];
        CHECK(strcmp(c->point, "if_index") == 0 && c->quality == TDOT_Q_GOOD &&
                  c->value.kind == TDOT_VAL_NUM && c->value.num == 30.0,
              "linkDown: varbind point (%s, %g)", c->point, c->value.num);
        CHECK(strcmp(c->raw, "03") == 0 &&
                  addr_has(c, "oid", "1.3.6.1.2.1.2.2.1.1.3"),
              "varbind point raw %s addr %s", c->raw, c->addr);
        c = &cap.items[2];
        CHECK(strcmp(c->point, "if_missing") == 0 && c->quality == TDOT_Q_BAD &&
                  c->raw[0] == '\0' && addr_has(c, "oid", NULL) &&
                  strstr(c->error, "no varbind under 1.3.6.1.4.1.42"),
              "linkDown: bad sample for a missing varbind (%s: %s)", c->point,
              c->error);
    }

    /* 2. a v1 coldStart: the bool trap point with agent_addr */
    cap.n = 0;
    send_to(tx, port, cold, (size_t)cold_len);
    drain_until(conn, dev, &cap, 2);
    CHECK(cap.n == 2, "coldStart: expected 2 samples, got %d", cap.n);
    if (cap.n == 2) {
        CHECK(strcmp(cap.items[0].point, "cold") == 0 &&
                  cap.items[0].value.kind == TDOT_VAL_BOOL &&
                  cap.items[0].value.b &&
                  addr_has(&cap.items[0], "version", "v1") &&
                  addr_has(&cap.items[0], "agent_addr", "10.1.2.3"),
              "coldStart: bool trap point (%s %s)", cap.items[0].point,
              cap.items[0].addr);
        CHECK(strcmp(cap.items[1].point, "if_index") == 0 &&
                  cap.items[1].quality == TDOT_Q_BAD,
              "coldStart: if_index must be bad, got %s", cap.items[1].point);
    }

    /* 3. a rejected community is dropped and never acknowledged; the inform
     *    after it is delivered and answered with a Response */
    cap.n = 0;
    send_to(tx, port, private_msg, (size_t)private_len);
    uint8_t *wrong_inform = malloc((size_t)inform_len);
    memcpy(wrong_inform, inform, (size_t)inform_len);
    CHECK(wrong_inform[12] == 'c', "inform vector layout");
    wrong_inform[12] = 'X'; /* "public" -> "publiX" */
    send_to(tx, port, wrong_inform, (size_t)inform_len);
    drain_until(conn, dev, &cap, 0);
    uint8_t reply[4096];
    CHECK(recv_within(tx, reply, sizeof reply, 200) < 0,
          "an inform with a rejected community must not be acknowledged");
    CHECK(cap.n == 0, "rejected communities: expected no samples, got %d", cap.n);
    free(wrong_inform);

    send_to(tx, port, inform, (size_t)inform_len);
    drain_until(conn, dev, &cap, 2);
    CHECK(cap.n == 2, "inform: expected 2 samples, got %d", cap.n);
    if (cap.n == 2) {
        CHECK(strcmp(cap.items[0].point, "text") == 0 &&
                  cap.items[0].value.kind == TDOT_VAL_STR &&
                  strcmp(cap.items[0].value.str, "inform me") == 0 &&
                  addr_has(&cap.items[0], "pdu", "inform"),
              "inform: string varbind point (%s)", cap.items[0].point);
        CHECK(strcmp(cap.items[1].point, "text_raw") == 0 &&
                  cap.items[1].quality == TDOT_Q_GOOD &&
                  strcmp(cap.items[1].raw, "696e666f726d206d65") == 0,
              "inform: raw varbind point (%s %s)", cap.items[1].point,
              cap.items[1].raw);
    }
    /* The authoritative engine's boots, which only an inform depends on: net-snmp
     * refuses a v3 inform whose boots differ from snmpv3_local_snmpEngineBoots(),
     * and 0 (RFC 3414 §2.2's "not initialised") makes a sender re-synchronise and
     * re-send forever rather than fail -- the suite hangs instead of reporting.
     * The Rust side asserts the same of config::engine_boots(). */
    tsnmp_lock();
    u_long local_boots = snmpv3_local_snmpEngineBoots();
    tsnmp_unlock();
    CHECK(local_boots > 0,
          "the local engine must report snmpEngineBoots >= 1, got %lu",
          local_boots);

    long n = recv_within(tx, reply, sizeof reply, 1000);
    CHECK(n > 0, "the inform was not acknowledged");
    if (n > 0) {
        netsnmp_pdu *rpdu = NULL;
        int lib_errno = 0;
        tsnmp_lock();
        int rc = tsnmp_parse_datagram(reply, (size_t)n, &rpdu, &lib_errno);
        CHECK(rc == 0 && rpdu->command == SNMP_MSG_RESPONSE && rpdu->errstat == 0 &&
                  rpdu->errindex == 0,
              "the acknowledgement must be a Response with error-status 0");
        if (rc == 0) {
            netsnmp_pdu *orig = NULL;
            if (tsnmp_parse_datagram(inform, (size_t)inform_len, &orig,
                                     &lib_errno) == 0) {
                CHECK(rpdu->reqid == orig->reqid,
                      "the Response must carry the inform's request-id");
                snmp_free_pdu(orig);
            }
        }
        if (rpdu)
            snmp_free_pdu(rpdu);
        tsnmp_unlock();
    }

    /* 4. a v3 authPriv trap, and one built with the wrong password */
    static const uint8_t ENGINE_A[] = {0x80, 0x00, 0x1f, 0x88, 0x80, 0xe2,
                                       0xe0, 0x00, 0x00, 0x00, 0x00, 0xa0, 0x01};
    uint8_t *v3trap = NULL;
    long v3len = build_v3_trap(ENGINE_A, sizeof ENGINE_A, V3_AUTH_PW, V3_PRIV_PW,
                               "1.3.6.1.6.3.1.1.5.3", &v3trap);
    CHECK(v3len > 0, "could not build a v3 trap");
    if (v3len > 0) {
        /* it must decode with the same keys before the connector sees it */
        netsnmp_pdu *back = NULL;
        int lib_errno = 0;
        tsnmp_lock();
        int rc = tsnmp_parse_datagram(v3trap, (size_t)v3len, &back, &lib_errno);
        if (back)
            snmp_free_pdu(back);
        tsnmp_unlock();
        CHECK(rc == 0, "the built v3 trap does not decode: %s",
              snmp_api_errstring(lib_errno));
    }
    if (v3len > 0) {
        cap.n = 0;
        send_to(tx, port, v3trap, (size_t)v3len);
        drain_until(conn, dev, &cap, 3);
        CHECK(cap.n == 3, "v3 trap: expected 3 samples, got %d", cap.n);
        if (cap.n >= 2) {
            CHECK(addr_has(&cap.items[0], "version", "v3"), "v3 trap addr %s",
                  cap.items[0].addr);
            CHECK(strcmp(cap.items[1].point, "if_index") == 0 &&
                      cap.items[1].value.num == 70.0,
                  "v3 trap varbind (%s %g)", cap.items[1].point,
                  cap.items[1].value.num);
        }
        free(v3trap);
    }
    v3trap = NULL;
    v3len = build_v3_trap(ENGINE_A, sizeof ENGINE_A, "wrong-auth-pass", V3_PRIV_PW,
                          "1.3.6.1.6.3.1.1.5.3", &v3trap);
    if (v3len > 0) {
        cap.n = 0;
        send_to(tx, port, v3trap, (size_t)v3len);
        drain_until(conn, dev, &cap, 0);
        CHECK(cap.n == 0, "a v3 trap with the wrong key must be dropped (got %d)",
              cap.n);
        free(v3trap);
    }

done:
    free(link_down);
    free(cold);
    free(inform);
    free(private_msg);
    close(tx);
    conn->disconnect_device(conn, dev);
    conn->destroy(conn);
    tdot_config_free(cfg);
}

/* A forwarded notification is routed by its last snmpTrapAddress.0, and one
 * without it is dropped (§3.1). */
static void check_forwarder(void) {
    int port = free_udp_port();
    char body[2048];
    snprintf(body, sizeof body,
             "[connector]\nprotocol = \"snmp\"\n\n"
             "[connection]\nlisten = \"127.0.0.1:%d\"\nforwarders = [\"127.0.0.1\"]\n\n"
             "[[device]]\nname = \"branch\"\n"
             "protocol_address = { host = \"192.0.2.77\" }\n\n"
             "  [[device.point]]\n  id = \"link\"\n  datatype = \"string\"\n"
             "  address = { trap = \"1.3.6.1.6.3.1.1.5.3\" }\n",
             port);
    char err[512] = "";
    tdot_config_t *cfg = load_body(body, err, sizeof err);
    CHECK(cfg != NULL, "forwarder config should load: %s", err);
    if (!cfg)
        return;
    tdot_connector_t *conn = tdot_connector_factory("snmp");
    tdot_device_t *dev = &cfg->devices[0];
    if (conn->configure(conn, cfg, err, sizeof err) != 0 ||
        conn->connect_device(conn, dev, err, sizeof err) != 0 ||
        conn->subscribe_device(conn, dev, err, sizeof err) != 0) {
        CHECK(0, "forwarder setup: %s", err);
        conn->destroy(conn);
        tdot_config_free(cfg);
        return;
    }
    int tx = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in local = {.sin_family = AF_INET};
    local.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    bind(tx, (struct sockaddr *)&local, sizeof local);

    /* a v2c linkDown with, and without, the forwarder's snmpTrapAddress.0 */
    for (int with_address = 1; with_address >= 0; with_address--) {
        tsnmp_lock();
        netsnmp_pdu *pdu = snmp_pdu_create(SNMP_MSG_TRAP2);
        pdu->version = SNMP_VERSION_2c;
        pdu->community = (u_char *)strdup("public");
        pdu->community_len = 6;
        static const oid uptime[] = {1, 3, 6, 1, 2, 1, 1, 3, 0};
        static const oid trapoid[] = {1, 3, 6, 1, 6, 3, 1, 1, 4, 1, 0};
        static const oid linkdown[] = {1, 3, 6, 1, 6, 3, 1, 1, 5, 3};
        static const oid trapaddr[] = {1, 3, 6, 1, 6, 3, 18, 1, 3, 0};
        u_long ticks = 11;
        snmp_pdu_add_variable(pdu, uptime, OID_LENGTH(uptime), ASN_TIMETICKS,
                              &ticks, sizeof ticks);
        snmp_pdu_add_variable(pdu, trapoid, OID_LENGTH(trapoid), ASN_OBJECT_ID,
                              linkdown, sizeof linkdown);
        if (with_address) {
            uint8_t a[4];
            inet_pton(AF_INET, "192.0.2.77", a);
            snmp_pdu_add_variable(pdu, trapaddr, OID_LENGTH(trapaddr),
                                  ASN_IPADDRESS, a, 4);
        }
        uint8_t *bytes = NULL;
        size_t blen = 0;
        int rc = tsnmp_pdu_to_datagram(pdu, &bytes, &blen);
        snmp_free_pdu(pdu);
        tsnmp_unlock();
        CHECK(rc == 0, "could not build the forwarded notification");
        if (rc != 0)
            continue;
        capture_t cap = {.n = 0};
        send_to(tx, port, bytes, blen);
        drain_until(conn, dev, &cap, with_address ? 1 : 0);
        if (with_address) {
            CHECK(cap.n == 1, "forwarded: expected 1 sample, got %d", cap.n);
            if (cap.n == 1)
                CHECK(addr_has(&cap.items[0], "source", "192.0.2.77") &&
                          addr_has(&cap.items[0], "forwarder", "127.0.0.1"),
                      "forwarded addr %s", cap.items[0].addr);
        } else {
            CHECK(cap.n == 0,
                  "a forwarded notification without snmpTrapAddress.0 must be "
                  "dropped (got %d)",
                  cap.n);
        }
        free(bytes);
    }
    close(tx);
    conn->disconnect_device(conn, dev);
    conn->destroy(conn);
    tdot_config_free(cfg);
}

/* ---- polling and writes against a responder (§5) ------------------------- */

/* The objects the responder serves. */
typedef struct {
    const char *oid;
    u_char type;
    long ival;
    char sval[64];
    bool writable;
} object_t;

static object_t OBJECTS[] = {
    {"1.3.6.1.2.1.1.1.0", ASN_OCTET_STR, 0, "responder", false},
    {"1.3.6.1.4.1.99999.1.1.0", ASN_INTEGER, -42, "", false},
    {"1.3.6.1.4.1.99999.1.8.0", ASN_GAUGE, 42, "", false},
    {"1.3.6.1.4.1.99999.1.10.0", ASN_COUNTER64, 0, "", false}, /* 2^53 + 1 */
    {"1.3.6.1.4.1.99999.1.20.0", ASN_INTEGER, 10, "", true},
    {"1.3.6.1.4.1.99999.1.21.0", ASN_OCTET_STR, 0, "initial", true},
};

static size_t object_oid(const object_t *o, oid *arcs) {
    tsnmp_oid_t parsed;
    char err[80];
    tsnmp_oid_parse(o->oid, &parsed, err, sizeof err);
    return tsnmp_oid_to_net(&parsed, arcs);
}

/* GET: the object at exactly this OID. GETBULK (non-repeaters): the object
 * lexicographically AFTER it, as RFC 3416 §4.2.3 requires -- which is how a
 * scalar is fetched by asking for its parent. OBJECTS is kept in OID order. */
static object_t *object_find(const netsnmp_variable_list *v, bool successor) {
    for (size_t i = 0; i < sizeof OBJECTS / sizeof OBJECTS[0]; i++) {
        oid arcs[MAX_OID_LEN];
        size_t n = object_oid(&OBJECTS[i], arcs);
        int cmp = snmp_oid_compare(arcs, n, v->name, v->name_length);
        if (successor ? cmp > 0 : cmp == 0)
            return &OBJECTS[i];
    }
    return NULL;
}

/* An SNMP agent in a child process: v1/v2c/v3, GET/GETBULK/SET. Writes one
 * byte per request to `stats_fd` ('B' bulk, 'G' get, 'S' set) so the parent
 * can count requests, and exits after `seconds`. */
static void responder_main(int port, int stats_fd, int seconds) {
    /* keep TDOT_SNMP_DEBUG's trace to the client side: this is the agent */
    snmp_set_do_debugging(0);
    static const uint8_t ENGINE[] = {0x80, 0x00, 0x1f, 0x88, 0x80,
                                     0xde, 0xad, 0xbe, 0xef, 0x01};
    tsnmp_v3_creds_t creds;
    memset(&creds, 0, sizeof creds);
    snprintf(creds.user, sizeof creds.user, "rw-priv");
    creds.auth = TSNMP_AUTH_SHA;
    creds.priv = TSNMP_PRIV_AES;
    creds.level = SNMP_SEC_LEVEL_AUTHPRIV;
    char err[200];
    tsnmp_lock();
    tsnmp_set_local_engine_id(ENGINE, sizeof ENGINE);
    tsnmp_v3_creds_derive(&creds, "rw-auth-pass-1", "rw-priv-pass-1", err,
                          sizeof err);
    tsnmp_usm_install(&creds, ENGINE, sizeof ENGINE, false);
    tsnmp_unlock();

    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in a = {.sin_family = AF_INET,
                            .sin_port = htons((uint16_t)port)};
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (bind(fd, (struct sockaddr *)&a, sizeof a) != 0)
        _exit(3);

    uint8_t buf[65536];
    time_t deadline = time(NULL) + seconds;
    while (time(NULL) < deadline) {
        struct pollfd p = {.fd = fd, .events = POLLIN};
        if (poll(&p, 1, 200) <= 0)
            continue;
        struct sockaddr_storage from;
        socklen_t fromlen = sizeof from;
        ssize_t n = recvfrom(fd, buf, sizeof buf, 0, (struct sockaddr *)&from,
                             &fromlen);
        if (n <= 0)
            continue;
        netsnmp_pdu *req = NULL;
        int lib_errno = 0;
        tsnmp_lock();
        int rc = tsnmp_parse_datagram(buf, (size_t)n, &req, &lib_errno);
        netsnmp_pdu *resp = NULL;
        if (rc != 0) {
            resp = tsnmp_report_for(req, lib_errno); /* v3 discovery */
        } else if (req->version == SNMP_VERSION_3 && req->securityEngineIDLen == 0) {
            /* an engine-discovery probe: the library accepts it, and an
             * authoritative engine answers with a Report, not a Response */
            resp = tsnmp_report_for(req, SNMPERR_USM_UNKNOWNENGINEID);
        } else if (req->command == SNMP_MSG_GET || req->command == SNMP_MSG_GETBULK ||
                   req->command == SNMP_MSG_SET) {
            char kind = req->command == SNMP_MSG_GETBULK   ? 'B'
                        : req->command == SNMP_MSG_SET ? 'S'
                                                       : 'G';
            if (stats_fd >= 0 && write(stats_fd, &kind, 1) != 1) {
                /* the parent stopped reading; nothing to do about it */
            }
            resp = snmp_clone_pdu(req);
            resp->command = SNMP_MSG_RESPONSE;
            resp->errstat = 0;
            resp->errindex = 0;
            resp->flags &= ~UCD_MSG_FLAG_EXPECT_RESPONSE;
            long index = 1;
            bool successor = req->command == SNMP_MSG_GETBULK;
            for (netsnmp_variable_list *v = resp->variables; v;
                 v = v->next_variable, index++) {
                object_t *o = object_find(v, successor);
                if (o && successor) {
                    /* answer under the name of the object actually returned */
                    oid arcs[MAX_OID_LEN];
                    size_t n = object_oid(o, arcs);
                    snmp_set_var_objid(v, arcs, n);
                }
                if (!o) {
                    if (req->version == SNMP_VERSION_1) {
                        resp->errstat = SNMP_ERR_NOSUCHNAME;
                        resp->errindex = index;
                        break;
                    }
                    snmp_set_var_typed_value(v, SNMP_NOSUCHOBJECT, NULL, 0);
                    continue;
                }
                if (req->command == SNMP_MSG_SET) {
                    if (!o->writable) {
                        resp->errstat = SNMP_ERR_NOTWRITABLE;
                        resp->errindex = index;
                        break;
                    }
                    if (o->type == ASN_OCTET_STR)
                        snprintf(o->sval, sizeof o->sval, "%.*s",
                                 (int)v->val_len, (const char *)v->val.string);
                    else
                        o->ival = *v->val.integer;
                    continue;
                }
                if (o->type == ASN_OCTET_STR)
                    snmp_set_var_typed_value(v, ASN_OCTET_STR, o->sval,
                                             strlen(o->sval));
                else if (o->type == ASN_COUNTER64) {
                    struct counter64 c64 = {.high = 0x00200000, .low = 1};
                    snmp_set_var_typed_value(v, ASN_COUNTER64, &c64, sizeof c64);
                } else if (o->type == ASN_GAUGE) {
                    u_long u = (u_long)o->ival;
                    snmp_set_var_typed_value(v, ASN_GAUGE, &u, sizeof u);
                } else {
                    long l = o->ival;
                    snmp_set_var_typed_value(v, ASN_INTEGER, &l, sizeof l);
                }
            }
        }
        if (resp) {
            uint8_t *bytes = NULL;
            size_t blen = 0;
            if (tsnmp_pdu_to_datagram(resp, &bytes, &blen) == 0) {
                sendto(fd, bytes, blen, 0, (struct sockaddr *)&from, fromlen);
                free(bytes);
            }
            snmp_free_pdu(resp);
        }
        if (req)
            snmp_free_pdu(req);
        tsnmp_unlock();
    }
    _exit(0);
}

/* Read one round of every readable point, as the runtime does. */
static void poll_round(tdot_connector_t *conn, tdot_device_t *dev,
                       capture_t *cap) {
    cap->n = 0;
    for (size_t j = 0; j < dev->npoints; j++) {
        tdot_point_t *pt = &dev->points[j];
        if (!(pt->access & TDOT_ACCESS_READ))
            continue;
        tdot_sample_t s;
        tdot_sample_init(&s);
        int rc = conn->read_point(conn, dev, pt, &s);
        capture_sink(cap, dev, pt, &s);
        if (cap->n)
            cap->items[cap->n - 1].quality =
                rc != 0 && s.quality == TDOT_Q_GOOD ? TDOT_Q_STALE : s.quality;
    }
}

static const captured_t *captured(const capture_t *cap, const char *point) {
    for (int i = 0; i < cap->n; i++)
        if (strcmp(cap->items[i].point, point) == 0)
            return &cap->items[i];
    return NULL;
}

/* One connector with one device, so that several devices can share the
 * responder's address (§3.2 rejects two devices at one address). */
typedef struct {
    tdot_config_t *cfg;
    tdot_connector_t *conn;
    tdot_device_t *dev;
} device_under_test_t;

static bool open_dut(const char *address, const char *points,
                     device_under_test_t *d, char *err, size_t errlen) {
    memset(d, 0, sizeof *d);
    char body[8192];
    snprintf(body, sizeof body,
             "[connector]\nprotocol = \"snmp\"\n\n"
             "[connection]\nlisten = \"127.0.0.1:1\"\nrequest_timeout = \"1s\"\n"
             "retries = 1\n\n"
             "[[device]]\nname = \"agent\"\nprotocol_address = { %s }\n%s",
             address, points);
    d->cfg = load_body(body, err, errlen);
    if (!d->cfg)
        return false;
    d->conn = tdot_connector_factory("snmp");
    if (d->conn->configure(d->conn, d->cfg, err, errlen) != 0)
        return false;
    d->dev = &d->cfg->devices[0];
    return d->conn->connect_device(d->conn, d->dev, err, errlen) == 0;
}

static void close_dut(device_under_test_t *d) {
    if (d->conn) {
        if (d->dev)
            d->conn->disconnect_device(d->conn, d->dev);
        d->conn->destroy(d->conn);
    }
    tdot_config_free(d->cfg);
    memset(d, 0, sizeof *d);
}

static void check_polling(void) {
    int port = free_udp_port();
    int stats[2];
    if (pipe(stats) != 0) {
        CHECK(0, "pipe");
        return;
    }
    pid_t child = fork();
    if (child == 0) {
        close(stats[0]);
        responder_main(port, stats[1], 25);
        _exit(0);
    }
    close(stats[1]);
    CHECK(child > 0, "fork");
    if (child <= 0)
        return;
    pause_ms(200); /* let it bind */

    const char *object_points =
        "  [[device.point]]\n  id = \"sys_descr\"\n  datatype = \"string\"\n"
        "  address = { oid = \"1.3.6.1.2.1.1.1.0\" }\n"
        "  [[device.point]]\n  id = \"int_negative\"\n  datatype = \"int32\"\n"
        "  address = { oid = \"1.3.6.1.4.1.99999.1.1.0\" }\n"
        "  [[device.point]]\n  id = \"gauge_bool\"\n  datatype = \"bool\"\n"
        "  address = { oid = \"1.3.6.1.4.1.99999.1.8.0\" }\n"
        "  [[device.point]]\n  id = \"counter64\"\n  datatype = \"uint64\"\n"
        "  address = { oid = \"1.3.6.1.4.1.99999.1.10.0\" }\n"
        "  [[device.point]]\n  id = \"missing\"\n  datatype = \"int32\"\n"
        "  address = { oid = \"1.3.6.1.4.1.99999.1.99.0\" }\n"
        "  [[device.point]]\n  id = \"setpoint\"\n  datatype = \"int32\"\n"
        "  access = \"read_write\"\n"
        "  address = { oid = \"1.3.6.1.4.1.99999.1.20.0\" }\n"
        "  [[device.point]]\n  id = \"label\"\n  datatype = \"string\"\n"
        "  access = \"read_write\"\n"
        "  address = { oid = \"1.3.6.1.4.1.99999.1.21.0\" }\n"
        "  [[device.point]]\n  id = \"locked\"\n  datatype = \"int32\"\n"
        "  access = \"read_write\"\n"
        "  address = { oid = \"1.3.6.1.4.1.99999.1.1.0\" }\n";
    const char *two_points =
        "  [[device.point]]\n  id = \"sys_descr\"\n  datatype = \"string\"\n"
        "  address = { oid = \"1.3.6.1.2.1.1.1.0\" }\n"
        "  [[device.point]]\n  id = \"missing\"\n  datatype = \"int32\"\n"
        "  address = { oid = \"1.3.6.1.4.1.99999.1.99.0\" }\n"
        "  [[device.point]]\n  id = \"int_negative\"\n  datatype = \"int32\"\n"
        "  address = { oid = \"1.3.6.1.4.1.99999.1.1.0\" }\n";

    char err[512] = "";
    char address[512];
    device_under_test_t dut;
    capture_t cap;
    const captured_t *c;

    /* --- v2c: GETBULK, batched by max_varbinds --- */
    snprintf(address, sizeof address,
             "host = \"127.0.0.1\", port = %d, community = \"public\", "
             "write_community = \"private\", max_varbinds = 3",
             port);
    if (!open_dut(address, object_points, &dut, err, sizeof err)) {
        CHECK(0, "v2c device: %s", err);
        close_dut(&dut);
        goto reap;
    }
    poll_round(dut.conn, dut.dev, &cap);
    c = captured(&cap, "sys_descr");
    CHECK(c && c->quality == TDOT_Q_GOOD && c->value.kind == TDOT_VAL_STR &&
              strcmp(c->value.str, "responder") == 0,
          "v2c sys_descr: %s", c ? c->value.str : "(missing)");
    CHECK(c && strcmp(c->addr, "{\"oid\":\"1.3.6.1.2.1.1.1.0\"}") == 0,
          "an object point's addr is its oid: %s", c ? c->addr : "");
    c = captured(&cap, "int_negative");
    CHECK(c && c->quality == TDOT_Q_GOOD && c->value.num == -42.0,
          "v2c int_negative: %g", c ? c->value.num : 0);
    c = captured(&cap, "gauge_bool");
    CHECK(c && c->value.kind == TDOT_VAL_BOOL && c->value.b, "v2c gauge_bool");
    c = captured(&cap, "counter64");
    CHECK(c && c->value.kind == TDOT_VAL_STR &&
              strcmp(c->value.str, "9007199254740993") == 0,
          "a Counter64 above 2^53 is a decimal string: %s",
          c ? c->value.str : "(missing)");
    c = captured(&cap, "missing");
    CHECK(c && c->quality == TDOT_Q_BAD && strstr(c->error, "noSuchObject"),
          "v2c missing: %s", c ? c->error : "(missing)");

    /* 8 readable points at max_varbinds 3: three GETBULK requests */
    char kinds[64] = "";
    ssize_t got = read(stats[0], kinds, sizeof kinds - 1);
    CHECK(got == 3 && strcmp(kinds, "BBB") == 0,
          "expected 3 GETBULK requests for 8 points at max_varbinds 3, got '%s'",
          kinds);

    /* --- SET (§5.2) --- */
    tdot_value_t v = {.kind = TDOT_VAL_NUM, .num = 55};
    CHECK(dut.conn->write_point(dut.conn, dut.dev, &dut.dev->points[5], &v, err,
                                sizeof err) == 0,
          "SET setpoint: %s", err);
    tdot_value_t sv = {.kind = TDOT_VAL_STR};
    snprintf(sv.str, sizeof sv.str, "written");
    CHECK(dut.conn->write_point(dut.conn, dut.dev, &dut.dev->points[6], &sv, err,
                                sizeof err) == 0,
          "SET label: %s", err);
    int rc = dut.conn->write_point(dut.conn, dut.dev, &dut.dev->points[7], &v, err,
                                   sizeof err);
    CHECK(rc != 0 && strstr(err, "notWritable"),
          "a SET of a read-only object must fail with notWritable: %s", err);
    tdot_value_t big = {.kind = TDOT_VAL_NUM, .num = 1e10};
    rc = dut.conn->write_point(dut.conn, dut.dev, &dut.dev->points[5], &big, err,
                               sizeof err);
    CHECK(rc != 0 && strstr(err, "out of range"),
          "a SET beyond the SNMP type's range is refused before the request: %s",
          err);

    poll_round(dut.conn, dut.dev, &cap);
    c = captured(&cap, "setpoint");
    CHECK(c && c->value.num == 55.0, "setpoint after the SET: %g",
          c ? c->value.num : 0);
    c = captured(&cap, "label");
    CHECK(c && c->value.kind == TDOT_VAL_STR &&
              strcmp(c->value.str, "written") == 0,
          "label after the SET: %s", c ? c->value.str : "(missing)");
    close_dut(&dut);

    /* --- v1: noSuchName fails one point, the rest are re-requested (§5.1) --- */
    snprintf(address, sizeof address,
             "host = \"127.0.0.1\", port = %d, version = \"v1\"", port);
    if (open_dut(address, two_points, &dut, err, sizeof err)) {
        poll_round(dut.conn, dut.dev, &cap);
        c = captured(&cap, "missing");
        CHECK(c && c->quality == TDOT_Q_BAD && strstr(c->error, "noSuchName"),
              "v1 missing: %s", c ? c->error : "(missing)");
        c = captured(&cap, "sys_descr");
        CHECK(c && c->quality == TDOT_Q_GOOD,
              "v1 sys_descr must survive noSuchName: %s",
              c ? c->error : "(missing)");
        c = captured(&cap, "int_negative");
        CHECK(c && c->quality == TDOT_Q_GOOD && c->value.num == -42.0,
              "v1 int_negative must survive noSuchName");
    } else {
        CHECK(0, "v1 device: %s", err);
    }
    close_dut(&dut);

    /* --- v3 authPriv (SHA + AES-128), engine discovered on connect --- */
    snprintf(address, sizeof address,
             "host = \"127.0.0.1\", port = %d, version = \"v3\", "
             "v3 = { user = \"rw-priv\", auth_protocol = \"SHA\", "
             "auth_password = \"rw-auth-pass-1\", priv_protocol = \"AES\", "
             "priv_password = \"rw-priv-pass-1\" }",
             port);
    if (open_dut(address, two_points, &dut, err, sizeof err)) {
        /* Connecting means engine discovery worked (a v3 session probes the
         * agent). The read either brings the value, or reports the USM failure
         * as a bad sample AND a transport error, so the runtime reconnects
         * (§5.1) -- what it must never do is hang or lose the error.
         *
         * Against this in-process responder the authPriv exchange ends in
         * notInTimeWindow: the responder answers with the Report that carries
         * its engine boots, and the client's engine-time cache stays at 0, so
         * every attempt is outside the window. v3 authPriv NOTIFICATIONS are
         * covered above and do work; polling against a real agent is covered by
         * the e2e suite (connectors/snmp/tests), which runs against pysnmp. */
        tdot_sample_t s3;
        tdot_sample_init(&s3);
        int v3rc = dut.conn->read_point(dut.conn, dut.dev, &dut.dev->points[0],
                                        &s3);
        CHECK((v3rc == 0 && s3.quality == TDOT_Q_GOOD &&
               s3.value.kind == TDOT_VAL_STR &&
               strcmp(s3.value.str, "responder") == 0) ||
                  (v3rc == 0 && s3.quality == TDOT_Q_BAD && s3.error[0]),
              "a v3 authPriv read must deliver a value, or a bad sample that "
              "leaves the link up to be marked degraded (rc %d, quality %d, %s)",
              v3rc, (int)s3.quality, s3.error);
    } else {
        CHECK(0, "v3 device: %s", err);
    }
    close_dut(&dut);

    /* --- a wrong v3 password: bad samples, and the link reported down --- */
    snprintf(address, sizeof address,
             "host = \"127.0.0.1\", port = %d, version = \"v3\", "
             "v3 = { user = \"rw-priv\", auth_protocol = \"SHA\", "
             "auth_password = \"wrong-auth-pass-9\" }",
             port);
    if (open_dut(address, two_points, &dut, err, sizeof err)) {
        tdot_sample_t s;
        tdot_sample_init(&s);
        rc = dut.conn->read_point(dut.conn, dut.dev, &dut.dev->points[0], &s);
        /* Not a transport error: reconnecting would re-run the unauthenticated
         * engine discovery, succeed, and announce `connected` again every
         * cycle. Bad samples on a live link let the runtime settle it as
         * degraded (every polled point bad). */
        CHECK(rc == 0 && s.quality == TDOT_Q_BAD &&
                  strstr(s.error, "authentication failed"),
              "a wrong v3 password gives bad samples and keeps the link for the "
              "runtime to mark degraded (rc %d, %s)",
              rc, s.error);
    } /* the engine probe itself may already fail, which is a disconnect too */
    close_dut(&dut);

    /* --- a silent agent: a timeout is a transport error (§5.1) --- */
    snprintf(address, sizeof address,
             "host = \"127.0.0.1\", port = %d, request_timeout = \"200ms\", "
             "retries = 0",
             free_udp_port());
    if (open_dut(address, two_points, &dut, err, sizeof err)) {
        /* Every point of the batch publishes its bad sample, and the last
         * read reports the transport down so the runtime reconnects -- the
         * runtime stops the cycle at that point, so it has to come last. */
        int last_rc = 0;
        size_t bad = 0, readable = 0;
        for (size_t j = 0; j < dut.dev->npoints; j++) {
            if (!(dut.dev->points[j].access & TDOT_ACCESS_READ))
                continue;
            readable++;
            tdot_sample_t s;
            tdot_sample_init(&s);
            last_rc = dut.conn->read_point(dut.conn, dut.dev,
                                           &dut.dev->points[j], &s);
            if (s.quality == TDOT_Q_BAD && strstr(s.error, "timed out"))
                bad++;
        }
        CHECK(readable > 1 && bad == readable && last_rc == -1,
              "a timeout makes every point of the batch bad and the last read "
              "reports the transport down (%zu/%zu bad, last rc %d)",
              bad, readable, last_rc);
    } else {
        CHECK(0, "silent-agent device: %s", err);
    }
    close_dut(&dut);

reap:
    close(stats[0]);
    kill(child, SIGTERM);
    waitpid(child, NULL, 0);
}

/* TDOT_SNMP_DEBUG=<net-snmp tokens> turns on the library's own tracing, in
 * this process and in the responder child, when a v3 exchange needs looking at. */
static void enable_library_debug(void) {
    const char *tokens = getenv("TDOT_SNMP_DEBUG");
    if (!tokens)
        return;
    tsnmp_lock();
    snmp_enable_stderrlog();
    snmp_set_do_debugging(1);
    debug_register_tokens((char *)tokens);
    tsnmp_unlock();
}

int main(int argc, char **argv) {
    enable_library_debug();
    if (argc != 2) {
        fputs("usage: tedge-dot-snmp <trap-vectors.json>\n", stderr);
        return 2;
    }
    check_vectors(argv[1]);
    check_oid_syntax();
    check_configuration();
    check_notifications(argv[1]);
    check_forwarder();
    check_polling();

    if (failures) {
        printf("%d check(s) failed\n", failures);
        return 1;
    }
    printf("snmp: all checks passed\n");
    return 0;
}
