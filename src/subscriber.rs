use std::cell::Cell;
use std::io;
use std::os::fd::AsFd;

use crate::{Event, EventpOps, Interest, Pinned, Registry};

pub trait Subscriber<Ep: EventpOps>: AsFd + HasInterest + Handler<Ep> {
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
    S: AsFd + HasInterest + Handler<Ep>,
    Ep: EventpOps,
{
}

pub trait HasInterest {
    fn interest(&self) -> &Cell<Interest>;
}

pub trait Handler<Ep: EventpOps> {
    fn handle(&mut self, event: Event, eventp: Pinned<'_, Ep>);
}
