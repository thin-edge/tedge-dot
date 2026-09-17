#!/usr/bin/env bash
# `tedge-dot pki` parity check: the Rust and C binaries must answer the same
# command sequence with the same exit codes and the same JSON, on identical
# PKI trees (doc/connectors/opcua-connector-spec.md §9).
#
#   impl/c/ci/pki-parity.sh
#
# Expects impl/c/build/tedge-dot and a Rust binary (impl/rust/target/{debug,release}/tedge-dot,
# or $RUST_BIN; built with cargo when neither exists), and a Python with `cryptography` for
# genpki.py ($TEDGE_DOT_PYTHON, the repository's .venv, or python3).
#
# Documents are compared parsed (serde_json sorts keys, cJSON keeps insertion order), with
# each build's scratch directory replaced by a placeholder. For certificates the two builds
# GENERATE (`create`, `show` after it) only what does not depend on the key and the clock is
# compared: file paths, application URI, host names, flags.
set -euo pipefail

repo=$(cd "$(dirname "$0")/../../.." && pwd)
c_bin="$repo/impl/c/build/tedge-dot"
rust_bin=${RUST_BIN:-}
py=${TEDGE_DOT_PYTHON:-}
if [ -z "$py" ]; then
    if [ -x "$repo/.venv/bin/python" ]; then py="$repo/.venv/bin/python"; else py=python3; fi
fi

if [ ! -x "$c_bin" ]; then
    echo "FAIL: $c_bin not built (cmake -B impl/c/build -S impl/c && cmake --build impl/c/build)" >&2
    exit 2
fi
if [ -z "$rust_bin" ]; then
    for candidate in "$repo/impl/rust/target/debug/tedge-dot" "$repo/impl/rust/target/release/tedge-dot"; do
        if [ -x "$candidate" ]; then
            rust_bin=$candidate
            break
        fi
    done
fi
if [ -z "$rust_bin" ]; then
    echo "== building the Rust binary (no impl/rust/target/{debug,release}/tedge-dot found)"
    (cd "$repo" && cargo build --quiet --manifest-path impl/rust/Cargo.toml)
    rust_bin="$repo/impl/rust/target/debug/tedge-dot"
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/tdot-pki-parity.XXXXXX")
trap 'rm -rf "$work"' EXIT
# Canonical (no "//", no symlinks), so both builds print the paths the comparison replaces.
work=$(cd "$work" && pwd -P)
"$py" "$repo/connectors/opcua/conformance/tools/genpki.py" "$work/vectors" >/dev/null
tp() { "$py" -c 'import json,sys; print(json.load(open(sys.argv[1]))["scenarios"][sys.argv[2]]["thumbprint"])' "$work/vectors/expected.json" "$1"; }
untrusted_tp=$(tp untrusted)
listed_tp=$(tp rejected_listed)
ca_issued_tp=$(tp ca_issued)
pinned_tp=$(tp pinned)
crl_file=$(ls "$work"/vectors/scenarios/intermediate/pki/issuers/crl/* | head -1)

# Run the whole sequence with one binary in its own copy of the trees; every step
# appends "<name>\t<exit code>\t<stdout>" to $out.
run_sequence() {
    local bin=$1 dir=$2 out=$3
    mkdir -p "$dir"
    for s in intermediate ca_no_crl untrusted rejected_listed intermediate_no_crl ca_issued; do
        cp -R "$work/vectors/scenarios/$s/pki" "$dir/$s"
    done
    cp "$work/vectors/scenarios/untrusted/server/cert.der" "$dir/untrusted/rejected/certs/quarantined.der"
    mkdir -p "$dir/bundle/trusted/certs"
    cat "$work/vectors/scenarios/pinned/server/cert.pem" "$work/vectors/scenarios/untrusted/server/cert.pem" \
        "$work/vectors/scenarios/pinned/server/cert.pem" >"$dir/bundle/trusted/certs/site.pem"
    printf '[connector]\nprotocol = "opcua"\n\n[connection]\napplication_uri = "urn:gw01"\npki_dir = "own-pki"\n' >"$dir/opcua.toml"
    printf '[connector]\nprotocol = "modbus"\n' >"$dir/modbus.toml"
    : >"$out"
    step() {
        local name=$1
        shift
        local stdout rc=0
        stdout=$("$bin" pki "$@" 2>/dev/null) || rc=$?
        printf '%s\t%s\t%s\n' "$name" "$rc" "$(printf '%s' "$stdout" | tr '\n' ' ')" >>"$out"
    }
    step list-intermediate list --pki-dir "$dir/intermediate" --json
    step list-trusted-no-crl list trusted --pki-dir "$dir/ca_no_crl" --json
    step trust-prefix trust "$(printf '%s' "${untrusted_tp:0:8}" | tr a-f A-F)" --pki-dir "$dir/untrusted" --json
    step reject reject "$untrusted_tp" --pki-dir "$dir/untrusted" --json
    step trust-unknown trust 0123abcd --pki-dir "$dir/untrusted"
    step trust-short trust 0123 --pki-dir "$dir/untrusted"
    step remove-ambiguous remove "${listed_tp:0:10}" --pki-dir "$dir/rejected_listed"
    step remove-group remove "$listed_tp" --group rejected --pki-dir "$dir/rejected_listed" --json
    step trust-leaf trust "$work/vectors/scenarios/ca_issued/server/cert.der" --pki-dir "$dir/ca_issued" --json
    step reject-vouched reject "$ca_issued_tp" --pki-dir "$dir/ca_issued" --json
    step bundle-reject reject "$pinned_tp" --pki-dir "$dir/bundle" --json
    step bundle-list list --pki-dir "$dir/bundle" --json
    step trust-file trust "$work/vectors/scenarios/intermediate/server/cert.pem" --pki-dir "$dir/imported" --json
    step add-issuer-not-ca add-issuer "$work/vectors/client/cert.der" --pki-dir "$dir/intermediate_no_crl"
    step add-crl add-crl "$crl_file" --pki-dir "$dir/intermediate_no_crl" --json
    step add-crl-unknown-ca add-crl "$crl_file" --pki-dir "$dir/empty"
    step add-issuer add-issuer "$work/vectors/ca/root.der" --pki-dir "$dir/empty" --json
    step show-none show --config "$dir/opcua.toml"
    step create create --hostname gw01.plant.local --hostname 10.1.2.3 --config "$dir/opcua.toml" --json
    step create-again create --config "$dir/opcua.toml"
    step show show --config "$dir/opcua.toml" --json
    step export-json export --config "$dir/opcua.toml" --json
    step usage-days create --days 0 --pki-dir "$dir/days"
    cat "$work/vectors/scenarios/ca_issued/server/cert.der" "$work/vectors/ca/root.der" >"$dir/chain.der"
    step der-chain trust "$dir/chain.der" --pki-dir "$dir/chain"
    step usage-group list bogus-group --pki-dir "$dir/empty"
    step usage-action frobnicate --pki-dir "$dir/empty"
    step wrong-protocol list --config "$dir/modbus.toml"
}

run_sequence "$rust_bin" "$work/rust" "$work/rust.out"
run_sequence "$c_bin" "$work/c" "$work/c.out"

"$py" - "$work" <<'PYEOF'
import json, sys

work = sys.argv[1]
GENERATED = {"create", "show", "export-json"}
VOLATILE = {"subject", "issuer", "thumbprint", "not_before", "not_after", "pem"}

def load(impl):
    steps = {}
    for line in open(f"{work}/{impl}.out"):
        name, code, stdout = line.rstrip("\n").split("\t", 2)
        stdout = stdout.replace(f"{work}/{impl}/", "<dir>/").strip()
        doc = None
        if stdout.startswith("{"):
            doc = json.loads(stdout)
            if name in GENERATED:
                doc = {k: v for k, v in doc.items() if k not in VOLATILE}
        steps[name] = (int(code), doc)
    return steps

rust, c = load("rust"), load("c")
failed = 0
for name in rust:
    if rust[name] == c.get(name):
        code, doc = rust[name]
        print(f"OK   {name}: exit {code}" + (" + JSON" if doc is not None else ""))
        continue
    failed += 1
    print(f"FAIL {name}", file=sys.stderr)
    print("  rust:", json.dumps(rust[name], sort_keys=True), file=sys.stderr)
    print("  c:   ", json.dumps(c.get(name), sort_keys=True), file=sys.stderr)
sys.exit(1 if failed else 0)
PYEOF
