//! A ternary subscriber, composed of a file descriptor, interest, and a handler.

use std::cell::Cell;
use std::marker::PhantomData;
use std::os::fd::{AsFd, BorrowedFd};

use crate::subscriber::{Handler, HasInterest};
use crate::{Event, EventpOps, Interest, Pinned};

/// A ternary subscriber, composed of a file descriptor, interest, and a handler.
///
/// This is a convenient helper struct, typically created with a builder-like
/// syntax such as:
///
/// ```rust,ignore
/// interest()
///     .edge_triggered()
///     .read()
///     .with_fd(fd)
///     .with_handler(handler)
/// ```
///
/// It is rarely constructed manually.
pub struct TriSubscriber<Fd, Args, F> {
    pub fd: Fd,
    pub interest: Cell<Interest>,
    pub handler: FnHandler<Args, F>,
}

/// A wrapper for `FnMut` closures.
///
/// The generic parameters should rarely be a concern. However, for those interested
/// in the details, `Args` is used to lock in the signature of `F`. A nuance of
/// Rust is that `trait FnMut<Args>` can be implemented multiple times for the
/// same type (though this is rarely done!), which can make it difficult for
/// static analysis to determine the signature of a type as a closure.
pub struct FnHandler<Args, F> {
    f: F,
    _marker: PhantomData<fn(Args)>,
}

impl<Fd, Args, F> AsFd for TriSubscriber<Fd, Args, F>
where
    Fd: AsFd,
{
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl<Fd, Args, F> HasInterest for TriSubscriber<Fd, Args, F> {
    fn interest(&self) -> &Cell<Interest> {
        &self.interest
    }
}

impl Interest {
    /// Combines this `Interest` with a file descriptor.
    ///
    /// This is a convenience method for chaining calls.
    pub const fn with_fd<Fd: AsFd>(self, fd: Fd) -> (Self, Fd) {
        (self, fd)
    }

    /// Combines this Interest with the event handler.
    ///
    /// This is a convenience method for chaining calls.
    pub const fn with_handler<Args, F>(self, f: F) -> (Self, FnHandler<Args, F>) {
        (
            self,
            FnHandler {
                f,
                _marker: PhantomData,
            },
        )
    }
}

/// A trait for types that can be combined with a file descriptor.
pub trait WithFd {
    /// The resulting output type after combining with a file descriptor.
    type Out<Fd>;

    /// Combines `self` with a file descriptor.
    fn with_fd<Fd: AsFd>(self, fd: Fd) -> Self::Out<Fd>;
}

impl<Args, F> WithFd for (Interest, FnHandler<Args, F>) {
    type Out<Fd> = TriSubscriber<Fd, Args, F>;

    fn with_fd<Fd: AsFd>(self, fd: Fd) -> Self::Out<Fd> {
        TriSubscriber {
            fd,
            interest: Cell::new(self.0),
            handler: self.1,
        }
    }
}

/// A trait for types that can be combined with a handler.
pub trait WithHandler {
    /// The resulting output type after combining with a handler.
    type Out<Args, F>;

    /// Combines `self` with an event handler.
    fn with_handler<Args, F>(self, f: F) -> Self::Out<Args, F>;
}

impl<Fd: AsFd> WithHandler for (Interest, Fd) {
    type Out<Args, F> = TriSubscriber<Fd, Args, F>;

    fn with_handler<Args, F>(self, f: F) -> Self::Out<Args, F> {
        TriSubscriber {
            fd: self.1,
            interest: Cell::new(self.0),
            handler: FnHandler {
                f,
                _marker: PhantomData,
            },
        }
    }
}

impl<Ep, Fd, F> Handler<Ep> for TriSubscriber<Fd, (), F>
where
    Ep: EventpOps,
    Fd: AsFd,
    F: FnMut(),
{
    fn handle(&mut self, _event: Event, _eventp: Pinned<'_, Ep>) {
        (self.handler.f)()
    }
}

macro_rules! expand_param_type {
    (fd) => { &mut Fd };
    (event) => { crate::Event };
    (interest) => { crate::Interest };
    (eventp) => { Pinned<'_, Ep> };
}

macro_rules! impl_handler {
    (@build_call ($s:ident, $e:ident, $i:ident, $ep:ident) -> @args( $($processed:expr,)* ) fd, $($tail:ident,)*) => {
        impl_handler!(@build_call ($s, $e, $i, $ep) -> @args( $($processed,)* &mut $s.fd, ) $($tail,)*)
    };
    (@build_call ($s:ident, $e:ident, $i:ident, $ep:ident) -> @args( $($processed:expr,)* ) event, $($tail:ident,)*) => {
        impl_handler!(@build_call ($s, $e, $i, $ep) -> @args( $($processed,)* $e, ) $($tail,)*)
    };
    (@build_call ($s:ident, $e:ident, $i:ident, $ep:ident) -> @args( $($processed:expr,)* ) interest, $($tail:ident,)*) => {
        impl_handler!(@build_call ($s, $e, $i, $ep) -> @args( $($processed,)* $i.interest.get(), ) $($tail,)*)
    };
    (@build_call ($s:ident, $e:ident, $i:ident, $ep:ident) -> @args( $($processed:expr,)* ) eventp, $($tail:ident,)*) => {
        impl_handler!(@build_call ($s, $e, $i, $ep) -> @args( $($processed,)* $ep, ) $($tail,)*)
    };
    (@build_call ($s:ident, $e:ident, $i:ident, $ep:ident) -> @args( $($processed:expr,)* )) => {
        ($s.handler.f)($($processed),*)
    };

    ( $( $param:ident ),+ ) => {
        impl<Ep, Fd, F> Handler<Ep> for TriSubscriber<Fd, ( $( expand_param_type!($param), )* ), F>
        where
            Ep: EventpOps,
            Fd: AsFd,
            F: FnMut( $( expand_param_type!($param), )* ),
        {
            #[allow(unused_variables)]
            fn handle(&mut self, event: Event, eventp: Pinned<'_, Ep>) {
                impl_handler!(@build_call (self, event, self, eventp) -> @args() $($param,)*);
            }
        }
    };
}

// 1 parameter (4 variants)
impl_handler!(fd);
impl_handler!(event);
impl_handler!(interest);
impl_handler!(eventp);

// 2 parameters (4P2 = 12 variants)
impl_handler!(fd, event);
impl_handler!(fd, interest);
impl_handler!(fd, eventp);
impl_handler!(event, fd);
impl_handler!(event, interest);
impl_handler!(event, eventp);
impl_handler!(interest, fd);
impl_handler!(interest, event);
impl_handler!(interest, eventp);
impl_handler!(eventp, fd);
impl_handler!(eventp, event);
impl_handler!(eventp, interest);

// 3 parameters (4P3 = 24 variants)
impl_handler!(fd, event, interest);
impl_handler!(fd, event, eventp);
impl_handler!(fd, interest, event);
impl_handler!(fd, interest, eventp);
impl_handler!(fd, eventp, event);
impl_handler!(fd, eventp, interest);
impl_handler!(event, fd, interest);
impl_handler!(event, fd, eventp);
impl_handler!(event, interest, fd);
impl_handler!(event, interest, eventp);
impl_handler!(event, eventp, fd);
impl_handler!(event, eventp, interest);
impl_handler!(interest, fd, event);
impl_handler!(interest, fd, eventp);
impl_handler!(interest, event, fd);
impl_handler!(interest, event, eventp);
impl_handler!(interest, eventp, fd);
impl_handler!(interest, eventp, event);
impl_handler!(eventp, fd, event);
impl_handler!(eventp, fd, interest);
impl_handler!(eventp, event, fd);
impl_handler!(eventp, event, interest);
impl_handler!(eventp, interest, fd);
impl_handler!(eventp, interest, event);

// 4 parameters (4P4 = 24 variants)
impl_handler!(fd, event, interest, eventp);
impl_handler!(fd, event, eventp, interest);
impl_handler!(fd, interest, event, eventp);
impl_handler!(fd, interest, eventp, event);
impl_handler!(fd, eventp, event, interest);
impl_handler!(fd, eventp, interest, event);
impl_handler!(event, fd, interest, eventp);
impl_handler!(event, fd, eventp, interest);
impl_handler!(event, interest, fd, eventp);
impl_handler!(event, interest, eventp, fd);
impl_handler!(event, eventp, fd, interest);
impl_handler!(event, eventp, interest, fd);
impl_handler!(interest, fd, event, eventp);
impl_handler!(interest, fd, eventp, event);
impl_handler!(interest, event, fd, eventp);
impl_handler!(interest, event, eventp, fd);
impl_handler!(interest, eventp, fd, event);
impl_handler!(interest, eventp, event, fd);
impl_handler!(eventp, fd, event, interest);
impl_handler!(eventp, fd, interest, event);
impl_handler!(eventp, event, fd, interest);
impl_handler!(eventp, event, interest, fd);
impl_handler!(eventp, interest, fd, event);
impl_handler!(eventp, interest, event, fd);
