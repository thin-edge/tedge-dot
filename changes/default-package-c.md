### `tedge-dot-c` is now the recommended package

The documentation now recommends **`tedge-dot-c`** by default: it is about 25 times smaller,
runs down to glibc 2.17 and ships every protocol, PROFIBUS-DP included. Choose `tedge-dot-rs`
when you need one of the features only the Rust build has (SNMPv3 with SHA-2 or AES-192/256,
OPC UA structured values, the deprecated Basic128Rsa15 policy, CAN FD); the
[package picker](https://thin-edge.github.io/tedge-dot/tools/package-picker/) helps you decide.

- Nothing changes for an existing install: both packages are still built and released, and a
  config, a flow or a cloud integration works unchanged with either.
- Whether both packages remain in the final product is not decided yet.
