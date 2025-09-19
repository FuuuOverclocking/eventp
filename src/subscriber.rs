use std::cell::Cell;
use std::os::fd::AsFd;

use nix::sys::epoll::EpollFlags;

use crate::EventP;

pub trait Subscriber: AsFd + WithInterests + Handler {}

pub trait WithInterests {
    fn interests(&self) -> &Cell<EpollFlags>;
}

pub trait Handler {
    fn handle(&mut self, eventp: &mut EventP, events: EpollFlags);
}
