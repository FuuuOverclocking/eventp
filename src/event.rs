use crate::epoll::{EpollEvent, EpollFlags};

/// A readiness event from the I/O reactor.
///
/// This is a wrapper around [`EpollFlags`], providing a ergonomic and type-safe
/// API for checking specific readiness states.
///
/// References for epoll flags provided on each method's documentation, or see
/// [epoll_ctl(2)](https://man.archlinux.org/man/epoll_ctl.2.en#EPOLLIN).
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Event(EpollFlags);

impl From<EpollFlags> for Event {
    fn from(value: EpollFlags) -> Self {
        Self::new(value)
    }
}

impl From<&EpollEvent> for Event {
    fn from(value: &EpollEvent) -> Self {
        let ptr = value as *const _ as *const libc::epoll_event;
        // SAFETY: EpollEvent is a transparent wrapper around libc::epoll_event.
        Self(EpollFlags::from_bits_retain(unsafe { *ptr }.events as i32))
    }
}

impl From<Event> for EpollFlags {
    fn from(value: Event) -> Self {
        value.bitflags()
    }
}

impl Event {
    /// Creates a new `Event` from the given `EpollFlags`.
    pub const fn new(flags: EpollFlags) -> Self {
        Self(flags)
    }

    /// Returns the underlying `EpollFlags` bitmask.
    pub const fn bitflags(&self) -> EpollFlags {
        self.0
    }

    /// Returns `true` if the event indicates readable readiness (`EPOLLIN`).
    ///
    /// The associated file is available for read(2) operations.
    pub const fn is_readable(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLIN)
    }

    /// Returns `true` if the event indicates writable readiness (`EPOLLOUT`).
    ///
    /// The associated file is available for write(2) operations.
    pub const fn is_writable(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLOUT)
    }

    /// Returns `true` if the event indicates priority readiness (`EPOLLPRI`).
    ///
    /// There is an exceptional condition on the file descriptor. See the discussion of
    /// POLLPRI in poll(2).
    pub const fn is_priority(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLPRI)
    }

    /// Returns `true` if the event indicates an error condition (`EPOLLERR`).
    ///
    /// Error condition happened on the associated file descriptor. This event is also
    /// reported for the write end of a pipe when the read end has been closed.
    ///
    /// epoll_wait(2) will always report for this event; it is not necessary to set it in
    /// events when calling epoll_ctl().
    pub const fn is_error(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLERR)
    }

    /// Returns `true` if the event indicates that a hang up has occurred (`EPOLLHUP`).
    ///
    /// Hang up happened on the associated file descriptor.
    ///
    /// epoll_wait(2) will always wait for this event; it is not necessary to set it in
    /// events when calling epoll_ctl().
    ///
    /// Note that when reading from a channel such as a pipe or a stream socket, this
    /// event merely indicates that the peer closed its end of the channel. Subsequent
    /// reads from the channel will return 0 (end of file) only after all outstanding
    /// data in the channel has been consumed.
    pub const fn is_hangup(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLHUP)
    }

    /// Returns `true` if the peer has closed their writing end of the connection (`EPOLLRDHUP`).
    ///
    /// Stream socket peer closed connection, or shut down writing half of connection.
    /// (This flag is especially useful for writing simple code to detect peer shutdown
    /// when using edge-triggered monitoring.)
    pub const fn is_read_closed(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLRDHUP)
    }
}
