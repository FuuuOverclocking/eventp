use std::io;
use std::os::fd::RawFd;

use crate::{Interest, ThinBoxSubscriber};

pub trait EventpOps: Sized {
    fn add(&mut self, subscriber: ThinBoxSubscriber<Self>) -> io::Result<()>;
    fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()>;
    fn delete(&mut self, fd: RawFd) -> io::Result<()>;
}
