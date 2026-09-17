#!/bin/sh
# `tedge-dot pki` (C build) over genpki trees: the scenarios of the Rust CLI tests
# (impl/rust/tests/pki_cli.rs) -- exit codes, JSON fields, files on disk. Parity of the
# JSON with the Rust build is impl/c/ci/pki-parity.sh.
#   opcua_pki_cli.sh <tedge-dot binary> <repository root>
set -eu
bin="$1"
repo="$2"
py="${TEDGE_DOT_PYTHON:-}"
if [ -z "$py" ]; then
    if [ -x "$repo/.venv/bin/python" ]; then py="$repo/.venv/bin/python"; else py=python3; fi
fi
work=$(mktemp -d "${TMPDIR:-/tmp}/tdot-pki-cli.XXXXXX")
trap 'rm -rf "$work"' EXIT
work=$(cd "$work" && pwd -P)
v="$work/v"
"$py" "$repo/connectors/opcua/conformance/tools/genpki.py" "$v" >/dev/null
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
# expect <code> <name> -- args...: run, keep stdout in $work/out, check the exit code
expect() {
    want=$1; name=$2; shift 3
    rc=0
    "$bin" pki "$@" >"$work/out" 2>"$work/err" || rc=$?
    [ "$rc" = "$want" ] || fail "$name: exit $rc, want $want ($(cat "$work/err"))"
}
# field <python expression over `d`>: evaluate against the last JSON output
check() {
    "$py" -c "import json,sys; d=json.load(open('$work/out')); sys.exit(0 if ($2) else 1)" ||
        fail "$1: $(cat "$work/out")"
}
tp() { "$py" -c "import json; print(json.load(open('$v/expected.json'))['scenarios']['$1']['thumbprint'])"; }
copy() { mkdir -p "$work/$2"; cp -R "$v/scenarios/$1/pki" "$work/$2/pki"; echo "$work/$2/pki"; }

# list: CAs and CRLs
d=$(copy intermediate list)
expect 0 list -- list --pki-dir "$d" --json
check "list" "len(d['certificates']) == 2 and d['pki_dir'] == '$d'"
check "list trusted CA" "[c for c in d['certificates'] if c['group'] == 'trusted'][0]['crl'] is True"
check "list subject" "d['certificates'][0]['subject'] == 'CN=tedge-dot test root CA, O=tedge-dot test'"
d=$(copy ca_no_crl nocrl)
expect 0 list-human -- list trusted --pki-dir "$d"
grep -q "NO CRL" "$work/out" || fail "human list does not flag the missing CRL"

# trust by (upper-case) prefix, reject again
d=$(copy untrusted quarantine)
t=$(tp untrusted)
cp "$v/scenarios/untrusted/server/cert.der" "$d/rejected/certs/whatever.der"
expect 0 trust -- trust "$(printf '%s' "$t" | cut -c1-8 | tr a-f A-F)" --pki-dir "$d" --json
check "trust" "d['certificates'][0]['group'] == 'trusted' and d['certificates'][0]['file'].endswith('trusted/certs/sim_unknown_$t.der')"
[ -z "$(ls "$d/rejected/certs")" ] || fail "trust left the certificate in rejected/"
expect 0 reject -- reject "$t" --pki-dir "$d" --json
check "reject" "d['certificates'][0]['group'] == 'rejected'"
[ -z "$(ls "$d/trusted/certs")" ] || fail "reject left the certificate in trusted/"

check "reject no warning" "'warning' not in d"

# rejecting a certificate a trusted CA still vouches for warns
d=$(copy ca_issued vouched)
t=$(tp ca_issued)
expect 0 trust-leaf -- trust "$v/scenarios/ca_issued/server/cert.der" --pki-dir "$d" --json
expect 0 reject-vouched -- reject "$t" --pki-dir "$d" --json
check "reject warns" "d['certificates'][0]['group'] == 'rejected' and 'CRL' in d['warning']"
expect 0 reject-vouched-human -- trust "$t" --pki-dir "$d"
expect 0 reject-vouched-human -- reject "$t" --pki-dir "$d"
grep -q "^warning: " "$work/out" || fail "human reject does not print the warning"

# a bundle keeps its other certificates (Rust: removing_one_certificate_of_a_bundle_keeps_the_others)
d="$work/bundle/pki"
mkdir -p "$d/trusted/certs"
cat "$v/scenarios/pinned/server/cert.pem" "$v/scenarios/untrusted/server/cert.pem" \
    "$v/scenarios/pinned/server/cert.pem" >"$d/trusted/certs/site.pem"
expect 0 bundle-reject -- reject "$(tp pinned)" --pki-dir "$d" --json
cmp -s "$d/trusted/certs/site.pem" "$v/scenarios/untrusted/server/cert.pem" ||
    fail "reject did not keep the other certificate of the bundle"
expect 0 bundle-remove -- remove "$(tp untrusted)" --pki-dir "$d"
[ ! -e "$d/trusted/certs/site.pem" ] || fail "remove kept an empty bundle"

# run as root: new files go to the PKI directory's owner, symlink targets are left alone
if [ "$(id -u)" = 0 ]; then
    d="$work/owned/pki"
    mkdir -p "$d/trusted/certs"
    : >"$work/root-file"
    ln -s "$work/root-file" "$d/trusted/link"
    chown -h 4321:4321 "$d" "$d/trusted" "$d/trusted/certs"
    expect 0 owned-import -- trust "$v/scenarios/pinned/server/cert.pem" --pki-dir "$d"
    uid() { ls -lnd "$1" | awk '{print $3}'; }
    [ "$(uid "$(ls "$d"/trusted/certs/*.der)")" = 4321 ] || fail "imported certificate not handed to the owner"
    [ "$(uid "$d/trusted/link")" = 4321 ] || fail "symlink not handed to the owner"
    [ "$(uid "$work/root-file")" = 0 ] || fail "adopt_owner followed a symlink"
fi

# unknown and malformed thumbprints
expect 2 unknown -- trust 0123abcd --pki-dir "$d"
grep -q 0123abcd "$work/err" || fail "unknown thumbprint not named"
expect 1 short -- reject 0123 --pki-dir "$d"

# ambiguous matches
d=$(copy rejected_listed ambiguous)
t=$(tp rejected_listed)
expect 1 ambiguous -- remove "$(printf '%s' "$t" | cut -c1-10)" --pki-dir "$d"
grep -q -- "--group" "$work/err" || fail "ambiguity does not suggest --group"
expect 0 remove-group -- remove "$t" --group rejected --pki-dir "$d" --json
check "remove" "d['certificates'][0]['group'] == 'rejected'"
[ -z "$(ls "$d/rejected/certs")" ] || fail "remove kept the rejected copy"
[ -n "$(ls "$d/trusted/certs")" ] || fail "remove deleted the trusted copy"

# import a PEM file as DER
expect 0 import -- trust "$v/scenarios/pinned/server/cert.pem" --pki-dir "$work/import/pki" --json
check "import" "d['certificates'][0]['subject'] == 'CN=sim pinned, O=tedge-dot test'"
f=$("$py" -c "import json; print(json.load(open('$work/out'))['certificates'][0]['file'])")
cmp -s "$f" "$v/scenarios/pinned/server/cert.der" || fail "imported certificate is not the DER"

# issuers and CRLs
d=$(copy intermediate_no_crl crls)
expect 1 not-a-ca -- add-issuer "$v/client/cert.der" --pki-dir "$d"
crl=$(ls "$v"/scenarios/intermediate/pki/issuers/crl/* | head -1)
expect 0 add-crl -- add-crl "$crl" --pki-dir "$d" --json
check "add-crl" "'issuers/crl/' in d['crls'][0]['file']"
expect 1 crl-unknown-ca -- add-crl "$crl" --pki-dir "$work/empty/pki"
expect 0 add-issuer -- add-issuer "$v/ca/root.der" --pki-dir "$work/empty/pki" --json
check "add-issuer" "d['certificates'][0]['group'] == 'issuers' and d['certificates'][0]['ca']"

# create / show / export
mkdir -p "$work/own"
printf '[connector]\nprotocol = "opcua"\n\n[connection]\napplication_uri = "urn:gw01"\npki_dir = "pki"\n' >"$work/own/opcua.toml"
c="$work/own/opcua.toml"
expect 2 show-none -- show --config "$c"
expect 0 create -- create --hostname gw01.plant.local --hostname 10.1.2.3 --config "$c" --json
check "create" "d['created'] and d['application_uri'] == 'urn:gw01' and d['hostnames'] == ['gw01.plant.local', '10.1.2.3'] and d['uri_matches'] and d['certificate'] == '$work/own/pki/own/certs/cert.der'"
first=$("$py" -c "import json; print(json.load(open('$work/out'))['thumbprint'])")
mode=$(ls -l "$work/own/pki/own/private/key.pem" | cut -c1-10)
[ "$mode" = "-rw-------" ] || fail "key mode $mode"
expect 0 show -- show --config "$c" --json
check "show" "d['thumbprint'] == '$first' and d['configured_application_uri'] == 'urn:gw01'"
expect 1 create-again -- create --config "$c"
grep -q -- "--force" "$work/err" || fail "refusal does not mention --force"
expect 0 export-pem -- export --pem --output "$work/own/out.pem" --config "$c" --json
check "export" "d['thumbprint'] == '$first'"
grep -q "BEGIN CERTIFICATE" "$work/own/out.pem" || fail "export wrote no PEM"
if grep -q "PRIVATE KEY" "$work/own/out.pem"; then fail "export wrote key material"; fi
expect 0 export-der -- export --config "$c"
cmp -s "$work/out" "$work/own/pki/own/certs/cert.der" || fail "DER export differs"
expect 0 renew -- create --force --config "$c" --json
check "renew" "d['thumbprint'] != '$first'"
[ "$(ls "$work/own/pki/own/certs" | grep -c '^cert.der\.')" = 1 ] || fail "no backup of the old certificate"
printf '[connector]\nprotocol = "opcua"\n\n[connection]\napplication_uri = "urn:other"\npki_dir = "pki"\n' >"$work/own/other.toml"
expect 0 other-uri -- show --config "$work/own/other.toml" --json
check "uri mismatch" "d['uri_matches'] is False"

# usage errors
expect 1 bad-group -- list bogus-group
expect 1 bad-action -- frobnicate
expect 0 help -- --help
printf '[connector]\nprotocol = "modbus"\n' >"$work/modbus.toml"
expect 1 wrong-protocol -- list --config "$work/modbus.toml"

if [ "$failures" -ne 0 ]; then
    echo "$failures failure(s)"
    exit 1
fi
echo "opcua pki cli: all checks passed"
