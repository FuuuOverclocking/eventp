use std::{cell::Cell, pin::Pin};
use std::io;
use std::os::fd::AsFd;

use crate::{Event, EventpOps, Interest, ThinBoxSubscriber};

pub trait Registry {
    type Ep: EventpOps;

    fn register<S>(self, subscriber: S) -> io::Result<()>
    where
        S: Subscriber<Self::Ep>;
}

impl<E: EventpOps> Registry for &mut E {
    type Ep = E;

    fn register<S>(self, subscriber: S) -> io::Result<()>
    where
        S: Subscriber<Self::Ep>,
    {
        self.add(ThinBoxSubscriber::new(subscriber))
    }
}

impl<E: EventpOps> Registry for Pin<&mut E> {
    type Ep = E;

    fn register<S>(self, subscriber: S) -> io::Result<()>
    where
        S: Subscriber<Self::Ep>,
    {
        self.add_pinned(ThinBoxSubscriber::new(subscriber))
    }
}

pub trait Subscriber<E: EventpOps>: AsFd + WithInterest + Handler<E> {
    fn register_into<R>(self, eventp: R) -> io::Result<()>
    where
        Self: Sized,
        R: Registry<Ep = E>
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
    fn handle(&mut self, event: Event, eventp: &mut E);
}
