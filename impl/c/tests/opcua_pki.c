/* OPC UA PKI checks (doc/connectors/opcua-connector-spec.md §4), over the
 * vectors of connectors/opcua/conformance/tools/genpki.py -- the same tree the
 * Rust build validates (impl/rust/crates/connector-opcua/tests/pki.rs), so
 * both builds reach the same verdict for every scenario.
 *
 *   tedge-dot-opcua-pki <vectors-dir>
 *
 * tests/opcua_pki.sh generates the vectors and runs this.
 */
#include <dirent.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "cjson/cJSON.h"
#include "tedge_dot/config.h"
#include "tedge_dot/connector.h"
#include "ua_pki.h"

static int failures = 0;

#define CHECK(cond, ...)                                                       \
    do {                                                                       \
        if (!(cond)) {                                                         \
            failures++;                                                        \
            fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__);               \
            fprintf(stderr, __VA_ARGS__);                                      \
            fprintf(stderr, "\n");                                             \
        }                                                                      \
    } while (0)

static const char *category(UA_StatusCode rc) {
    switch (rc) {
    case UA_STATUSCODE_GOOD: return "trusted";
    case UA_STATUSCODE_BADCERTIFICATEUNTRUSTED:
    case UA_STATUSCODE_BADSECURITYCHECKSFAILED: return "untrusted";
    case UA_STATUSCODE_BADCERTIFICATEREVOKED:
    case UA_STATUSCODE_BADCERTIFICATEISSUERREVOKED:
    case UA_STATUSCODE_BADCERTIFICATEREVOCATIONUNKNOWN:
    case UA_STATUSCODE_BADCERTIFICATEISSUERREVOCATIONUNKNOWN: return "revoked";
    default: return "invalid";
    }
}

static cJSON *load_json(const char *path) {
    size_t len = 0;
    unsigned char *buf = ua_pki_read_file(path, &len);
    if (!buf)
        return NULL;
    char *text = realloc(buf, len + 1);
    text[len] = '\0';
    cJSON *j = cJSON_Parse(text);
    free(text);
    return j;
}

static unsigned char *server_cert(const char *vectors, const char *scenario,
                                  size_t *len) {
    char path[1024];
    snprintf(path, sizeof path, "%s/scenarios/%s/server/cert.der", vectors,
             scenario);
    return ua_pki_read_file(path, len);
}

static int count_files(const char *dir) {
    char cmd[1200];
    snprintf(cmd, sizeof cmd, "ls -A '%s' 2>/dev/null | grep -v '^\\.' | wc -l", dir);
    FILE *p = popen(cmd, "r");
    int n = -1;
    if (p) {
        if (fscanf(p, "%d", &n) != 1)
            n = -1;
        pclose(p);
    }
    return n;
}

static void test_vectors(const char *vectors, cJSON *expected) {
    const char *host = cJSON_GetArrayItem(cJSON_GetObjectItem(expected, "hostnames"), 0)->valuestring;
    const char *uri = cJSON_GetObjectItem(expected, "application_uri")->valuestring;
    cJSON *scenarios = cJSON_GetObjectItem(expected, "scenarios");
    int n = 0;
    cJSON *s;
    cJSON_ArrayForEach(s, scenarios) {
        n++;
        const char *want = cJSON_GetObjectItem(s, "outcome")->valuestring;
        size_t len = 0;
        unsigned char *der = server_cert(vectors, s->string, &len);
        CHECK(der, "%s: no server certificate", s->string);
        if (!der)
            continue;
        char root[1024];
        snprintf(root, sizeof root, "%s/scenarios/%s/pki", vectors, s->string);
        UA_StatusCode rc = ua_pki_validate(root, der, len, 2048, 4096, host, uri);
        CHECK(!strcmp(category(rc), want), "%s: want %s, got %s (%s)",
              s->string, want, category(rc), UA_StatusCode_name(rc));
        char tp[41];
        ua_pki_thumbprint(der, len, tp);
        CHECK(!strcmp(tp, cJSON_GetObjectItem(s, "thumbprint")->valuestring),
              "%s: thumbprint %s", s->string, tp);
        free(der);
    }
    CHECK(n >= 16, "too few scenarios: %d", n);
}

static void test_quarantine(const char *vectors, cJSON *expected) {
    char root[1024], rejected[1100];
    snprintf(root, sizeof root, "%s/scenarios/untrusted/pki", vectors);
    snprintf(rejected, sizeof rejected, "%s/rejected/certs", root);
    const char *tp = cJSON_GetObjectItem(
        cJSON_GetObjectItem(cJSON_GetObjectItem(expected, "scenarios"), "untrusted"),
        "thumbprint")->valuestring;
    size_t len = 0;
    unsigned char *der = server_cert(vectors, "untrusted", &len);
    ua_pki_validate(root, der, len, 2048, 4096, NULL, NULL);
    ua_pki_validate(root, der, len, 2048, 4096, NULL, NULL);
    CHECK(count_files(rejected) == 1, "rejected/certs holds %d files", count_files(rejected));

    ua_cert_list_t list;
    ua_pki_list(root, UA_PKI_REJECTED, &list);
    CHECK(list.n == 1, "listed %zu rejected", list.n);
    if (list.n == 1) {
        char want[128];
        snprintf(want, sizeof want, "/sim_unknown_%s.der", tp);
        size_t pl = strlen(list.items[0].path), wl = strlen(want);
        CHECK(pl > wl && !strcmp(list.items[0].path + pl - wl, want),
              "canonical name: %s", list.items[0].path);
        CHECK(!strcmp(list.items[0].subject, "CN=sim unknown, O=tedge-dot test"),
              "subject: %s", list.items[0].subject);
        /* trust it: the next validation succeeds */
        char path[1024], err[256];
        CHECK(ua_pki_store(root, UA_PKI_TRUSTED, list.items[0].der,
                           list.items[0].len, path, sizeof path, err,
                           sizeof err) == 0, "store: %s", err);
        CHECK(ua_pki_remove(root, &list.items[0], err, sizeof err) == 0,
              "remove: %s", err);
        CHECK(ua_pki_validate(root, der, len, 2048, 4096, NULL, NULL) ==
              UA_STATUSCODE_GOOD, "trusted after moving");
    }
    ua_cert_list_free(&list);
    free(der);
}

static void test_listing(const char *vectors) {
    char root[1024];
    ua_cert_list_t list;
    snprintf(root, sizeof root, "%s/scenarios/corrupt_file/pki", vectors);
    ua_pki_list(root, UA_PKI_TRUSTED, &list);
    CHECK(list.n == 1 && !list.items[0].is_ca && list.items[0].has_crl == -1,
          "corrupt_file: %zu entries", list.n);
    ua_cert_list_free(&list);

    snprintf(root, sizeof root, "%s/scenarios/intermediate/pki", vectors);
    ua_pki_list(root, UA_PKI_TRUSTED, &list);
    CHECK(list.n == 1 && list.items[0].is_ca && list.items[0].has_crl == 1,
          "intermediate trusted CA with CRL");
    CHECK(list.n == 1 && !strcmp(list.items[0].subject,
                                 "CN=tedge-dot test root CA, O=tedge-dot test"),
          "root subject");
    ua_cert_list_free(&list);
    ua_pki_list(root, UA_PKI_ISSUERS, &list);
    CHECK(list.n == 1 && list.items[0].is_ca && list.items[0].has_crl == 1,
          "intermediate issuer CA with CRL");
    ua_cert_list_free(&list);

    snprintf(root, sizeof root, "%s/scenarios/ca_no_crl/pki", vectors);
    ua_pki_list(root, UA_PKI_TRUSTED, &list);
    CHECK(list.n == 1 && list.items[0].has_crl == 0, "ca_no_crl has no CRL");
    ua_cert_list_free(&list);
}

static void test_add_crl(const char *vectors) {
    char root[1024], crl_path[1100], out[1024], err[256];
    snprintf(root, sizeof root, "%s/scenarios/intermediate_no_crl/pki", vectors);
    /* the intermediate's CRL, from the scenario that has one */
    snprintf(crl_path, sizeof crl_path, "%s/scenarios/intermediate/pki/issuers/crl", vectors);
    char cmd[1200];
    snprintf(cmd, sizeof cmd, "ls '%s'/* | head -1", crl_path);
    FILE *p = popen(cmd, "r");
    char file[1024] = "";
    if (p) {
        if (!fgets(file, sizeof file, p))
            file[0] = '\0';
        pclose(p);
    }
    file[strcspn(file, "\n")] = '\0';
    size_t len = 0;
    unsigned char *text = ua_pki_read_file(file, &len);
    ua_blobs_t crls = {0};
    CHECK(text && ua_pki_parse_crls(text, len, &crls, file) == 1, "parse PEM CRL %s", file);
    if (crls.n == 1) {
        CHECK(ua_pki_add_crl(root, crls.items[0].der, crls.items[0].len, out,
                             sizeof out, err, sizeof err) == 0, "add_crl: %s", err);
        CHECK(strstr(out, "/issuers/crl/") != NULL, "crl stored at %s", out);
        char empty[] = "/tmp/tdot-pki-empty-XXXXXX";
        CHECK(mkdtemp(empty) != NULL, "mkdtemp");
        CHECK(ua_pki_add_crl(empty, crls.items[0].der, crls.items[0].len, out,
                             sizeof out, err, sizeof err) != 0, "unknown CA refused");
    }
    ua_blobs_free(&crls);
    free(text);
}

static ua_cert_request_t REQ = {
    .application_name = "tedge-dot",
    .application_uri = "urn:tedge-dot:test",
    .hostnames = NULL,
    .nhostnames = 0,
    .days = 30,
};

typedef struct {
    const char *root;
    bool generated;
    char thumbprint[41];
    int rc;
} race_t;

static void *race(void *arg) {
    race_t *r = arg;
    char cert[1100], key[1100], err[256];
    snprintf(cert, sizeof cert, "%s/%s", r->root, UA_PKI_OWN_CERT);
    snprintf(key, sizeof key, "%s/%s", r->root, UA_PKI_OWN_KEY);
    UA_ByteString c, k;
    r->rc = ua_pki_load_or_create_own(r->root, cert, key, false, true, &REQ,
                                      &c, &k, &r->generated, err, sizeof err);
    if (r->rc == 0) {
        ua_pki_thumbprint(c.data, c.length, r->thumbprint);
        UA_ByteString_clear(&c);
        UA_ByteString_clear(&k);
    } else {
        fprintf(stderr, "load_or_create: %s\n", err);
    }
    return NULL;
}

static void test_own(const char *vectors) {
    char dir[] = "/tmp/tdot-pki-own-XXXXXX";
    CHECK(mkdtemp(dir) != NULL, "mkdtemp");
    char root[256];
    snprintf(root, sizeof root, "%s/pki", dir);

    /* concurrent first start: one certificate */
    race_t r[4];
    pthread_t t[4];
    for (int i = 0; i < 4; i++) {
        r[i] = (race_t){.root = root};
        pthread_create(&t[i], NULL, race, &r[i]);
    }
    int generated = 0;
    for (int i = 0; i < 4; i++) {
        pthread_join(t[i], NULL);
        CHECK(r[i].rc == 0, "thread %d failed", i);
        generated += r[i].generated;
        CHECK(!strcmp(r[i].thumbprint, r[0].thumbprint), "thread %d: other certificate", i);
    }
    CHECK(generated == 1, "generated %d certificates", generated);

    char cert[1100], key[1100], err[256];
    snprintf(cert, sizeof cert, "%s/%s", root, UA_PKI_OWN_CERT);
    snprintf(key, sizeof key, "%s/%s", root, UA_PKI_OWN_KEY);
    struct stat st;
    CHECK(stat(key, &st) == 0 && (st.st_mode & 0777) == 0600, "key mode %o", st.st_mode & 0777);
    snprintf(err, sizeof err, "%s/own/private", root);
    CHECK(stat(err, &st) == 0 && (st.st_mode & 0777) == 0700, "private dir mode");

    size_t len = 0;
    unsigned char *der = ua_pki_read_file(cert, &len);
    ua_cert_info_t info;
    CHECK(der && ua_pki_cert_info(der, len, &info) == 0, "read generated cert");
    if (der) {
        CHECK(info.nalt >= 2 && !strcmp(info.alt_names[0], "urn:tedge-dot:test"),
              "URI SAN first: %s", info.nalt ? info.alt_names[0] : "(none)");
        CHECK(info.key_bits == 2048, "key bits %zu", info.key_bits);
        CHECK(!info.is_ca, "generated certificate is no CA");
        CHECK(!strcmp(info.subject, "CN=tedge-dot, O=tedge-dot"), "subject %s", info.subject);
        free(der);
    }

    /* reuse: same certificate, not generated */
    race_t again = {.root = root};
    race(&again);
    CHECK(again.rc == 0 && !again.generated && !strcmp(again.thumbprint, r[0].thumbprint),
          "reused");

    /* an interrupted generation (key, no certificate): set aside, generate again
     * (Rust: orphan_key_is_set_aside_and_a_certificate_without_key_refused) */
    CHECK(unlink(cert) == 0, "unlink cert");
    race_t orphan = {.root = root};
    race(&orphan);
    CHECK(orphan.rc == 0 && orphan.generated && strcmp(orphan.thumbprint, r[0].thumbprint),
          "orphan key: regenerated");
    snprintf(err, sizeof err, "%s/own/private", root);
    DIR *pd = opendir(err);
    int aside = 0;
    for (struct dirent *e; pd && (e = readdir(pd));)
        aside += !strncmp(e->d_name, "key.pem.", 8);
    if (pd)
        closedir(pd);
    CHECK(aside == 1, "orphan key kept aside (%d)", aside);
    /* a certificate without its key is the operator's to fix */
    CHECK(unlink(key) == 0, "unlink key");
    {
        UA_ByteString oc, ok;
        bool og;
        CHECK(ua_pki_load_or_create_own(root, cert, key, false, true, &REQ, &oc, &ok, &og, err,
                                        sizeof err) != 0 &&
                  strstr(err, "no private key"),
              "certificate without key: %s", err);
    }

    /* URI mismatch with an explicit certificate */
    char vcert[1100], vkey[1100];
    snprintf(vcert, sizeof vcert, "%s/client/cert.der", vectors);
    snprintf(vkey, sizeof vkey, "%s/client/key.pem", vectors);
    ua_cert_request_t other = REQ;
    other.application_uri = "urn:somebody-else";
    UA_ByteString c, k;
    bool gen;
    CHECK(ua_pki_load_or_create_own("/nonexistent", vcert, vkey, true, true, &other,
                                    &c, &k, &gen, err, sizeof err) != 0 &&
          strstr(err, "urn:tedge-dot") && strstr(err, "urn:somebody-else"),
          "uri mismatch: %s", err);
    /* a key of another certificate */
    snprintf(vkey, sizeof vkey, "%s/users/operator.key.pem", vectors);
    other.application_uri = "urn:tedge-dot";
    CHECK(ua_pki_load_or_create_own("/nonexistent", vcert, vkey, true, true, &other,
                                    &c, &k, &gen, err, sizeof err) != 0 &&
          strstr(err, "does not belong"), "key mismatch: %s", err);
    /* create_certificate = false */
    char dir2[] = "/tmp/tdot-pki-nocreate-XXXXXX";
    CHECK(mkdtemp(dir2) != NULL, "mkdtemp");
    snprintf(cert, sizeof cert, "%s/%s", dir2, UA_PKI_OWN_CERT);
    snprintf(key, sizeof key, "%s/%s", dir2, UA_PKI_OWN_KEY);
    CHECK(ua_pki_load_or_create_own(dir2, cert, key, false, false, &REQ, &c, &k,
                                    &gen, err, sizeof err) != 0 &&
          strstr(err, "create_certificate"), "no create: %s", err);
    /* a key readable by others */
    snprintf(vkey, sizeof vkey, "%s/client/key.pem", vectors);
    chmod(vkey, 0640);
    other.application_uri = "urn:tedge-dot";
    CHECK(ua_pki_load_or_create_own("/nonexistent", vcert, vkey, true, true, &other,
                                    &c, &k, &gen, err, sizeof err) != 0 &&
          strstr(err, "0640"), "key mode: %s", err);
    chmod(vkey, 0600);
}

static void test_generate_hostnames(void) {
    const char *hosts[] = {"gw01.plant.local", "10.1.2.3"};
    ua_cert_request_t req = REQ;
    req.application_uri = "urn:gw01";
    req.hostnames = hosts;
    req.nhostnames = 2;
    unsigned char *der = NULL;
    size_t len = 0;
    char *pem = NULL, err[256];
    CHECK(ua_pki_generate(&req, &der, &len, &pem, err, sizeof err) == 0, "generate: %s", err);
    ua_cert_info_t info;
    if (der && ua_pki_cert_info(der, len, &info) == 0) {
        CHECK(info.nalt == 3 && !strcmp(info.alt_names[0], "urn:gw01") &&
              !strcmp(info.alt_names[1], "gw01.plant.local") &&
              !strcmp(info.alt_names[2], "10.1.2.3"), "SANs");
        CHECK(ua_pki_key_matches(der, len, (unsigned char *)pem, strlen(pem)), "key matches");
    }
    /* two generated certificates never share a serial (upstream generators use a constant) */
    unsigned char *der2 = NULL;
    size_t len2 = 0;
    char *pem2 = NULL;
    CHECK(ua_pki_generate(&req, &der2, &len2, &pem2, err, sizeof err) == 0, "generate 2");
    CHECK(len != len2 || memcmp(der, der2, len) != 0, "distinct certificates");
    free(der);
    free(der2);
    free(pem);
    free(pem2);
}

/* An expired application certificate keeps secured devices disconnected with
 * the "application certificate:" reason (spec §3.2), before any network I/O. */
static void test_expired_own_certificate(const char *vectors) {
    char dir[] = "/tmp/tdot-pki-expired-XXXXXX";
    CHECK(mkdtemp(dir) != NULL, "mkdtemp");
    char path[256];
    snprintf(path, sizeof path, "%s/opcua.toml", dir);
    FILE *fp = fopen(path, "w");
    fprintf(fp,
            "[connector]\nprotocol = \"opcua\"\n\n[connection]\n"
            "pki_dir = \"pki\"\napplication_uri = \"urn:tedge:opcua-sim\"\n"
            "certificate = \"%s/scenarios/expired/server/cert.der\"\n"
            "private_key = \"%s/scenarios/expired/server/key.pem\"\n\n"
            "[[device]]\nname = \"plc\"\n"
            "protocol_address = { endpoint = \"opc.tcp://127.0.0.1:1\", "
            "security_policy = \"Basic256Sha256\" }\n"
            "  [[device.point]]\n  id = \"t\"\n  datatype = \"float64\"\n"
            "  address = { node_id = \"ns=2;s=T\" }\n",
            vectors, vectors);
    fclose(fp);
    char err[512] = "";
    tdot_config_t *cfg = tdot_config_load(path, err, sizeof err);
    CHECK(cfg != NULL, "load: %s", err);
    if (!cfg)
        return;
    tdot_connector_t *c = tdot_connector_factory("opcua");
    CHECK(c->configure(c, cfg, err, sizeof err) == 0, "configure: %s", err);
    err[0] = '\0';
    CHECK(c->connect_device(c, &cfg->devices[0], err, sizeof err) != 0 &&
              !strncmp(err, "application certificate:", 24) && strstr(err, "expired"),
          "expired own certificate: %s", err);
    char *info = c->device_info(c, &cfg->devices[0]);
    CHECK(info && strstr(info, "\"security_policy\":\"Basic256Sha256\"") &&
              strstr(info, "\"security_mode\":\"sign_and_encrypt\""),
          "device info: %s", info ? info : "(null)");
    free(info);
    c->disconnect_device(c, &cfg->devices[0]);
    tdot_config_free(cfg);
    c->destroy(c);
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s <genpki vectors dir>\n", argv[0]);
        return 2;
    }
    char path[1024];
    snprintf(path, sizeof path, "%s/expected.json", argv[1]);
    cJSON *expected = load_json(path);
    if (!expected) {
        fprintf(stderr, "cannot read %s\n", path);
        return 2;
    }
    test_vectors(argv[1], expected);
    test_quarantine(argv[1], expected);
    test_listing(argv[1]);
    test_add_crl(argv[1]);
    test_own(argv[1]);
    test_generate_hostnames();
    test_expired_own_certificate(argv[1]);
    cJSON_Delete(expected);
    if (failures) {
        fprintf(stderr, "%d failure(s)\n", failures);
        return 1;
    }
    printf("opcua pki: all checks passed\n");
    return 0;
}
