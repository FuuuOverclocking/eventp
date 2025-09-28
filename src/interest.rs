//! This module defines `Interest`, a type used to specify readiness events
//! for file descriptors when registering with an I/O reactor (like epoll).
//!
//! `Interest` is a type-safe wrapper around raw `epoll` flags, providing a
//! fluent builder-style API to construct the desired set of events to monitor.
//! It also includes query methods to interpret the event set returned by the reactor.

use std::cell::Cell;
use std::os::fd::AsFd;

use crate::bin_subscriber::BinSubscriber;
use crate::epoll::EpollFlags;
use crate::subscriber::Handler;
use crate::EventpOps;

/// Represents interest in I/O readiness events.
///
/// This is a wrapper around `EpollFlags` that provides a fluent API for building
/// an interest set. It can be used to specify what events (e.g., readable,
/// writable) a user is interested in for a particular file descriptor.
///
/// It also serves to interpret the events returned by `epoll_wait`.
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

    /// A private helper to add flags in a const context.
    const fn add(self, flags: EpollFlags) -> Self {
        Self(EpollFlags::from_bits_retain(self.0.bits() | flags.bits()))
    }

    /// A private helper to remove flags in a const context.
    const fn remove(self, flags: EpollFlags) -> Self {
        Self(self.0.difference(flags))
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

    /// Removes readable interest (`EPOLLIN`).
    pub const fn remove_read(self) -> Self {
        self.remove(EpollFlags::EPOLLIN)
    }

    /// Removes writable interest (`EPOLLOUT`).
    pub const fn remove_write(self) -> Self {
        self.remove(EpollFlags::EPOLLOUT)
    }

    /// Removes interest in the peer closing the write half of the connection (`EPOLLRDHUP`).
    pub const fn remove_rdhup(self) -> Self {
        self.remove(EpollFlags::EPOLLRDHUP)
    }

    /// Removes interest in priority events (`EPOLLPRI`).
    pub const fn remove_pri(self) -> Self {
        self.remove(EpollFlags::EPOLLPRI)
    }

    /// Unsets edge-triggered mode (`EPOLLET`), reverting to level-triggered.
    pub const fn remove_edge_triggered(self) -> Self {
        self.remove(EpollFlags::EPOLLET)
    }

    /// Unsets one-shot mode (`EPOLLONESHOT`).
    pub const fn remove_oneshot(self) -> Self {
        self.remove(EpollFlags::EPOLLONESHOT)
    }

    /// Unsets the `EPOLLWAKEUP` flag.
    #[cfg(not(target_arch = "mips"))]
    pub const fn remove_wakeup(self) -> Self {
        self.remove(EpollFlags::EPOLLWAKEUP)
    }

    /// Unsets exclusive wake-up mode (`EPOLLEXCLUSIVE`).
    pub const fn remove_exclusive(self) -> Self {
        self.remove(EpollFlags::EPOLLEXCLUSIVE)
    }
}

/// Creates a new, empty `Interest` set.
///
/// This is a convenience function equivalent to `Interest::default()`.
/// It's the starting point for building an interest set using the fluent API.
pub const fn interest() -> Interest {
    Interest::new(EpollFlags::empty())
}
