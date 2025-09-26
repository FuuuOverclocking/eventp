# Eventp

[![crates.io](https://img.shields.io/crates/v/eventp)](https://crates.io/crates/eventp)
[![docs.rs](https://img.shields.io/docsrs/eventp)](https://docs.rs/eventp/)

A high-performance Linux event loop library built on epoll with type-safe interest registration and flexible event handling.

- [Documentation](https://docs.rs/eventp/)
- [Examples](https://github.com/FuuuOverclocking/eventp/tree/main/examples)
- [Technical](https://docs.rs/eventp/latest/eventp/_technical/index.html)
- [Technical (中文)](https://docs.rs/eventp/latest/eventp/_technical_zh/index.html)

## Quick start

```sh
cargo add eventp
cargo add eventp --dev --features mock
```

Both commands need to be executed. When writing unit tests, you may find that the `mock` feature makes life much easier - it's even indispensable.

## Example

Here is an example shows almost everything users needed: [examples/echo-server.rs](https://github.com/FuuuOverclocking/eventp/blob/main/examples/echo-server.rs).

## License

MIT.
