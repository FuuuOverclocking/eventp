Registers a new subscriber with the event loop.

Takes ownership of `subscriber` and registers its file descriptor
with the underlying `epoll` instance. The subscriber's thin pointer
is stashed in the `epoll_event.data` field for zero-cost dispatch.

# Re-entrancy

Calling this from inside a handler is supported, but the new
subscriber will not fire until the next
[`run_once_with_timeout`](crate::Eventp::run_once_with_timeout) iteration.

# Errors

- [`io::ErrorKind::AlreadyExists`](std::io::ErrorKind::AlreadyExists)
  if a subscriber for the same [`RawFd`](std::os::fd::RawFd) is already
  registered.
- Otherwise, the [`io::Error`](std::io::Error) returned by
  `epoll_ctl(EPOLL_CTL_ADD)`.

# Panics

Cannot be triggered through the public API. Internally, this method
could panic if `subscriber` has already been dropped in place.
