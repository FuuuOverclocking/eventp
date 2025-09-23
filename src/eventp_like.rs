use super::*;

#[cfg_attr(feature = "mock", mockall::automock)]
pub trait EventpLike {
    fn add(&mut self, subscriber: ThinBoxSubscriber) -> io::Result<()>;
    fn modify(&mut self, fd: RawFd, interests: EpollFlags) -> io::Result<()>;
    fn delete(&mut self, fd: RawFd) -> io::Result<()>;
    fn run(&mut self) -> io::Result<()>;
    fn run_with_timeout(&mut self, timeout: EpollTimeout) -> io::Result<()>;
}
