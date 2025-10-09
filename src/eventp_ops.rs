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
/// `Eventp` in function signatures to make them easy to test.
///
/// # Relationship with [`Registry`]
///
/// Roughly,
///
/// ```rust,ignore
/// Registry = EventpOps + { Pinned<'_, impl EventpOps> }.
/// ```
///
/// In this crate, [`Eventp`] and [`MockEventp`] implement `EventpOps`.
///
/// Thus, [`Eventp`], [`MockEventp`], `Pinned<'_, Eventp>` and `Pinned<'_, MockEventp>`
/// implement [`Registry`].
///
/// [`Eventp`]: crate::Eventp
/// [`MockEventp`]: crate::MockEventp
/// [`Registry`]: crate::Registry
/// [`Pinned<'_, impl EventpOps>`]: crate::Pinned
pub trait EventpOps: EventpOpsAdd<Self> + Sized {
    fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()>;
    fn delete(&mut self, fd: RawFd) -> io::Result<()>;
}

pub trait EventpOpsAdd<Ep: EventpOps> {
    fn add(&mut self, subscriber: ThinBoxSubscriber<Ep>) -> io::Result<()>;
}
