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
/// # Sealed
///
/// This trait is sealed and cannot be implemented for types outside of this crate.
///
/// [`Eventp`]: crate::Eventp
/// [`MockEventp`]: crate::MockEventp
pub trait EventpOps: EventpOpsAdd<Self> + sealed::Sealed + Sized {
    #[doc = include_str!("../docs/eventp-ops.modify.md")]
    fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()>;

    #[doc = include_str!("../docs/eventp-ops.delete.md")]
    fn delete(&mut self, fd: RawFd) -> io::Result<()>;
}

/// A helper trait that lets [`Subscriber::register_into`] accept both
/// `&mut Ep` and [`Pinned<'_, Ep>`].
///
/// # Sealed
///
/// This trait is sealed and cannot be implemented for types outside of this crate.
///
/// [`Subscriber::register_into`]: crate::Subscriber::register_into
/// [`Pinned<'_, Ep>`]: crate::Pinned
pub trait EventpOpsAdd<Ep: EventpOps>: sealed::Sealed {
    #[doc = include_str!("../docs/eventp-ops.add.md")]
    fn add(&mut self, subscriber: ThinBoxSubscriber<Ep>) -> io::Result<()>;
}

pub(crate) mod sealed {
    pub trait Sealed {}

    impl Sealed for crate::Eventp {}
    impl<Ep: super::EventpOps> Sealed for crate::Pinned<'_, Ep> {}
    #[cfg(feature = "mock")]
    impl Sealed for crate::mock::MockEventp {}
}
