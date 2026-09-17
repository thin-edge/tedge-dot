#!/bin/sh
# Generate the shared OPC UA PKI vectors (genpki.py) and run the C checks over them.
#   opcua_pki.sh <tedge-dot-opcua-pki binary> <repository root>
# Python: $TEDGE_DOT_PYTHON, the repository's .venv, or python3 -- with `cryptography`
# (`just venv`, or `pip install cryptography`, or Debian's python3-cryptography).
set -eu
bin="$1"
repo="$2"
py="${TEDGE_DOT_PYTHON:-}"
if [ -z "$py" ]; then
    if [ -x "$repo/.venv/bin/python" ]; then py="$repo/.venv/bin/python"; else py=python3; fi
fi
out=$(mktemp -d "${TMPDIR:-/tmp}/tdot-pki-vectors.XXXXXX")
trap 'rm -rf "$out"' EXIT
"$py" "$repo/connectors/opcua/conformance/tools/genpki.py" "$out/v"
"$bin" "$out/v"
