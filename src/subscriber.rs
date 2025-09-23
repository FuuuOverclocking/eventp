use std::cell::Cell;
use std::io;
use std::os::fd::AsFd;

use crate::{Event, EventpLike, Interest, ThinBoxSubscriber};

pub trait Subscriber<E: EventpLike>: AsFd + WithInterest + Handler<E> {
    fn register_into(self, eventp: &mut E) -> io::Result<()>
    where
        Self: Sized,
    {
        eventp.add(ThinBoxSubscriber::new(self))
    }
}

impl<S, E> Subscriber<E> for S
where
    S: AsFd + WithInterest + Handler<E>,
    E: EventpLike,
{
}

pub trait WithInterest {
    fn interest(&self) -> &Cell<Interest>;
}

pub trait Handler<E: EventpLike> {
    fn handle(&mut self, event: Event, eventp: &mut E);
}
