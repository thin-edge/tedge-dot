# Changelog

## [0.0.18](https://github.com/thin-edge/tedge-dot/compare/0.0.17...0.0.18) (2026-10-28)


### Features

* **docs:** documentation site (Astro + Starlight) on GitHub Pages ([#77](https://github.com/thin-edge/tedge-dot/issues/77)) ([1452296](https://github.com/thin-edge/tedge-dot/commit/145229611a970320db8005b52becd1b3ef1fe8dd))


### Bug Fixes

* **opcua:** a link lost mid-pass takes the link down at once (Rust) ([24f9c2a](https://github.com/thin-edge/tedge-dot/commit/24f9c2added4dc7e74c0dd6dd4c421bb89a59eac)), closes [#97](https://github.com/thin-edge/tedge-dot/issues/97)
* **opcua:** check the retry interval before scanning every point (C) ([8a471e9](https://github.com/thin-edge/tedge-dot/commit/8a471e9112728faf6ad9ef2f98fb30c25e0b8e60))
* **opcua:** check the retry interval before scanning every point (C) ([fa0e018](https://github.com/thin-edge/tedge-dot/commit/fa0e018f0b28dd1fc8e0acfb08ee22de7d3a3967))
* **sdk:** resolve a point library in linear time (C and Rust) ([#87](https://github.com/thin-edge/tedge-dot/issues/87)) ([4843b25](https://github.com/thin-edge/tedge-dot/commit/4843b2595a0ccafc4d33c919a9e90461886cb100))
* **sdk:** stamp the stall heartbeat after every read (C) ([93dd44f](https://github.com/thin-edge/tedge-dot/commit/93dd44f728ea303ab8e949d29b343cdf263c0463)), closes [#97](https://github.com/thin-edge/tedge-dot/issues/97)


### Performance

* **opcua:** poll in blocks of at most MaxNodesPerRead (C and Rust) ([9c1254d](https://github.com/thin-edge/tedge-dot/commit/9c1254d267869d02c21e74bf8a74ddaa11d239ef))
* **opcua:** poll in blocks of at most MaxNodesPerRead (C) ([a0add82](https://github.com/thin-edge/tedge-dot/commit/a0add82256590bc02eed7989235dd1471c3aa568)), closes [#97](https://github.com/thin-edge/tedge-dot/issues/97)
* **opcua:** poll in blocks of at most MaxNodesPerRead (Rust) ([8858e5f](https://github.com/thin-edge/tedge-dot/commit/8858e5f2543710cb54456b7e35d4b3b56b0a776a)), closes [#97](https://github.com/thin-edge/tedge-dot/issues/97)

## [0.0.17](https://github.com/thin-edge/tedge-dot/compare/0.0.16...0.0.17) (2026-10-13)


### Bug Fixes

* **sdk:** log link changes, failed writes and unreadable points ([#74](https://github.com/thin-edge/tedge-dot/issues/74)) ([2d6efe4](https://github.com/thin-edge/tedge-dot/commit/2d6efe423b5a320b2fb79345caca20cbf30451b3))

## [0.0.16](https://github.com/thin-edge/tedge-dot/compare/0.0.15...0.0.16) (2026-10-12)


### Bug Fixes

* ignore local .zig-cache ([#72](https://github.com/thin-edge/tedge-dot/issues/72)) ([6b265be](https://github.com/thin-edge/tedge-dot/commit/6b265beb03fffb126e5d3f21124994e32a18eed5))

## [0.0.15](https://github.com/thin-edge/tedge-dot/compare/0.0.14...0.0.15) (2026-10-12)


### Features

* **opcua:** call OPC UA methods (opcua-method-calls) ([#71](https://github.com/thin-edge/tedge-dot/issues/71)) ([39c205f](https://github.com/thin-edge/tedge-dot/commit/39c205ff6c8fbc80e9a48c684d4e7d70f34a77a1))
* **opcua:** namespace tables for numeric node ids (opcua-namespace-table) ([#69](https://github.com/thin-edge/tedge-dot/issues/69)) ([fed70e7](https://github.com/thin-edge/tedge-dot/commit/fed70e71f5f1c4ca3b8409e8bbcfff98aae77f8d))

## [0.0.14](https://github.com/thin-edge/tedge-dot/compare/0.0.13...0.0.14) (2026-10-09)


### Features

* performance fixes from the OPC UA load test, in both builds (load-test-performance) ([#66](https://github.com/thin-edge/tedge-dot/issues/66)) ([ad45fe8](https://github.com/thin-edge/tedge-dot/commit/ad45fe8d5f546ea35cb4b7e2e51927d7e621b1e8))
