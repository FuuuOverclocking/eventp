use std::cell::Cell;
use std::os::fd::{AsFd, BorrowedFd};

use crate::{Event, EventpOps, Handler, Interest, Pinned, WithInterest};

pub struct BinSubscriber<S> {
    pub(crate) interest: Cell<Interest>,
    pub(crate) fd_with_handler: S,
}

impl<S> WithInterest for BinSubscriber<S> {
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
    fn handle(&mut self, event: Event, interest: Interest, eventp: Pinned<'_, Ep>) {
        self.fd_with_handler.handle(event, interest, eventp);
    }
}
