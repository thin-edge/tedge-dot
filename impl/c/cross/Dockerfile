# Cross-compilation image for the C implementation: zig supplies the compiler and libc,
# Debian supplies the target-architecture shared libraries via multiarch.
#
# This is what lets one runner (or one laptop) build every architecture. The
# payoff is not just "fewer runners" — zig lets us pin the *minimum glibc*
# (see GLIBC_MIN in build.sh), so the packages install on distros far older
# than the build host, which a native build can never do.
#
# open62541 is not packaged by Debian, so it is still built from source by
# CMake (FetchContent) — cross-compiled with the same zig toolchain.
FROM debian:bookworm-slim

# Debian architectures whose libraries we want to link against. Adding the
# image's own native architecture is harmless.
ARG DEB_ARCHS="amd64 arm64 armhf"
ARG ZIG_VERSION=0.16.0

SHELL ["/bin/sh", "-eux", "-c"]

# net-snmp (the SNMP connector's libnetsnmp) is also built from source by CMake, with
# autotools: it needs make, perl and file, plus the target's binutils because libtool
# insists on a real `ld` next to zig cc (see TDOT_NETSNMP_HOST in cross/build.sh).
RUN for a in ${DEB_ARCHS}; do dpkg --add-architecture "$a"; done; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
      ca-certificates cmake curl git ninja-build pkg-config python3 xz-utils \
      make perl file patch \
      binutils-x86-64-linux-gnu binutils-aarch64-linux-gnu binutils-arm-linux-gnueabihf; \
    for a in ${DEB_ARCHS}; do \
      apt-get install -y --no-install-recommends \
        "libcjson-dev:$a" "libmodbus-dev:$a" "libmosquitto-dev:$a"; \
    done; \
    rm -rf /var/lib/apt/lists/*

# zig ships the C compiler, the linker and glibc/musl for every target, so no
# per-architecture gcc cross toolchain is needed.
RUN host="$(uname -m)"; \
    base="https://ziglang.org/download/${ZIG_VERSION}"; \
    curl -fsSL "${base}/zig-${host}-linux-${ZIG_VERSION}.tar.xz" -o /tmp/zig.tar.xz \
      || curl -fsSL "${base}/zig-linux-${host}-${ZIG_VERSION}.tar.xz" -o /tmp/zig.tar.xz; \
    mkdir -p /opt/zig; \
    tar -xJf /tmp/zig.tar.xz -C /opt/zig --strip-components=1; \
    rm /tmp/zig.tar.xz; \
    ln -s /opt/zig/zig /usr/local/bin/zig; \
    zig version

COPY toolchain.cmake build.sh /opt/tdot-cross/
RUN chmod +x /opt/tdot-cross/build.sh

ENTRYPOINT ["/opt/tdot-cross/build.sh"]
