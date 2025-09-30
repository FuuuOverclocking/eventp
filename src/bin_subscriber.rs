use std::cell::Cell;
use std::os::fd::{AsFd, BorrowedFd};

use crate::subscriber::{Handler, HasInterest};
use crate::{Event, EventpOps, Interest, Pinned};

pub struct BinSubscriber<S> {
    pub(crate) interest: Cell<Interest>,
    pub(crate) fd_with_handler: S,
}

impl<S> HasInterest for BinSubscriber<S> {
    fn interest(&self) -> &Cell<Interest> {
        &self.interest
    }
}

impl<S: AsFd> AsFd for BinSubscriber<S> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd_with_handler.as_fd()
    }
}

impl<S: AsFd + Handler<Ep>, Ep: EventpOps> Handler<Ep> for BinSubscriber<S> {
    fn handle(&mut self, event: Event, eventp: Pinned<'_, Ep>) {
        self.fd_with_handler.handle(event, eventp);
    }
}

impl Interest {
    /// Combines this `Interest` with a file descriptor and handler to create a `Subscriber`.
    ///
    /// This is a low-level method for building a subscriber. Higher-level abstractions
    /// like those in `tri_subscriber` are often more convenient.
    pub const fn with_fd_and_handler<S, Ep>(self, fd_with_handler: S) -> BinSubscriber<S>
    where
        S: AsFd + Handler<Ep>,
        Ep: EventpOps,
    {
        BinSubscriber {
            interest: Cell::new(self),
            fd_with_handler,
        }
    }
}
