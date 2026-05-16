Unregisters a subscriber from the event loop.

Performs `epoll_ctl(EPOLL_CTL_DEL)` first; on success, the
subscriber's registry entry is taken care of as follows:

1. **Outside event dispatch**: the registry entry is removed and
   the subscriber's destructor runs synchronously inside this call.
2. **Inside event dispatch** (i.e. when called from a handler):
   - **Self-delete** (`fd` is the currently-handled subscriber):
     the registry entry and the subscriber are kept alive for the
     rest of the current handler invocation. Once the handler
     returns, the registry entry is removed and the subscriber is
     dropped before the dispatch loop moves on to the next event.
   - **Delete another fd**: the subscriber is removed from the
     registry and dropped immediately, while the release of its
     heap space is delayed until after this batch of events
     finishes dispatching.
     - If the deleted fd had a pending event in the same batch, the
       dispatch loop will detect the dropped slot and skip the
       handler invocation instead of re-entering the destructed
       value.
     - Re-adding the same [`RawFd`](std::os::fd::RawFd) from inside
       the same handler *is* permitted (the registry entry was freed
       above). The new subscriber will start receiving events on the
       next [`run_once_with_timeout`](crate::Eventp::run_once_with_timeout)
       iteration; the skipped pending event from the current batch
       is **not** redelivered.

# Errors

- [`io::ErrorKind::NotFound`](std::io::ErrorKind::NotFound) if no
  subscriber is registered for `fd`.
- Otherwise, the [`io::Error`](std::io::Error) returned by
  `epoll_ctl(EPOLL_CTL_DEL)` (e.g. `EBADF` if `fd` has already been
  closed by another path). When the syscall fails the registry and
  the in-flight handling state are left untouched, so the call may
  be retried after the underlying problem is fixed.
