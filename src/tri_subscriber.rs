use std::cell::Cell;
use std::marker::PhantomData;
use std::os::fd::{AsFd, BorrowedFd};
use std::pin::Pin;

use crate::{Event, EventpOps, Handler, Interest, WithInterest};

pub struct FdWithInterest<Fd> {
    pub(crate) fd: Fd,
    pub(crate) interest: Interest,
}

impl<Fd: AsFd> FdWithInterest<Fd> {
    pub fn with_handler<T, F>(self, f: F) -> TriSubscriber<Fd, T, F> {
        TriSubscriber {
            fd: self.fd,
            interest: Cell::new(self.interest),
            handler: FnHandler {
                f,
                _marker: PhantomData,
            },
        }
    }
}

pub struct FnHandler<Args, F> {
    f: F,
    _marker: PhantomData<fn(Args)>,
}

pub struct TriSubscriber<Fd, Args, F> {
    pub(crate) fd: Fd,
    pub(crate) interest: Cell<Interest>,
    pub(crate) handler: FnHandler<Args, F>,
}

impl<Fd, Args, F> AsFd for TriSubscriber<Fd, Args, F>
where
    Fd: AsFd,
{
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl<Fd, Args, F> WithInterest for TriSubscriber<Fd, Args, F> {
    fn interest(&self) -> &Cell<Interest> {
        &self.interest
    }
}

impl<Ep, Fd, F> Handler<Ep> for TriSubscriber<Fd, (), F>
where
    Ep: EventpOps,
    Fd: AsFd,
    F: FnMut(),
{
    fn handle(&mut self, _event: Event, _interest: Interest, _eventp: Pin<&mut Ep>) {
        (self.handler.f)()
    }
}

impl<Ep, Fd, F> Handler<Ep> for TriSubscriber<Fd, (&mut Fd,), F>
where
    Ep: EventpOps,
    Fd: AsFd,
    F: FnMut(&mut Fd),
{
    fn handle(&mut self, _event: Event, _interest: Interest, _eventp: Pin<&mut Ep>) {
        (self.handler.f)(&mut self.fd)
    }
}

impl<Ep, Fd, F> Handler<Ep> for TriSubscriber<Fd, (Event,), F>
where
    Ep: EventpOps,
    Fd: AsFd,
    F: FnMut(Event),
{
    fn handle(&mut self, event: Event, _interest: Interest, _eventp: Pin<&mut Ep>) {
        (self.handler.f)(event)
    }
}

impl<Ep, Fd, F> Handler<Ep> for TriSubscriber<Fd, (Interest,), F>
where
    Ep: EventpOps,
    Fd: AsFd,
    F: FnMut(Interest),
{
    fn handle(&mut self, _event: Event, interest: Interest, _eventp: Pin<&mut Ep>) {
        (self.handler.f)(interest)
    }
}

impl<Ep, Fd, F> Handler<Ep> for TriSubscriber<Fd, (Pin<&mut Ep>,), F>
where
    Ep: EventpOps,
    Fd: AsFd,
    F: FnMut(Pin<&mut Ep>),
{
    fn handle(&mut self, _event: Event, _interest: Interest, eventp: Pin<&mut Ep>) {
        (self.handler.f)(eventp)
    }
}

impl<Ep, Fd, F> Handler<Ep> for TriSubscriber<Fd, (&mut Fd, Event,), F>
where
    Ep: EventpOps,
    Fd: AsFd,
    F: FnMut(&mut Fd, Event),
{
    fn handle(&mut self, event: Event, _interest: Interest, _eventp: Pin<&mut Ep>) {
        (self.handler.f)(&mut self.fd, event)
    }
}
