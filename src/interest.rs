//! This module defines `Interest`, a type used to specify readiness events
//! for file descriptors when registering with an I/O reactor (like epoll).
//!
//! `Interest` is a type-safe wrapper around raw `epoll` flags, providing a
//! fluent builder-style API to construct the desired set of events to monitor.
//! It also includes query methods to interpret the event set returned by the reactor.

use std::cell::Cell;
use std::marker::PhantomData;
use std::os::fd::AsFd;

use crate::epoll::EpollFlags;
use crate::tri_subscriber::FnHandler;
use crate::{BinSubscriber, EventpOps, Handler, TriSubscriber};

/// Represents interest in I/O readiness events.
///
/// This is a wrapper around `EpollFlags` that provides a fluent API for building
/// an interest set. It can be used to specify what events (e.g., readable,
/// writable) a user is interested in for a particular file descriptor.
///
/// It also serves to interpret the events returned by `epoll_wait`.
///
/// # Examples
///
/// ```
/// # use eventp::interest;
/// // Create an interest for readable and edge-triggered events.
/// let interest = interest().read().edge_triggered();
///
/// // Check if an event set indicates readability.
/// assert!(interest.is_readable());
/// ```
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
    ///
    /// This is generally used for converting from a raw event mask returned by
    /// the operating system.
    pub const fn new(flags: EpollFlags) -> Self {
        Self(flags)
    }

    /// Returns the underlying `EpollFlags` bitmask.
    pub const fn bitflags(&self) -> EpollFlags {
        self.0
    }

    // /// Combines this `Interest` with a file descriptor.
    // ///
    // /// This is a convenience method for chaining calls.
    // pub const fn with_fd<Fd>(self, fd: Fd) -> (Self, Fd)
    // where
    //     Fd: AsFd,
    // {
    //     (self, fd)
    // }

    /// Combines this `Interest` with a handler to create a full `Subscriber`.
    ///
    /// This finalizes the setup for a subscribable I/O source.
    pub const fn with_fd_and_handler<S, Ep>(self, fd_with_handler: S) -> BinSubscriber<S>
    where
        S: AsFd + Handler<Ep>,
        Ep: EventpOps,
    {
        BinSubscriber {
            interest: Cell::new(self),
            fd_with_handler,
        }
    }

    // --- Builder Methods (for registering interest with the kernel) ---

    /// A private helper to add flags in a const context.
    const fn add(self, flags: EpollFlags) -> Self {
        Self(EpollFlags::from_bits_retain(self.0.bits() | flags.bits()))
    }

    /// Adds readable interest (`EPOLLIN`).
    pub const fn read(self) -> Self {
        self.add(EpollFlags::EPOLLIN)
    }

    /// Adds writable interest (`EPOLLOUT`).
    pub const fn write(self) -> Self {
        self.add(EpollFlags::EPOLLOUT)
    }

    /// Adds both readable and writable interest.
    pub const fn read_write(self) -> Self {
        self.add(EpollFlags::EPOLLIN).add(EpollFlags::EPOLLOUT)
    }

    /// Adds interest in the peer closing the write half of the connection (`EPOLLRDHUP`).
    pub const fn rdhup(self) -> Self {
        self.add(EpollFlags::EPOLLRDHUP)
    }

    /// Adds interest in priority events (`EPOLLPRI`).
    pub const fn pri(self) -> Self {
        self.add(EpollFlags::EPOLLPRI)
    }

    /// Sets edge-triggered mode (`EPOLLET`).
    ///
    /// Note: Level-triggered mode is the default and cannot be explicitly added.
    pub const fn edge_triggered(self) -> Self {
        self.add(EpollFlags::EPOLLET)
    }

    /// Sets one-shot mode (`EPOLLONESHOT`).
    ///
    /// After an event is pulled for the file descriptor, it is disabled until
    /// it is re-armed.
    pub const fn oneshot(self) -> Self {
        self.add(EpollFlags::EPOLLONESHOT)
    }

    /// A flag that can be used to prevent suspend/hibernate (`EPOLLWAKEUP`).
    #[cfg(not(target_arch = "mips"))]
    pub const fn wakeup(self) -> Self {
        self.add(EpollFlags::EPOLLWAKEUP)
    }

    /// Sets exclusive wake-up mode (`EPOLLEXCLUSIVE`).
    ///
    /// This is useful for preventing "thundering herd" problems.
    pub const fn exclusive(self) -> Self {
        self.add(EpollFlags::EPOLLEXCLUSIVE)
    }

    // --- Query Methods (for interpreting events returned from the kernel) ---

    /// Returns `true` if the interest contains readable readiness.
    ///
    /// This corresponds to the `EPOLLIN` flag.
    pub const fn is_readable(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLIN)
    }

    /// Returns `true` if the interest contains writable readiness.
    ///
    /// This corresponds to the `EPOLLOUT` flag.
    pub const fn is_writable(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLOUT)
    }

    /// Returns `true` if the interest contains priority readiness.
    ///
    /// This corresponds to the `EPOLLPRI` flag, indicating urgent out-of-band data.
    pub const fn is_priority(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLPRI)
    }

    /// Returns `true` if the interest contains an error.
    ///
    /// This corresponds to the `EPOLLERR` flag. Note that this flag is
    /// always reported on a file descriptor, even if not explicitly requested
    /// in the interest set.
    pub const fn is_error(&self) -> bool {
        self.0.contains(EpollFlags::EPOLLERR)
    }

    /// Returns `true` if the interest contains a "hang up" event.
    ///
    /// This corresponds to the `EPOLLHUP` flag. This can mean the peer has
    /// closed the connection, or the write end of a pipe is closed. Note
    /// that this flag is always reported on a file descriptor, even if not
    //  explicitly requested in the interest set.
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

pub trait WithFd {
    type Out<Fd>;

    fn with_fd<Fd: AsFd>(self, fd: Fd) -> Self::Out<Fd>;
}

impl WithFd for Interest {
    type Out<Fd> = (Interest, Fd);

    fn with_fd<Fd: AsFd>(self, fd: Fd) -> Self::Out<Fd> {
        (self, fd)
    }
}

impl<Args, F> WithFd for (Interest, FnHandler<Args, F>) {
    type Out<Fd> = (Interest, Fd, FnHandler<Args, F>);

    fn with_fd<Fd: AsFd>(self, fd: Fd) -> Self::Out<Fd> {
        (self.0, fd, self.1)
    }
}

pub trait WithHandler {
    type Out<Args, F>;

    fn with_handler<Args, F>(self, f: F) -> Self::Out<Args, F>;
}

impl WithHandler for Interest {
    type Out<Args, F> = (Interest, FnHandler<Args, F>);

    fn with_handler<Args, F>(self, f: F) -> Self::Out<Args, F> {
        (
            self,
            FnHandler {
                f,
                _marker: PhantomData,
            },
        )
    }
}

impl<Fd> WithHandler for (Interest, Fd)
where
    Fd: AsFd,
{
    type Out<Args, F> = (Interest, Fd, FnHandler<Args, F>);

    fn with_handler<Args, F>(self, f: F) -> Self::Out<Args, F> {
        (
            self.0,
            self.1,
            FnHandler {
                f,
                _marker: PhantomData,
            },
        )
    }
}

// pub trait WithHandler<Fd> {
//     fn with_handler<Args, F>(self, f: F) -> TriSubscriber<Fd, Args, F>;
// }

// impl<Fd> WithHandler<Fd> for (Interest, Fd) where Fd: AsFd {
//     fn with_handler<Args, F>(self, f: F) -> TriSubscriber<Fd, Args, F> {
//         TriSubscriber {
//             fd: self.1,
//             interest: Cell::new(self.0),
//             handler: FnHandler {
//                 f,
//                 _marker: PhantomData,
//             },
//         }
//     }
// }

/// Creates a new, empty `Interest` set.
///
/// This is a convenience function equivalent to `Interest::default()`.
/// It's the starting point for building an interest set using the fluent API.
///
/// # Examples
///
/// ```
/// # use eventp::interest;
/// let interest = interest().read().write();
/// assert!(interest.is_readable());
/// assert!(interest.is_writable());
/// ```
pub const fn interest() -> Interest {
    Interest::new(EpollFlags::empty())
}
