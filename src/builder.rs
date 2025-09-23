use std::cell::Cell;
use std::marker::PhantomData;
use std::os::fd::{AsFd, BorrowedFd};

use nix::sys::epoll::EpollFlags;

use crate::interests::Interests;
use crate::subscriber::{Handler, WithInterests};
use crate::Eventp;

pub struct FdWithInterests<Fd> {
    pub(crate) fd: Fd,
    pub(crate) interests: Interests,
}

impl<Fd: AsFd> FdWithInterests<Fd> {
    pub fn finish<T, F>(self, f: F) -> Subscriber1<Fd, T, F> {
        Subscriber1 {
            fd: self.fd,
            interests: self.interests,
            handler: FnHandler {
                f,
                _marker: PhantomData,
            },
        }
    }
}

pub struct FnHandler<T, F> {
    f: F,
    _marker: PhantomData<fn(T)>,
}

pub struct Subscriber1<Fd, T, F> {
    pub(crate) fd: Fd,
    pub(crate) interests: Interests,
    pub(crate) handler: FnHandler<T, F>,
}

impl<Fd, T, F> AsFd for Subscriber1<Fd, T, F>
where
    Fd: AsFd,
{
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl<Fd, T, F> WithInterests for Subscriber1<Fd, T, F> {
    fn interests(&self) -> &Cell<EpollFlags> {
        self.interests.interests()
    }
}

// 0
impl<Fd, F> Handler for Subscriber1<Fd, (), F>
where
    F: FnMut(),
    Fd: AsFd,
{
    fn handle(&mut self, _events: EpollFlags, _eventp: &mut Eventp) {
        (self.handler.f)()
    }
}

// 1
impl<Fd, F> Handler for Subscriber1<Fd, (&mut Fd,), F>
where
    F: FnMut(&mut Fd),
    Fd: AsFd,
{
    fn handle(&mut self, _events: EpollFlags, _eventp: &mut Eventp) {
        (self.handler.f)(&mut self.fd)
    }
}

// 2
impl<Fd, F> Handler for Subscriber1<Fd, (EpollFlags,), F>
where
    F: FnMut(EpollFlags),
    Fd: AsFd,
{
    fn handle(&mut self, events: EpollFlags, _eventp: &mut Eventp) {
        (self.handler.f)(events)
    }
}

// 3
impl<Fd, F> Handler for Subscriber1<Fd, (&mut Eventp,), F>
where
    F: FnMut(&mut Eventp),
    Fd: AsFd,
{
    fn handle(&mut self, _events: EpollFlags, eventp: &mut Eventp) {
        (self.handler.f)(eventp)
    }
}

// 12
impl<Fd, F> Handler for Subscriber1<Fd, (&mut Fd, EpollFlags), F>
where
    F: FnMut(&mut Fd, EpollFlags),
    Fd: AsFd,
{
    fn handle(&mut self, events: EpollFlags, _eventp: &mut Eventp) {
        (self.handler.f)(&mut self.fd, events)
    }
}

// 13
impl<Fd, F> Handler for Subscriber1<Fd, (&mut Fd, &mut Eventp), F>
where
    F: FnMut(&mut Fd, &mut Eventp),
    Fd: AsFd,
{
    fn handle(&mut self, _events: EpollFlags, eventp: &mut Eventp) {
        (self.handler.f)(&mut self.fd, eventp)
    }
}

// 23
impl<Fd, F> Handler for Subscriber1<Fd, (EpollFlags, &mut Eventp), F>
where
    F: FnMut(EpollFlags, &mut Eventp),
    Fd: AsFd,
{
    fn handle(&mut self, events: EpollFlags, eventp: &mut Eventp) {
        (self.handler.f)(events, eventp)
    }
}

// 123
impl<Fd, F> Handler for Subscriber1<Fd, (&mut Fd, EpollFlags, &mut Eventp), F>
where
    F: FnMut(&mut Fd, EpollFlags, &mut Eventp),
    Fd: AsFd,
{
    fn handle(&mut self, events: EpollFlags, eventp: &mut Eventp) {
        (self.handler.f)(&mut self.fd, events, eventp)
    }
}

pub struct Subscriber2<S> {
    pub(crate) interests: Interests,
    pub(crate) fd_with_handler: S,
}

impl<S> WithInterests for Subscriber2<S> {
    fn interests(&self) -> &Cell<EpollFlags> {
        self.interests.interests()
    }
}

impl<S: AsFd> AsFd for Subscriber2<S> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd_with_handler.as_fd()
    }
}

impl<S: AsFd + Handler> Handler for Subscriber2<S> {
    fn handle(&mut self, events: EpollFlags, eventp: &mut Eventp) {
        self.fd_with_handler.handle(events, eventp);
    }
}
