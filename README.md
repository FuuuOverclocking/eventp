# Eventp

[![crates.io](https://img.shields.io/crates/v/eventp)](https://crates.io/crates/eventp)
[![docs.rs](https://img.shields.io/docsrs/eventp)](https://docs.rs/eventp/)

Safe Rust abstraction over Linux's `epoll`, offering a true zero-cost event dispatch mechanism.

- [Documentation](https://docs.rs/eventp/)
- [Examples](https://github.com/FuuuOverclocking/eventp/tree/main/examples)
- [Technical](https://docs.rs/eventp/latest/eventp/_technical/index.html)
- [Technical (中文)](https://docs.rs/eventp/latest/eventp/_technical_zh/index.html)

## Quick start

```sh
cargo add eventp
cargo add eventp --dev --features mock
```

or,

```toml
[dependencies]
eventp = "1.0.0-rc.6"

[dev-dependencies]
eventp = { version = "1.0.0-rc.6", features = ["mock"] }
```

> When writing tests, you may find the `mock` feature makes life much easier :)

Here is a full example shows almost everything you need: [examples/echo-server.rs](https://github.com/FuuuOverclocking/eventp/blob/main/examples/echo-server.rs).

## License

MIT.
