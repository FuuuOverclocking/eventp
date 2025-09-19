use std::cell::Cell;
use std::os::fd::AsRawFd;

use nix::sys::epoll::EpollFlags;

pub trait Subscriber: Handler<Eventp = Self::Ep> + WithInterests + AsRawFd {
    type Ep;
}

impl<S, E> Subscriber for S where S: Handler<Eventp = E> + WithInterests + AsRawFd {
    type Ep = E;
}

pub trait WithInterests {
    fn interests(&self) -> &Cell<EpollFlags>;
}

pub trait Handler {
    type Eventp;

    fn handle(&mut self, events: EpollFlags, eventp: &mut Self::Eventp);
}
