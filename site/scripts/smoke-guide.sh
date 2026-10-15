#!/usr/bin/env bash
# Run the commands of "Your first device in 5 minutes" (the Modbus quick start) from the
# repository root, exactly as the guide shows them, and check what the guide says they print.
#
#   site/scripts/smoke-guide.sh            # needs docker, just and a Rust toolchain
#
# `tedge-dot read` exits 1 when a point has bad quality, which the guide's demo shows on
# purpose (bad_point), so 1 is accepted from `read`; any other failure fails the smoke.
set -euo pipefail
cd "$(dirname "$0")/../.."
guide=site/src/content/docs/get-started/first-device.mdx
log=$(mktemp)
trap 'just sim-down modbus >/dev/null 2>&1 || true' EXIT

shopt -s expand_aliases
while IFS= read -r cmd; do
  # The smoke already runs in a checkout.
  case "$cmd" in "git clone "* | "cd tedge-dot") continue ;; esac
  echo "\$ $cmd"
  # In this shell, not a pipeline's subshell: the guide's `alias` and `export` must stick.
  set +e
  eval "$cmd" > >(tee -a "$log") 2>&1
  status=$?
  set -e
  if [ "$status" -ne 0 ] && ! { [ "$status" -eq 1 ] && [[ "$cmd" == "tedge-dot read"* ]]; }; then
    echo "smoke: '$cmd' failed with exit code $status" >&2
    exit 1
  fi
done < <(node site/scripts/guide-commands.mjs "$guide")

# What the guide says the reads show.
for expected in 17001 404.17 heating standby; do
  grep -q -- "$expected" "$log" || { echo "smoke: the guide promises '$expected', the commands did not print it" >&2; exit 1; }
done
echo "smoke: $guide works as written"
