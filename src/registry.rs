use std::io;
use std::pin::Pin;

use crate::{EventpOps, Subscriber, ThinBoxSubscriber};

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
