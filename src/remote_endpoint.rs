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
    /// The reactor-side end. Register it with an `Eventp` via
    /// [`Pair::register_into`] (or manually) to start servicing remote calls.
    pub subscriber: Subscriber<Ep>,

    /// The remote-side end. Hand it out to other threads to dispatch closures
    /// onto the reactor thread.
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

fn err_subscriber_dropped() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "`remote_endpoint::Subscriber` dropped",
    )
}

macro_rules! call_variant {
    ($self:ident, $f:ident, |$rx:ident| $rx_expr:expr, |$rx_err:ident| $err_map:expr) => {{
        let (tx, $rx) = oneshot::channel();

        $self
            .tx
            .send(Box::new(move |ep| {
                let _ = tx.send($f(ep));
            }))
            .map_err(|_| err_subscriber_dropped())?;
        $self.eventfd.write(1).map_err(io::Error::from)?;

        match $rx_expr {
            Ok(v) => v,
            Err($rx_err) => return Err($err_map),
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
    /// - [`io::ErrorKind::BrokenPipe`] if the `Eventp` thread has panicked or
    ///   the [`Subscriber`] has been dropped.
    /// - Otherwise, the [`io::Error`] returned by the underlying `eventfd` write.
    pub async fn call_blocking_async<F, T>(&self, f: F) -> io::Result<T>
    where
        F: 'static + FnOnce(Pinned<'_, Ep>) -> io::Result<T> + Send,
        T: 'static + Send,
    {
        // `oneshot::Receiver::await` only fails with `RecvError`, which means
        // the sender (the reactor-side closure) was dropped without producing
        // a value -- typically because the `Subscriber` itself was dropped.
        call_variant!(self, f, |rx| rx.await, |_e| err_subscriber_dropped())
    }

    /// Sends a closure to the `Eventp` thread and blocks the current thread until it returns a result.
    ///
    /// The provided closure `f` will be executed on the `Eventp` thread. This method
    /// will block until the closure has finished execution and returned a result.
    ///
    /// # Errors
    ///
    /// - [`io::ErrorKind::BrokenPipe`] if the `Eventp` thread has panicked or
    ///   the [`Subscriber`] has been dropped.
    /// - Otherwise, the [`io::Error`] returned by the underlying `eventfd` write.
    pub fn call_blocking<F, T>(&self, f: F) -> io::Result<T>
    where
        F: 'static + FnOnce(Pinned<'_, Ep>) -> io::Result<T> + Send,
        T: 'static + Send,
    {
        // See the note in `call_blocking_async` -- `RecvError` is the only
        // failure mode and it always means the reactor end is gone.
        call_variant!(self, f, |rx| rx.recv(), |_e| err_subscriber_dropped())
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
    /// - [`io::ErrorKind::TimedOut`] if `timeout` elapses before the closure
    ///   produces a result.
    /// - [`io::ErrorKind::BrokenPipe`] if the `Eventp` thread has panicked or
    ///   the [`Subscriber`] has been dropped.
    /// - Otherwise, the [`io::Error`] returned by the underlying `eventfd` write.
    pub fn call_blocking_with_timeout<F, T>(&self, f: F, timeout: Duration) -> io::Result<T>
    where
        F: 'static + FnOnce(Pinned<'_, Ep>) -> io::Result<T> + Send,
        T: 'static + Send,
    {
        call_variant!(self, f, |rx| rx.recv_timeout(timeout), |e| match e {
            oneshot::RecvTimeoutError::Timeout => {
                io::Error::new(io::ErrorKind::TimedOut, "remote call timed out")
            }
            oneshot::RecvTimeoutError::Disconnected => err_subscriber_dropped(),
        })
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
    /// - [`io::ErrorKind::BrokenPipe`] if the `Eventp` thread has panicked or
    ///   the [`Subscriber`] has been dropped.
    /// - Otherwise, the [`io::Error`] returned by the underlying `eventfd` write.
    pub fn call_nonblocking<F>(&self, f: F) -> io::Result<()>
    where
        F: 'static + FnOnce(Pinned<'_, Ep>) + Send,
    {
        self.tx
            .send(Box::new(f))
            .map_err(|_| err_subscriber_dropped())?;
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
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc as StdArc, Barrier};
    use std::thread;

    use nix::sys::epoll::EpollTimeout;

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

    /// Short timeout for `epoll_wait`; matches the convention in `lib.rs` tests.
    fn poll_timeout() -> EpollTimeout {
        EpollTimeout::from(500u16)
    }

    /// Spawns the reactor on a background thread that pumps `run_once` until the
    /// returned `stop` flag is set. Returns the endpoint, the join handle, and
    /// the stop flag so each test can shut the worker down deterministically.
    ///
    /// `Eventp` is `!Send`, so it must be constructed inside the worker thread
    /// and the endpoint shipped back over a channel.
    fn spawn_reactor() -> (
        RemoteEndpoint<Eventp>,
        thread::JoinHandle<()>,
        StdArc<AtomicU32>,
    ) {
        let stop = StdArc::new(AtomicU32::new(0));
        let stop_for_thread = stop.clone();

        let (tx, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let mut eventp = Eventp::default();
            let endpoint = remote_endpoint()
                .unwrap()
                .register_into(&mut eventp)
                .unwrap();
            tx.send(endpoint).expect("main thread receiving endpoint");

            while stop_for_thread.load(Ordering::Acquire) == 0 {
                eventp.run_once_with_timeout(poll_timeout()).unwrap();
            }
        });

        let endpoint = rx.recv().expect("reactor thread sending endpoint");
        (endpoint, handle, stop)
    }

    fn shutdown(stop: StdArc<AtomicU32>, handle: thread::JoinHandle<()>) {
        stop.store(1, Ordering::Release);
        // The worker may already be parked in `epoll_wait`. The poll timeout
        // bounds how long we have to wait for it to observe the flag.
        handle.join().expect("reactor thread panicked");
    }

    #[test]
    fn call_blocking_runs_closure_on_reactor_thread() {
        let (endpoint, handle, stop) = spawn_reactor();
        let reactor_tid = handle.thread().id();

        // The closure runs on the reactor thread, not on the caller's thread.
        let observed_tid = endpoint
            .call_blocking(move |_| Ok(thread::current().id()))
            .unwrap();
        assert_eq!(observed_tid, reactor_tid);

        // Errors propagate back faithfully across the channel.
        let err = endpoint
            .call_blocking(|_| -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            })
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

        shutdown(stop, handle);
    }

    #[test]
    fn call_blocking_with_timeout_elapses_when_reactor_idle() {
        // Build a Pair but never register the subscriber and never run the
        // reactor: the eventfd write succeeds, yet no one drains the channel.
        let pair = remote_endpoint::<Eventp>().unwrap();
        let endpoint = pair.endpoint.clone();
        // Keep the subscriber alive so `tx.send` does not fail; we want the
        // *recv* side to time out, not the send side.
        let _keep = pair.subscriber;

        let err = endpoint
            .call_blocking_with_timeout(|_| Ok(()), Duration::from_millis(50))
            .unwrap_err();

        // A timeout (as opposed to a peer drop) must surface as `TimedOut`
        // -- the two failure modes are distinguishable by `ErrorKind` alone.
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn call_nonblocking_executes_and_drains_batch() {
        let (endpoint, handle, stop) = spawn_reactor();

        // Queue several closures back-to-back. The subscriber's handler must
        // drain *all* of them per wake-up (the `while try_recv` loop), not
        // just one. We then submit one final blocking call as a fence: when
        // it returns, every preceding non-blocking closure must have run.
        let counter = StdArc::new(AtomicU32::new(0));
        for _ in 0..16 {
            let c = counter.clone();
            endpoint
                .call_nonblocking(move |_| {
                    c.fetch_add(1, Ordering::Relaxed);
                })
                .unwrap();
        }
        endpoint.call_blocking(|_| Ok(())).unwrap();

        assert_eq!(counter.load(Ordering::Relaxed), 16);

        shutdown(stop, handle);
    }

    #[test]
    fn endpoint_returns_error_after_subscriber_dropped() {
        // Drop the subscriber without ever registering it; the channel is now
        // disconnected on the receiver side.
        let pair = remote_endpoint::<Eventp>().unwrap();
        let endpoint = pair.endpoint.clone();
        drop(pair.subscriber);

        // All call variants share the same disconnect path, and all of them
        // must surface it as `BrokenPipe` (semantically: the channel's peer
        // is gone) rather than the catch-all `Other`.
        let e1 = endpoint.call_nonblocking(|_| {}).unwrap_err();
        assert_eq!(e1.kind(), io::ErrorKind::BrokenPipe);

        let e2 = endpoint.call_blocking(|_| Ok(())).unwrap_err();
        assert_eq!(e2.kind(), io::ErrorKind::BrokenPipe);

        let e3 = endpoint
            .call_blocking_with_timeout(|_| Ok(()), Duration::from_millis(10))
            .unwrap_err();
        assert_eq!(e3.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn cloned_endpoints_serve_multiple_threads() {
        let (endpoint, handle, stop) = spawn_reactor();

        // A barrier ensures the workers race to send concurrently rather than
        // one finishing before the next starts, exercising the MPSC channel
        // and the eventfd's coalescing semantics under contention.
        let n = 8usize;
        let barrier = StdArc::new(Barrier::new(n));
        let counter = StdArc::new(AtomicU32::new(0));

        let workers: Vec<_> = (0..n)
            .map(|_| {
                let ep = endpoint.clone();
                let b = barrier.clone();
                let c = counter.clone();
                thread::spawn(move || {
                    b.wait();
                    let v = ep
                        .call_blocking(move |_| Ok(c.fetch_add(1, Ordering::Relaxed) + 1))
                        .unwrap();
                    assert!(v >= 1 && v <= n as u32);
                })
            })
            .collect();

        for w in workers {
            w.join().unwrap();
        }
        assert_eq!(counter.load(Ordering::Relaxed), n as u32);

        shutdown(stop, handle);
    }

    #[test]
    fn closure_can_mutate_reactor_state() {
        // The whole point of `RemoteEndpoint` is to give external threads a
        // safe handle to mutate the reactor. Have the remote closure call
        // `delete` on a non-existent fd: the error reported back proves that
        // the closure actually ran on the reactor and exercised `Pinned::delete`.
        let (endpoint, handle, stop) = spawn_reactor();

        let err = endpoint
            .call_blocking(|mut ep| ep.delete(424242))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        shutdown(stop, handle);
    }
}
