use std::io;
use std::os::fd::RawFd;
use std::pin::Pin;

use crate::{Interest, ThinBoxSubscriber};

#[cfg_attr(feature = "mock", mockall::automock)]
pub trait EventpOps: Sized {
    fn add(&mut self, subscriber: ThinBoxSubscriber<Self>) -> io::Result<()>;
    fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()>;
    fn delete(&mut self, fd: RawFd) -> io::Result<()>;

    fn add_pinned(self: Pin<&mut Self>, subscriber: ThinBoxSubscriber<Self>) -> io::Result<()> {
        unsafe { self.get_unchecked_mut().add(subscriber) }
    }

    fn modify_pinned(self: Pin<&mut Self>, fd: RawFd, interest: Interest) -> io::Result<()> {
        unsafe { self.get_unchecked_mut().modify(fd, interest) }
    }

    fn delete_pinned(self: Pin<&mut Self>, fd: RawFd) -> io::Result<()> {
        unsafe { self.get_unchecked_mut().delete(fd) }
    }
}
