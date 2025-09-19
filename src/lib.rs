mod r#dyn;
mod utils;

use std::cell::Cell;
use std::os::fd::AsRawFd;

use nix::sys::epoll::EpollFlags;


pub trait Subscriber<Ep>: Handler<Ep> + WithInterests + AsRawFd {}

impl<S, Ep> Subscriber<Ep> for S where S: Handler<Ep> + WithInterests + AsRawFd {}

pub trait WithInterests {
    fn interests(&self) -> &Cell<EpollFlags>;
}

pub trait Handler<Ep> {
    fn handle(&mut self, events: EpollFlags, eventp: &mut Ep);
}
