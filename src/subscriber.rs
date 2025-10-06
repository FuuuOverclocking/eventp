//! [`Subscriber`] represents types that can be registered with an [`Eventp`](crate::Eventp)
//! to receive and handle I/O events.
//!
//! It is composed of 3 parts:
//!
//! - [`AsFd`] - Can borrow a file descriptor.
//! - [`HasInterest`] - Provides the interest in I/O readiness events for the fd, held in a `Cell`.
//! - [`Handler`] - Can handle the triggered events.
//!
//! You do not have to implement [`Subscriber`] manually. It is automatically implemented
//! for any type that implements these three traits.
//!
//! The **most common** approach is not to create a new type and implement them, but to start
//! a method chain with [`interest()`](crate::interest), which would be simpler and more
//! testable.

use std::cell::Cell;
use std::io;
use std::os::fd::AsFd;

use crate::{Event, EventpOps, Interest, Pinned, Registry};

/// See [module level docs](self) for more information.
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

/// See [module level docs](self) for more information.
pub trait HasInterest {
    /// Returns the interest in IO-readiness event.
    fn interest(&self) -> &Cell<Interest>;
}

/// See [module level docs](self) for more information.
pub trait Handler<Ep: EventpOps> {
    /// Handle the triggered event
    fn handle(&mut self, event: Event, eventp: Pinned<'_, Ep>);
}
