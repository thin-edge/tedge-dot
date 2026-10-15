#!/usr/bin/env python3
"""Pull every registry image a compose stack needs, once, before any suite starts.

Covers both the runtime `image:` references and the `FROM` images of every service the stack
builds, resolved from `docker compose config` with the current environment (BASE_REGISTRY,
CONNECTOR_DOCKERFILE, ...). Once they are in the local image store, neither `compose up` (pull
policy "missing") nor the builds (BuildKit's docker driver uses a local `FROM` image) contact a
registry for them again.

Third-party images come from BASE_REGISTRY: in CI the repository's GHCR mirror
(.github/mirror-images.txt), locally ECR Public. A reference under any of the known library
registries falls back through the others -- mirror, ECR Public, Docker Hub -- and a fallback
pull is tagged with the reference the stack asked for, so the mirror being unavailable or
missing an image degrades to the old behaviour instead of failing the job.

ECR Public and Docker Hub meter anonymous pulls per egress IP, which GitHub-hosted runners share
with every other Actions user, and answer `429 toomanyrequests` (rate or data limit) when it runs
out. That is retried with a growing, jittered backoff; any other error moves on to the next
source straight away.

Best effort by design: always exits 0. An image that cannot be pulled from anywhere is left for
compose to report with the real error in context.

Usage: pull-stack-images.py <compose-file>
"""

import json
import os
import random
import re
import subprocess
import sys
import time

ECR = "public.ecr.aws/docker/library"
DOCKER_HUB = "docker.io/library"
# Seconds to wait after each throttled attempt (plus jitter): ~2.5 min per source in total.
BACKOFF = [10, 20, 40, 80]


def library_registries():
    """The interchangeable registries holding the same library images, preferred first."""
    regs = []
    for reg in (os.environ.get("BASE_REGISTRY", ""), ECR, DOCKER_HUB):
        reg = reg.rstrip("/")
        if reg and reg not in regs:
            regs.append(reg)
    return regs


def compose_config(compose_file):
    out = subprocess.run(
        ["docker", "compose", "-f", compose_file, "config", "--format", "json"],
        check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def substitute(text, values):
    return re.sub(
        r"\$\{(\w+)\}|\$(\w+)",
        lambda m: values.get(m.group(1) or m.group(2), ""),
        text,
    )


def base_images(dockerfile, build_args):
    """`FROM` images of a Dockerfile, with ARG defaults and build args substituted."""
    values, stages, images = {}, set(), []
    seen_from = False
    with open(dockerfile) as f:
        lines = f.read().replace("\\\n", " ").splitlines()
    for line in lines:
        words = line.split()
        if not words:
            continue
        keyword = words[0].upper()
        # Only global ARGs (before the first FROM) can parameterise a FROM line.
        if keyword == "ARG" and not seen_from and len(words) > 1:
            name, _, default = words[1].partition("=")
            values[name] = build_args.get(name, default.strip('"'))
        elif keyword == "FROM":
            seen_from = True
            rest = [w for w in words[1:] if not w.startswith("--")]
            if not rest:
                continue
            ref = substitute(rest[0], values)
            if len(rest) >= 3 and rest[1].upper() == "AS":
                stages.add(rest[2])
            if ref != "scratch" and ref not in stages and ref not in images:
                images.append(ref)
    return images


def stack_images(compose_file):
    images = []
    for service in compose_config(compose_file).get("services", {}).values():
        build = service.get("build")
        if build is None:
            if service.get("image"):
                images.append(service["image"])
            continue
        context = build.get("context", ".")
        if "://" in context:  # a remote (git) build context: its Dockerfile is not ours to read
            continue
        dockerfile = os.path.join(context, build.get("dockerfile", "Dockerfile"))
        args = {k: str(v) for k, v in (build.get("args") or {}).items()}
        try:
            images.extend(base_images(dockerfile, args))
        except OSError as e:
            print(f"warning: cannot read {dockerfile}: {e}", file=sys.stderr)
    return list(dict.fromkeys(images))


def sources(image):
    """Where to try pulling `image` from, in order."""
    regs = library_registries()
    for reg in regs:
        if image.startswith(reg + "/"):
            suffix = image[len(reg) + 1:]
            return [image] + [f"{r}/{suffix}" for r in regs if r != reg]
    return [image]


def pull(ref):
    """Pull one reference, retrying only while throttled. True on success."""
    for attempt in range(len(BACKOFF) + 1):
        res = subprocess.run(["docker", "pull", "-q", ref], capture_output=True, text=True)
        if res.returncode == 0:
            return True
        err = (res.stderr or res.stdout).strip().splitlines()
        reason = err[-1] if err else f"exit {res.returncode}"
        # Not a bare "429": digests in the message can contain those digits.
        throttled = "toomanyrequests" in reason or "429 Too Many Requests" in reason
        if not throttled or attempt == len(BACKOFF):
            print(f"pull of {ref} failed: {reason}", file=sys.stderr)
            return False
        delay = BACKOFF[attempt] + random.uniform(0, BACKOFF[attempt] / 2)
        print(f"pull of {ref} throttled (attempt {attempt + 1}/{len(BACKOFF) + 1}); "
              f"retrying in {delay:.0f}s", file=sys.stderr)
        time.sleep(delay)
    return False


def have(image):
    return subprocess.run(["docker", "image", "inspect", image], capture_output=True).returncode == 0


def main():
    if len(sys.argv) != 2:
        print(__doc__.strip().splitlines()[-1], file=sys.stderr)
        return 0
    try:
        images = stack_images(sys.argv[1])
    except (subprocess.CalledProcessError, ValueError) as e:
        print(f"warning: could not resolve the stack's images ({e}); letting compose try",
              file=sys.stderr)
        return 0
    for image in images:
        # Locally built tags (no registry host, e.g. tedge-dot-rust-modbus-connector) are not
        # pulled; they come from the stack's own builds.
        if "/" not in image or have(image):
            continue
        for src in sources(image):
            if pull(src):
                if src != image:
                    subprocess.run(["docker", "tag", src, image], check=False)
                    print(f"pulled {image} (from {src})")
                else:
                    print(f"pulled {image}")
                break
        else:
            print(f"warning: could not pull {image} from any source; letting compose try",
                  file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
