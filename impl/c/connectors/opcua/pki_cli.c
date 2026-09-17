/* tedge-dot pki — the C build's `tedge-dot pki` (doc/connectors/opcua-connector-spec.md §9).
 *
 * The C rendering of impl/rust/crates/connector-opcua/src/pki_cli.rs: the same
 * actions, options, exit codes (0 ok, 1 usage/input error, 2 no such
 * certificate) and the same JSON fields, so impl/c/ci/pki-parity.sh can diff
 * the two builds' output.
 */
#include "pki_cli.h"

#include <errno.h>
#include <limits.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#include <arpa/inet.h>
#include <grp.h>

#include "cjson/cJSON.h"
#include "tedge_dot/config.h"
#include "ua_pki.h"

#define DEFAULT_CONFIG "/etc/tedge/plugins/ot/opcua.toml"
#define EXIT_OK 0
#define EXIT_ERROR 1
#define EXIT_NOT_FOUND 2

typedef struct {
    char application_name[256];
    char application_uri[256];
    char configured_uri[256];
    char pki_root[UA_PKI_PATH_MAX];
    char cert_path[UA_PKI_PATH_MAX];
    char key_path[UA_PKI_PATH_MAX];
    bool explicit_cert;
} ctx_t;

static int fail(int code, const char *fmt, ...) __attribute__((format(printf, 2, 3)));
static int fail(int code, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    fputs("error: ", stderr);
    vfprintf(stderr, fmt, ap);
    fputc('\n', stderr);
    va_end(ap);
    return code;
}

static void usage(void) {
    fputs(
        "tedge-dot pki — inspect and manage the OPC UA PKI directory\n"
        "\n"
        "USAGE:\n"
        "  tedge-dot pki <action> [--pki-dir <dir>] [-c|--config <file>] [--json]\n"
        "\n"
        "ACTIONS:\n"
        "  show                               the application instance certificate\n"
        "  export [--pem] [-o|--output <f>]   write the certificate (never the key)\n"
        "  create [--application-uri <uri>] [--hostname <name>]... [--days <n>] [--force]\n"
        "  list [trusted|issuers|rejected]    trusted, issuer and rejected certificates\n"
        "  trust <thumbprint|file>            trust a rejected certificate, or import one\n"
        "  reject <thumbprint>                move a trusted certificate to rejected/\n"
        "  remove <thumbprint> [--group <g>]  delete a certificate\n"
        "  add-issuer <file>                  import an intermediate CA certificate\n"
        "  add-crl <file>                     import a CRL next to its CA\n"
        "\n"
        "Default configuration: " DEFAULT_CONFIG " (when it exists).\n",
        stderr);
}

static void join(const char *base, const char *path, char *out, size_t outlen) {
    if (path[0] == '/' || !*base)
        snprintf(out, outlen, "%s", path);
    else
        snprintf(out, outlen, "%s/%s", base, path);
}

/* `dir` made absolute the way Rust's std::path::absolute does it: prefixed with
 * the working directory, symlinks left alone (so both builds print the same
 * paths). */
static void absolute_dir(const char *dir, char *out, size_t outlen) {
    char cwd[PATH_MAX];
    if (dir[0] == '/' || !getcwd(cwd, sizeof cwd))
        snprintf(out, outlen, "%s", dir);
    else if (!strcmp(dir, "."))
        snprintf(out, outlen, "%s", cwd);
    else
        snprintf(out, outlen, "%s/%s", cwd, dir[0] == '.' && dir[1] == '/' ? dir + 2 : dir);
}

static int string_in(toml_table_t *t, const char *key, char *out, size_t outlen) {
    if (!t)
        return 0;
    toml_datum_t d = toml_string_in(t, key);
    if (!d.ok)
        return 0;
    snprintf(out, outlen, "%s", d.u.s);
    free(d.u.s);
    return 1;
}

/* The connection settings and the PKI directory the options select. */
static int context(const char *pki_dir, const char *config, ctx_t *ctx) {
    memset(ctx, 0, sizeof *ctx);
    snprintf(ctx->application_name, sizeof ctx->application_name, "tedge-dot");
    snprintf(ctx->application_uri, sizeof ctx->application_uri, "urn:tedge-dot");
    char base[UA_PKI_PATH_MAX] = "";
    char pki[UA_PKI_PATH_MAX] = UA_PKI_DEFAULT_DIR;
    char cert[UA_PKI_PATH_MAX] = "", key[UA_PKI_PATH_MAX] = "";
    struct stat st;
    const char *path = config;
    if (!path && stat(DEFAULT_CONFIG, &st) == 0)
        path = DEFAULT_CONFIG;
    if (path) {
        char err[512];
        tdot_config_t *cfg = tdot_config_load(path, err, sizeof err);
        if (!cfg)
            return fail(EXIT_ERROR, "%s", err);
        if (strcmp(cfg->protocol, "opcua") != 0) {
            int rc = fail(EXIT_ERROR, "%s is a %s configuration, not an opcua one",
                          path, cfg->protocol);
            tdot_config_free(cfg);
            return rc;
        }
        string_in(cfg->connection, "application_name", ctx->application_name,
                  sizeof ctx->application_name);
        string_in(cfg->connection, "application_uri", ctx->application_uri,
                  sizeof ctx->application_uri);
        string_in(cfg->connection, "pki_dir", pki, sizeof pki);
        string_in(cfg->connection, "certificate", cert, sizeof cert);
        string_in(cfg->connection, "private_key", key, sizeof key);
        char dir[UA_PKI_PATH_MAX];
        snprintf(dir, sizeof dir, "%s", path);
        char *slash = strrchr(dir, '/');
        if (slash)
            *slash = '\0';
        else
            snprintf(dir, sizeof dir, ".");
        absolute_dir(dir, base, sizeof base);
        tdot_config_free(cfg);
    }
    if (pki_dir)
        snprintf(ctx->pki_root, sizeof ctx->pki_root, "%s", pki_dir);
    else
        join(base, pki, ctx->pki_root, sizeof ctx->pki_root);
    snprintf(ctx->configured_uri, sizeof ctx->configured_uri, "%s", ctx->application_uri);
    if (*cert && *key) {
        ctx->explicit_cert = true;
        join(base, cert, ctx->cert_path, sizeof ctx->cert_path);
        join(base, key, ctx->key_path, sizeof ctx->key_path);
    } else {
        snprintf(ctx->cert_path, sizeof ctx->cert_path, "%s/%s", ctx->pki_root,
                 UA_PKI_OWN_CERT);
        snprintf(ctx->key_path, sizeof ctx->key_path, "%s/%s", ctx->pki_root,
                 UA_PKI_OWN_KEY);
    }
    return EXIT_OK;
}

static void print_json(cJSON *obj) {
    char *text = cJSON_PrintUnformatted(obj);
    puts(text);
    free(text);
}

static cJSON *cert_json(const ua_cert_entry_t *e) {
    cJSON *o = cJSON_CreateObject();
    cJSON_AddStringToObject(o, "group", ua_pki_group_name(e->group));
    cJSON_AddStringToObject(o, "thumbprint", e->thumbprint);
    cJSON_AddStringToObject(o, "subject", e->subject);
    cJSON_AddStringToObject(o, "not_after", e->not_after);
    cJSON_AddBoolToObject(o, "ca", e->is_ca);
    if (e->has_crl < 0)
        cJSON_AddNullToObject(o, "crl");
    else
        cJSON_AddBoolToObject(o, "crl", e->has_crl);
    cJSON_AddStringToObject(o, "file", e->path);
    return o;
}

static bool is_ip(const char *s) {
    unsigned char buf[16];
    return inet_pton(AF_INET, s, buf) == 1 || inet_pton(AF_INET6, s, buf) == 1;
}

/* show/create output for the certificate at the context's paths. */
static int describe_own(const ctx_t *ctx, const unsigned char *der, size_t len,
                        const char *cert_path, const char *key_path, bool json,
                        bool created, const char *extra) {
    ua_cert_info_t info;
    if (ua_pki_cert_info(der, len, &info) != 0)
        return fail(EXIT_ERROR, "%s is not a certificate", cert_path);
    const char *uri = NULL;
    bool uri_matches = false;
    cJSON *hosts = cJSON_CreateArray();
    char host_text[1024] = "";
    for (size_t i = 0; i < info.nalt; i++) {
        const char *n = info.alt_names[i];
        if (!strcmp(n, ctx->configured_uri))
            uri_matches = true;
        if (strchr(n, ':') && !is_ip(n)) {
            if (!uri)
                uri = n;
        } else {
            cJSON_AddItemToArray(hosts, cJSON_CreateString(n));
            if (*host_text)
                strncat(host_text, ", ", sizeof host_text - strlen(host_text) - 1);
            strncat(host_text, n, sizeof host_text - strlen(host_text) - 1);
        }
    }
    if (json) {
        cJSON *o = cJSON_CreateObject();
        cJSON_AddStringToObject(o, "certificate", cert_path);
        cJSON_AddStringToObject(o, "private_key", key_path);
        cJSON_AddStringToObject(o, "subject", info.subject);
        cJSON_AddStringToObject(o, "issuer", info.issuer);
        cJSON_AddStringToObject(o, "thumbprint", info.thumbprint);
        cJSON_AddStringToObject(o, "not_before", info.not_before);
        cJSON_AddStringToObject(o, "not_after", info.not_after);
        if (uri)
            cJSON_AddStringToObject(o, "application_uri", uri);
        else
            cJSON_AddNullToObject(o, "application_uri");
        cJSON_AddItemToObject(o, "hostnames", hosts);
        cJSON_AddStringToObject(o, "configured_application_uri", ctx->configured_uri);
        cJSON_AddBoolToObject(o, "uri_matches", uri_matches);
        if (created)
            cJSON_AddBoolToObject(o, "created", true);
        print_json(o);
        cJSON_Delete(o);
        return EXIT_OK;
    }
    cJSON_Delete(hosts);
    if (created)
        printf("created the application certificate\n");
    printf("certificate:      %s\n"
           "private key:      %s\n"
           "subject:          %s\n"
           "thumbprint:       %s\n"
           "valid:            %s .. %s\n"
           "application URI:  %s%s%s%s\n"
           "host names:       %s\n",
           cert_path, key_path, info.subject, info.thumbprint, info.not_before,
           info.not_after, uri ? uri : "(none)",
           uri_matches ? "" : "  (does NOT match application_uri ",
           uri_matches ? "" : ctx->configured_uri, uri_matches ? "" : ")",
           *host_text ? host_text : "(none)");
    if (extra)
        fputs(extra, stdout);
    return EXIT_OK;
}

/* ---- acting as the PKI directory's owner --------------------------------
 *
 * Run as root on a PKI directory another user owns (the packaged one belongs
 * to tedge), the PKI work runs with that user's effective IDs and only its
 * group: root never touches files in a directory an unprivileged user could
 * rearrange with symlinks. Only what the administrator named (the file an
 * action reads, the file `export --output` writes) is accessed as root,
 * between invoker_begin() and invoker_end(). Mirrors privilege.rs (Rust). */
static struct {
    bool active;
    uid_t uid;
    gid_t gid, saved_egid;
    gid_t *saved_groups;
    int nsaved;
} owner;

static int to_owner(void) {
    if (setgroups(1, &owner.gid) != 0 || setegid(owner.gid) != 0 || seteuid(owner.uid) != 0)
        return -1;
    return 0;
}

static int to_root(void) {
    if (seteuid(0) != 0 || setegid(owner.saved_egid) != 0 ||
        setgroups(owner.nsaved, owner.saved_groups) != 0)
        return -1;
    return 0;
}

/* A half-switched identity is not safe to continue with. */
static void fatal_identity(void) {
    fprintf(stderr, "error: cannot switch identity: %s\n", strerror(errno));
    exit(EXIT_ERROR);
}

static int owner_enter(const char *root) {
    struct stat st;
    if (lstat(root, &st) != 0) /* not there yet: root creates it, owned by root */
        return EXIT_OK;
    /* A symbolic link would decide the identity by its target: the packaged
     * parent directory belongs to tedge, so that user could point the PKI
     * directory at a root-owned one and have everything run as root in it.
     * Refused whoever runs, so both builds answer alike (privilege.rs). */
    if (S_ISLNK(st.st_mode))
        return fail(EXIT_ERROR, "%s is a symbolic link; give the directory itself with --pki-dir",
                    root);
    if (geteuid() != 0 || st.st_uid == 0)
        return EXIT_OK;
    int n = getgroups(0, NULL);
    gid_t *groups = n > 0 ? malloc((size_t)n * sizeof *groups) : NULL;
    if (n < 0 || (n > 0 && (!groups || (n = getgroups(n, groups)) < 0))) {
        free(groups);
        return fail(EXIT_ERROR, "cannot read supplementary groups: %s", strerror(errno));
    }
    owner.uid = st.st_uid;
    owner.gid = st.st_gid;
    owner.saved_egid = getegid();
    owner.saved_groups = groups;
    owner.nsaved = n;
    if (to_owner() != 0) {
        int e = errno;
        if (to_root() != 0)
            fatal_identity();
        free(groups);
        return fail(EXIT_ERROR, "cannot act as the owner of %s: %s", root, strerror(e));
    }
    owner.active = true;
    return EXIT_OK;
}

static void owner_leave(void) {
    if (!owner.active)
        return;
    if (to_root() != 0)
        fatal_identity();
    owner.active = false;
    free(owner.saved_groups);
    owner.saved_groups = NULL;
}

static void invoker_begin(void) {
    if (owner.active && to_root() != 0)
        fatal_identity();
}

static void invoker_end(void) {
    if (owner.active && to_owner() != 0)
        fatal_identity();
}

/* A file the administrator named, read as the invoker. */
static unsigned char *read_input(const char *path, size_t *len) {
    invoker_begin();
    unsigned char *bytes = ua_pki_read_file(path, len);
    int e = errno;
    invoker_end();
    errno = e;
    return bytes;
}

static int read_own(const char *path, unsigned char **der, size_t *len) {
    size_t n = 0;
    unsigned char *bytes = ua_pki_read_file(path, &n);
    if (!bytes)
        return fail(EXIT_NOT_FOUND, "no application certificate at %s", path);
    ua_blobs_t certs = {0};
    ua_pki_parse_certs(bytes, n, &certs, path);
    free(bytes);
    if (certs.n == 0)
        return fail(EXIT_ERROR, "%s is not a certificate", path);
    *der = certs.items[0].der;
    *len = certs.items[0].len;
    certs.items[0].der = NULL;
    ua_blobs_free(&certs);
    return EXIT_OK;
}

static int cmd_show(const ctx_t *ctx, bool json) {
    unsigned char *der = NULL;
    size_t len = 0;
    int rc = read_own(ctx->cert_path, &der, &len);
    if (rc == EXIT_OK)
        rc = describe_own(ctx, der, len, ctx->cert_path, ctx->key_path, json, false, NULL);
    free(der);
    return rc;
}

static void base64(const unsigned char *in, size_t len, char *out) {
    static const char tab[] =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    size_t o = 0;
    for (size_t i = 0; i < len; i += 3) {
        unsigned v = (unsigned)in[i] << 16;
        if (i + 1 < len)
            v |= (unsigned)in[i + 1] << 8;
        if (i + 2 < len)
            v |= in[i + 2];
        out[o++] = tab[(v >> 18) & 63];
        out[o++] = tab[(v >> 12) & 63];
        out[o++] = i + 1 < len ? tab[(v >> 6) & 63] : '=';
        out[o++] = i + 2 < len ? tab[v & 63] : '=';
    }
    out[o] = '\0';
}

static char *to_pem(const unsigned char *der, size_t len) {
    char *b64 = malloc(len * 4 / 3 + 8);
    if (!b64)
        return NULL;
    base64(der, len, b64);
    size_t n = strlen(b64);
    char *pem = malloc(n + n / 64 + 64);
    if (!pem) {
        free(b64);
        return NULL;
    }
    char *p = pem + sprintf(pem, "-----BEGIN CERTIFICATE-----\n");
    for (size_t i = 0; i < n; i += 64)
        p += sprintf(p, "%.64s\n", b64 + i);
    sprintf(p, "-----END CERTIFICATE-----\n");
    free(b64);
    return pem;
}

static int cmd_export(const ctx_t *ctx, bool pem, const char *output, bool json) {
    unsigned char *der = NULL;
    size_t len = 0;
    int rc = read_own(ctx->cert_path, &der, &len);
    if (rc != EXIT_OK)
        return rc;
    char tp[41];
    ua_pki_thumbprint(der, len, tp);
    char *pem_text = to_pem(der, len);
    const char *format = pem ? "pem" : "der";
    if (output) {
        char err[512];
        const unsigned char *data = pem ? (const unsigned char *)pem_text : der;
        size_t n = pem ? strlen(pem_text) : len;
        invoker_begin();
        int wrc = ua_pki_write_atomic(output, data, n, 0644, err, sizeof err);
        invoker_end();
        if (wrc != 0)
            rc = fail(EXIT_ERROR, "%s", err);
        else if (json) {
            cJSON *o = cJSON_CreateObject();
            cJSON_AddStringToObject(o, "thumbprint", tp);
            cJSON_AddStringToObject(o, "format", format);
            cJSON_AddStringToObject(o, "file", output);
            print_json(o);
            cJSON_Delete(o);
        } else {
            printf("wrote %s (%s) to %s\n", tp, format, output);
        }
    } else if (json) {
        cJSON *o = cJSON_CreateObject();
        cJSON_AddStringToObject(o, "thumbprint", tp);
        cJSON_AddStringToObject(o, "format", "pem");
        cJSON_AddStringToObject(o, "pem", pem_text);
        print_json(o);
        cJSON_Delete(o);
    } else if (pem) {
        fputs(pem_text, stdout);
    } else {
        fwrite(der, 1, len, stdout);
    }
    free(pem_text);
    free(der);
    return rc;
}

static bool exists(const char *path) {
    struct stat st;
    return stat(path, &st) == 0;
}

static int cmd_create(const ctx_t *ctx, const char *uri, const char **hosts,
                      size_t nhosts, int days, bool force, bool json) {
    char err[512];
    char cert[UA_PKI_PATH_MAX], key[UA_PKI_PATH_MAX];
    snprintf(cert, sizeof cert, "%s/%s", ctx->pki_root, UA_PKI_OWN_CERT);
    snprintf(key, sizeof key, "%s/%s", ctx->pki_root, UA_PKI_OWN_KEY);
    if (ua_pki_ensure_layout(ctx->pki_root, err, sizeof err) != 0)
        return fail(EXIT_ERROR, "%s", err);
    int lock = ua_pki_lock_own(ctx->pki_root, err, sizeof err);
    if (lock < 0)
        return fail(EXIT_ERROR, "%s", err);
    int rc = EXIT_OK;
    if (exists(cert) || exists(key)) {
        if (!force) {
            ua_pki_unlock(lock);
            return fail(EXIT_ERROR,
                        "an application certificate already exists at %s (use "
                        "--force to replace it)",
                        cert);
        }
        long stamp = (long)time(NULL);
        const char *paths[2] = {cert, key};
        for (int i = 0; i < 2; i++) {
            if (!exists(paths[i]))
                continue;
            char backup[UA_PKI_PATH_MAX + 32];
            snprintf(backup, sizeof backup, "%s.%ld", paths[i], stamp);
            if (rename(paths[i], backup) != 0) {
                ua_pki_unlock(lock);
                return fail(EXIT_ERROR, "cannot move %s aside: %s", paths[i],
                            strerror(errno));
            }
        }
    }
    ua_cert_request_t req = {
        .application_name = ctx->application_name,
        .application_uri = uri ? uri : ctx->application_uri,
        .hostnames = hosts,
        .nhostnames = nhosts,
        .days = days,
    };
    unsigned char *der = NULL;
    size_t len = 0;
    char *pem = NULL;
    if (ua_pki_generate(&req, &der, &len, &pem, err, sizeof err) != 0 ||
        ua_pki_write_atomic(key, (const unsigned char *)pem, strlen(pem), 0600, err,
                            sizeof err) != 0 ||
        ua_pki_write_atomic(cert, der, len, 0644, err, sizeof err) != 0)
        rc = fail(EXIT_ERROR, "%s", err);
    ua_pki_unlock(lock);
    if (rc == EXIT_OK) {
        ctx_t shown = *ctx;
        snprintf(shown.configured_uri, sizeof shown.configured_uri, "%s",
                 req.application_uri);
        char extra[512] = "";
        if (ctx->explicit_cert)
            strcat(extra, "note: the configuration names its own certificate/private_key, "
                          "so connectors do not use this one\n");
        if (force)
            strcat(extra, "reload (SIGHUP) or restart the connector to use the new certificate\n");
        rc = describe_own(&shown, der, len, cert, key, json, true, extra);
    }
    if (pem) {
        memset(pem, 0, strlen(pem));
        free(pem);
    }
    free(der);
    return rc;
}

static int cmd_list(const ctx_t *ctx, const char *group, bool json) {
    ua_pki_group_t groups[3] = {UA_PKI_TRUSTED, UA_PKI_ISSUERS, UA_PKI_REJECTED};
    size_t ngroups = 3;
    if (group) {
        if (ua_pki_group_parse(group, &groups[0]) != 0)
            return fail(EXIT_ERROR, "unknown group '%s' (trusted, issuers or rejected)", group);
        ngroups = 1;
    }
    cJSON *arr = cJSON_CreateArray();
    if (!json)
        printf("PKI directory %s\n", ctx->pki_root);
    size_t total = 0;
    for (size_t g = 0; g < ngroups; g++) {
        ua_cert_list_t list;
        ua_pki_list(ctx->pki_root, groups[g], &list);
        for (size_t i = 0; i < list.n; i++) {
            const ua_cert_entry_t *e = &list.items[i];
            total++;
            if (json) {
                cJSON_AddItemToArray(arr, cert_json(e));
                continue;
            }
            const char *kind = !e->is_ca        ? "certificate"
                               : e->has_crl == 1 ? "CA, CRL present"
                                                 : "CA, NO CRL (certificates it issued are refused)";
            printf("%-9s %s  %s  (expires %s; %s)\n", ua_pki_group_name(e->group),
                   e->thumbprint, e->subject, e->not_after, kind);
        }
        ua_cert_list_free(&list);
    }
    if (json) {
        cJSON *o = cJSON_CreateObject();
        cJSON_AddStringToObject(o, "pki_dir", ctx->pki_root);
        cJSON_AddItemToObject(o, "certificates", arr);
        print_json(o);
        cJSON_Delete(o);
    } else {
        cJSON_Delete(arr);
        if (!total)
            printf("(no certificates)\n");
    }
    return EXIT_OK;
}

/* The single certificate in `groups` whose thumbprint starts with `prefix`. */
static int find(const ctx_t *ctx, const char *prefix, const ua_pki_group_t *groups,
                size_t ngroups, ua_cert_entry_t *out) {
    size_t plen = strlen(prefix);
    char lower[64];
    bool hex = plen >= 8 && plen < sizeof lower;
    for (size_t i = 0; hex && i < plen; i++) {
        char c = prefix[i];
        if (c >= 'A' && c <= 'F')
            c = (char)(c - 'A' + 'a');
        hex = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
        lower[i] = c;
    }
    if (!hex)
        return fail(EXIT_ERROR,
                    "'%s' is not a thumbprint (at least 8 hexadecimal digits)", prefix);
    lower[plen] = '\0';

    ua_cert_entry_t matches[16];
    size_t n = 0;
    char names[64] = "";
    for (size_t g = 0; g < ngroups; g++) {
        if (g)
            strcat(names, ", ");
        strcat(names, ua_pki_group_name(groups[g]));
        ua_cert_list_t list;
        ua_pki_list(ctx->pki_root, groups[g], &list);
        for (size_t i = 0; i < list.n; i++) {
            ua_cert_entry_t *e = &list.items[i];
            if (strncmp(e->thumbprint, lower, plen) != 0)
                continue;
            bool dup = false;
            for (size_t k = 0; k < n; k++)
                if (matches[k].group == e->group && matches[k].len == e->len &&
                    !memcmp(matches[k].der, e->der, e->len))
                    dup = true;
            if (dup || n == 16)
                continue;
            matches[n] = *e;
            e->der = NULL; /* moved */
            n++;
        }
        ua_cert_list_free(&list);
    }
    if (n == 0)
        return fail(EXIT_NOT_FOUND, "no certificate with thumbprint %s in %s", prefix, names);
    if (n > 1) {
        fprintf(stderr, "error: thumbprint %s matches %zu certificates (give more digits%s):\n",
                prefix, n, ngroups > 1 ? " or --group" : "");
        for (size_t k = 0; k < n; k++) {
            fprintf(stderr, "  %s %s %s\n", ua_pki_group_name(matches[k].group),
                    matches[k].thumbprint, matches[k].subject);
            free(matches[k].der);
        }
        return EXIT_ERROR;
    }
    *out = matches[0];
    return EXIT_OK;
}

static int finish_action_warn(const char *action, cJSON *certs, const char *text,
                              const char *warning, bool json) {
    if (json) {
        cJSON *o = cJSON_CreateObject();
        cJSON_AddStringToObject(o, "action", action);
        cJSON_AddItemToObject(o, "certificates", certs);
        if (warning)
            cJSON_AddStringToObject(o, "warning", warning);
        print_json(o);
        cJSON_Delete(o);
    } else {
        cJSON_Delete(certs);
        fputs(text, stdout);
        if (warning)
            printf("warning: %s\n", warning);
    }
    return EXIT_OK;
}

static int finish_action(const char *action, cJSON *certs, const char *text, bool json) {
    return finish_action_warn(action, certs, text, NULL, json);
}

/* Move an entry to another group (every copy leaves its old group). */
static int relocate(const ctx_t *ctx, ua_cert_entry_t *e, ua_pki_group_t to,
                    cJSON *certs, char *text, size_t textlen) {
    char err[512], path[UA_PKI_PATH_MAX];
    if (ua_pki_store(ctx->pki_root, to, e->der, e->len, path, sizeof path, err,
                     sizeof err) != 0 ||
        ua_pki_remove(ctx->pki_root, e, err, sizeof err) != 0)
        return fail(EXIT_ERROR, "%s", err);
    size_t used = strlen(text);
    snprintf(text + used, textlen - used, "%s %s -> %s\n", e->thumbprint, e->subject,
             ua_pki_group_name(to));
    e->group = to;
    snprintf(e->path, sizeof e->path, "%s", path);
    cJSON_AddItemToArray(certs, cert_json(e));
    return EXIT_OK;
}

static int cmd_move(const ctx_t *ctx, const char *action, const char *thumbprint,
                    ua_pki_group_t from, ua_pki_group_t to, bool json) {
    ua_cert_entry_t e;
    int rc = find(ctx, thumbprint, &from, 1, &e);
    if (rc != EXIT_OK)
        return rc;
    cJSON *certs = cJSON_CreateArray();
    char text[1024] = "";
    unsigned char *der = e.der;
    size_t len = e.len;
    rc = relocate(ctx, &e, to, certs, text, sizeof text);
    /* rejected/ does not override trust: say so when a CA still vouches for it. */
    const char *warning = NULL;
    if (rc == EXIT_OK && to == UA_PKI_REJECTED) {
        ua_trust_t trust;
        ua_pki_load_trust(ctx->pki_root, &trust);
        if (ua_trust_verify(&trust, der, len, time(NULL)) == UA_STATUSCODE_GOOD)
            warning = "the certificate is still trusted through a CA in trusted/; "
                      "revoke it in that CA's CRL to stop trusting it";
        ua_trust_free(&trust);
    }
    free(der);
    if (rc != EXIT_OK) {
        cJSON_Delete(certs);
        return rc;
    }
    return finish_action_warn(action, certs, text, warning, json);
}

/* Import every certificate of a file into `group`; `require_ca` for issuers. */
static int import(const ctx_t *ctx, const char *action, const char *file,
                  ua_pki_group_t group, bool require_ca, bool json) {
    size_t len = 0;
    unsigned char *bytes = read_input(file, &len);
    if (!bytes)
        return fail(EXIT_ERROR, "cannot read %s: %s", file, strerror(errno));
    ua_blobs_t certs = {0};
    ua_pki_parse_certs(bytes, len, &certs, file);
    free(bytes);
    if (certs.n == 0)
        return fail(EXIT_ERROR, "%s holds no certificate", file);
    int rc = EXIT_OK;
    for (size_t i = 0; require_ca && i < certs.n && rc == EXIT_OK; i++) {
        ua_cert_info_t info;
        if (ua_pki_cert_info(certs.items[i].der, certs.items[i].len, &info) == 0 &&
            !info.is_ca)
            rc = fail(EXIT_ERROR, "%s is not a CA certificate", info.subject);
    }
    char err[512];
    if (rc == EXIT_OK && ua_pki_ensure_layout(ctx->pki_root, err, sizeof err) != 0)
        rc = fail(EXIT_ERROR, "%s", err);
    cJSON *out = cJSON_CreateArray();
    char text[4096] = "";
    for (size_t i = 0; rc == EXIT_OK && i < certs.n; i++) {
        if (group == UA_PKI_TRUSTED) {
            /* leaving rejected/ */
            ua_cert_list_t rejected;
            ua_pki_list(ctx->pki_root, UA_PKI_REJECTED, &rejected);
            for (size_t k = 0; k < rejected.n; k++)
                if (rejected.items[k].len == certs.items[i].len &&
                    !memcmp(rejected.items[k].der, certs.items[i].der, certs.items[i].len))
                    ua_pki_remove(ctx->pki_root, &rejected.items[k], err, sizeof err);
            ua_cert_list_free(&rejected);
        }
        char path[UA_PKI_PATH_MAX];
        if (ua_pki_store(ctx->pki_root, group, certs.items[i].der, certs.items[i].len,
                         path, sizeof path, err, sizeof err) != 0) {
            rc = fail(EXIT_ERROR, "%s", err);
            break;
        }
        ua_cert_list_t list;
        ua_pki_list(ctx->pki_root, group, &list);
        for (size_t k = 0; k < list.n; k++) {
            if (strcmp(list.items[k].path, path) != 0)
                continue;
            cJSON_AddItemToArray(out, cert_json(&list.items[k]));
            size_t used = strlen(text);
            snprintf(text + used, sizeof text - used, "%s %s -> %s\n",
                     list.items[k].thumbprint, list.items[k].subject,
                     ua_pki_group_name(group));
        }
        ua_cert_list_free(&list);
    }
    ua_blobs_free(&certs);
    if (rc != EXIT_OK) {
        cJSON_Delete(out);
        return rc;
    }
    return finish_action(action, out, text, json);
}

static int cmd_trust(const ctx_t *ctx, const char *target, bool json) {
    struct stat st;
    invoker_begin();
    bool is_file = stat(target, &st) == 0 && S_ISREG(st.st_mode);
    invoker_end();
    if (is_file)
        return import(ctx, "trust", target, UA_PKI_TRUSTED, false, json);
    return cmd_move(ctx, "trust", target, UA_PKI_REJECTED, UA_PKI_TRUSTED, json);
}

static int cmd_remove(const ctx_t *ctx, const char *thumbprint, const char *group,
                      bool json) {
    ua_pki_group_t groups[3] = {UA_PKI_TRUSTED, UA_PKI_ISSUERS, UA_PKI_REJECTED};
    size_t n = 3;
    if (group) {
        if (ua_pki_group_parse(group, &groups[0]) != 0)
            return fail(EXIT_ERROR, "unknown group '%s' (trusted, issuers or rejected)", group);
        n = 1;
    }
    ua_cert_entry_t e;
    int rc = find(ctx, thumbprint, groups, n, &e);
    if (rc != EXIT_OK)
        return rc;
    char err[512];
    if (ua_pki_remove(ctx->pki_root, &e, err, sizeof err) != 0) {
        free(e.der);
        return fail(EXIT_ERROR, "%s", err);
    }
    cJSON *certs = cJSON_CreateArray();
    cJSON_AddItemToArray(certs, cert_json(&e));
    char text[256];
    snprintf(text, sizeof text, "removed %s from %s\n", e.thumbprint,
             ua_pki_group_name(e.group));
    free(e.der);
    return finish_action("remove", certs, text, json);
}

static int cmd_add_crl(const ctx_t *ctx, const char *file, bool json) {
    size_t len = 0;
    unsigned char *bytes = read_input(file, &len);
    if (!bytes)
        return fail(EXIT_ERROR, "cannot read %s: %s", file, strerror(errno));
    ua_blobs_t crls = {0};
    ua_pki_parse_crls(bytes, len, &crls, file);
    free(bytes);
    if (crls.n == 0)
        return fail(EXIT_ERROR, "%s holds no CRL", file);
    cJSON *arr = cJSON_CreateArray();
    char text[2048] = "";
    int rc = EXIT_OK;
    for (size_t i = 0; i < crls.n; i++) {
        char path[UA_PKI_PATH_MAX], err[512];
        if (ua_pki_add_crl(ctx->pki_root, crls.items[i].der, crls.items[i].len, path,
                           sizeof path, err, sizeof err) != 0) {
            rc = fail(EXIT_ERROR, "%s", err);
            break;
        }
        cJSON *o = cJSON_CreateObject();
        cJSON_AddStringToObject(o, "file", path);
        cJSON_AddItemToArray(arr, o);
        size_t used = strlen(text);
        snprintf(text + used, sizeof text - used, "CRL -> %s\n", path);
    }
    ua_blobs_free(&crls);
    if (rc != EXIT_OK) {
        cJSON_Delete(arr);
        return rc;
    }
    if (json) {
        cJSON *o = cJSON_CreateObject();
        cJSON_AddStringToObject(o, "action", "add-crl");
        cJSON_AddItemToObject(o, "crls", arr);
        print_json(o);
        cJSON_Delete(o);
    } else {
        cJSON_Delete(arr);
        fputs(text, stdout);
    }
    return EXIT_OK;
}

int tdot_opcua_pki_main(int argc, char **argv) {
    /* argv[0] is "pki" */
    const char *action = NULL, *pki_dir = NULL, *config = NULL, *output = NULL;
    const char *uri = NULL, *group = NULL;
    const char *positional[4];
    size_t npos = 0;
    const char *hosts[16];
    size_t nhosts = 0;
    int days = UA_PKI_DEFAULT_DAYS;
    bool json = false, pem = false, force = false;
    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];
#define VALUE(dst)                                                             \
    do {                                                                       \
        if (i + 1 >= argc)                                                     \
            return fail(EXIT_ERROR, "%s needs a value", a);                    \
        dst = argv[++i];                                                       \
    } while (0)
        if (!strcmp(a, "-h") || !strcmp(a, "--help")) {
            usage();
            return EXIT_OK;
        } else if (!strcmp(a, "--json"))
            json = true;
        else if (!strcmp(a, "--pem"))
            pem = true;
        else if (!strcmp(a, "--force"))
            force = true;
        else if (!strcmp(a, "--pki-dir"))
            VALUE(pki_dir);
        else if (!strcmp(a, "-c") || !strcmp(a, "--config"))
            VALUE(config);
        else if (!strcmp(a, "-o") || !strcmp(a, "--output"))
            VALUE(output);
        else if (!strcmp(a, "--application-uri"))
            VALUE(uri);
        else if (!strcmp(a, "--group"))
            VALUE(group);
        else if (!strcmp(a, "--hostname")) {
            if (nhosts == 16)
                return fail(EXIT_ERROR, "too many --hostname values");
            VALUE(hosts[nhosts]);
            nhosts++;
        } else if (!strcmp(a, "--days")) {
            const char *v;
            VALUE(v);
            char *end;
            long d = strtol(v, &end, 10);
            if (*end || d <= 0 || d > 100000)
                return fail(EXIT_ERROR, "--days needs a positive number of days");
            days = (int)d;
        } else if (a[0] == '-') {
            usage();
            return fail(EXIT_ERROR, "unknown option %s", a);
        } else if (!action)
            action = a;
        else if (npos < 4)
            positional[npos++] = a;
        else
            return fail(EXIT_ERROR, "unexpected argument %s", a);
#undef VALUE
    }
    if (!action) {
        usage();
        return EXIT_ERROR;
    }

    /* Validate the action and its arguments before touching anything. */
    typedef struct {
        const char *name;
        size_t min, max;
    } spec_t;
    static const spec_t specs[] = {
        {"show", 0, 0},  {"export", 0, 0},     {"create", 0, 0},
        {"list", 0, 1},  {"trust", 1, 1},      {"reject", 1, 1},
        {"remove", 1, 1}, {"add-issuer", 1, 1}, {"add-crl", 1, 1},
    };
    const spec_t *spec = NULL;
    for (size_t i = 0; i < sizeof specs / sizeof specs[0]; i++)
        if (!strcmp(specs[i].name, action))
            spec = &specs[i];
    if (!spec) {
        usage();
        return fail(EXIT_ERROR, "unknown action '%s'", action);
    }
    if (npos < spec->min || npos > spec->max)
        return fail(EXIT_ERROR, "%s takes %zu argument(s)", action, spec->max);
    ua_pki_group_t g;
    if (!strcmp(action, "list") && npos == 1 && ua_pki_group_parse(positional[0], &g) != 0)
        return fail(EXIT_ERROR, "unknown group '%s' (trusted, issuers or rejected)",
                    positional[0]);

    ctx_t ctx;
    int rc = context(pki_dir, config, &ctx);
    if (rc != EXIT_OK)
        return rc;
    rc = owner_enter(ctx.pki_root);
    if (rc != EXIT_OK)
        return rc;

    if (!strcmp(action, "show"))
        rc = cmd_show(&ctx, json);
    else if (!strcmp(action, "export"))
        rc = cmd_export(&ctx, pem, output, json);
    else if (!strcmp(action, "create"))
        rc = cmd_create(&ctx, uri, hosts, nhosts, days, force, json);
    else if (!strcmp(action, "list"))
        rc = cmd_list(&ctx, npos ? positional[0] : NULL, json);
    else if (!strcmp(action, "trust"))
        rc = cmd_trust(&ctx, positional[0], json);
    else if (!strcmp(action, "reject"))
        rc = cmd_move(&ctx, "reject", positional[0], UA_PKI_TRUSTED, UA_PKI_REJECTED, json);
    else if (!strcmp(action, "remove"))
        rc = cmd_remove(&ctx, positional[0], group, json);
    else if (!strcmp(action, "add-issuer"))
        rc = import(&ctx, "add-issuer", positional[0], UA_PKI_ISSUERS, true, json);
    else
        rc = cmd_add_crl(&ctx, positional[0], json);
    owner_leave();
    return rc;
}
