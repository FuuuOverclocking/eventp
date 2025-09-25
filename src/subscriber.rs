use std::cell::Cell;
use std::io;
use std::os::fd::AsFd;

use crate::{Event, EventpOps, Interest, Pinned, Registry};

pub trait Subscriber<E: EventpOps>: AsFd + WithInterest + Handler<E> {
    fn register_into<R>(self, eventp: &mut R) -> io::Result<()>
    where
        Self: Sized,
        R: Registry<Ep = E>,
    {
        eventp.register(self)
    }
}

impl<S, E> Subscriber<E> for S
where
    S: AsFd + WithInterest + Handler<E>,
    E: EventpOps,
{
}

pub trait WithInterest {
    fn interest(&self) -> &Cell<Interest>;
}

pub trait Handler<E: EventpOps> {
    fn handle(&mut self, event: Event, interest: Interest, eventp: Pinned<'_, E>);
}
