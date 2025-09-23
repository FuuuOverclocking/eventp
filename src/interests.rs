use std::cell::Cell;
use std::os::fd::AsFd;

use nix::sys::epoll::EpollFlags;

use crate::builder::{FdWithInterests, Subscriber2};
use crate::subscriber::{Handler, WithInterests};

#[derive(Debug, Clone)]
pub struct Interests {
    interests: Cell<EpollFlags>,
}

impl Default for Interests {
    fn default() -> Self {
        Self {
            interests: Cell::new(EpollFlags::empty()),
        }
    }
}

impl Interests {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn with_fd<Fd>(self, fd: Fd) -> FdWithInterests<Fd>
    where
        Fd: AsFd,
    {
        FdWithInterests {
            fd,
            interests: self,
        }
    }

    pub fn finish<S>(self, fd_with_handler: S) -> Subscriber2<S>
    where
        S: AsFd + Handler,
    {
        Subscriber2 {
            interests: self,
            fd_with_handler,
        }
    }

    fn add(self, flags: EpollFlags) -> Self {
        let new_flags = self.interests.get() | flags;
        Self {
            interests: Cell::new(new_flags),
        }
    }

    pub fn read(self) -> Self {
        self.add(EpollFlags::EPOLLIN)
    }

    pub fn write(self) -> Self {
        self.add(EpollFlags::EPOLLOUT)
    }

    pub fn read_write(self) -> Self {
        self.add(EpollFlags::EPOLLIN | EpollFlags::EPOLLOUT)
    }

    pub fn rdhup(self) -> Self {
        self.add(EpollFlags::EPOLLRDHUP)
    }

    pub fn pri(self) -> Self {
        self.add(EpollFlags::EPOLLPRI)
    }

    /// Set edge-triggered mode. Note: level-triggered mode is the default and cannot be added.
    pub fn edge_triggered(self) -> Self {
        self.add(EpollFlags::EPOLLET)
    }

    pub fn oneshot(self) -> Self {
        self.add(EpollFlags::EPOLLONESHOT)
    }

    pub fn wakeup(self) -> Self {
        self.add(EpollFlags::EPOLLWAKEUP)
    }

    pub fn exclusive(self) -> Self {
        self.add(EpollFlags::EPOLLEXCLUSIVE)
    }
}
pub fn interests() -> Interests {
    Default::default()
}

impl WithInterests for Interests {
    fn interests(&self) -> &Cell<EpollFlags> {
        &self.interests
    }
}
