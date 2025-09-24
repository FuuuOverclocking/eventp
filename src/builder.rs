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

pub struct FnHandler<Args, F> {
    f: F,
    _marker: PhantomData<fn(Args)>,
}

pub struct Subscriber1<Fd, Args, F> {
    pub(crate) fd: Fd,
    pub(crate) interest: Cell<Interest>,
    pub(crate) handler: FnHandler<Args, F>,
}

impl<Fd, Args, F> AsFd for Subscriber1<Fd, Args, F>
where
    Fd: AsFd,
{
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl<Fd, Args, F> WithInterest for Subscriber1<Fd, Args, F> {
    fn interest(&self) -> &Cell<Interest> {
        &self.interest
    }
}

pub struct Inputs<'fd, 'ep, Fd, Ep> {
    fd: Option<&'fd mut Fd>,
    event: Option<Event>,
    interest: Option<Interest>,
    eventp: Option<Pin<&'ep mut Ep>>,
}

pub trait FromInputs<'fd, 'ep, Fd, Ep> {
    fn from_inputs(inputs: &mut Inputs<'fd, 'ep, Fd, Ep>) -> Self;
}

impl<'fd, 'ep, Fd, Ep> FromInputs<'fd, 'ep, Fd, Ep> for &'fd mut Fd {
    fn from_inputs(inputs: &mut Inputs<'fd, 'ep, Fd, Ep>) -> Self {
        inputs
            .fd
            .take()
            .expect("The same type parameter declared multiple times.")
    }
}

impl<'fd, 'ep, Fd, Ep> FromInputs<'fd, 'ep, Fd, Ep> for Event {
    fn from_inputs(inputs: &mut Inputs<'fd, 'ep, Fd, Ep>) -> Self {
        inputs
            .event
            .take()
            .expect("The same type parameter declared multiple times.")
    }
}

impl<'fd, 'ep, Fd, Ep> FromInputs<'fd, 'ep, Fd, Ep> for Interest {
    fn from_inputs(inputs: &mut Inputs<'fd, 'ep, Fd, Ep>) -> Self {
        inputs
            .interest
            .take()
            .expect("The same type parameter declared multiple times.")
    }
}

impl<'fd, 'ep, Fd, Ep> FromInputs<'fd, 'ep, Fd, Ep> for Pin<&'ep mut Ep> {
    fn from_inputs(inputs: &mut Inputs<'fd, 'ep, Fd, Ep>) -> Self {
        inputs
            .eventp
            .take()
            .expect("The same type parameter declared multiple times.")
    }
}

impl<Fd, F, E> Handler<E> for Subscriber1<Fd, (), F>
where
    F: FnMut(),
    Fd: AsFd,
    E: EventpOps,
{
    fn handle(&mut self, _event: Event, _interest: Interest, _eventp: Pin<&mut E>) {
        (self.handler.f)()
    }
}

pub trait Call1<'a, 'b, Fd, Ep>: FnMut(Self::T1) {
    type T1: FromInputs<'a, 'b, Fd, Ep>;
}

// impl<'a, 'b, Fd, Ep> Call1<'a, 'b, Fd, Ep> for fn 

impl<Fd, F, Ep> Handler<Ep> for Subscriber1<Fd, (<F as Call1<'_, '_, Fd, Ep>>::T1,), F>
where
    F: for<'a, 'b> Call1<'a, 'b, Fd, Ep>,
    Fd: AsFd,
    Ep: EventpOps,
{
    fn handle(&mut self, event: Event, interest: Interest, eventp: Pin<&mut Ep>) {
        let mut inputs = Inputs {
            fd: Some(&mut self.fd),
            event: Some(event),
            interest: Some(interest),
            eventp: Some(eventp),
        };
        (self.handler.f)(F::T1::from_inputs(&mut inputs))
    }
}

// impl<Fd, F, E, T1, T2> Handler<E> for Subscriber1<Fd, (T1, T2), F>
// where
//     F: FnMut(T1, T2),
//     Fd: AsFd,
//     E: EventpOps,
//     T1: for<'a, 'b> FromInputs<'a, 'b, Fd, E>,
//     T2: for<'a, 'b> FromInputs<'a, 'b, Fd, E>,
// {
//     fn handle(&mut self, event: Event, interest: Interest, eventp: Pin<&mut E>) {
//         let mut inputs = Inputs {
//             fd: Some(&mut self.fd),
//             event: Some(event),
//             interest: Some(interest),
//             eventp: Some(eventp),
//         };
//         (self.handler.f)(T1::from_inputs(&mut inputs), T2::from_inputs(&mut inputs))
//     }
// }

// impl<Fd, F, E, T1, T2, T3> Handler<E> for Subscriber1<Fd, (T1, T2, T3), F>
// where
//     F: FnMut(T1, T2, T3),
//     Fd: AsFd,
//     E: EventpOps,
//     T1: for<'a, 'b> FromInputs<'a, 'b, Fd, E>,
//     T2: for<'a, 'b> FromInputs<'a, 'b, Fd, E>,
//     T3: for<'a, 'b> FromInputs<'a, 'b, Fd, E>,
// {
//     fn handle(&mut self, event: Event, interest: Interest, eventp: Pin<&mut E>) {
//         let mut inputs = Inputs {
//             fd: Some(&mut self.fd),
//             event: Some(event),
//             interest: Some(interest),
//             eventp: Some(eventp),
//         };
//         (self.handler.f)(
//             T1::from_inputs(&mut inputs),
//             T2::from_inputs(&mut inputs),
//             T3::from_inputs(&mut inputs),
//         )
//     }
// }

// impl<Fd, F, E, T1, T2, T3, T4> Handler<E> for Subscriber1<Fd, (T1, T2, T3, T4), F>
// where
//     F: FnMut(T1, T2, T3, T4),
//     Fd: AsFd,
//     E: EventpOps,
//     T1: for<'a, 'b> FromInputs<'a, 'b, Fd, E>,
//     T2: for<'a, 'b> FromInputs<'a, 'b, Fd, E>,
//     T3: for<'a, 'b> FromInputs<'a, 'b, Fd, E>,
//     T4: for<'a, 'b> FromInputs<'a, 'b, Fd, E>,
// {
//     fn handle(&mut self, event: Event, interest: Interest, eventp: Pin<&mut E>) {
//         let mut inputs = Inputs {
//             fd: Some(&mut self.fd),
//             event: Some(event),
//             interest: Some(interest),
//             eventp: Some(eventp),
//         };
//         (self.handler.f)(
//             T1::from_inputs(&mut inputs),
//             T2::from_inputs(&mut inputs),
//             T3::from_inputs(&mut inputs),
//             T4::from_inputs(&mut inputs),
//         )
//     }
// }

// // 123
// impl<Fd, F, E> Handler<E> for Subscriber1<Fd, (&mut Fd, Event, &mut E), F>
// where
//     F: FnMut(&mut Fd, Event, &mut E),
//     Fd: AsFd,
//     E: EventpOps,
// {
//     fn handle(&mut self, event: Event, eventp: &mut E) {
//         (self.handler.f)(&mut self.fd, event, eventp)
//     }
// }

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

impl<S: AsFd + Handler<E>, E: EventpOps> Handler<E> for Subscriber2<S> {
    fn handle(&mut self, event: Event, interest: Interest, eventp: Pin<&mut E>) {
        self.fd_with_handler.handle(event, interest, eventp);
    }
}
