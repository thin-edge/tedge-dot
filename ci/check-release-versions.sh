#!/bin/sh
# Check that every place release-please stamps the version agrees, and that its Cargo.lock
# filter (release-please-config.json) names every workspace crate.
#
# release-please only WARNS when an extra-files jsonpath matches nothing, so a crate added to
# the workspace but not to the filter would keep its old version in Cargo.lock. Nothing would
# notice until the release build's `cargo fetch --locked` failed on the new tag.
set -eu
cd "$(dirname "$0")/.."

python3 - <<'PY'
import json, re, sys, tomllib

errors = []
manifest = json.load(open(".release-please-manifest.json"))["."]
config = json.load(open("release-please-config.json"))

versions = {
    ".release-please-manifest.json": manifest,
    "version.txt": open("version.txt").read().strip(),
    "impl/rust/Cargo.toml": tomllib.load(open("impl/rust/Cargo.toml", "rb"))["workspace"]["package"]["version"],
}
m = re.search(r"^project\(tedge-dot VERSION (\S+) .*x-release-please-version", open("impl/c/CMakeLists.txt").read(), re.M)
versions["impl/c/CMakeLists.txt"] = m.group(1) if m else "<no x-release-please-version line>"

workspace = tomllib.load(open("impl/rust/Cargo.toml", "rb"))["workspace"]
members = {"tedge-dot"}
for path in workspace["members"]:
    members.add(tomllib.load(open(f"impl/rust/{path}/Cargo.toml", "rb"))["package"]["name"])
lock = tomllib.load(open("impl/rust/Cargo.lock", "rb"))["package"]
for p in lock:
    if p["name"] in members:
        versions[f"impl/rust/Cargo.lock ({p['name']})"] = p["version"]

for where, v in versions.items():
    if v != manifest:
        errors.append(f"{where}: {v}, expected {manifest}")

lock_filter = next(f["jsonpath"] for f in config["packages"]["."]["extra-files"]
                   if f["path"] == "impl/rust/Cargo.lock")
listed = set(re.findall(r"@\.name\.value == '([^']+)'", lock_filter))
for name in sorted(members - listed):
    errors.append(f"release-please-config.json: Cargo.lock filter is missing workspace crate {name}")
for name in sorted(listed - members):
    errors.append(f"release-please-config.json: Cargo.lock filter names {name}, not a workspace crate")

if errors:
    print("Release version check failed:", *errors, sep="\n  ", file=sys.stderr)
    sys.exit(1)
print(f"release versions consistent at {manifest} ({len(members)} workspace crates)")
PY
