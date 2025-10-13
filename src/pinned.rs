use std::io;
use std::os::fd::RawFd;
use std::pin::Pin;

use crate::thin::ThinBoxSubscriber;
use crate::{EventpOps, EventpOpsAdd, Interest};

/// This involves some magic. For details on the underlying mechanism, see
/// [technical](crate::_technical).
///
/// In essence, this can be treated as a `&mut Eventp`,
/// allowing you to add, modify, and delete subscribers just like an [Eventp](crate::Eventp).
pub struct Pinned<'a, Ep>(pub Pin<&'a mut Ep>);

impl<'a, Ep> Pinned<'a, Ep> {
    pub fn as_mut(&mut self) -> Pinned<'_, Ep> {
        Pinned(self.0.as_mut())
    }
}

impl<'a, Ep: EventpOps> EventpOpsAdd<Ep> for Pinned<'a, Ep> {
    fn add(&mut self, subscriber: ThinBoxSubscriber<Ep>) -> io::Result<()> {
        unsafe { self.0.as_mut().get_unchecked_mut().add(subscriber) }
    }
}

impl<'a, Ep> Pinned<'a, Ep>
where
    Ep: EventpOps,
{
    pub fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()> {
        unsafe { self.0.as_mut().get_unchecked_mut().modify(fd, interest) }
    }

    pub fn delete(&mut self, fd: RawFd) -> io::Result<()> {
        unsafe { self.0.as_mut().get_unchecked_mut().delete(fd) }
    }
}

/// This macro is primarily used in tests with [MockEventp](crate::MockEventp) to
/// create a `Pinned<'_, MockEventp>`.
/// For details on the underlying magic, see [technical](crate::_technical).
///
/// # Examples
///
/// ```rust
/// # use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd};
/// use eventp::{pinned, EventpOps, MockEventp, Pinned};
/// use mockall::predicate::*;
///
/// fn fn_to_test(fd: impl AsFd, mut eventp: Pinned<'_, impl EventpOps>) {
///     // do something..
///     eventp.delete(fd.as_fd().as_raw_fd()).unwrap();
/// }
///
/// let fd = unsafe { BorrowedFd::borrow_raw(1) };
/// let mut mock_eventp = MockEventp::new();
/// mock_eventp
///     .expect_delete()
///     .with(eq(fd.as_raw_fd()))
///     .times(1)
///     .returning(|_| Ok(()));
/// fn_to_test(fd, pinned!(mock_eventp))
/// ```
#[macro_export]
macro_rules! pinned {
    ($value:expr $(,)?) => {
        $crate::Pinned(::std::pin::pin!($value))
    };
}
