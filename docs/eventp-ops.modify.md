Modifies the event interest for an existing subscriber.

Updates both the kernel-side `epoll` registration for `fd` and the
subscriber's own `Cell<Interest>` so that the value seen by the
handler stays in sync with what the kernel monitors.

# Errors

- [`io::ErrorKind::NotFound`](std::io::ErrorKind::NotFound) if no
  subscriber is registered for `fd`.
- Otherwise, the [`io::Error`](std::io::Error) returned by
  `epoll_ctl(EPOLL_CTL_MOD)`.
