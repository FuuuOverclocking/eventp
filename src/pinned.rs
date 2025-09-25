use std::io;
use std::os::fd::RawFd;
use std::pin::Pin;

use crate::{EventpOps, Interest, ThinBoxSubscriber};

pub struct Pinned<'a, Ep>(pub Pin<&'a mut Ep>);

impl<'a, Ep> Pinned<'a, Ep>
where
    Ep: EventpOps,
{
    pub fn add(&mut self, subscriber: ThinBoxSubscriber<Ep>) -> io::Result<()> {
        unsafe { self.0.as_mut().get_unchecked_mut().add(subscriber) }
    }

    pub fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()> {
        unsafe { self.0.as_mut().get_unchecked_mut().modify(fd, interest) }
    }

    pub fn delete(&mut self, fd: RawFd) -> io::Result<()> {
        unsafe { self.0.as_mut().get_unchecked_mut().delete(fd) }
    }
}

#[macro_export]
macro_rules! pinned {
    ($value:expr $(,)?) => {
        $crate::Pinned(::std::pin::pin!($value))
    }
}
