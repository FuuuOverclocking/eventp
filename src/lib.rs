//! A safe, zero-cost, and high-performance event loop library for Linux.
//!
//! This crate provides a low-level abstraction over `epoll`, designed to be
//! both efficient and easy to use. It leverages Rust's type system to ensure
//! safety while employing advanced techniques for zero-cost abstractions.
//!
//! # Key Features
//!
//! - **Type-safe API**: Wraps raw `epoll` calls in a safe, idiomatic Rust interface.
//! - **Zero-cost Abstractions**: Uses traits, thin pointers (`ThinBoxSubscriber`),
//!   and compile-time mechanisms to minimize runtime overhead.
//! - **Testability**: Designed with dependency injection and mocking in mind.
//! - **Re-entrancy Safety**: The event loop is safe to modify from within event handlers.
//!
//! # Core Concepts
//!
//! - [`Eventp`]: The main event loop reactor. It manages file descriptors and dispatches events.
//! - [`Interest`]: Specifies the readiness events (e.g., readable, writable) to monitor.
//! - [`Subscriber`]: Represents an I/O source (like a `TcpStream`) combined with an event handler.
//!
//! # Examples
//!
//! ```rust
//! # use std::io;
//! # use eventp::{interest, tri_subscriber::WithHandler, Eventp, Subscriber};
//! use nix::sys::eventfd::EventFd;
//!
//! fn thread_main(efd: EventFd) -> io::Result<()> {
//!     let mut eventp = Eventp::default();
//!     interest()
//!         .read()
//!         .with_fd(efd)
//!         .with_handler(|efd: &mut EventFd| {
//!             efd.read().unwrap();
//!             do_something();
//!         })
//!         .register_into(&mut eventp)?;
//!
//!     eventp.run_forever()
//! }
//! # fn do_something() {}
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

pub mod bin_subscriber;
mod event;
mod eventp_ops;
mod interest;
#[cfg(feature = "mock")]
pub mod mock;
mod pinned;
mod registry;
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
use std::mem::{self, transmute, MaybeUninit};
use std::os::fd::{AsRawFd, RawFd};
use std::pin::Pin;
use std::{io, ptr};

use rustc_hash::FxHashMap;

use crate::epoll::*;
pub use crate::event::Event;
pub use crate::eventp_ops::EventpOps;
pub use crate::interest::{interest, Interest};
#[cfg(feature = "mock")]
pub use crate::mock::MockEventp;
pub use crate::pinned::Pinned;
pub use crate::registry::Registry;
pub use crate::subscriber::Subscriber;
use crate::thin::ThinBoxSubscriber;

const DEFAULT_EVENT_BUF_CAPACITY: usize = 256;

/// The central event loop reactor, built on top of `epoll`.
///
/// `Eventp` manages a set of registered I/O sources (file descriptors) and their
/// associated interests and handlers. It waits for I/O readiness events and dispatches
/// them to the corresponding handlers.
///
/// # Safety and Pinning
///
/// This struct is `!Unpin`. Once an `Eventp` instance is created, its memory location
/// must not be moved. This is crucial because `epoll` is given raw pointers to the
/// heap-allocated subscribers owned by `Eventp`. Moving `Eventp` would invalidate
/// these pointers, leading to undefined behavior.
///
/// To use `Eventp` safely, you must pin it in memory, for example, by putting it
/// in a `Box` and using `Box::pin`, or by using stack-pinning utilities.
///
/// # Internal Mechanics
///
/// `Eventp` stores `Subscriber`s in a hash map. To avoid self-referential borrowing
/// issues (where a handler needs a mutable reference to the `Eventp` that owns it),
/// a pointer laundering technique is used:
///
/// 1.  When a subscriber is added, its `ThinBoxSubscriber` (a thin pointer) is
///     transmuted into a `usize` and stored in the `epoll` event data.
/// 2.  When an event is received, the `usize` is transmuted back into a `ThinBoxSubscriber`.
/// 3.  The handler is called. `mem::forget` is used on the reconstructed subscriber to
///     prevent a double-free, as ownership remains with the `registered` map.
///
/// This design enables handlers to safely modify the `Eventp` instance (e.g., add or
/// remove other subscribers) via the `Pinned<impl EventpOps>` handle.
pub struct Eventp {
    /// A map from a raw file descriptor to its heap-allocated subscriber.
    /// This is the single source of truth for subscriber ownership.
    registered: FxHashMap<RawFd, ThinBoxSubscriber<Eventp>>,
    /// The underlying `epoll` file descriptor wrapper.
    epoll: Epoll,
    /// A pre-allocated buffer for receiving events from `epoll_wait`.
    event_buf: Vec<MaybeUninit<EpollEvent>>,
    /// State machine to safely handle modifications while dispatching events.
    ///
    /// When `Some`, the event loop is iterating through events, and any removals
    /// are deferred. When `None`, modifications can be performed immediately.
    handling: Option<Handling>,
    /// Ensures that `Eventp` is `!Unpin`.
    ///
    /// This prevents the `Eventp` instance from being moved in memory after it has
    /// been created, which is critical for the safety of the raw pointers stored
    /// in `epoll`.
    _pinned: PhantomPinned,
}

/// The internal state used during event dispatching to handle re-entrancy.
struct Handling {
    /// The file descriptor of the event currently being handled.
    /// Used to prevent a handler from replacing its own subscriber.
    fd: RawFd,
    /// A list of file descriptors to be removed after the dispatch loop finishes.
    /// This defers removal operations to avoid iterator invalidation on `registered`.
    to_remove: Vec<RawFd>,
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
    ///
    /// # Parameters
    ///
    /// * `capacity`: The maximum number of events to receive in a single `epoll_wait` call.
    /// * `flags`: Flags for the `epoll_create1` syscall, e.g., `EPOLL_CLOEXEC`.
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

    /// Returns a shared reference to the inner `Epoll` instance.
    pub fn inner(&self) -> &Epoll {
        &self.epoll
    }

    /// Returns a mutable reference to the inner `Epoll` instance.
    pub fn inner_mut(&mut self) -> &mut Epoll {
        &mut self.epoll
    }

    /// Consumes the `Eventp`, returning the inner `Epoll` instance.
    pub fn into_inner(self) -> Epoll {
        self.epoll
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
        self.handling = Some(Handling {
            fd: -1, // Invalid fd, will be updated for each event.
            to_remove: vec![],
        });

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
                self.handling.as_mut().unwrap_unchecked().fd = subscriber.as_fd().as_raw_fd();
            }

            // Dispatch the event to the subscriber's handler.
            // SAFETY: The `self` pointer is pinned, so `Pin::new_unchecked` is sound.
            // The handler receives a `Pinned<Eventp>` to safely interact with the loop.
            subscriber.handle(
                ev.events().into(),
                Pinned(unsafe { Pin::new_unchecked(self) }),
            );

            // The subscriber was reconstructed from a raw pointer and does not have
            // true ownership. We must `forget` it to prevent its destructor from
            // running and causing a double-free. The real owner is `self.registered`.
            mem::forget(subscriber);
        }

        // Take the handling state to process deferred removals.
        // SAFETY: `self.handling` is guaranteed to be `Some` at this point.
        let handling = unsafe { self.handling.take().unwrap_unchecked() };

        // Process all deferred removals now that iteration is complete.
        for fd in handling.to_remove {
            // The subscriber's memory will be freed when its `ThinBoxSubscriber` is dropped from the map.
            self.registered.remove(&fd);
        }

        Ok(())
    }
}

impl EventpOps for Eventp {
    fn add(&mut self, subscriber: ThinBoxSubscriber<Eventp>) -> io::Result<()> {
        let raw_fd = subscriber.as_fd().as_raw_fd();

        // Re-entrancy check: prevent a handler from replacing its own subscriber
        // while it is being executed.
        if let Some(handling) = &self.handling {
            if handling.fd == raw_fd {
                return Err(io::Error::other(
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

    fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()> {
        let subscriber = self
            .registered
            .get(&fd)
            .ok_or(io::Error::new(io::ErrorKind::NotFound, "fd not registered"))?;

        // Perform the same pointer laundering as in `add` to get the address for `epoll_ctl`.
        let addr = unsafe { mem::transmute_copy::<_, usize>(subscriber) };
        let mut epoll_event = EpollEvent::new(interest.bitflags(), addr as u64);

        self.epoll.modify(subscriber.as_fd(), &mut epoll_event)?;
        // Update the interest stored within the subscriber itself.
        subscriber.interest().set(interest);

        Ok(())
    }

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
            handling.to_remove.push(fd);
        } else {
            // Otherwise, it's safe to remove immediately.
            self.registered.remove(&fd);
        }
        Ok(())
    }
}
