use std::io;
use std::os::fd::RawFd;

use crate::epoll::EpollTimeout;
use crate::{Interest, ThinBoxSubscriber};

#[cfg_attr(feature = "mock", mockall::automock)]
pub trait EventpLike: Sized {
    fn add(&mut self, subscriber: ThinBoxSubscriber<Self>) -> io::Result<()>;
    fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()>;
    fn delete(&mut self, fd: RawFd) -> io::Result<()>;
    fn run(&mut self) -> io::Result<()>;
    fn run_with_timeout(&mut self, timeout: EpollTimeout) -> io::Result<()>;
}
