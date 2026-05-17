# Eventp

[![crates.io](https://img.shields.io/crates/v/eventp)](https://crates.io/crates/eventp)
[![docs.rs](https://img.shields.io/docsrs/eventp)](https://docs.rs/eventp/)
[![CI](https://github.com/FuuuOverclocking/eventp/actions/workflows/rust.yml/badge.svg)](https://github.com/FuuuOverclocking/eventp/actions/workflows/rust.yml)
[![codecov](https://codecov.io/gh/FuuuOverclocking/eventp/branch/main/graph/badge.svg)](https://codecov.io/gh/FuuuOverclocking/eventp)

Safe Rust abstraction over Linux epoll, offering a truly zero-cost event dispatch mechanism.

- [Documentation](https://docs.rs/eventp/)
- [Examples](https://github.com/FuuuOverclocking/eventp/tree/main/examples)
- [Technical](https://docs.rs/eventp/latest/eventp/_technical/index.html)
- [Technical (中文)](https://docs.rs/eventp/latest/eventp/_technical_zh/index.html)

*Minimum supported Rust version: 1.71.0*

## Platform support

Linux only, on 64-bit targets. Non-Linux and non-64-bit platforms are rejected at compile time.

Tested in CI on `x86_64` and `aarch64`.

## Quick start

```sh
cargo add eventp
cargo add eventp --dev --features mock
```

or,

```toml
[dependencies]
eventp = "1.0.0"

[dev-dependencies]
eventp = { version = "1.0.0", features = ["mock"] }
```

> When writing tests, you may find the `mock` feature makes life much easier :)

Here is a full example shows almost everything you need: [examples/echo-server.rs](https://github.com/FuuuOverclocking/eventp/blob/main/examples/echo-server.rs).

## License

MIT.
