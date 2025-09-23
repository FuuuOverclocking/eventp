use std::cell::Cell;
use std::marker::PhantomData;
use std::os::fd::{AsFd, BorrowedFd};

use crate::{Event, EventpLike, Handler, Interest, WithInterest};

pub struct FdWithInterest<Fd> {
    pub(crate) fd: Fd,
    pub(crate) interest: Interest,
}

impl<Fd: AsFd> FdWithInterest<Fd> {
    pub fn with_handler<T, F>(self, f: F) -> Subscriber1<Fd, T, F> {
        Subscriber1 {
            fd: self.fd,
            interest: Cell::new(self.interest),
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
    pub(crate) interest: Cell<Interest>,
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

impl<Fd, T, F> WithInterest for Subscriber1<Fd, T, F> {
    fn interest(&self) -> &Cell<Interest> {
        &self.interest
    }
}

// 0
impl<Fd, F, E> Handler<E> for Subscriber1<Fd, (), F>
where
    F: FnMut(),
    Fd: AsFd,
    E: EventpLike,
{
    fn handle(&mut self, _event: Event, _eventp: &mut E) {
        (self.handler.f)()
    }
}

// 1
impl<Fd, F, E> Handler<E> for Subscriber1<Fd, (&mut Fd,), F>
where
    F: FnMut(&mut Fd),
    Fd: AsFd,
    E: EventpLike,
{
    fn handle(&mut self, _event: Event, _eventp: &mut E) {
        (self.handler.f)(&mut self.fd)
    }
}

// 2
impl<Fd, F, E> Handler<E> for Subscriber1<Fd, (Event,), F>
where
    F: FnMut(Event),
    Fd: AsFd,
    E: EventpLike,
{
    fn handle(&mut self, event: Event, _eventp: &mut E) {
        (self.handler.f)(event)
    }
}

// 3
// impl<Fd, F, E> Handler<E> for Subscriber1<Fd, (&mut E,), F>
// where
//     F: FnMut(&mut E),
//     Fd: AsFd,
//     E: EventpLike,
// {
//     fn handle(&mut self, _event: Event, eventp: &mut E) {
//         (self.handler.f)(eventp)
//     }
// }

// 12
impl<Fd, F, E> Handler<E> for Subscriber1<Fd, (&mut Fd, Event), F>
where
    F: FnMut(&mut Fd, Event),
    Fd: AsFd,
    E: EventpLike,
{
    fn handle(&mut self, event: Event, _eventp: &mut E) {
        (self.handler.f)(&mut self.fd, event)
    }
}

// 13
impl<Fd, F, E> Handler<E> for Subscriber1<Fd, (&mut Fd, &mut E), F>
where
    F: FnMut(&mut Fd, &mut E),
    Fd: AsFd,
    E: EventpLike,
{
    fn handle(&mut self, _event: Event, eventp: &mut E) {
        (self.handler.f)(&mut self.fd, eventp)
    }
}

// 23
impl<Fd, F, E> Handler<E> for Subscriber1<Fd, (Event, &mut E), F>
where
    F: FnMut(Event, &mut E),
    Fd: AsFd,
    E: EventpLike,
{
    fn handle(&mut self, event: Event, eventp: &mut E) {
        (self.handler.f)(event, eventp)
    }
}

// 123
impl<Fd, F, E> Handler<E> for Subscriber1<Fd, (&mut Fd, Event, &mut E), F>
where
    F: FnMut(&mut Fd, Event, &mut E),
    Fd: AsFd,
    E: EventpLike,
{
    fn handle(&mut self, event: Event, eventp: &mut E) {
        (self.handler.f)(&mut self.fd, event, eventp)
    }
}

pub struct Subscriber2<S> {
    pub(crate) interest: Cell<Interest>,
    pub(crate) fd_with_handler: S,
}

impl<S> WithInterest for Subscriber2<S> {
    fn interest(&self) -> &Cell<Interest> {
        &self.interest
    }
}

impl<S: AsFd> AsFd for Subscriber2<S> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd_with_handler.as_fd()
    }
}

impl<S: AsFd + Handler<E>, E: EventpLike> Handler<E> for Subscriber2<S> {
    fn handle(&mut self, event: Event, eventp: &mut E) {
        self.fd_with_handler.handle(event, eventp);
    }
}
