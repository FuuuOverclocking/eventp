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

// impl<Fd, F, Ep> Handler<Ep> for TriSubscriber<Fd, (<F as Call1<'_, '_, Fd, Ep>>::T1,), F>
// where
//     F: for<'a, 'b> Call1<'a, 'b, Fd, Ep>,
//     Fd: AsFd,
//     Ep: EventpOps,
// {
//     fn handle(&mut self, event: Event, interest: Interest, eventp: Pin<&mut Ep>) {
//         let mut inputs = Inputs {
//             fd: Some(&mut self.fd),
//             event: Some(event),
//             interest: Some(interest),
//             eventp: Some(eventp),
//         };
//         (self.handler.f)(F::T1::from_inputs(&mut inputs))
//     }
// }

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
