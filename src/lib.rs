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

use std::hint;
use std::marker::PhantomPinned;
use std::mem::{self, transmute, MaybeUninit};
use std::os::fd::{AsRawFd, RawFd};
use std::pin::Pin;
use std::{io, ptr};

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

const DEFAULT_EVENT_BUF_CAPACITY: usize = 256;

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
    deferred_remove: Vec<RawFd>,
}

impl Default for Eventp {
    /// Creates a new `Eventp` with default capacity and flags.
    ///
    /// # Panics
    ///
    /// Panics if the underlying `epoll_create` syscall fails.
    fn default() -> Self {
        Self::new(DEFAULT_EVENT_BUF_CAPACITY, EpollCreateFlags::EPOLL_CLOEXEC)
            .expect("Failed to create epoll instance")
    }
}

impl Eventp {
    /// Creates a new `Eventp` instance with a specified event buffer capacity and `epoll` flags.
    pub fn new(capacity: usize, flags: EpollCreateFlags) -> io::Result<Self> {
        let mut buf = Vec::with_capacity(capacity);
        // SAFETY: The buffer is immediately used with `epoll_wait`, which will
        //         only write initialized `EpollEvent` values into it. The `MaybeUninit`
        //         wrapper is used to satisfy allocation requirements without initializing
        //         the memory, which is sound here.
        unsafe { buf.set_len(capacity) };

        Ok(Self {
            epoll: Epoll::new(flags).map_err(io::Error::from)?,
            registered: Default::default(),
            event_buf: buf,
            handling: None,
            _pinned: PhantomPinned,
        })
    }

    /// Consumes the `Eventp`, returning the inner `Epoll` instance and hash map.
    pub fn into_inner(self) -> (Epoll, FxHashMap<RawFd, ThinBoxSubscriber<Eventp>>) {
        (self.epoll, self.registered)
    }

    /// Runs the event loop indefinitely, blocking until an error occurs.
    ///
    /// This is the main entry point for starting the event loop. It continuously calls
    /// `run_once` and handles `EINTR` errors by retrying.
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

    /// Runs the event loop for a single iteration, blocking until at least one event is ready.
    ///
    /// This is equivalent to calling `run_once_with_timeout` with an infinite timeout.
    pub fn run_once(&mut self) -> io::Result<()> {
        self.run_once_with_timeout(EpollTimeout::NONE)
    }

    /// Runs the event loop for a single iteration with a specified timeout.
    ///
    /// This method performs one `epoll_wait` call and dispatches all ready events.
    ///
    /// # Panics
    ///
    /// Panics if called recursively (i.e., from within an event handler), as this
    /// would violate the re-entrancy safety model.
    pub fn run_once_with_timeout(&mut self, timeout: EpollTimeout) -> io::Result<()> {
        if self.handling.is_some() {
            // Recursive calls would corrupt the `handling` state and could lead to
            // iterator invalidation issues. This panic prevents such misuse.
            panic!("Recursive call to Eventp::run_with_timeout");
        }

        // SAFETY: `self.event_buf` is a `Vec<MaybeUninit<EpollEvent>>`. `epoll_wait`
        // expects a `&mut [EpollEvent]`. This transmute is safe because `EpollEvent`
        // has no drop glue and is a simple C-style struct. The kernel guarantees
        // it will only write valid `EpollEvent` data into the buffer.
        let buf: &mut [MaybeUninit<EpollEvent>] = &mut self.event_buf;
        let buf: &mut [EpollEvent] = unsafe { mem::transmute(buf) };

        let n = self.epoll.wait(buf, timeout)?;
        let buf = &buf[..n];

        // Enter the 'handling' state to manage re-entrancy safely.
        if self.handling.is_some() {
            // Avoid unnecessary drop check.
            // SAFETY: `self.handling` is guaranteed to be `Some` at the start of this function,
            //         and epoll_wait will not change it.
            unsafe { hint::unreachable_unchecked() }
        } else {
            self.handling = Some(Handling {
                    fd: -1, // Invalid fd, will be updated for each event.
                deferred_remove: vec![],
            });
        }

        for ev in buf {
            // Reconstruct the subscriber pointer from the `epoll` event data.
            let addr = ev.data() as usize;
            // SAFETY: `addr` was created from a valid `ThinBoxSubscriber` in `add()`.
            // Because `Eventp` is `!Unpin`, we know the `registered` map has not moved,
            // so the subscriber pointers are still valid.
            let mut subscriber = unsafe { transmute::<usize, ThinBoxSubscriber<Eventp>>(addr) };

            // Update the currently handled fd in the `Handling` state.
            // SAFETY: `self.handling` is guaranteed to be `Some` within this loop.
            unsafe {
                self.handling.as_mut().unwrap_unchecked().fd = subscriber.raw_fd();
            }

            // Dispatch the event to the subscriber's handler.
            // SAFETY: The `self` pointer is pinned, so `Pin::new_unchecked` is sound.
            // The handler receives a `Pinned<Eventp>` to safely interact with the loop.
            subscriber.handle(Event::from(ev), Pinned(unsafe { Pin::new_unchecked(self) }));

            // The subscriber was reconstructed from a raw pointer and does not have
            // true ownership. We must `forget` it to prevent its destructor from
            // running and causing a double-free. The real owner is `self.registered`.
            mem::forget(subscriber);
        }

        // Take the handling state to process deferred removals.
        // SAFETY: `self.handling` is guaranteed to be `Some` at this point.
        let handling = unsafe { self.handling.take().unwrap_unchecked() };

        // Process all deferred removals now that iteration is complete.
        for fd in handling.deferred_remove {
            // The subscriber's memory will be freed when its `ThinBoxSubscriber` is dropped from the map.
            self.registered.remove(&fd);
        }

        Ok(())
    }
}

impl EventpOpsAdd<Self> for Eventp {
    /// Registers a new subscriber with the event loop.
    ///
    /// This method takes ownership of the `subscriber` and registers its file descriptor
    /// with the underlying `epoll` instance. The subscriber's thin pointer is stored
    /// in the `epoll` event data for zero-cost dispatch.
    ///
    /// If a subscriber with the same file descriptor already exists, it will be replaced.
    ///
    /// # Re-entrancy
    ///
    /// This method is safe to call from within an event handler. However, a handler
    /// cannot replace its own subscriber while it is being executed. Attempting to do so
    /// will result in an `io::Error`.
    fn add(&mut self, subscriber: ThinBoxSubscriber<Self>) -> io::Result<()> {
        let raw_fd = subscriber.as_fd().as_raw_fd();

        // Re-entrancy check: prevent a handler from replacing its own subscriber
        // while it is being executed.
        if let Some(handling) = &self.handling {
            if handling.fd == raw_fd {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "cannot replace the subscriber of itself at running",
                ));
            }
        }

        let interest = subscriber.interest().get();

        // Pointer laundering: Convert the subscriber's thin pointer into a `usize`.
        // This breaks the lifetime link for the borrow checker, allowing us to store
        // it in `epoll`.
        // SAFETY: `ThinBoxSubscriber` is a `repr(transparent)` wrapper around a pointer,
        // so transmuting it to `usize` is safe. We use `transmute_copy` to avoid
        // consuming the subscriber, as we need to move it into `self.registered`.
        let addr = unsafe { mem::transmute_copy::<_, usize>(&subscriber) };
        let epoll_event = EpollEvent::new(interest.bitflags(), addr as u64);

        self.epoll.add(subscriber.as_fd(), epoll_event)?;

        // Take ownership of the subscriber. This is the only place that owns it.
        self.registered.insert(raw_fd, subscriber);

        Ok(())
    }
}

impl EventpOps for Eventp {
    /// Modifies the event interest for an existing subscriber.
    ///
    /// This updates the `epoll` registration for the given `fd` to monitor for events
    /// specified by the new `interest`.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` with `ErrorKind::NotFound` if no subscriber is registered
    /// for the given `fd`.
    fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()> {
        let subscriber = self
            .registered
            .get(&fd)
            .ok_or(io::Error::new(io::ErrorKind::NotFound, "fd not registered"))?;

        // Perform the same pointer laundering as in `add` to get the address for `epoll_ctl`.
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
        subscriber.interest().set(interest);

        Ok(())
    }

    /// Unregisters a subscriber from the event loop.
    ///
    /// This removes the file descriptor `fd` from the `epoll` instance and drops the
    /// associated subscriber, freeing its resources.
    ///
    /// # Re-entrancy
    ///
    /// This method is safe to call from within an event handler. If called during event
    /// dispatch, the removal is deferred until all events in the current batch have been
    /// processed. This prevents iterator invalidation on the internal subscriber map.
    fn delete(&mut self, fd: RawFd) -> io::Result<()> {
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

        // Handle re-entrancy. If we are in the middle of event dispatching,
        // defer the removal from our map to avoid iterator invalidation.
        if let Some(handling) = &mut self.handling {
            handling.deferred_remove.push(fd);
        } else {
            // Otherwise, it's safe to remove immediately.
            self.registered.remove(&fd);
        }
        Ok(())
    }
}
