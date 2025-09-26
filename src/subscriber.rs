use std::cell::Cell;
use std::io;
use std::os::fd::AsFd;

use crate::{Event, EventpOps, Interest, Pinned, Registry};

pub trait Subscriber<Ep: EventpOps>: AsFd + WithInterest + Handler<Ep> {
    fn register_into<R>(self, eventp: &mut R) -> io::Result<()>
    where
        Self: Sized,
        R: Registry<Ep = Ep>,
    {
        eventp.register(self)
    }
}

impl<S, Ep> Subscriber<Ep> for S
where
    S: AsFd + WithInterest + Handler<Ep>,
    Ep: EventpOps,
{
}

pub trait WithInterest {
    fn interest(&self) -> &Cell<Interest>;
}

pub trait Handler<Ep: EventpOps> {
    fn handle(&mut self, event: Event, interest: Interest, eventp: Pinned<'_, Ep>);
}
