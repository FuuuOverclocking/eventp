use crate::epoll::EpollFlags;

/// A wrapper around [`EpollFlags`], represents interest in I/O readiness events
/// for a file descriptor.
///
/// References for epoll flags provided on each method's documentation, or see
/// [epoll_ctl(2)](https://man.archlinux.org/man/epoll_ctl.2.en#EPOLLIN).
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Interest(EpollFlags);

impl Default for Interest {
    /// Creates a default `Interest` with no flags set.
    fn default() -> Self {
        Self(EpollFlags::empty())
    }
}

impl From<EpollFlags> for Interest {
    fn from(value: EpollFlags) -> Self {
        Self::new(value)
    }
}

impl From<Interest> for EpollFlags {
    fn from(value: Interest) -> Self {
        value.bitflags()
    }
}

impl Interest {
    /// Creates a new `Interest` from raw `EpollFlags`.
    pub const fn new(flags: EpollFlags) -> Self {
        Self(flags)
    }

    /// Returns the underlying `EpollFlags` bitmask.
    pub const fn bitflags(&self) -> EpollFlags {
        self.0
    }

    /// Adds the given flags to this interest set.
    const fn add(self, flags: EpollFlags) -> Self {
        Self(self.0.union(flags))
    }

    /// Removes the given flags from this interest set.
    const fn remove(self, flags: EpollFlags) -> Self {
        Self(self.0.difference(flags))
    }

    /// Adds interest in readable events (`EPOLLIN`).
    ///
    /// The associated file is available for read(2) operations.
    pub const fn read(self) -> Self {
        self.add(EpollFlags::EPOLLIN)
    }

    /// Adds interest in writable events (`EPOLLOUT`).
    ///
    /// The associated file is available for write(2) operations.
    pub const fn write(self) -> Self {
        self.add(EpollFlags::EPOLLOUT)
    }

    /// Adds interest in both readable and writable events.
    pub const fn read_write(self) -> Self {
        self.read().write()
    }

    /// Adds interest in the peer closing the write half of the connection (`EPOLLRDHUP`).
    ///
    /// Stream socket peer closed connection, or shut down writing half of connection.
    /// (This flag is especially useful for writing simple code to detect peer shutdown
    /// when using edge-triggered monitoring.)
    pub const fn rdhup(self) -> Self {
        self.add(EpollFlags::EPOLLRDHUP)
    }

    /// Adds interest in priority events (`EPOLLPRI`), such as out-of-band data.
    ///
    /// There is an exceptional condition on the file descriptor. See the discussion of
    /// POLLPRI in poll(2).
    pub const fn pri(self) -> Self {
        self.add(EpollFlags::EPOLLPRI)
    }

    /// Sets edge-triggered mode (`EPOLLET`). Note that it is level-triggered by default,
    /// therefore that method is not available.
    ///
    /// Requests edge-triggered notification for the associated file descriptor. The default
    /// behavior for epoll is level-triggered. See epoll(7) for more detailed information
    /// about edge-triggered and level-triggered notification.
    pub const fn edge_triggered(self) -> Self {
        self.add(EpollFlags::EPOLLET)
    }

    /// Sets one-shot mode (`EPOLLONESHOT`).
    ///
    /// Requests one-shot notification for the associated file descriptor. This means that
    /// after an event notified for the file descriptor by epoll_wait(2), the file
    /// descriptor is disabled in the interest list and no other events will be reported
    /// by the epoll interface. The user must call epoll_ctl() with EPOLL_CTL_MOD to rearm
    /// the file descriptor with a new event mask.
    pub const fn oneshot(self) -> Self {
        self.add(EpollFlags::EPOLLONESHOT)
    }

    /// Adds a flag to prevent system suspend/hibernate while events are pending (`EPOLLWAKEUP`).
    ///
    /// If EPOLLONESHOT and EPOLLET are clear and the process has the CAP_BLOCK_SUSPEND
    /// capability, ensure that the system does not enter "suspend" or "hibernate" while
    /// this event is pending or being processed. The event is considered as being
    /// "processed" from the time when it is returned by a call to epoll_wait(2) until the
    /// next call to epoll_wait(2) on the same epoll(7) file descriptor, the closure of
    /// that file descriptor, the removal of the event file descriptor with EPOLL_CTL_DEL,
    /// or the clearing of EPOLLWAKEUP for the event file descriptor with EPOLL_CTL_MOD.
    /// See also BUGS.
    #[cfg(not(target_arch = "mips"))]
    pub const fn wakeup(self) -> Self {
        self.add(EpollFlags::EPOLLWAKEUP)
    }

    /// Sets exclusive wake-up mode (`EPOLLEXCLUSIVE`).
    ///
    /// Sets an exclusive wakeup mode for the epoll file descriptor that is being attached
    /// to the target file descriptor, fd. When a wakeup event occurs and multiple epoll
    /// file descriptors are attached to the same target file using EPOLLEXCLUSIVE, one or
    /// more of the epoll file descriptors will receive an event with epoll_wait(2). The
    /// default in this scenario (when EPOLLEXCLUSIVE is not set) is for all epoll file
    /// descriptors to receive an event. EPOLLEXCLUSIVE is thus useful for avoiding
    /// thundering herd problems in certain scenarios.
    ///
    /// If the same file descriptor is in multiple epoll instances, some with the
    /// EPOLLEXCLUSIVE flag, and others without, then events will be provided to all epoll
    /// instances that did not specify EPOLLEXCLUSIVE, and at least one of the epoll
    /// instances that did specify EPOLLEXCLUSIVE.
    ///
    /// The following values may be specified in conjunction with EPOLLEXCLUSIVE: EPOLLIN,
    /// EPOLLOUT, EPOLLWAKEUP, and EPOLLET. EPOLLHUP and EPOLLERR can also be specified,
    /// but this is not required: as usual, these events are always reported if they occur,
    /// regardless of whether they are specified in events. Attempts to specify other values
    /// in events yield the error EINVAL.
    ///
    /// EPOLLEXCLUSIVE may be used only in an EPOLL_CTL_ADD operation; attempts to employ
    /// it with EPOLL_CTL_MOD yield an error. If EPOLLEXCLUSIVE has been set using epoll_ctl(),
    /// then a subsequent EPOLL_CTL_MOD on the same epfd, fd pair yields an error. A call
    /// to epoll_ctl() that specifies EPOLLEXCLUSIVE in events and specifies the target
    /// file descriptor fd as an epoll instance will likewise fail. The error in all of
    /// these cases is EINVAL.
    pub const fn exclusive(self) -> Self {
        self.add(EpollFlags::EPOLLEXCLUSIVE)
    }

    /// Removes interest in readable events.
    pub const fn remove_read(self) -> Self {
        self.remove(EpollFlags::EPOLLIN)
    }

    /// Removes interest in writable events.
    pub const fn remove_write(self) -> Self {
        self.remove(EpollFlags::EPOLLOUT)
    }

    /// Removes interest in the `EPOLLRDHUP` event.
    pub const fn remove_rdhup(self) -> Self {
        self.remove(EpollFlags::EPOLLRDHUP)
    }

    /// Removes interest in priority events.
    pub const fn remove_pri(self) -> Self {
        self.remove(EpollFlags::EPOLLPRI)
    }

    /// Unsets edge-triggered mode, reverting to the default level-triggered behavior.
    pub const fn remove_edge_triggered(self) -> Self {
        self.remove(EpollFlags::EPOLLET)
    }

    /// Unsets one-shot mode.
    pub const fn remove_oneshot(self) -> Self {
        self.remove(EpollFlags::EPOLLONESHOT)
    }

    /// Unsets the `EPOLLWAKEUP` flag.
    #[cfg(not(target_arch = "mips"))]
    pub const fn remove_wakeup(self) -> Self {
        self.remove(EpollFlags::EPOLLWAKEUP)
    }

    /// Unsets exclusive wake-up mode.
    pub const fn remove_exclusive(self) -> Self {
        self.remove(EpollFlags::EPOLLEXCLUSIVE)
    }
}

/// Creates a new, empty [`Interest`] set. This is the **recommended** API entry point.
///
/// Use this function to start fluently configuring the interest set (e.g., `.read()`).
/// Chain the configuration with [`with_fd`] and [`with_handler`] to create a [`Subscriber`],
/// and then use [`register_into`] to register it with an [`Eventp`].
///
/// # Examples
///
/// ```rust
/// # use std::io;
/// # use eventp::{interest, tri_subscriber::WithHandler, Eventp, Subscriber};
/// use nix::sys::eventfd::EventFd;
///
/// fn thread_main(efd: EventFd) -> io::Result<()> {
///     let mut eventp = Eventp::default();
///     interest()
///         .read()
///         .with_fd(efd)
///         .with_handler(|efd: &mut EventFd| {
///             efd.read().unwrap();
///             on_eventfd();
///         })
///         .register_into(&mut eventp)?;
///
///     eventp.run_forever()
/// }
///
/// fn on_eventfd() {
///     // do somethings...
/// }
/// ```
///
/// [`with_fd`]: Interest::with_fd
/// [`with_handler`]: crate::tri_subscriber::WithHandler::with_handler
/// [`Subscriber`]: crate::Subscriber
/// [`register_into`]: crate::Subscriber::register_into
/// [`Eventp`]: crate::Eventp
pub const fn interest() -> Interest {
    Interest::new(EpollFlags::empty())
}
