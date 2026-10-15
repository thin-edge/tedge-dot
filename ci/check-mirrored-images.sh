#!/usr/bin/env bash
# Check that every third-party image the Dockerfiles and compose stacks use is mirrored.
#
# CI builds and runs from the GHCR mirror (BASE_REGISTRY, .github/mirror-images.txt). An image
# that is referenced but not listed would only fall back to the shared-quota registries the
# mirror exists to avoid -- so fail here, in seconds, instead of in a throttled e2e run. Also
# fails on an upstream registry written out literally, which bypasses BASE_REGISTRY entirely.
set -euo pipefail
cd "$(dirname "$0")/.."

list=.github/mirror-images.txt
mirrored=$(grep -vE '^[[:space:]]*(#|$)' "$list" | sort -u)
status=0

# `FROM ${BASE_REGISTRY}/<image>` in Dockerfiles, `${BASE_REGISTRY:-...}/<image>` in compose files.
refs=$(
    {
        git grep -nE 'FROM +\$\{BASE_REGISTRY\}/' -- '*Dockerfile*' \
            | sed -E 's|^([^:]+:[0-9]+):.*\$\{BASE_REGISTRY\}/([^ ]+).*|\1 \2|'
        git grep -nE '\$\{BASE_REGISTRY:-[^}]*\}/' -- '*.yaml' '*.yml' ':!openspec/' \
            | sed -E 's|^([^:]+:[0-9]+):.*\$\{BASE_REGISTRY:-[^}]*\}/([^ "]+).*|\1 \2|'
    } || true
)
while read -r where image; do
    [ -n "$where" ] || continue
    if ! grep -qxF "$image" <<< "$mirrored"; then
        echo "error: $where uses $image, which is not in $list" >&2
        status=1
    fi
done <<< "$refs"

# Upstream registries written out, bypassing BASE_REGISTRY (and so the mirror).
literal=$(
    {
        git grep -nE '^[[:space:]]*image:[[:space:]]*"?(public\.ecr\.aws|(index\.)?docker\.io)/' -- '*.yaml' '*.yml' ':!openspec/'
        git grep -nE '^FROM +(--[^ ]+ +)*(public\.ecr\.aws|(index\.)?docker\.io)/' -- '*Dockerfile*'
    } || true
)
if [ -n "$literal" ]; then
    while read -r line; do
        echo "error: upstream registry written out instead of \${BASE_REGISTRY}: $line" >&2
    done <<< "$literal"
    status=1
fi

# Listed but unused: harmless (just a wasted weekly copy), so only a note.
used=$(cut -d' ' -f2 <<< "$refs" | sort -u)
while read -r image; do
    grep -qxF "$image" <<< "$used" || echo "note: $image is mirrored but no longer referenced" >&2
done <<< "$mirrored"

[ "$status" = 0 ] && echo "all $(wc -l <<< "$used" | tr -d ' ') referenced third-party images are mirrored"
exit "$status"
