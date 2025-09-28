use std::io;

use crate::thin::ThinBoxSubscriber;
use crate::{EventpOps, Pinned, Subscriber};

pub trait Registry {
    type Ep: EventpOps;

    fn register<S>(&mut self, subscriber: S) -> io::Result<()>
    where
        S: Subscriber<Self::Ep>;
}

impl<Ep: EventpOps> Registry for Ep {
    type Ep = Ep;

    fn register<S>(&mut self, subscriber: S) -> io::Result<()>
    where
        S: Subscriber<Self::Ep>,
    {
        self.add(ThinBoxSubscriber::new(subscriber))
    }
}

impl<'a, Ep> Registry for Pinned<'a, Ep>
where
    Ep: EventpOps,
{
    type Ep = Ep;

    fn register<S>(&mut self, subscriber: S) -> io::Result<()>
    where
        S: Subscriber<Self::Ep>,
    {
        self.add(ThinBoxSubscriber::new(subscriber))
    }
}
