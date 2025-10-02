use std::io;

use crate::thin::ThinBoxSubscriber;
use crate::{EventpOps, Pinned, Subscriber};

/// A trait for types that can register subscribers.
///
/// # Relationship with [`EventpOps`]
///
/// The `Registry` trait is implemented for types that implement `EventpOps` and for
/// [`Pinned<'_, impl EventpOps>`].
///
/// For example, since the types [`Eventp`] and [`MockEventp`] implement `EventpOps`,
/// they also implement `Registry`. Similarly, `Pinned<'_, Eventp>` and
/// `Pinned<'_, MockEventp>` also implement `Registry`.
///
/// [`Eventp`]: crate::Eventp
/// [`MockEventp`]: crate::MockEventp
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
