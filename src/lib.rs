//! Safe Rust abstraction over Linux's `epoll`, offering a true zero-cost event dispatch mechanism.
//!
//! *Minimum supported Rust version: 1.71.0*
//!
//! # Motivation
//!
//! `epoll` allows the user to associate a custom `u64` with a file descriptor (`fd`) when adding it.
//! This is intended to store the address of an event context object, but in Rust,
//! I've rarely seen it used correctly. Instead, it's often used to store things like the `fd`
//! itself, a `token` ([mio](https://docs.rs/mio/latest/mio/)), or a `subscriber id`
//! ([event_manager](https://docs.rs/event-manager/latest/event_manager/)).
//! This introduces unnecessary overhead of a branch instruction, or even one or two `HashMap` lookups.
//!
//! This is often due to the challenges of safely managing ownership and fat pointers in Rust when
//! interfacing with pointer-based C APIs. This crate aims to demonstrate how to leverage the Rust
//! type system to handle these issues safely and with zero cost, and how to use a few tricks to wrap
//! it all in a fluent, test-friendly API. See the [Technical](_technical) chapter for the principles
//! behind this approach.
//!
//! # Examples
//!
//! See a full example with a demo of writing unit tests on GitHub:
//! [examples/echo-server.rs](https://github.com/FuuuOverclocking/eventp/blob/main/examples/echo-server.rs).
//!
//! ```rust
//! # use std::io;
//! use eventp::{interest, tri_subscriber::WithHandler, Eventp, Subscriber};
//! use nix::sys::eventfd::EventFd;
//!
//! fn thread_main(eventfd: EventFd) -> io::Result<()> {
//!     let mut eventp = Eventp::default();
//!     interest()
//!         .read()
//!         .with_fd(eventfd)
//!         .with_handler(on_eventfd)
//!         .register_into(&mut eventp)?;
//!
//!     eventp.run_forever()
//! }
//!
//! fn on_eventfd(
//!     eventfd: &mut EventFd,
//!     // Other available parameters: Interest, Event, Pinned<'_, impl EventpOps>
//! ) {
//!     // do somethings...
//! }
//! ```
//!
//! The `with_handler` method supports a form of dependency injection for the handler function.
//! You can define a handler that accepts only the parameters it needs, in any order. See the
//! [`tri_subscriber`] module for more details.
//!
//! # Concepts
//!
//! 1.  **The [`Eventp`] Reactor**: The central event loop that manages all I/O sources.
//! 2.  **The [`Subscriber`]**: A combination of an I/O source (anything that is [`AsFd`](std::os::fd::AsFd)),
//!     its event [`Interest`] (e.g., readable, writable), and a [`Handler`](subscriber::Handler) function.
//!     -   [`Interest`] vs [`Event`]: Both wrap [`EpollFlags`]. `Interest` is what you ask the OS to
//!         monitor (e.g., `EPOLLIN`). `Event` is what the OS reports back (e.g., `EPOLLIN | EPOLLHUP`).
//!         The two sets overlap but are not identical.
//!
//! ![subscriber-eventp-relationship](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/subscriber-eventp.svg)
//!
//! # Built-in Subscribers
//!
//! -   [`tri_subscriber`]: The helper subscriber constructed by the builder-like API starting from
//!     [`interest()`], where is the **recommended** API entry point.
//! -   `remote_endpoint` <span class="stab portability" title="Available on crate feature `remote-endpoint` only"><code>remote-endpoint</code></span>:
//!     A remote control for an `Eventp` instance running on another thread, allows sending closures
//!     to the `Eventp` thread to be executed.
//!
//! # Testability and Type Hierarchy
//!
//! ![type-hierarchy](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/type-hierarchy.svg)
//!
//! To make unit testing easier, this crate provides [`MockEventp`], a mock implementation based on
//! [mockall], available under the <span class="stab portability" title="Available on crate feature `mock` only"><code>mock</code></span>
//! feature. Therefore, it's recommended to use the abstract [`EventpOps`] trait in function signatures.
//! This allows your code to accept both the real `Eventp` and `MockEventp`, making it highly testable.
//!
//! [`Pinned<'_, impl EventpOps>`](Pinned) is the handle passed to your handler when an event is triggered.
//! It acts like `&mut impl EventpOps` but prevents operations that could move the underlying Eventp
//! instance (like `std::mem::replace`), thus ensuring memory safety.
//!
//! The diagram also mentions [`EventpOpsAdd`]. You will rarely use this trait directly. It's a helper
//! trait that allows methods like `register_into()` to accept both types.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

mod event;
mod eventp_ops;
mod interest;
#[cfg(feature = "mock")]
pub mod mock;
mod pinned;
#[cfg(feature = "remote-endpoint")]
pub mod remote_endpoint;
pub mod subscriber;
pub mod thin;
pub mod tri_subscriber;

pub mod epoll {
    //! Re-exports of epoll related types from the [`nix` crate](nix::sys::epoll).
    pub use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
}

#[cfg(docsrs)]
pub mod _technical {
    #![doc = include_str!("../docs/technical.md")]
}

#[cfg(docsrs)]
pub mod _technical_zh {
    #![doc = include_str!("../docs/technical.zh.md")]
}

use std::marker::PhantomPinned;
use std::mem::{self, ManuallyDrop, MaybeUninit};
use std::os::fd::{AsRawFd, RawFd};
use std::pin::Pin;
use std::{hint, io, ptr};

use rustc_hash::FxHashMap;

use crate::epoll::*;
pub use crate::event::Event;
pub use crate::eventp_ops::{EventpOps, EventpOpsAdd};
pub use crate::interest::{interest, Interest};
#[cfg(feature = "mock")]
pub use crate::mock::MockEventp;
pub use crate::pinned::Pinned;
#[cfg(feature = "remote-endpoint")]
pub use crate::remote_endpoint::remote_endpoint;
pub use crate::subscriber::Subscriber;
use crate::thin::ThinBoxSubscriber;

const DEFAULT_EVENT_BUF_CAPACITY: usize = 512;

/// The central event loop reactor, built on top of Linux's `epoll`.
///
/// `Eventp` manages a set of registered I/O sources (file descriptors) and their
/// associated interests and handlers. It waits for I/O readiness events and dispatches
/// them to the corresponding handlers.
///
/// See the [crate-level documentation](crate) for a detailed overview of the design,
/// motivation, and key concepts.
pub struct Eventp {
    registered: FxHashMap<RawFd, ThinBoxSubscriber<Eventp>>,
    epoll: Epoll,
    event_buf: Vec<MaybeUninit<EpollEvent>>,
    handling: Option<Handling>,
    _pinned: PhantomPinned,
}

struct Handling {
    fd: RawFd,
    drop_current: bool,
    deferred_drop: Vec<ThinBoxSubscriber<Eventp>>,
}

impl Default for Eventp {
    /// Creates a new `Eventp` with an event buffer capacity of 256 and the
    /// `EPOLL_CLOEXEC` flag set.
    ///
    /// # Panics
    ///
    /// Panics if the underlying `epoll_create1` syscall fails. Use
    /// [`Eventp::new`] if you need to handle that error.
    fn default() -> Self {
        Self::new(DEFAULT_EVENT_BUF_CAPACITY, EpollCreateFlags::EPOLL_CLOEXEC)
            .expect("Failed to create epoll instance")
    }
}

impl Eventp {
    /// Creates a new `Eventp` instance with a specified event buffer
    /// capacity and `epoll_create1` flags.
    ///
    /// `capacity` is the number of [`EpollEvent`] slots reserved for one
    /// `epoll_wait` call, i.e. the maximum number of events that can be
    /// dispatched per [`run_once`](Self::run_once) iteration.
    ///
    /// # Errors
    ///
    /// Returns the underlying `io::Error` if `epoll_create1` fails.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize, flags: EpollCreateFlags) -> io::Result<Self> {
        assert!(capacity > 0, "Capacity must be greater than zero");

        let mut buf = Vec::with_capacity(capacity);
        // SAFETY: `Vec::set_len` requires `new_len <= capacity` and that the
        //         first `new_len` elements be initialized. The element type
        //         here is `MaybeUninit<EpollEvent>`, for which any bit pattern
        //         (including uninit) is a valid value, so the second condition
        //         is trivially satisfied.
        unsafe { buf.set_len(capacity) };

        Ok(Self {
            epoll: Epoll::new(flags).map_err(io::Error::from)?,
            registered: Default::default(),
            event_buf: buf,
            handling: None,
            _pinned: PhantomPinned,
        })
    }

    /// Consumes the `Eventp`, returning the underlying [`Epoll`] handle and
    /// the registry of subscribers, keyed by their raw file descriptor.
    pub fn into_inner(self) -> (Epoll, impl Iterator<Item = ThinBoxSubscriber<Eventp>>) {
        (self.epoll, self.registered.into_values())
    }

    /// Runs the event loop until a non-`EINTR` error occurs.
    ///
    /// This is the typical entry point for starting the event loop. It
    /// repeatedly calls [`run_once`](Self::run_once); if `epoll_wait` is
    /// interrupted by a signal (`EINTR` /
    /// [`io::ErrorKind::Interrupted`]), the loop transparently retries.
    ///
    /// # Errors
    ///
    /// Returns the first `io::Error` from `epoll_wait` that is not
    /// [`io::ErrorKind::Interrupted`]. The function never returns `Ok(())`.
    pub fn run_forever(&mut self) -> io::Result<()> {
        loop {
            match self.run_once() {
                Ok(_) => continue,
                // `epoll_wait` can be interrupted by a signal. This is not a fatal
                // error, so we simply continue the loop.
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Performs one `epoll_wait` with no timeout and dispatches every ready
    /// event to its handler.
    ///
    /// Equivalent to calling
    /// [`run_once_with_timeout`](Self::run_once_with_timeout) with
    /// [`EpollTimeout::NONE`] (i.e. block indefinitely until at least one
    /// event is ready, or until interrupted by a signal).
    ///
    /// # Errors
    ///
    /// Forwards any `io::Error` from `epoll_wait`, including
    /// [`io::ErrorKind::Interrupted`] when the syscall is interrupted by a
    /// signal. Use [`run_forever`](Self::run_forever) if you want `EINTR`
    /// to be retried automatically.
    ///
    /// # Panics
    ///
    /// Panics if called recursively from within an event handler -- see
    /// [`run_once_with_timeout`](Self::run_once_with_timeout).
    pub fn run_once(&mut self) -> io::Result<()> {
        self.run_once_with_timeout(EpollTimeout::NONE)
    }

    /// Performs one `epoll_wait` with the given timeout and dispatches every
    /// ready event to its handler.
    ///
    /// # Errors
    ///
    /// Forwards any `io::Error` from `epoll_wait`.
    ///
    /// # Panics
    ///
    /// Panics if called recursively (i.e. from within an event handler).
    /// Recursing would corrupt the internal `handling` state and risk
    /// invalidating iterators on the registry.
    pub fn run_once_with_timeout(&mut self, timeout: EpollTimeout) -> io::Result<()> {
        if let Some(handling) = &self.handling {
            // Recursive calls would corrupt the `handling` state and could lead to
            // iterator invalidation issues. This panic prevents such misuse.
            panic!(
                "Recursive call to `Eventp::run_once_with_timeout` while handling fd {}",
                handling.fd
            );
        }

        // SAFETY: `EpollEvent` is a POD wrapping `libc::epoll_event`, so any bit
        // pattern is a valid `EpollEvent` value -- meaning `MaybeUninit<EpollEvent>`
        // and `EpollEvent` have the same layout and the latter is sound to read
        // even before the kernel writes into it. We immediately re-slice to the
        // first `n` elements that `epoll_wait` actually wrote, so any consumer of
        // `buf` only observes kernel-initialized entries.
        let buf: &mut [MaybeUninit<EpollEvent>] = &mut self.event_buf;
        let buf: &mut [EpollEvent] = unsafe { mem::transmute(buf) };

        let n = self.epoll.wait(buf, timeout)?;
        let buf = &buf[..n];

        // Enter the 'handling' state to manage re-entrancy safely.
        if self.handling.is_some() {
            // SAFETY: The recursion guard at the top of this function panics if
            //         `self.handling` is `Some`, and `epoll.wait` cannot mutate
            //         `self.handling`. So this branch is unreachable; the assignment
            //         in the `else` branch is the only path that initializes the
            //         field, which avoids an unnecessary drop check on the prior
            //         (`None`) value.
            unsafe { hint::unreachable_unchecked() }
        } else {
            self.handling = Some(Handling {
                fd: -1, // Invalid fd, will be updated for each event.
                drop_current: false,
                deferred_drop: vec![],
            });
        }

        for ev in buf {
            // Reconstruct the subscriber pointer from the `epoll` event data.
            // SAFETY: `addr` was set from a `ThinBoxSubscriber` in `add()` whose
            // owning entry still lives in `self.registered` (or, for an in-flight
            // delete, in `handling.deferred_drop` after a `drop_in_place`). The
            // thin pointer's heap target is therefore still allocated. We wrap
            // the reconstructed value in `ManuallyDrop` because the real owner
            // is elsewhere; if we let `Drop` run -- including during a panic
            // unwind out of `handle()` -- the heap slot would be double-freed.
            let addr = ev.data() as usize;
            let mut subscriber = ManuallyDrop::new(unsafe {
                mem::transmute::<usize, ThinBoxSubscriber<Eventp>>(addr)
            });

            // Update the currently handled fd in the `Handling` state.
            {
                let handling = unsafe { self.handling.as_mut().unwrap_unchecked() };
                handling.fd = *subscriber.raw_fd_ref();
            }

            // Dispatch the event to the subscriber's handler.
            // SAFETY: `Eventp` is `!Unpin` (via `_pinned: PhantomPinned`), so once
            // exposed as `Pin<&mut Eventp>` the handler cannot, in safe code,
            // recover an `&mut Eventp` and `mem::replace` the loop out from under
            // us. We further wrap the pin in `Pinned`, which only re-exposes
            // add/modify/delete, none of which move `self`. The original
            // `&mut self` passed into this function is the unique mutable borrow
            // for the duration of dispatch, so pinning it here is sound.
            if let Some(s) = subscriber.try_deref_mut() {
                s.handle(Event::from(ev), Pinned(unsafe { Pin::new_unchecked(self) }));
            }

            let handling = unsafe { self.handling.as_mut().unwrap_unchecked() };
            if handling.drop_current {
                handling.drop_current = false;

                debug_assert!(handling.fd >= 0, "Invalid fd in handling state.");
                self.registered.remove(&handling.fd);
            }
        }

        // Take the handling state to process deferred removals.
        // SAFETY: `self.handling` is guaranteed to be `Some` at this point.
        unsafe { self.handling.take().unwrap_unchecked() };

        Ok(())
    }
}

impl EventpOpsAdd<Self> for Eventp {
    #[doc = include_str!("../docs/eventp-ops.add.md")]
    fn add(&mut self, mut subscriber: ThinBoxSubscriber<Self>) -> io::Result<()> {
        // Pointer laundering: convert the subscriber's thin pointer into a `usize`
        // so it can be stashed in `epoll_event.data` without a borrow-checker tie.
        // SAFETY: `ThinBoxSubscriber<Self>` consists of a single `NonNull<u8>`
        // field plus a ZST `PhantomData`, giving it the same size and alignment
        // as `usize` on a 64-bit target. `transmute_copy` only requires equal
        // size and is used here (rather than `transmute`) to avoid consuming
        // the value, since we still need to move it into `self.registered`.
        let addr = unsafe { mem::transmute_copy::<_, usize>(&subscriber) };

        let dyn_subscriber = match subscriber.try_deref_mut() {
            Some(s) => s,
            None => panic!("Subscriber is already dropped"),
        };

        let raw_fd = dyn_subscriber.as_fd().as_raw_fd();
        if self.registered.contains_key(&raw_fd) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "subscriber with same fd already registered",
            ));
        }

        let interest = dyn_subscriber.interest().get();

        let epoll_event = EpollEvent::new(interest.bitflags(), addr as u64);
        self.epoll.add(dyn_subscriber.as_fd(), epoll_event)?;

        // Take ownership of the subscriber. This is the only place that owns it.
        self.registered.insert(raw_fd, subscriber);

        Ok(())
    }
}

impl EventpOps for Eventp {
    #[doc = include_str!("../docs/eventp-ops.modify.md")]
    fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()> {
        let subscriber = self
            .registered
            .get_mut(&fd)
            .ok_or(io::Error::new(io::ErrorKind::NotFound, "fd not registered"))?;

        // Perform the same pointer laundering as in `add` to get the address for `epoll_ctl`.
        // SAFETY: see the SAFETY note in `add()` -- `ThinBoxSubscriber` and `usize`
        // have the same size on a 64-bit target.
        let addr = unsafe { mem::transmute_copy::<_, usize>(subscriber) };
        let mut epoll_event = EpollEvent::new(interest.bitflags(), addr as u64);

        // SAFETY: This is a direct FFI call to `epoll_ctl`. The arguments are
        // constructed correctly, so it's as safe as the underlying syscall.
        let ret = unsafe {
            libc::epoll_ctl(
                self.epoll.0.as_raw_fd(),
                libc::EPOLL_CTL_MOD,
                fd,
                &mut epoll_event as *mut _ as _,
            )
        };
        if ret == -1 {
            return Err(io::Error::last_os_error());
        }
        // Update the interest stored within the subscriber itself.
        if let Some(s) = subscriber.try_deref_mut() {
            s.interest().set(interest);
        }

        Ok(())
    }

    #[doc = include_str!("../docs/eventp-ops.delete.md")]
    fn delete(&mut self, fd: RawFd) -> io::Result<()> {
        if !self.registered.contains_key(&fd) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "fd not registered"));
        }

        // Use a direct syscall for `EPOLL_CTL_DEL` as `nix`'s `epoll.delete`
        // requires a `AsFd` source, which we may not have if the source is already dropped.
        // We only need the raw fd.
        // SAFETY: This is a direct FFI call to `epoll_ctl`. The arguments are
        // constructed correctly, so it's as safe as the underlying syscall.
        let ret = unsafe {
            libc::epoll_ctl(
                self.epoll.0.as_raw_fd(),
                libc::EPOLL_CTL_DEL,
                fd,
                ptr::null_mut(),
            )
        };
        if ret == -1 {
            return Err(io::Error::last_os_error());
        }

        if let Some(handling) = &mut self.handling {
            if handling.fd == fd {
                // Delete self while handling. This will actually do the drop
                // after the handler returns.
                handling.drop_current = true;
            } else {
                // Delete another fd while handling.

                // Safe to unwrap, because just checked that it exists.
                let mut subscriber = self.registered.remove(&fd).unwrap();

                // Drop in place immediately. This will not release the heap memory.
                subscriber.drop_in_place();

                // Defer the dealloc to the end of the event dispatch.
                handling.deferred_drop.push(subscriber);
            }
        } else {
            // Otherwise, it's safe to remove immediately.
            self.registered.remove(&fd);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::os::fd::{AsFd, BorrowedFd};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::rc::Rc;
    use std::time::Duration;

    use nix::sys::eventfd::{EfdFlags, EventFd};

    use super::*;
    use crate::subscriber::{Handler, HasInterest};

    fn new_eventfd() -> EventFd {
        EventFd::from_flags(EfdFlags::EFD_CLOEXEC | EfdFlags::EFD_NONBLOCK).unwrap()
    }

    /// Wakes an `eventfd` so the next `epoll_wait` call returns it as ready.
    fn fire(efd: &EventFd) {
        efd.write(1).expect("eventfd write");
    }

    /// Drains an `eventfd` so a level-triggered subscriber doesn't keep firing.
    fn drain(efd: &EventFd) {
        // Ignore WouldBlock; we only care that the kernel-side counter resets.
        let _ = efd.read();
    }

    /// A subscriber backed by an `EventFd` whose handler invokes a closure.
    /// The closure receives the fd's raw value and the `Pinned` reactor handle,
    /// so re-entrancy scenarios (delete/add/modify from inside a handler) can
    /// be exercised directly.
    struct CbSub<F> {
        eventfd: EventFd,
        interest: Cell<Interest>,
        f: F,
    }

    impl<F> AsFd for CbSub<F> {
        fn as_fd(&self) -> BorrowedFd<'_> {
            self.eventfd.as_fd()
        }
    }

    impl<F> HasInterest for CbSub<F> {
        fn interest(&self) -> &Cell<Interest> {
            &self.interest
        }
    }

    impl<F> Handler<Eventp> for CbSub<F>
    where
        F: FnMut(&EventFd, Pinned<'_, Eventp>),
    {
        fn handle(&mut self, _event: Event, eventp: Pinned<'_, Eventp>) {
            // Drain so level-triggered notifications don't repeat within the
            // same batch (each test wires a single, deterministic sequence).
            drain(&self.eventfd);
            (self.f)(&self.eventfd, eventp);
        }
    }

    fn cb_sub<F>(efd: EventFd, f: F) -> CbSub<F>
    where
        F: FnMut(&EventFd, Pinned<'_, Eventp>),
    {
        CbSub {
            eventfd: efd,
            interest: Cell::new(crate::interest().read()),
            f,
        }
    }

    /// A subscriber that *borrows* an fd it does not own. Useful for tests
    /// that need to register the same `RawFd` twice (which is impossible with
    /// `OwnedFd`, since dup yields a new fd number).
    struct BorrowSub {
        raw: RawFd,
        interest: Cell<Interest>,
    }
    impl AsFd for BorrowSub {
        fn as_fd(&self) -> BorrowedFd<'_> {
            // SAFETY: each test that constructs a `BorrowSub` keeps the
            // owning `EventFd` alive for the whole test body.
            unsafe { BorrowedFd::borrow_raw(self.raw) }
        }
    }
    impl HasInterest for BorrowSub {
        fn interest(&self) -> &Cell<Interest> {
            &self.interest
        }
    }
    impl Handler<Eventp> for BorrowSub {
        fn handle(&mut self, _: Event, _: Pinned<'_, Eventp>) {}
    }

    /// Short timeout used by every test; long enough to absorb scheduling
    /// jitter on a busy CI runner, short enough to fail fast on a real bug.
    fn poll_timeout() -> EpollTimeout {
        EpollTimeout::from(500u16)
    }

    #[test]
    fn default_creates_usable_reactor() {
        let mut ep = Eventp::default();
        // No subscribers: epoll_wait should time out cleanly.
        ep.run_once_with_timeout(EpollTimeout::from(10u16)).unwrap();
    }

    #[test]
    #[should_panic]
    fn new_with_zero_capacity_panics() {
        // A zero-length event buffer cannot dispatch anything (`epoll_wait`
        // would also reject it with EINVAL), so the constructor asserts the
        // precondition up front.
        let _ = Eventp::new(0, EpollCreateFlags::EPOLL_CLOEXEC);
    }

    #[test]
    fn add_duplicate_fd_returns_already_exists() {
        let mut ep = Eventp::default();
        let efd = new_eventfd();
        let raw = efd.as_fd().as_raw_fd();

        // First registration owns the fd.
        cb_sub(efd, |_, _| {}).register_into(&mut ep).unwrap();

        // Second registration references the same `RawFd` via a non-owning
        // wrapper. `Eventp::add` must reject it before reaching `epoll_ctl`,
        // returning `AlreadyExists`.
        let err = BorrowSub {
            raw,
            interest: Cell::new(crate::interest().read()),
        }
        .register_into(&mut ep)
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn modify_unknown_fd_returns_not_found() {
        let mut ep = Eventp::default();
        let err = ep.modify(424242, crate::interest().read()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn delete_unknown_fd_returns_not_found() {
        let mut ep = Eventp::default();
        let err = ep.delete(424242).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn ready_event_dispatches_to_handler() {
        let mut ep = Eventp::default();

        let efd = new_eventfd();
        let counter = Rc::new(Cell::new(0u32));
        let c = counter.clone();
        cb_sub(efd, move |_, _| c.set(c.get() + 1))
            .register_into(&mut ep)
            .unwrap();

        // Use the registered fd to wake itself.
        let raw = ep.registered.keys().copied().next().unwrap();
        let dup = unsafe { BorrowedFd::borrow_raw(raw) }
            .try_clone_to_owned()
            .unwrap();
        let writer = unsafe { EventFd::from_owned_fd(dup) };
        fire(&writer);

        ep.run_once_with_timeout(poll_timeout()).unwrap();
        assert_eq!(counter.get(), 1);
    }

    #[test]
    fn handler_can_delete_self_and_subscriber_is_dropped() {
        let mut ep = Eventp::default();

        // Track whether the subscriber's destructor ran by tying it to an Rc
        // that we observe externally.
        let drop_witness = Rc::new(());
        let weak = Rc::downgrade(&drop_witness);

        struct Witness(#[allow(dead_code)] Rc<()>);

        let efd = new_eventfd();
        let raw = efd.as_fd().as_raw_fd();

        let sub = CbSub {
            eventfd: efd,
            interest: Cell::new(crate::interest().read()),
            f: {
                let _w = Witness(drop_witness);
                move |fd: &EventFd, mut ep: Pinned<'_, Eventp>| {
                    drain(fd);
                    // Touching `_w` here keeps it alive until the closure drops.
                    let _ = &_w;
                    ep.delete(fd.as_fd().as_raw_fd()).unwrap();
                }
            },
        };
        sub.register_into(&mut ep).unwrap();

        let dup = unsafe { BorrowedFd::borrow_raw(raw) }
            .try_clone_to_owned()
            .unwrap();
        let writer = unsafe { EventFd::from_owned_fd(dup) };
        fire(&writer);

        ep.run_once_with_timeout(poll_timeout()).unwrap();

        // After dispatch, the registry must no longer contain the fd, and the
        // subscriber (which captured the strong Rc) must have been dropped.
        assert!(!ep.registered.contains_key(&raw));
        assert!(
            weak.upgrade().is_none(),
            "self-delete must drop the subscriber"
        );
    }

    #[test]
    fn handler_can_delete_other_fd_during_dispatch() {
        let mut ep = Eventp::default();

        // Subscriber A: when triggered, deletes B.
        let efd_a = new_eventfd();
        let raw_a = efd_a.as_fd().as_raw_fd();

        // Subscriber B: tracks its own destruction.
        let efd_b = new_eventfd();
        let raw_b = efd_b.as_fd().as_raw_fd();
        let b_drop = Rc::new(());
        let b_weak = Rc::downgrade(&b_drop);

        struct B {
            eventfd: EventFd,
            interest: Cell<Interest>,
            _w: Rc<()>,
        }
        impl AsFd for B {
            fn as_fd(&self) -> BorrowedFd<'_> {
                self.eventfd.as_fd()
            }
        }
        impl HasInterest for B {
            fn interest(&self) -> &Cell<Interest> {
                &self.interest
            }
        }
        impl Handler<Eventp> for B {
            fn handle(&mut self, _: Event, _: Pinned<'_, Eventp>) {}
        }

        B {
            eventfd: efd_b,
            interest: Cell::new(crate::interest().read()),
            _w: b_drop,
        }
        .register_into(&mut ep)
        .unwrap();

        cb_sub(efd_a, move |_, mut ep| {
            ep.delete(raw_b).unwrap();
            // While we are still inside A's handler, B's heap slot must NOT
            // yet be deallocated -- the deferred_drop list keeps it alive
            // until end-of-batch. We can't observe that directly, but we can
            // confirm B's entry was already removed from the registry.
        })
        .register_into(&mut ep)
        .unwrap();

        let dup = unsafe { BorrowedFd::borrow_raw(raw_a) }
            .try_clone_to_owned()
            .unwrap();
        fire(&unsafe { EventFd::from_owned_fd(dup) });

        ep.run_once_with_timeout(poll_timeout()).unwrap();

        assert!(!ep.registered.contains_key(&raw_b));
        assert!(
            b_weak.upgrade().is_none(),
            "B must be deallocated by end of dispatch"
        );
    }

    #[test]
    fn handler_can_re_add_other_fd_after_delete() {
        // CHANGELOG note: deleting another fd from inside a handler removes
        // its registry entry *immediately* (its heap slot is parked on
        // `deferred_drop` and dealloc'd at end-of-batch). That gap allows the
        // same `RawFd` to be re-added inside the same dispatch.
        //
        // Note: the analogous self-delete + self-re-add is NOT supported,
        // because self-delete only sets `drop_current = true` and leaves the
        // registry entry in place until the dispatch loop tail. We pin that
        // contract in `self_delete_then_re_add_same_fd_returns_already_exists`
        // below.
        let mut ep = Eventp::default();

        // Subscriber A wakes B's deletion + re-add.
        let efd_a = new_eventfd();
        let raw_a = efd_a.as_fd().as_raw_fd();

        // B is owned by the test; the subscribers only borrow its fd.
        let efd_b = new_eventfd();
        let raw_b = efd_b.as_fd().as_raw_fd();

        let marker = Rc::new(Cell::new(0u32));
        let m_for_first = marker.clone();
        // First B subscriber: never fires in this test, only its re-add does.
        struct CountingBorrowSub {
            raw: RawFd,
            interest: Cell<Interest>,
            marker: Rc<Cell<u32>>,
        }
        impl AsFd for CountingBorrowSub {
            fn as_fd(&self) -> BorrowedFd<'_> {
                unsafe { BorrowedFd::borrow_raw(self.raw) }
            }
        }
        impl HasInterest for CountingBorrowSub {
            fn interest(&self) -> &Cell<Interest> {
                &self.interest
            }
        }
        impl Handler<Eventp> for CountingBorrowSub {
            fn handle(&mut self, _: Event, _: Pinned<'_, Eventp>) {
                self.marker.set(self.marker.get() + 1);
            }
        }

        CountingBorrowSub {
            raw: raw_b,
            interest: Cell::new(crate::interest().read()),
            marker: m_for_first,
        }
        .register_into(&mut ep)
        .unwrap();

        let m_for_readd = marker.clone();
        cb_sub(efd_a, move |_, mut ep| {
            ep.delete(raw_b).unwrap();
            CountingBorrowSub {
                raw: raw_b,
                interest: Cell::new(crate::interest().read()),
                marker: m_for_readd.clone(),
            }
            .register_into(&mut ep)
            .expect("re-add of same fd within handler must succeed");
        })
        .register_into(&mut ep)
        .unwrap();

        // Trigger A; its handler swaps out B's subscriber.
        let dup_a = unsafe { BorrowedFd::borrow_raw(raw_a) }
            .try_clone_to_owned()
            .unwrap();
        fire(&unsafe { EventFd::from_owned_fd(dup_a) });
        ep.run_once_with_timeout(poll_timeout()).unwrap();
        assert!(ep.registered.contains_key(&raw_b));
        assert_eq!(marker.get(), 0, "B's handler should not have run yet");

        // Now wake B; the *re-added* subscriber should be the one that fires.
        fire(&efd_b);
        ep.run_once_with_timeout(poll_timeout()).unwrap();
        assert_eq!(marker.get(), 1);

        // Drain the registry before `efd_b` goes out of scope so the
        // borrow-style subscriber's fd reference doesn't outlive the owner.
        let _ = ep.into_inner();
        drop(efd_b);
    }

    #[test]
    fn self_delete_then_re_add_same_fd_returns_already_exists() {
        // Documents the *opposite* contract from the test above: the
        // currently-handled subscriber's registry entry survives until the
        // dispatch-loop tail (because self-delete only flips `drop_current`),
        // so an attempt to re-add the same `RawFd` inside the same handler
        // is rejected with `AlreadyExists`. If this behaviour ever changes,
        // the change is breaking and the test must be updated deliberately.
        let mut ep = Eventp::default();
        let efd = new_eventfd();
        let raw = efd.as_fd().as_raw_fd();

        let observed_kind = Rc::new(Cell::new(None::<io::ErrorKind>));
        let ok = observed_kind.clone();
        cb_sub(efd, move |_, mut ep| {
            ep.delete(raw).unwrap();
            let err = BorrowSub {
                raw,
                interest: Cell::new(crate::interest().read()),
            }
            .register_into(&mut ep)
            .unwrap_err();
            ok.set(Some(err.kind()));
        })
        .register_into(&mut ep)
        .unwrap();

        let raw_dup = unsafe { BorrowedFd::borrow_raw(raw) }
            .try_clone_to_owned()
            .unwrap();
        fire(&unsafe { EventFd::from_owned_fd(raw_dup) });
        ep.run_once_with_timeout(poll_timeout()).unwrap();

        assert_eq!(observed_kind.get(), Some(io::ErrorKind::AlreadyExists));
    }

    #[test]
    fn handler_add_new_fd_fires_on_next_iteration() {
        let mut ep = Eventp::default();

        let trigger = new_eventfd();
        let raw_trigger = trigger.as_fd().as_raw_fd();

        // The subscriber added from inside the handler will fire on the next
        // poll. We pre-create its eventfd so we can fire it from outside.
        let added = new_eventfd();
        let added_raw = added.as_fd().as_raw_fd();
        let added_slot = Rc::new(RefCell::new(Some(added)));
        let added_fired = Rc::new(Cell::new(false));
        let af = added_fired.clone();
        let slot = added_slot.clone();

        cb_sub(trigger, move |_, mut ep| {
            let efd = slot.borrow_mut().take().unwrap();
            let af = af.clone();
            cb_sub(efd, move |_, _| af.set(true))
                .register_into(&mut ep)
                .unwrap();
        })
        .register_into(&mut ep)
        .unwrap();

        let dup = unsafe { BorrowedFd::borrow_raw(raw_trigger) }
            .try_clone_to_owned()
            .unwrap();
        fire(&unsafe { EventFd::from_owned_fd(dup) });
        ep.run_once_with_timeout(poll_timeout()).unwrap();
        assert!(
            !added_fired.get(),
            "newly-added fd must not fire in same batch"
        );

        // Now wake the newly-added fd and poll again.
        let dup2 = unsafe { BorrowedFd::borrow_raw(added_raw) }
            .try_clone_to_owned()
            .unwrap();
        fire(&unsafe { EventFd::from_owned_fd(dup2) });
        ep.run_once_with_timeout(poll_timeout()).unwrap();
        assert!(added_fired.get());
    }

    #[test]
    fn modify_updates_subscriber_interest_cell() {
        let mut ep = Eventp::default();
        let efd = new_eventfd();
        let raw = efd.as_fd().as_raw_fd();

        cb_sub(efd, |_, _| {}).register_into(&mut ep).unwrap();

        let new_interest = crate::interest().read().write();
        ep.modify(raw, new_interest).unwrap();

        // The subscriber's internal Cell<Interest> must reflect the new value
        // (not just the kernel-side state).
        let stored = ep
            .registered
            .get_mut(&raw)
            .unwrap()
            .try_deref_mut()
            .unwrap()
            .interest()
            .get();
        assert_eq!(stored, new_interest);
    }

    /// Re-entrancy guard test.
    ///
    /// `Eventp::run_once_with_timeout` documents a panic when called
    /// recursively from a handler. The dispatch loop wraps the
    /// reconstructed `ThinBoxSubscriber` in `ManuallyDrop`, so the panic
    /// unwinding out of `handle()` cannot double-free the heap slot still
    /// owned by the registry. This test exercises that path end-to-end.
    #[test]
    fn recursive_run_inside_handler_panics() {
        let mut ep = Eventp::default();
        let efd = new_eventfd();

        cb_sub(efd, |_, mut ep| {
            // SAFETY: deliberately bypassing `Pinned`'s narrow API to invoke
            // the recursion guard, which is the property under test.
            let inner: &mut Eventp = unsafe { ep.0.as_mut().get_unchecked_mut() };
            let _ = inner.run_once_with_timeout(EpollTimeout::from(1u16));
        })
        .register_into(&mut ep)
        .unwrap();

        let raw = *ep.registered.keys().next().unwrap();
        let dup = unsafe { BorrowedFd::borrow_raw(raw) }
            .try_clone_to_owned()
            .unwrap();
        fire(&unsafe { EventFd::from_owned_fd(dup) });

        let result = catch_unwind(AssertUnwindSafe(|| {
            ep.run_once_with_timeout(poll_timeout()).unwrap();
        }));
        assert!(result.is_err(), "recursive run_once must panic");
    }

    #[test]
    fn into_inner_returns_registered_subscribers() {
        let mut ep = Eventp::default();
        let efd = new_eventfd();
        let raw = efd.as_fd().as_raw_fd();
        cb_sub(efd, |_, _| {}).register_into(&mut ep).unwrap();

        let (_epoll, mut registered) = ep.into_inner();
        assert!(registered.any(|s| {
            let boxed: Box<dyn Subscriber<Eventp>> = s.try_into().ok().unwrap();
            boxed.as_fd().as_raw_fd() == raw
        }));
    }

    #[test]
    fn timeout_with_no_ready_fd_does_not_dispatch() {
        let mut ep = Eventp::default();
        let efd = new_eventfd();
        let counter = Rc::new(Cell::new(0u32));
        let c = counter.clone();
        cb_sub(efd, move |_, _| c.set(c.get() + 1))
            .register_into(&mut ep)
            .unwrap();

        // Don't fire; the wait should return after the timeout with nothing
        // dispatched.
        let start = std::time::Instant::now();
        ep.run_once_with_timeout(EpollTimeout::from(20u16)).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(15));
        assert_eq!(counter.get(), 0);
    }
}
