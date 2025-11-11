//! Provides a mechanism for cross-thread communication with an `Eventp` event loop.
//!
//! This module allows threads to safely queue closures for execution on the `Eventp`
//! thread. It is useful for managing I/O resources or other state owned by the
//! event loop from external threads.
//!
//! # How It Works
//!
//! The [`remote_endpoint()`] function creates a connected pair:
//! - A [`Subscriber`]: An event handler that is registered with the `Eventp` instance.
//!   It listens on an `eventfd` for notifications.
//! - A [`RemoteEndpoint`]: A cloneable "handle" that can be sent to other threads.
//!
//! When a method like [`RemoteEndpoint::call_blocking`] is called, it sends a closure
//! over an MPSC channel to the `Subscriber` and then writes to the `eventfd` to wake
//! up the event loop. The `Subscriber`'s handler then drains the channel and executes
//! the received closures.
//!
//! # Examples
//!
//! ```
//! # use std::io;
//! use eventp::{Eventp, EventpOps, remote_endpoint};
//! use eventp::remote_endpoint::RemoteEndpoint;
//!
//! # fn main() -> io::Result<()> {
//! let mut eventp = Eventp::default();
//! // Create the pair and register the subscriber part into the event loop.
//! let endpoint = remote_endpoint()?.register_into(&mut eventp)?;
//!
//! // Now, the endpoint can be cloned and sent to other threads.
//! let endpoint_for_thread = endpoint.clone();
//! # Ok(()) }
//!
//! // In another thread, you can use the endpoint to interact with the Eventp loop.
//! async fn thread_main(endpoint: RemoteEndpoint<impl EventpOps>) -> io::Result<()> {
//!     endpoint.call_blocking_async(|mut eventp| {
//!         let mysterious_fd = 42;
//!         eventp.delete(mysterious_fd)
//!     }).await?;
//!
//!     Ok(())
//! }
//! ```

use std::cell::Cell;
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use nix::sys::eventfd::{EfdFlags, EventFd};

use crate::subscriber::{Handler, HasInterest};
use crate::thin::ThinBoxSubscriber;
use crate::{interest, Event, EventpOps, EventpOpsAdd, Interest, Pinned};

type BoxFn<Ep> = Box<dyn FnOnce(Pinned<Ep>) + Send>;

/// Creates a [`Pair`] of [`RemoteEndpoint`] and [`Subscriber`].
///
/// For more information, see the [mod-level documentation](self).
pub fn remote_endpoint<Ep>() -> io::Result<Pair<Ep>> {
    let eventfd = EventFd::from_flags(EfdFlags::EFD_CLOEXEC | EfdFlags::EFD_NONBLOCK)
        .map_err(io::Error::from)?;
    let eventfd = Arc::new(eventfd);

    let (tx, rx) = mpsc::channel();

    let subscriber = Subscriber {
        eventfd: Arc::clone(&eventfd),
        interest: Cell::new(interest().read()),
        rx,
    };
    let endpoint = RemoteEndpoint { eventfd, tx };

    Ok(Pair {
        subscriber,
        endpoint,
    })
}

/// Just a pair of [`Subscriber`] and [`RemoteEndpoint`], nothing strange.
pub struct Pair<Ep> {
    pub subscriber: Subscriber<Ep>,
    pub endpoint: RemoteEndpoint<Ep>,
}

/// An event handler that executes closures sent from a [`RemoteEndpoint`].
///
/// This struct is created by [`remote_endpoint`] and is intended to be registered
/// with an `Eventp` instance. It listens for notifications on an `eventfd` and,
/// when woken up, executes all pending closures from the MPSC channel.
pub struct Subscriber<Ep> {
    eventfd: Arc<EventFd>,
    interest: Cell<Interest>,
    rx: mpsc::Receiver<BoxFn<Ep>>,
}

/// A remote control for an `Eventp` instance running on another thread.
///
/// It allows sending closures to the `Eventp` thread to be executed, providing a
/// way to perform thread-safe operations on the `Eventp` instance and its
/// registered sources.
///
/// `RemoteEndpoint` is cheap to clone and is both `Send` and `Sync`.
pub struct RemoteEndpoint<Ep> {
    eventfd: Arc<EventFd>,
    tx: mpsc::Sender<BoxFn<Ep>>,
}

impl<Ep: EventpOps> Pair<Ep> {
    /// Registers the `Subscriber` into the `Eventp` and returns the `RemoteEndpoint` back.
    pub fn register_into<R>(self, eventp: &mut R) -> io::Result<RemoteEndpoint<Ep>>
    where
        Self: Sized,
        R: EventpOpsAdd<Ep>,
    {
        eventp.add(ThinBoxSubscriber::new(self.subscriber))?;

        Ok(self.endpoint)
    }
}

impl<Ep> AsFd for Subscriber<Ep> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.eventfd.as_fd()
    }
}

impl<Ep> HasInterest for Subscriber<Ep> {
    fn interest(&self) -> &Cell<Interest> {
        &self.interest
    }
}

impl<Ep: EventpOps> Handler<Ep> for Subscriber<Ep> {
    fn handle(&mut self, _event: Event, mut eventp: Pinned<'_, Ep>) {
        let _ = self.eventfd.read();

        while let Ok(f) = self.rx.try_recv() {
            (f)(eventp.as_mut())
        }
    }
}

macro_rules! call_variant {
    ($self:ident, $f:ident, |$rx:ident| $rx_expr:expr) => {{
        let (tx, $rx) = oneshot::channel();

        $self
            .tx
            .send(Box::new(move |ep| {
                let _ = tx.send($f(ep));
            }))
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::Other,
                    "cannot call because `remote_endpoint::Subscriber` dropped",
                )
            })?;
        $self.eventfd.write(1).map_err(io::Error::from)?;

        let result = $rx_expr.map_err(|_| {
            io::Error::new(
                io::ErrorKind::Other,
                "cannot recv from epoll thread because tx dropped",
            )
        });
        match result {
            Ok(inner) => inner,
            Err(e) => Err(e),
        }
    }};
}

impl<Ep> RemoteEndpoint<Ep> {
    /// Asynchronously sends a closure to the `Eventp` thread and waits for its result.
    ///
    /// The provided closure `f` will be executed on the `Eventp` thread. This method
    /// returns a future that resolves to the `io::Result<T>` returned by the closure.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The `Eventp` thread has panicked or the [`Subscriber`] has been dropped.
    /// - Writing to the underlying `eventfd` fails.
    pub async fn call_blocking_async<F, T>(&self, f: F) -> io::Result<T>
    where
        F: 'static + FnOnce(Pinned<'_, Ep>) -> io::Result<T> + Send,
        T: 'static + Send,
    {
        call_variant!(self, f, |rx| rx.await)
    }

    /// Sends a closure to the `Eventp` thread and blocks the current thread until it returns a result.
    ///
    /// The provided closure `f` will be executed on the `Eventp` thread. This method
    /// will block until the closure has finished execution and returned a result.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The `Eventp` thread has panicked or the [`Subscriber`] has been dropped.
    /// - Writing to the underlying `eventfd` fails.
    pub fn call_blocking<F, T>(&self, f: F) -> io::Result<T>
    where
        F: 'static + FnOnce(Pinned<'_, Ep>) -> io::Result<T> + Send,
        T: 'static + Send,
    {
        call_variant!(self, f, |rx| rx.recv())
    }

    /// Sends a closure to the `Eventp` thread and blocks the current thread until it returns a result,
    /// with a timeout.
    ///
    /// The provided closure `f` will be executed on the `Eventp` thread. This method
    /// will block until the closure has finished execution and returned a result, or
    /// until the specified `timeout` has elapsed.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The `Eventp` thread has panicked or the [`Subscriber`] has been dropped.
    /// - Writing to the underlying `eventfd` fails.
    /// - The timeout is reached.
    pub fn call_blocking_with_timeout<F, T>(&self, f: F, timeout: Duration) -> io::Result<T>
    where
        F: 'static + FnOnce(Pinned<'_, Ep>) -> io::Result<T> + Send,
        T: 'static + Send,
    {
        call_variant!(self, f, |rx| rx.recv_timeout(timeout))
    }

    /// Sends a closure to the `Eventp` thread for execution without waiting for a result.
    ///
    /// This is a "fire-and-forget" method. The provided closure `f` is queued for
    /// execution on the `Eventp` thread, but this method returns immediately without
    /// waiting for its completion. There is no way to retrieve a return value or
    /// determine if the closure executed successfully.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The `Eventp` thread has panicked or the [`Subscriber`] has been dropped.
    /// - Writing to the underlying `eventfd` fails.
    pub fn call_nonblocking<F>(&self, f: F) -> io::Result<()>
    where
        F: 'static + FnOnce(Pinned<'_, Ep>) + Send,
    {
        self.tx.send(Box::new(f)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::Other,
                "cannot call because `remote_endpoint::Subscriber` dropped",
            )
        })?;
        self.eventfd.write(1).map_err(io::Error::from)?;

        Ok(())
    }
}

impl<Ep> Clone for RemoteEndpoint<Ep> {
    fn clone(&self) -> Self {
        Self {
            eventfd: self.eventfd.clone(),
            tx: self.tx.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Eventp;
    #[cfg(feature = "mock")]
    use crate::MockEventp;

    const fn assert_send<T: Send>() {}
    const fn assert_sync<T: Send>() {}

    const _: () = {
        assert_send::<RemoteEndpoint<Eventp>>();
        assert_sync::<RemoteEndpoint<Eventp>>();

        #[cfg(feature = "mock")]
        assert_send::<RemoteEndpoint<MockEventp>>();
        #[cfg(feature = "mock")]
        assert_sync::<RemoteEndpoint<MockEventp>>();
    };
}
