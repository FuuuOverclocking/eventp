use std::io;
use std::os::fd::RawFd;

use crate::thin::ThinBoxSubscriber;
use crate::Interest;

/// A trait for types that can add subscribers, modify interests, and delete subscribers.
///
/// # Primary Implementors
///
/// In this crate, [`Eventp`] and [`MockEventp`] are the primary implementors.
/// Therefore, you should prefer using the abstract `EventpOps` over the concrete
/// `Eventp` in function signatures to make them more testable.
///
/// # Relationship with [`Registry`]
///
/// The [`Registry`] trait is implemented for types that implement `EventpOps` and for
/// [`Pinned<'_, impl EventpOps>`].
///
/// For example, since the types [`Eventp`] and [`MockEventp`] implement `EventpOps`,
/// they also implement [`Registry`]. Similarly, `Pinned<'_, Eventp>` and
/// `Pinned<'_, MockEventp>` also implement `Registry`.
///
/// [`Eventp`]: crate::Eventp
/// [`MockEventp`]: crate::MockEventp
/// [`Registry`]: crate::Registry
/// [`Pinned<'_, impl EventpOps>`]: crate::Pinned
pub trait EventpOps: Sized {
    fn add(&mut self, subscriber: ThinBoxSubscriber<Self>) -> io::Result<()>;
    fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()>;
    fn delete(&mut self, fd: RawFd) -> io::Result<()>;
}
