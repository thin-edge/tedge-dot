/* tedge-dot — OPC UA PKI directory and certificate trust (mbedTLS).
 * See ua_pki.h. Every decision here has a counterpart in the Rust build
 * (connector-opcua/src/pki.rs, vendor/async-opcua-crypto/src/trust_list.rs
 * and certificate_store.rs); keep them in step.
 */
#define _GNU_SOURCE /* timegm */
#include "ua_pki.h"

#include <arpa/inet.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <unistd.h>

#include <mbedtls/ctr_drbg.h>
#include <mbedtls/entropy.h>
#include <mbedtls/md.h>
#include <mbedtls/oid.h>
#include <mbedtls/pem.h>
#include <mbedtls/platform_util.h>
#include <mbedtls/pk.h>
#include <mbedtls/rsa.h>
#include <mbedtls/sha1.h>
#include <mbedtls/x509_crl.h>
#include <mbedtls/x509_crt.h>

#define MAX_CHAIN_DEPTH 8

static const char *const LAYOUT[] = {
    "own/certs",     "own/private",   "trusted/certs", "trusted/crl",
    "issuers/certs", "issuers/crl",   "rejected/certs",
};

static const char *certs_dir(ua_pki_group_t g) {
    switch (g) {
    case UA_PKI_TRUSTED: return "trusted/certs";
    case UA_PKI_ISSUERS: return "issuers/certs";
    default: return "rejected/certs";
    }
}

static const char *crl_dir(ua_pki_group_t g) {
    switch (g) {
    case UA_PKI_TRUSTED: return "trusted/crl";
    case UA_PKI_ISSUERS: return "issuers/crl";
    default: return NULL;
    }
}

const char *ua_pki_group_name(ua_pki_group_t g) {
    switch (g) {
    case UA_PKI_TRUSTED: return "trusted";
    case UA_PKI_ISSUERS: return "issuers";
    default: return "rejected";
    }
}

int ua_pki_group_parse(const char *s, ua_pki_group_t *out) {
    if (!strcmp(s, "trusted")) *out = UA_PKI_TRUSTED;
    else if (!strcmp(s, "issuers")) *out = UA_PKI_ISSUERS;
    else if (!strcmp(s, "rejected")) *out = UA_PKI_REJECTED;
    else return -1;
    return 0;
}

/* ---- blobs ------------------------------------------------------------- */

static void blobs_push(ua_blobs_t *b, const unsigned char *der, size_t len,
                       const char *path) {
    ua_blob_t *items = realloc(b->items, (b->n + 1) * sizeof *items);
    if (!items)
        return;
    b->items = items;
    ua_blob_t *it = &b->items[b->n];
    it->der = malloc(len);
    if (!it->der)
        return;
    memcpy(it->der, der, len);
    it->len = len;
    snprintf(it->path, sizeof it->path, "%s", path ? path : "");
    b->n++;
}

void ua_blobs_free(ua_blobs_t *b) {
    for (size_t i = 0; i < b->n; i++)
        free(b->items[i].der);
    free(b->items);
    b->items = NULL;
    b->n = 0;
}

static bool cert_der_ok(const unsigned char *der, size_t len) {
    mbedtls_x509_crt c;
    mbedtls_x509_crt_init(&c);
    /* The whole buffer, as Rust's strict DER parser requires: mbedTLS ignores
     * trailing bytes (such as a second certificate of a DER chain). */
    int rc = mbedtls_x509_crt_parse_der(&c, der, len);
    bool whole = rc == 0 && c.raw.len == len;
    mbedtls_x509_crt_free(&c);
    return whole;
}

static bool crl_der_ok(const unsigned char *der, size_t len) {
    mbedtls_x509_crl c;
    mbedtls_x509_crl_init(&c);
    /* mbedTLS already refuses trailing data here (unlike certificates, where
     * it stops after the first one); the length check keeps the two parsers
     * alike should that change. */
    int rc = mbedtls_x509_crl_parse_der(&c, der, len);
    bool whole = rc == 0 && c.raw.len == len;
    mbedtls_x509_crl_free(&c);
    return whole;
}

/* Every PEM block labelled `label`; -1 when the buffer is not PEM text. */
static long pem_blocks(const unsigned char *buf, size_t len, const char *label,
                       bool (*ok)(const unsigned char *, size_t),
                       ua_blobs_t *out, const char *path) {
    size_t skip = 0;
    while (skip < len && (buf[skip] == ' ' || buf[skip] == '\t' ||
                          buf[skip] == '\r' || buf[skip] == '\n'))
        skip++;
    if (len - skip < 10 || memcmp(buf + skip, "-----BEGIN", 10) != 0)
        return -1;
    char *text = malloc(len + 1);
    if (!text)
        return 0;
    memcpy(text, buf, len);
    text[len] = '\0';
    char header[64], footer[64];
    snprintf(header, sizeof header, "-----BEGIN %s-----", label);
    snprintf(footer, sizeof footer, "-----END %s-----", label);
    long found = 0;
    char *p = text;
    while ((p = strstr(p, header)) != NULL) {
        mbedtls_pem_context pem;
        mbedtls_pem_init(&pem);
        size_t used = 0;
        int rc = mbedtls_pem_read_buffer(&pem, header, footer,
                                         (const unsigned char *)p, NULL, 0,
                                         &used);
        if (rc == 0) {
            size_t der_len = 0;
            const unsigned char *der = mbedtls_pem_get_buffer(&pem, &der_len);
            if (ok(der, der_len)) {
                blobs_push(out, der, der_len, path);
                found++;
            }
            p += used;
        } else {
            p += strlen(header);
        }
        mbedtls_pem_free(&pem);
    }
    free(text);
    return found;
}

size_t ua_pki_parse_certs(const unsigned char *buf, size_t len, ua_blobs_t *out,
                          const char *path) {
    long n = pem_blocks(buf, len, "CERTIFICATE", cert_der_ok, out, path);
    if (n >= 0)
        return (size_t)n;
    if (cert_der_ok(buf, len)) {
        blobs_push(out, buf, len, path);
        return 1;
    }
    return 0;
}

size_t ua_pki_parse_crls(const unsigned char *buf, size_t len, ua_blobs_t *out,
                         const char *path) {
    long n = pem_blocks(buf, len, "X509 CRL", crl_der_ok, out, path);
    if (n >= 0)
        return (size_t)n;
    if (crl_der_ok(buf, len)) {
        blobs_push(out, buf, len, path);
        return 1;
    }
    return 0;
}

unsigned char *ua_pki_read_file(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (!f)
        return NULL;
    unsigned char *buf = NULL;
    size_t cap = 0, n = 0;
    for (;;) {
        if (n == cap) {
            cap = cap ? cap * 2 : 4096;
            unsigned char *nb = realloc(buf, cap);
            if (!nb) {
                free(buf);
                fclose(f);
                return NULL;
            }
            buf = nb;
        }
        size_t r = fread(buf + n, 1, cap - n, f);
        n += r;
        if (r == 0)
            break;
    }
    fclose(f);
    *len = n;
    return buf;
}

/* Sorted regular, non-hidden files of a directory. */
static size_t list_files(const char *dir, char ***out) {
    *out = NULL;
    DIR *d = opendir(dir);
    if (!d)
        return 0;
    size_t n = 0;
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        if (e->d_name[0] == '.')
            continue;
        char path[UA_PKI_PATH_MAX];
        snprintf(path, sizeof path, "%s/%s", dir, e->d_name);
        struct stat st;
        if (stat(path, &st) != 0 || !S_ISREG(st.st_mode))
            continue;
        char **grown = realloc(*out, (n + 1) * sizeof *grown);
        if (!grown)
            break;
        *out = grown;
        (*out)[n++] = strdup(path);
    }
    closedir(d);
    for (size_t i = 1; i < n; i++) /* insertion sort: directories are small */
        for (size_t j = i; j > 0 && strcmp((*out)[j - 1], (*out)[j]) > 0; j--) {
            char *t = (*out)[j];
            (*out)[j] = (*out)[j - 1];
            (*out)[j - 1] = t;
        }
    return n;
}

static void free_files(char **files, size_t n) {
    for (size_t i = 0; i < n; i++)
        free(files[i]);
    free(files);
}

static void read_dir_into(const char *root, const char *sub, bool crls,
                          ua_blobs_t *out) {
    char dir[UA_PKI_PATH_MAX];
    snprintf(dir, sizeof dir, "%s/%s", root, sub);
    char **files;
    size_t n = list_files(dir, &files);
    for (size_t i = 0; i < n; i++) {
        size_t len = 0;
        unsigned char *buf = ua_pki_read_file(files[i], &len);
        if (!buf) {
            fprintf(stderr, "[opcua] cannot read %s\n", files[i]);
            continue;
        }
        size_t found = crls ? ua_pki_parse_crls(buf, len, out, files[i])
                            : ua_pki_parse_certs(buf, len, out, files[i]);
        if (!found)
            fprintf(stderr, "[opcua] %s is not a %s; skipped\n", files[i],
                    crls ? "CRL" : "certificate");
        free(buf);
    }
    free_files(files, n);
}

void ua_pki_load_trust(const char *root, ua_trust_t *t) {
    memset(t, 0, sizeof *t);
    read_dir_into(root, "trusted/certs", false, &t->trusted);
    read_dir_into(root, "trusted/crl", true, &t->trusted_crls);
    read_dir_into(root, "issuers/certs", false, &t->issuers);
    read_dir_into(root, "issuers/crl", true, &t->issuer_crls);
    read_dir_into(root, "rejected/certs", false, &t->rejected);
}

void ua_trust_free(ua_trust_t *t) {
    ua_blobs_free(&t->trusted);
    ua_blobs_free(&t->trusted_crls);
    ua_blobs_free(&t->issuers);
    ua_blobs_free(&t->issuer_crls);
    ua_blobs_free(&t->rejected);
}

/* ---- certificate facts ------------------------------------------------ */

void ua_pki_thumbprint(const unsigned char *der, size_t len, char out[41]) {
    unsigned char digest[20];
    mbedtls_sha1(der, len, digest);
    for (int i = 0; i < 20; i++)
        sprintf(out + 2 * i, "%02x", digest[i]);
    out[40] = '\0';
}

void ua_pki_format_time(time_t t, char *out, size_t outlen) {
    struct tm tm;
    gmtime_r(&t, &tm);
    strftime(out, outlen, "%Y-%m-%dT%H:%M:%SZ", &tm);
}

static time_t x509_time(const mbedtls_x509_time *t) {
    struct tm tm = {0};
    tm.tm_year = t->year - 1900;
    tm.tm_mon = t->mon - 1;
    tm.tm_mday = t->day;
    tm.tm_hour = t->hour;
    tm.tm_min = t->min;
    tm.tm_sec = t->sec;
    return timegm(&tm);
}

static bool same_buf(const mbedtls_x509_buf *a, const mbedtls_x509_buf *b) {
    return a->len == b->len && memcmp(a->p, b->p, a->len) == 0;
}

static bool crt_is_ca(const mbedtls_x509_crt *c) {
    return mbedtls_x509_crt_has_ext_type(c, MBEDTLS_X509_EXT_BASIC_CONSTRAINTS) &&
           c->MBEDTLS_PRIVATE(ca_istrue);
}

/* The text of one SAN; 0 when it has one. */
static int san_text(const mbedtls_x509_buf *buf, char *out, size_t outlen) {
    mbedtls_x509_subject_alternative_name san;
    memset(&san, 0, sizeof san);
    if (mbedtls_x509_parse_subject_alt_name(buf, &san) != 0)
        return -1;
    int rc = 0;
    const mbedtls_x509_buf *u = &san.san.unstructured_name;
    switch (san.type) {
    case MBEDTLS_X509_SAN_DNS_NAME:
    case MBEDTLS_X509_SAN_UNIFORM_RESOURCE_IDENTIFIER:
    case MBEDTLS_X509_SAN_RFC822_NAME:
        snprintf(out, outlen, "%.*s", (int)u->len, (const char *)u->p);
        break;
    case MBEDTLS_X509_SAN_IP_ADDRESS:
        if (u->len == 4)
            inet_ntop(AF_INET, u->p, out, (socklen_t)outlen);
        else if (u->len == 16)
            inet_ntop(AF_INET6, u->p, out, (socklen_t)outlen);
        else
            rc = -1;
        break;
    case MBEDTLS_X509_SAN_DIRECTORY_NAME:
        if (mbedtls_x509_dn_gets(out, outlen, &san.san.directory_name) < 0)
            rc = -1;
        break;
    default:
        rc = -1;
    }
    mbedtls_x509_free_subject_alt_name(&san);
    return rc;
}

static void common_name(const mbedtls_x509_crt *c, char *out, size_t outlen) {
    out[0] = '\0';
    for (const mbedtls_x509_name *n = &c->subject; n; n = n->next) {
        if (n->oid.p && MBEDTLS_OID_CMP(MBEDTLS_OID_AT_CN, &n->oid) == 0) {
            snprintf(out, outlen, "%.*s", (int)n->val.len,
                     (const char *)n->val.p);
            return;
        }
    }
}

static void info_from_crt(const mbedtls_x509_crt *c, const unsigned char *der,
                          size_t len, ua_cert_info_t *out) {
    memset(out, 0, sizeof *out);
    if (mbedtls_x509_dn_gets(out->subject, sizeof out->subject, &c->subject) < 0)
        out->subject[0] = '\0';
    if (mbedtls_x509_dn_gets(out->issuer, sizeof out->issuer, &c->issuer) < 0)
        out->issuer[0] = '\0';
    common_name(c, out->common_name, sizeof out->common_name);
    ua_pki_thumbprint(der, len, out->thumbprint);
    out->not_before_t = x509_time(&c->valid_from);
    out->not_after_t = x509_time(&c->valid_to);
    ua_pki_format_time(out->not_before_t, out->not_before, sizeof out->not_before);
    ua_pki_format_time(out->not_after_t, out->not_after, sizeof out->not_after);
    for (const mbedtls_x509_sequence *s = &c->subject_alt_names;
         s && s->buf.p && out->nalt < 16; s = s->next) {
        if (san_text(&s->buf, out->alt_names[out->nalt],
                     sizeof out->alt_names[0]) == 0)
            out->nalt++;
    }
    out->is_ca = crt_is_ca(c);
    out->key_bits = mbedtls_pk_get_bitlen(&c->pk);
}

int ua_pki_cert_info(const unsigned char *der, size_t len, ua_cert_info_t *out) {
    mbedtls_x509_crt c;
    mbedtls_x509_crt_init(&c);
    if (mbedtls_x509_crt_parse_der(&c, der, len) != 0) {
        mbedtls_x509_crt_free(&c);
        return -1;
    }
    info_from_crt(&c, der, len, out);
    mbedtls_x509_crt_free(&c);
    return 0;
}

void ua_pki_canonical_name(const unsigned char *der, size_t len, char *out,
                           size_t outlen) {
    ua_cert_info_t info;
    char cn[256] = "";
    if (ua_pki_cert_info(der, len, &info) == 0)
        snprintf(cn, sizeof cn, "%s", info.common_name);
    for (char *p = cn; *p; p++) {
        unsigned char ch = (unsigned char)*p;
        bool keep = (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
                    (ch >= '0' && ch <= '9') || ch == '-' || ch == '.';
        if (!keep)
            *p = '_';
    }
    char tp[41];
    ua_pki_thumbprint(der, len, tp);
    snprintf(out, outlen, "%s_%s.der", cn, tp);
}

/* RSA/ECDSA signature of `tbs` by `issuer`'s key. */
static bool signature_valid(mbedtls_pk_context *key, mbedtls_md_type_t md,
                            mbedtls_pk_type_t pk_type, void *opts,
                            const mbedtls_x509_buf *tbs,
                            const mbedtls_x509_buf *sig) {
    const mbedtls_md_info_t *info = mbedtls_md_info_from_type(md);
    if (!info)
        return false;
    unsigned char hash[MBEDTLS_MD_MAX_SIZE];
    if (mbedtls_md(info, tbs->p, tbs->len, hash) != 0)
        return false;
    return mbedtls_pk_verify_ext(pk_type, opts, key, md, hash,
                                 mbedtls_md_get_size(info), sig->p,
                                 sig->len) == 0;
}

static bool crt_signed_by(const mbedtls_x509_crt *c, mbedtls_x509_crt *issuer) {
    return signature_valid(&issuer->pk, c->MBEDTLS_PRIVATE(sig_md),
                           c->MBEDTLS_PRIVATE(sig_pk),
                           c->MBEDTLS_PRIVATE(sig_opts), &c->tbs,
                           &c->MBEDTLS_PRIVATE(sig));
}

static bool crl_signed_by(const mbedtls_x509_crl *crl, mbedtls_x509_crt *ca) {
    return same_buf(&crl->issuer_raw, &ca->subject_raw) &&
           signature_valid(&ca->pk, crl->MBEDTLS_PRIVATE(sig_md),
                           crl->MBEDTLS_PRIVATE(sig_pk),
                           crl->MBEDTLS_PRIVATE(sig_opts), &crl->tbs,
                           &crl->MBEDTLS_PRIVATE(sig));
}

bool ua_pki_crl_issued_by(const unsigned char *crl_der, size_t crl_len,
                          const unsigned char *ca_der, size_t ca_len) {
    mbedtls_x509_crl crl;
    mbedtls_x509_crt ca;
    mbedtls_x509_crl_init(&crl);
    mbedtls_x509_crt_init(&ca);
    bool ok = mbedtls_x509_crl_parse_der(&crl, crl_der, crl_len) == 0 &&
              mbedtls_x509_crt_parse_der(&ca, ca_der, ca_len) == 0 &&
              crl_signed_by(&crl, &ca);
    mbedtls_x509_crl_free(&crl);
    mbedtls_x509_crt_free(&ca);
    return ok;
}

static int rng(void *ctx, unsigned char *out, size_t len) {
    return mbedtls_ctr_drbg_random(ctx, out, len);
}

/* A seeded DRBG; returns 0 on success. */
static int drbg_init(mbedtls_entropy_context *e, mbedtls_ctr_drbg_context *d) {
    mbedtls_entropy_init(e);
    mbedtls_ctr_drbg_init(d);
    static const unsigned char pers[] = "tedge-dot-opcua";
    return mbedtls_ctr_drbg_seed(d, mbedtls_entropy_func, e, pers,
                                 sizeof pers - 1);
}

/* mbedtls_pk_parse_key wants PEM NUL-terminated (length included). */
static int parse_key(mbedtls_pk_context *pk, const unsigned char *key,
                     size_t key_len, mbedtls_ctr_drbg_context *d) {
    unsigned char *buf = malloc(key_len + 1);
    if (!buf)
        return -1;
    memcpy(buf, key, key_len);
    buf[key_len] = '\0';
    bool pem = key_len > 10 && memmem(key, key_len, "-----BEGIN", 10) != NULL;
    int rc = mbedtls_pk_parse_key(pk, buf, pem ? key_len + 1 : key_len, NULL, 0,
                                  rng, d);
    mbedtls_platform_zeroize(buf, key_len);
    free(buf);
    return rc;
}

bool ua_pki_key_matches(const unsigned char *cert, size_t cert_len,
                        const unsigned char *key, size_t key_len) {
    mbedtls_entropy_context e;
    mbedtls_ctr_drbg_context d;
    mbedtls_x509_crt c;
    mbedtls_pk_context pk;
    mbedtls_x509_crt_init(&c);
    mbedtls_pk_init(&pk);
    bool ok = drbg_init(&e, &d) == 0 &&
              mbedtls_x509_crt_parse_der(&c, cert, cert_len) == 0 &&
              parse_key(&pk, key, key_len, &d) == 0 &&
              mbedtls_pk_check_pair(&c.pk, &pk, rng, &d) == 0;
    mbedtls_pk_free(&pk);
    mbedtls_x509_crt_free(&c);
    mbedtls_ctr_drbg_free(&d);
    mbedtls_entropy_free(&e);
    return ok;
}

/* ---- trust ------------------------------------------------------------ */

typedef struct {
    mbedtls_x509_crt crt;
    const ua_blob_t *blob;
    bool trusted;
} ca_t;

typedef struct {
    mbedtls_x509_crl crl;
    bool ok;
} crl_t;

static bool blobs_contain(const ua_blobs_t *b, const unsigned char *der,
                          size_t len) {
    for (size_t i = 0; i < b->n; i++)
        if (b->items[i].len == len && !memcmp(b->items[i].der, der, len))
            return true;
    return false;
}

UA_StatusCode ua_trust_verify(const ua_trust_t *t, const unsigned char *der,
                              size_t len, time_t now) {
    /* rejected/certs is a list for review (Part 12), not a deny list: pinning
     * or a trusted chain wins over a copy there (see TrustList::verify, Rust). */
    if (blobs_contain(&t->trusted, der, len))
        return UA_STATUSCODE_GOOD;

    mbedtls_x509_crt leaf;
    mbedtls_x509_crt_init(&leaf);
    if (mbedtls_x509_crt_parse_der(&leaf, der, len) != 0) {
        mbedtls_x509_crt_free(&leaf);
        return UA_STATUSCODE_BADCERTIFICATEUNTRUSTED;
    }

    /* Candidate CAs: trusted first, then issuers (the Rust search order). */
    size_t ncas = t->trusted.n + t->issuers.n;
    ca_t *cas = calloc(ncas ? ncas : 1, sizeof *cas);
    size_t ncrls = t->trusted_crls.n + t->issuer_crls.n;
    crl_t *crls = calloc(ncrls ? ncrls : 1, sizeof *crls);
    UA_StatusCode result = UA_STATUSCODE_BADCERTIFICATEUNTRUSTED;
    if (!cas || !crls)
        goto done;
    for (size_t i = 0; i < ncas; i++) {
        const ua_blob_t *b = i < t->trusted.n ? &t->trusted.items[i]
                                              : &t->issuers.items[i - t->trusted.n];
        mbedtls_x509_crt_init(&cas[i].crt);
        cas[i].blob = b;
        cas[i].trusted = blobs_contain(&t->trusted, b->der, b->len);
        if (mbedtls_x509_crt_parse_der(&cas[i].crt, b->der, b->len) != 0)
            cas[i].blob = NULL;
    }
    for (size_t i = 0; i < ncrls; i++) {
        const ua_blob_t *b = i < t->trusted_crls.n
                                 ? &t->trusted_crls.items[i]
                                 : &t->issuer_crls.items[i - t->trusted_crls.n];
        mbedtls_x509_crl_init(&crls[i].crl);
        crls[i].ok = mbedtls_x509_crl_parse_der(&crls[i].crl, b->der, b->len) == 0;
    }

    /* Build the path first: an untrusted certificate is reported as
     * untrusted, whatever else is wrong with the CAs it names. */
    size_t path[MAX_CHAIN_DEPTH];
    size_t depth = 0;
    mbedtls_x509_crt *current = &leaf;
    for (;;) {
        if (depth >= MAX_CHAIN_DEPTH)
            goto done;
        size_t found = ncas;
        for (size_t i = 0; i < ncas && found == ncas; i++) {
            if (!cas[i].blob || !crt_is_ca(&cas[i].crt))
                continue;
            bool on_path = false;
            for (size_t k = 0; k < depth; k++)
                if (cas[path[k]].blob->len == cas[i].blob->len &&
                    !memcmp(cas[path[k]].blob->der, cas[i].blob->der,
                            cas[i].blob->len))
                    on_path = true;
            if (on_path || !same_buf(&cas[i].crt.subject_raw, &current->issuer_raw))
                continue;
            if (crt_signed_by(current, &cas[i].crt))
                found = i;
        }
        if (found == ncas)
            goto done; /* untrusted */
        path[depth++] = found;
        if (cas[found].trusted)
            break;
        if (same_buf(&cas[found].crt.subject_raw, &cas[found].crt.issuer_raw))
            goto done; /* a root that is only an issuer is no trust anchor */
        current = &cas[found].crt;
    }

    /* Revocation of every certificate on the path, and the CAs' validity. */
    current = &leaf;
    for (size_t d = 0; d < depth; d++) {
        mbedtls_x509_crt *ca = &cas[path[d]].crt;
        bool issuer_level = d > 0;
        bool any = false, revoked = false;
        for (size_t i = 0; i < ncrls; i++) {
            if (!crls[i].ok || !crl_signed_by(&crls[i].crl, ca))
                continue;
            any = true;
            for (const mbedtls_x509_crl_entry *e = &crls[i].crl.entry;
                 e && e->raw.p; e = e->next)
                if (e->serial.len == current->serial.len &&
                    !memcmp(e->serial.p, current->serial.p, e->serial.len))
                    revoked = true;
        }
        if (!any) {
            result = issuer_level ? UA_STATUSCODE_BADCERTIFICATEISSUERREVOCATIONUNKNOWN
                                  : UA_STATUSCODE_BADCERTIFICATEREVOCATIONUNKNOWN;
            goto done;
        }
        if (revoked) {
            result = issuer_level ? UA_STATUSCODE_BADCERTIFICATEISSUERREVOKED
                                  : UA_STATUSCODE_BADCERTIFICATEREVOKED;
            goto done;
        }
        if (x509_time(&ca->valid_from) > now || x509_time(&ca->valid_to) < now) {
            result = UA_STATUSCODE_BADCERTIFICATEISSUERTIMEINVALID;
            goto done;
        }
        current = ca;
    }
    result = UA_STATUSCODE_GOOD;

done:
    if (cas)
        for (size_t i = 0; i < ncas; i++)
            mbedtls_x509_crt_free(&cas[i].crt);
    if (crls)
        for (size_t i = 0; i < ncrls; i++)
            mbedtls_x509_crl_free(&crls[i].crl);
    free(cas);
    free(crls);
    mbedtls_x509_crt_free(&leaf);
    return result;
}

UA_StatusCode ua_pki_validate(const char *root, const unsigned char *der,
                              size_t len, size_t min_bits, size_t max_bits,
                              const char *hostname,
                              const char *application_uri) {
    ua_cert_info_t info;
    if (ua_pki_cert_info(der, len, &info) != 0)
        return UA_STATUSCODE_BADCERTIFICATEINVALID;
    time_t now = time(NULL);

    ua_trust_t trust;
    ua_pki_load_trust(root, &trust);
    UA_StatusCode rc = ua_trust_verify(&trust, der, len, now);
    if (rc == UA_STATUSCODE_BADCERTIFICATEUNTRUSTED &&
        !blobs_contain(&trust.rejected, der, len)) {
        char path[UA_PKI_PATH_MAX], err[256];
        if (ua_pki_store(root, UA_PKI_REJECTED, der, len, path, sizeof path,
                         err, sizeof err) == 0)
            fprintf(stderr, "[opcua] certificate %s is untrusted; stored in %s\n",
                    info.thumbprint, path);
        else
            fprintf(stderr, "[opcua] certificate %s is untrusted; %s\n",
                    info.thumbprint, err);
    }
    ua_trust_free(&trust);
    if (rc != UA_STATUSCODE_GOOD)
        return rc;

    if (info.key_bits < min_bits || info.key_bits > max_bits)
        return UA_STATUSCODE_BADCERTIFICATEPOLICYCHECKFAILED;
    if (now < info.not_before_t || now > info.not_after_t)
        return UA_STATUSCODE_BADCERTIFICATETIMEINVALID;
    if (hostname) {
        bool ok = false;
        /* The first SAN is the application URI (as async-opcua reads it). */
        for (size_t i = 1; i < info.nalt && !ok; i++)
            ok = strcasecmp(info.alt_names[i], hostname) == 0;
        if (!ok || !*hostname)
            return UA_STATUSCODE_BADCERTIFICATEHOSTNAMEINVALID;
    }
    if (application_uri &&
        (info.nalt == 0 || strcmp(info.alt_names[0], application_uri) != 0))
        return UA_STATUSCODE_BADCERTIFICATEURIINVALID;
    return UA_STATUSCODE_GOOD;
}

/* ---- directory -------------------------------------------------------- */

static int mkdirs(const char *path, char *err, size_t errlen) {
    char buf[UA_PKI_PATH_MAX];
    snprintf(buf, sizeof buf, "%s", path);
    for (char *p = buf + 1; *p; p++) {
        if (*p != '/')
            continue;
        *p = '\0';
        if (mkdir(buf, 0755) != 0 && errno != EEXIST) {
            snprintf(err, errlen, "cannot create %s: %s", buf, strerror(errno));
            return -1;
        }
        *p = '/';
    }
    if (mkdir(buf, 0755) != 0 && errno != EEXIST) {
        snprintf(err, errlen, "cannot create %s: %s", buf, strerror(errno));
        return -1;
    }
    return 0;
}

int ua_pki_ensure_layout(const char *root, char *err, size_t errlen) {
    char path[UA_PKI_PATH_MAX];
    for (size_t i = 0; i < sizeof LAYOUT / sizeof LAYOUT[0]; i++) {
        snprintf(path, sizeof path, "%s/%s", root, LAYOUT[i]);
        if (mkdirs(path, err, errlen) != 0)
            return -1;
    }
    snprintf(path, sizeof path, "%s/own/private", root);
    if (chmod(path, 0700) != 0) {
        snprintf(err, errlen, "cannot set permissions of %s: %s", path,
                 strerror(errno));
        return -1;
    }
    return 0;
}

int ua_pki_write_atomic(const char *path, const unsigned char *data,
                        size_t len, int mode, char *err, size_t errlen) {
    char dir[UA_PKI_PATH_MAX];
    snprintf(dir, sizeof dir, "%s", path);
    char *slash = strrchr(dir, '/');
    const char *base = path;
    if (slash) {
        *slash = '\0';
        base = path + (slash - dir) + 1;
        if (*dir && mkdirs(dir, err, errlen) != 0)
            return -1;
    } else {
        snprintf(dir, sizeof dir, ".");
    }
    /* A fresh temporary file: never an existing file or a symlink someone
     * placed at the name, and its mode set through the descriptor. */
    char tmp[UA_PKI_PATH_MAX];
    int fd = -1;
    for (unsigned attempt = 0; fd < 0 && attempt < 16; attempt++) {
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        snprintf(tmp, sizeof tmp, "%s/.%s.%ld.%lx.tmp", dir, base, (long)getpid(),
                 ((unsigned long)ts.tv_nsec << 8) | attempt);
        fd = open(tmp, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, mode);
        if (fd < 0 && errno != EEXIST)
            break;
    }
    if (fd < 0 || fchmod(fd, (mode_t)mode) != 0) {
        if (fd < 0 && errno == EEXIST) /* every candidate name was taken */
            snprintf(err, errlen, "cannot write %s: no free temporary name", path);
        else
            snprintf(err, errlen, "cannot write %s: %s", path, strerror(errno));
        if (fd >= 0) {
            close(fd);
            unlink(tmp);
        }
        return -1;
    }
    size_t off = 0;
    while (off < len) {
        ssize_t w = write(fd, data + off, len - off);
        if (w < 0) {
            if (errno == EINTR)
                continue;
            snprintf(err, errlen, "cannot write %s: %s", path, strerror(errno));
            close(fd);
            unlink(tmp);
            return -1;
        }
        off += (size_t)w;
    }
    fsync(fd);
    close(fd);
    /* rename(2) replaces a symlink at `path` itself, never its target. */
    if (rename(tmp, path) != 0) {
        snprintf(err, errlen, "cannot write %s: %s", path, strerror(errno));
        unlink(tmp);
        return -1;
    }
    return 0;
}

int ua_pki_lock_own(const char *root, char *err, size_t errlen) {
    char path[UA_PKI_PATH_MAX];
    snprintf(path, sizeof path, "%s/own/.lock", root);
    int fd = open(path, O_RDWR | O_CREAT, 0644);
    if (fd < 0) {
        snprintf(err, errlen, "cannot open %s: %s", path, strerror(errno));
        return -1;
    }
    while (flock(fd, LOCK_EX) != 0) {
        if (errno != EINTR) {
            snprintf(err, errlen, "cannot lock %s: %s", path, strerror(errno));
            close(fd);
            return -1;
        }
    }
    return fd;
}

void ua_pki_unlock(int fd) {
    if (fd >= 0) {
        flock(fd, LOCK_UN);
        close(fd);
    }
}

int ua_pki_store(const char *root, ua_pki_group_t g, const unsigned char *der,
                 size_t len, char *path_out, size_t pathlen, char *err,
                 size_t errlen) {
    if (!cert_der_ok(der, len)) {
        snprintf(err, errlen, "not a certificate");
        return -1;
    }
    char name[400];
    ua_pki_canonical_name(der, len, name, sizeof name);
    snprintf(path_out, pathlen, "%s/%s/%s", root, certs_dir(g), name);
    return ua_pki_write_atomic(path_out, der, len, 0644, err, errlen);
}

void ua_pki_list(const char *root, ua_pki_group_t g, ua_cert_list_t *out) {
    memset(out, 0, sizeof *out);
    ua_blobs_t certs = {0}, crls = {0};
    read_dir_into(root, certs_dir(g), false, &certs);
    if (crl_dir(g))
        read_dir_into(root, crl_dir(g), true, &crls);
    out->items = calloc(certs.n ? certs.n : 1, sizeof *out->items);
    for (size_t i = 0; out->items && i < certs.n; i++) {
        ua_cert_info_t info;
        if (ua_pki_cert_info(certs.items[i].der, certs.items[i].len, &info) != 0)
            continue;
        ua_cert_entry_t *e = &out->items[out->n++];
        e->group = g;
        snprintf(e->path, sizeof e->path, "%s", certs.items[i].path);
        e->der = certs.items[i].der;
        e->len = certs.items[i].len;
        certs.items[i].der = NULL; /* moved */
        snprintf(e->thumbprint, sizeof e->thumbprint, "%s", info.thumbprint);
        snprintf(e->subject, sizeof e->subject, "%s", info.subject);
        snprintf(e->not_after, sizeof e->not_after, "%s", info.not_after);
        e->is_ca = info.is_ca;
        e->has_crl = -1;
        if (info.is_ca) {
            e->has_crl = 0;
            for (size_t k = 0; k < crls.n && !e->has_crl; k++)
                if (ua_pki_crl_issued_by(crls.items[k].der, crls.items[k].len,
                                         e->der, e->len))
                    e->has_crl = 1;
        }
    }
    ua_blobs_free(&certs);
    ua_blobs_free(&crls);
}

void ua_cert_list_free(ua_cert_list_t *l) {
    for (size_t i = 0; i < l->n; i++)
        free(l->items[i].der);
    free(l->items);
    l->items = NULL;
    l->n = 0;
}

/* Take the certificate `der` out of the file `path`: the file is deleted when
 * nothing else in it is a certificate; otherwise (a PEM bundle) only the
 * matching CERTIFICATE blocks are dropped and everything else is kept.
 * Mirrors remove_from_file (Rust). */
static int remove_from_file(const char *path, const unsigned char *der, size_t len,
                            char *err, size_t errlen) {
    size_t n = 0;
    unsigned char *bytes = ua_pki_read_file(path, &n);
    if (!bytes)
        return errno == ENOENT ? 0 : (snprintf(err, errlen, "cannot read %s: %s", path,
                                                strerror(errno)), -1);
    ua_blobs_t certs = {0};
    ua_pki_parse_certs(bytes, n, &certs, path);
    size_t keep = 0;
    for (size_t i = 0; i < certs.n; i++)
        if (!(certs.items[i].len == len && !memcmp(certs.items[i].der, der, len)))
            keep++;
    size_t total = certs.n;
    ua_blobs_free(&certs);
    if (total == 0 || keep == total) {
        free(bytes);
        return 0;
    }
    if (keep == 0) {
        free(bytes);
        if (unlink(path) != 0 && errno != ENOENT) {
            snprintf(err, errlen, "cannot remove %s: %s", path, strerror(errno));
            return -1;
        }
        return 0;
    }
    /* A bundle: rebuild the text without the matching blocks. */
    char *text = malloc(n + 1);
    char *out = malloc(n + 1);
    if (!text || !out) {
        free(text);
        free(out);
        free(bytes);
        snprintf(err, errlen, "out of memory");
        return -1;
    }
    memcpy(text, bytes, n);
    text[n] = '\0';
    free(bytes);
    static const char BEGIN[] = "-----BEGIN CERTIFICATE-----";
    static const char END[] = "-----END CERTIFICATE-----";
    size_t used = 0;
    const char *p = text;
    for (;;) {
        const char *b = strstr(p, BEGIN);
        const char *e = b ? strstr(b, END) : NULL;
        if (!b || !e) {
            size_t rest = strlen(p);
            memcpy(out + used, p, rest);
            used += rest;
            break;
        }
        e += sizeof END - 1;
        ua_blobs_t one = {0};
        ua_pki_parse_certs((const unsigned char *)b, (size_t)(e - b), &one, path);
        bool drop = one.n == 1 && one.items[0].len == len && !memcmp(one.items[0].der, der, len);
        ua_blobs_free(&one);
        if (drop) {
            memcpy(out + used, p, (size_t)(b - p));
            used += (size_t)(b - p);
            if (*e == '\r')
                e++;
            if (*e == '\n')
                e++;
        } else {
            memcpy(out + used, p, (size_t)(e - p));
            used += (size_t)(e - p);
        }
        p = e;
    }
    struct stat st;
    int mode = stat(path, &st) == 0 ? (int)(st.st_mode & 07777) : 0644;
    int rc = ua_pki_write_atomic(path, (const unsigned char *)out, used, mode, err, errlen);
    free(text);
    free(out);
    return rc;
}

int ua_pki_remove(const char *root, const ua_cert_entry_t *e, char *err,
                  size_t errlen) {
    ua_cert_list_t all;
    ua_pki_list(root, e->group, &all);
    int rc = 0;
    for (size_t i = 0; i < all.n && rc == 0; i++) {
        if (all.items[i].len != e->len || memcmp(all.items[i].der, e->der, e->len))
            continue;
        bool seen = false; /* one file listed twice (a duplicate inside it) */
        for (size_t k = 0; k < i && !seen; k++)
            seen = !strcmp(all.items[k].path, all.items[i].path);
        if (!seen)
            rc = remove_from_file(all.items[i].path, e->der, e->len, err, errlen);
    }
    ua_cert_list_free(&all);
    return rc;
}

int ua_pki_add_crl(const char *root, const unsigned char *der, size_t len,
                   char *path_out, size_t pathlen, char *err, size_t errlen) {
    const ua_pki_group_t groups[] = {UA_PKI_TRUSTED, UA_PKI_ISSUERS};
    for (size_t gi = 0; gi < 2; gi++) {
        ua_cert_list_t cas;
        ua_pki_list(root, groups[gi], &cas);
        bool match = false;
        for (size_t i = 0; i < cas.n && !match; i++)
            match = cas.items[i].is_ca &&
                    ua_pki_crl_issued_by(der, len, cas.items[i].der, cas.items[i].len);
        ua_cert_list_free(&cas);
        if (match) {
            char tp[41];
            ua_pki_thumbprint(der, len, tp);
            snprintf(path_out, pathlen, "%s/%s/%s.crl", root, crl_dir(groups[gi]), tp);
            return ua_pki_write_atomic(path_out, der, len, 0644, err, errlen);
        }
    }
    snprintf(err, errlen, "the CRL was not issued by any CA in trusted/ or issuers/");
    return -1;
}

int ua_pki_check_key_mode(const char *path, const char *field, char *err,
                          size_t errlen) {
    struct stat st;
    if (stat(path, &st) != 0)
        return 0;
    unsigned mode = st.st_mode & 0777;
    if (mode & 077) {
        snprintf(err, errlen,
                 "%s '%s' is accessible by other users (mode %04o); restrict it "
                 "to the owner (chmod 600)",
                 field, path, mode);
        return -1;
    }
    return 0;
}

/* ---- application certificate ----------------------------------------- */

/* RFC 4514 escaping for the value of a name attribute. */
static void escape_dn(const char *in, char *out, size_t outlen) {
    size_t o = 0;
    for (const char *p = in; *p && o + 2 < outlen; p++) {
        if (strchr(",+\"\\<>;=", *p))
            out[o++] = '\\';
        out[o++] = *p;
    }
    out[o] = '\0';
}

int ua_pki_generate(const ua_cert_request_t *req, unsigned char **cert_der,
                    size_t *cert_len, char **key_pem, char *err,
                    size_t errlen) {
    int rc = -1;
    mbedtls_entropy_context entropy;
    mbedtls_ctr_drbg_context drbg;
    mbedtls_pk_context key;
    mbedtls_x509write_cert crt;
    mbedtls_pk_init(&key);
    mbedtls_x509write_crt_init(&crt);
    unsigned char *buf = NULL;
    mbedtls_x509_san_list *sans = NULL;
    size_t nsans = 0;
    *cert_der = NULL;
    *key_pem = NULL;

    if (drbg_init(&entropy, &drbg) != 0) {
        snprintf(err, errlen, "cannot seed the random number generator");
        goto out;
    }
    if (mbedtls_pk_setup(&key, mbedtls_pk_info_from_type(MBEDTLS_PK_RSA)) != 0 ||
        mbedtls_rsa_gen_key(mbedtls_pk_rsa(key), rng, &drbg, UA_PKI_KEY_BITS,
                            65537) != 0) {
        snprintf(err, errlen, "cannot generate an RSA key");
        goto out;
    }

    char name_esc[256], subject[600];
    escape_dn(req->application_name, name_esc, sizeof name_esc);
    snprintf(subject, sizeof subject, "CN=%s,O=%s", name_esc, name_esc);
    unsigned char serial[16];
    mbedtls_ctr_drbg_random(&drbg, serial, sizeof serial);
    serial[0] = (unsigned char)((serial[0] & 0x7f) | 0x01);

    time_t now = time(NULL);
    time_t until = now + (time_t)req->days * 86400;
    char nb[16], na[16];
    struct tm tm;
    gmtime_r(&now, &tm);
    strftime(nb, sizeof nb, "%Y%m%d%H%M%S", &tm);
    gmtime_r(&until, &tm);
    strftime(na, sizeof na, "%Y%m%d%H%M%S", &tm);

    /* Subject alternative names: the URI first, then host names/IPs. */
    char hostname[256] = "";
    const char *const *hosts = req->hostnames;
    size_t nhosts = req->nhostnames;
    const char *own_host[1];
    if (!hosts || nhosts == 0) {
        if (gethostname(hostname, sizeof hostname - 1) == 0 && *hostname) {
            own_host[0] = hostname;
            hosts = own_host;
            nhosts = 1;
        } else {
            nhosts = 0;
        }
    }
    sans = calloc(nhosts + 1, sizeof *sans);
    unsigned char (*ips)[16] = calloc(nhosts + 1, 16);
    if (!sans || !ips) {
        free(ips);
        snprintf(err, errlen, "out of memory");
        goto out;
    }
    sans[nsans].node.type = MBEDTLS_X509_SAN_UNIFORM_RESOURCE_IDENTIFIER;
    sans[nsans].node.san.unstructured_name.p = (unsigned char *)req->application_uri;
    sans[nsans].node.san.unstructured_name.len = strlen(req->application_uri);
    nsans++;
    for (size_t i = 0; i < nhosts; i++) {
        mbedtls_x509_subject_alternative_name *n = &sans[nsans].node;
        if (inet_pton(AF_INET, hosts[i], ips[i]) == 1) {
            n->type = MBEDTLS_X509_SAN_IP_ADDRESS;
            n->san.unstructured_name.p = ips[i];
            n->san.unstructured_name.len = 4;
        } else if (inet_pton(AF_INET6, hosts[i], ips[i]) == 1) {
            n->type = MBEDTLS_X509_SAN_IP_ADDRESS;
            n->san.unstructured_name.p = ips[i];
            n->san.unstructured_name.len = 16;
        } else {
            n->type = MBEDTLS_X509_SAN_DNS_NAME;
            n->san.unstructured_name.p = (unsigned char *)hosts[i];
            n->san.unstructured_name.len = strlen(hosts[i]);
        }
        nsans++;
    }
    /* mbedTLS encodes the list back to front: link it reversed, so the
     * certificate carries the URI first (where OPC UA stacks look for it). */
    for (size_t i = 1; i < nsans; i++)
        sans[i].next = &sans[i - 1];
    mbedtls_x509_san_list *san_head = &sans[nsans - 1];

    mbedtls_asn1_sequence eku_client = {
        .buf = {.tag = MBEDTLS_ASN1_OID,
                .len = MBEDTLS_OID_SIZE(MBEDTLS_OID_CLIENT_AUTH),
                .p = (unsigned char *)MBEDTLS_OID_CLIENT_AUTH},
        .next = NULL};
    mbedtls_asn1_sequence eku = {
        .buf = {.tag = MBEDTLS_ASN1_OID,
                .len = MBEDTLS_OID_SIZE(MBEDTLS_OID_SERVER_AUTH),
                .p = (unsigned char *)MBEDTLS_OID_SERVER_AUTH},
        .next = &eku_client};

    mbedtls_x509write_crt_set_version(&crt, MBEDTLS_X509_CRT_VERSION_3);
    mbedtls_x509write_crt_set_md_alg(&crt, MBEDTLS_MD_SHA256);
    mbedtls_x509write_crt_set_subject_key(&crt, &key);
    mbedtls_x509write_crt_set_issuer_key(&crt, &key);
    if (mbedtls_x509write_crt_set_subject_name(&crt, subject) != 0 ||
        mbedtls_x509write_crt_set_issuer_name(&crt, subject) != 0 ||
        mbedtls_x509write_crt_set_serial_raw(&crt, serial, sizeof serial) != 0 ||
        mbedtls_x509write_crt_set_validity(&crt, nb, na) != 0 ||
        mbedtls_x509write_crt_set_basic_constraints(&crt, 0, -1) != 0 ||
        mbedtls_x509write_crt_set_key_usage(
            &crt, MBEDTLS_X509_KU_DIGITAL_SIGNATURE | MBEDTLS_X509_KU_NON_REPUDIATION |
                      MBEDTLS_X509_KU_KEY_ENCIPHERMENT |
                      MBEDTLS_X509_KU_DATA_ENCIPHERMENT |
                      MBEDTLS_X509_KU_KEY_CERT_SIGN) != 0 ||
        mbedtls_x509write_crt_set_ext_key_usage(&crt, &eku) != 0 ||
        mbedtls_x509write_crt_set_subject_key_identifier(&crt) != 0 ||
        mbedtls_x509write_crt_set_authority_key_identifier(&crt) != 0 ||
        mbedtls_x509write_crt_set_subject_alternative_name(&crt, san_head) != 0) {
        free(ips);
        snprintf(err, errlen, "cannot build the certificate");
        goto out;
    }

    size_t cap = 8192;
    buf = malloc(cap);
    if (!buf) {
        free(ips);
        goto out;
    }
    int n = mbedtls_x509write_crt_der(&crt, buf, cap, rng, &drbg);
    free(ips);
    if (n <= 0) {
        snprintf(err, errlen, "cannot sign the certificate");
        goto out;
    }
    *cert_der = malloc((size_t)n);
    if (!*cert_der)
        goto out;
    memcpy(*cert_der, buf + cap - (size_t)n, (size_t)n);
    *cert_len = (size_t)n;

    if (mbedtls_pk_write_key_pem(&key, buf, cap) != 0) {
        snprintf(err, errlen, "cannot encode the private key");
        goto out;
    }
    *key_pem = strdup((char *)buf);
    rc = *key_pem ? 0 : -1;

out:
    if (rc != 0) {
        free(*cert_der);
        *cert_der = NULL;
    }
    if (buf) {
        mbedtls_platform_zeroize(buf, 8192);
        free(buf);
    }
    free(sans);
    mbedtls_x509write_crt_free(&crt);
    mbedtls_pk_free(&key);
    mbedtls_ctr_drbg_free(&drbg);
    mbedtls_entropy_free(&entropy);
    return rc;
}

static bool exists(const char *path) {
    struct stat st;
    return stat(path, &st) == 0;
}

int ua_pki_load_or_create_own(const char *root, const char *cert_path,
                              const char *key_path, bool explicit_paths,
                              bool create, const ua_cert_request_t *req,
                              UA_ByteString *cert, UA_ByteString *key,
                              bool *generated, char *err, size_t errlen) {
    UA_ByteString_init(cert);
    UA_ByteString_init(key);
    *generated = false;
    int lock = -1;
    if (!explicit_paths) {
        if (ua_pki_ensure_layout(root, err, errlen) != 0)
            return -1;
        lock = ua_pki_lock_own(root, err, errlen);
        if (lock < 0)
            return -1;
        /* The key is written first, so a key without a certificate is an
         * interrupted generation: set it aside and generate again rather than
         * fail every start (Pki::load_or_create_own, Rust). */
        if (create && exists(key_path) && !exists(cert_path)) {
            char aside[UA_PKI_PATH_MAX];
            snprintf(aside, sizeof aside, "%s.%lld", key_path, (long long)time(NULL));
            if (rename(key_path, aside) != 0) {
                snprintf(err, errlen, "cannot move %s aside: %s", key_path, strerror(errno));
                ua_pki_unlock(lock);
                return -1;
            }
            fprintf(stderr,
                    "warn  %s has no certificate (interrupted generation?); moved it to %s "
                    "and generating a new certificate\n",
                    key_path, aside);
        }
        if (exists(cert_path) && !exists(key_path)) {
            snprintf(err, errlen,
                     "%s has no private key at %s; restore the key or remove the "
                     "certificate to generate a new one",
                     cert_path, key_path);
            ua_pki_unlock(lock);
            return -1;
        }
        if (!exists(cert_path) && !exists(key_path)) {
            if (!create) {
                snprintf(err, errlen,
                         "no certificate at %s and create_certificate is false",
                         cert_path);
                ua_pki_unlock(lock);
                return -1;
            }
            unsigned char *der = NULL;
            size_t der_len = 0;
            char *pem = NULL;
            int rc = ua_pki_generate(req, &der, &der_len, &pem, err, errlen);
            /* Key first: a certificate without its key is never visible. */
            if (rc == 0)
                rc = ua_pki_write_atomic(key_path, (const unsigned char *)pem,
                                         strlen(pem), 0600, err, errlen);
            if (rc == 0)
                rc = ua_pki_write_atomic(cert_path, der, der_len, 0644, err, errlen);
            if (pem) {
                mbedtls_platform_zeroize(pem, strlen(pem));
                free(pem);
            }
            free(der);
            if (rc != 0) {
                ua_pki_unlock(lock);
                return -1;
            }
            *generated = true;
        }
    }

    int rc = -1;
    size_t len = 0;
    unsigned char *bytes = ua_pki_read_file(cert_path, &len);
    ua_blobs_t certs = {0};
    if (!bytes) {
        snprintf(err, errlen, "cannot read certificate %s: %s", cert_path,
                 strerror(errno));
        goto out;
    }
    if (!ua_pki_parse_certs(bytes, len, &certs, cert_path)) {
        snprintf(err, errlen, "%s is not a certificate", cert_path);
        goto out;
    }
    if (ua_pki_check_key_mode(key_path, "private_key", err, errlen) != 0)
        goto out;
    size_t key_len = 0;
    unsigned char *key_bytes = ua_pki_read_file(key_path, &key_len);
    if (!key_bytes) {
        snprintf(err, errlen, "cannot read private key %s", key_path);
        goto out;
    }
    if (!ua_pki_key_matches(certs.items[0].der, certs.items[0].len, key_bytes,
                            key_len)) {
        snprintf(err, errlen,
                 "private key %s does not belong to certificate %s", key_path,
                 cert_path);
        mbedtls_platform_zeroize(key_bytes, key_len);
        free(key_bytes);
        goto out;
    }
    ua_cert_info_t info;
    ua_pki_cert_info(certs.items[0].der, certs.items[0].len, &info);
    bool uri_ok = false;
    const char *shown = "none";
    for (size_t i = 0; i < info.nalt; i++) {
        if (!strcmp(info.alt_names[i], req->application_uri))
            uri_ok = true;
        struct in6_addr a6;
        struct in_addr a4;
        if (!strcmp(shown, "none") && strchr(info.alt_names[i], ':') &&
            inet_pton(AF_INET6, info.alt_names[i], &a6) != 1 &&
            inet_pton(AF_INET, info.alt_names[i], &a4) != 1)
            shown = info.alt_names[i];
    }
    if (!uri_ok) {
        snprintf(err, errlen,
                 "the certificate's application URI (%s) does not match "
                 "application_uri (%s)",
                 shown, req->application_uri);
        mbedtls_platform_zeroize(key_bytes, key_len);
        free(key_bytes);
        goto out;
    }
    /* open62541 parses the key with mbedTLS, which wants PEM NUL-terminated. */
    UA_ByteString_allocBuffer(key, key_len + 1);
    memcpy(key->data, key_bytes, key_len);
    key->data[key_len] = '\0';
    mbedtls_platform_zeroize(key_bytes, key_len);
    free(key_bytes);
    UA_ByteString_allocBuffer(cert, certs.items[0].len);
    memcpy(cert->data, certs.items[0].der, certs.items[0].len);
    rc = 0;

out:
    free(bytes);
    ua_blobs_free(&certs);
    ua_pki_unlock(lock);
    return rc;
}
