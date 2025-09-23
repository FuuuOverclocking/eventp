use std::cell::Cell;
use std::io;
use std::os::fd::AsFd;

use nix::sys::epoll::EpollFlags;

use crate::{Eventp, EventpLike, ThinBoxSubscriber};

pub trait Subscriber: AsFd + WithInterests + Handler {
    fn register_into(self, eventp: &mut Eventp) -> io::Result<()>
    where
        Self: Sized,
    {
        eventp.add(ThinBoxSubscriber::new(self))
    }
}

impl<S> Subscriber for S where S: AsFd + WithInterests + Handler {}

pub trait WithInterests {
    fn interests(&self) -> &Cell<EpollFlags>;
}

pub trait Handler {
    fn handle(&mut self, events: EpollFlags, eventp: &mut Eventp);
}
