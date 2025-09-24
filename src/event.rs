use crate::epoll::EpollFlags;

#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Event(pub(crate) EpollFlags);

impl From<Event> for EpollFlags {
    fn from(value: Event) -> Self {
        value.bitflags()
    }
}

impl Event {
    /// Returns the underlying `EpollFlags` bitmask.
    pub const fn bitflags(&self) -> EpollFlags {
        self.0
    }

    /// Returns `true` if the Event contains readable readiness.
    ///
    /// This corresponds to the `EPOLLIN` flag.
    pub const fn is_readable(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLIN)
    }

    /// Returns `true` if the Event contains writable readiness.
    ///
    /// This corresponds to the `EPOLLOUT` flag.
    pub const fn is_writable(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLOUT)
    }

    /// Returns `true` if the Event contains priority readiness.
    ///
    /// This corresponds to the `EPOLLPRI` flag, indicating urgent out-of-band data.
    pub const fn is_priority(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLPRI)
    }

    /// Returns `true` if the Event contains an error.
    ///
    /// This corresponds to the `EPOLLERR` flag. Note that this flag is
    /// always reported on a file descriptor, even if not explicitly requested
    /// in the Event set.
    pub const fn is_error(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLERR)
    }

    /// Returns `true` if the Event contains a "hang up" event.
    ///
    /// This corresponds to the `EPOLLHUP` flag. This can mean the peer has
    /// closed the connection, or the write end of a pipe is closed. Note
    /// that this flag is always reported on a file descriptor, even if not
    //  explicitly requested in the Event set.
    pub const fn is_hangup(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLHUP)
    }

    /// Returns `true` if the peer has closed their writing end of the connection.
    ///
    /// This corresponds to the `EPOLLRDHUP` flag. It indicates that the
    /// stream socket peer has closed their connection, or has shut down
    /// their writing half of the connection.
    pub const fn is_read_closed(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLRDHUP)
    }
}
