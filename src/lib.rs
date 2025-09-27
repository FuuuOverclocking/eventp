mod bin_subscriber;
mod event;
mod eventp_ops;
mod interest;
mod pinned;
mod registry;
mod subscriber;
mod thin;
mod tri_subscriber;

pub mod epoll {
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

pub use crate::bin_subscriber::BinSubscriber;
use crate::epoll::*;
pub use crate::event::Event;
pub use crate::eventp_ops::EventpOps;
#[cfg(feature = "mock")]
#[cfg_attr(docsrs, doc(cfg(feature = "mock")))]
pub use crate::eventp_ops::MockEventp;
pub use crate::interest::{interest, Interest};
pub use crate::pinned::Pinned;
pub use crate::registry::Registry;
pub use crate::subscriber::{Handler, Subscriber, WithInterest};
pub use crate::thin::ThinBoxSubscriber;
pub use crate::tri_subscriber::{FdWithInterest, TriSubscriber};

const DEFAULT_EVENT_BUF_CAPACITY: usize = 256;

pub struct Eventp {
    registered: FxHashMap<RawFd, ThinBoxSubscriber<Eventp>>,
    epoll: Epoll,
    event_buf: Vec<MaybeUninit<EpollEvent>>,
    handling: Option<Handling>,
    _pinned: PhantomPinned,
}

struct Handling {
    fd: RawFd,
    to_remove: Vec<RawFd>,
}

impl Default for Eventp {
    fn default() -> Self {
        Self::new(DEFAULT_EVENT_BUF_CAPACITY, EpollCreateFlags::EPOLL_CLOEXEC)
            .expect("Failed to create epoll instance")
    }
}

impl Eventp {
    pub fn new(capacity: usize, flags: EpollCreateFlags) -> io::Result<Self> {
        let mut buf = Vec::with_capacity(capacity);
        unsafe { buf.set_len(capacity) };

        Ok(Self {
            epoll: Epoll::new(flags).map_err(io::Error::from)?,
            registered: Default::default(),
            event_buf: buf,
            handling: None,
            _pinned: PhantomPinned,
        })
    }

    pub fn inner(&self) -> &Epoll {
        &self.epoll
    }

    pub fn inner_mut(&mut self) -> &mut Epoll {
        &mut self.epoll
    }

    pub fn into_inner(self) -> Epoll {
        self.epoll
    }

    pub fn run_forever(&mut self) -> io::Result<()> {
        loop {
            match self.run_once() {
                Ok(_) => continue,

                // The only source of error is epoll_wait.
                // Ref: https://man.archlinux.org/man/epoll_wait.2.en#ERRORS
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }

    pub fn run_once(&mut self) -> io::Result<()> {
        self.run_once_with_timeout(EpollTimeout::NONE)
    }

    pub fn run_once_with_timeout(&mut self, timeout: EpollTimeout) -> io::Result<()> {
        if self.handling.is_some() {
            panic!("Recursive call to Eventp::run_with_timeout");
        }

        // Use `BorrowedBuf` instead, once it becomes stable.
        let buf: &mut [MaybeUninit<EpollEvent>] = &mut self.event_buf;
        let buf: &mut [EpollEvent] = unsafe { mem::transmute(buf) };

        let n = self.epoll.wait(buf, timeout)?;
        let buf = &buf[..n];

        self.handling = Some(Handling {
            fd: -1,
            to_remove: vec![],
        });
        for ev in buf {
            let addr = ev.data() as usize;
            let mut subscriber = unsafe { transmute::<usize, ThinBoxSubscriber<Eventp>>(addr) };
            unsafe {
                self.handling.as_mut().unwrap_unchecked().fd = subscriber.as_fd().as_raw_fd();
            }
            let interest = subscriber.interest().get();

            subscriber.handle(
                ev.events().into(),
                interest,
                Pinned(unsafe { Pin::new_unchecked(self) }),
            );
            mem::forget(subscriber);
        }
        let handling = unsafe { self.handling.take().unwrap_unchecked() };

        for fd in handling.to_remove {
            self.registered.remove(&fd);
        }

        Ok(())
    }
}

impl EventpOps for Eventp {
    fn add(&mut self, subscriber: ThinBoxSubscriber<Eventp>) -> io::Result<()> {
        let raw_fd = subscriber.as_fd().as_raw_fd();

        if let Some(handling) = &self.handling {
            if handling.fd == raw_fd {
                return Err(io::Error::other(
                    "cannot replace the subscriber of itself at running",
                ));
            }
        }

        let interest = subscriber.interest().get();

        let addr = unsafe { mem::transmute_copy::<_, usize>(&subscriber) };
        let epoll_event = EpollEvent::new(interest.bitflags(), addr as u64);

        self.epoll.add(subscriber.as_fd(), epoll_event)?;
        self.registered.insert(raw_fd, subscriber);

        Ok(())
    }

    fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()> {
        let subscriber = self
            .registered
            .get(&fd)
            .ok_or(io::Error::new(io::ErrorKind::NotFound, "fd not registered"))?;
        let addr = unsafe { mem::transmute_copy::<_, usize>(subscriber) };
        let mut epoll_event = EpollEvent::new(interest.bitflags(), addr as u64);

        self.epoll.modify(subscriber.as_fd(), &mut epoll_event)?;
        subscriber.interest().set(interest);

        Ok(())
    }

    fn delete(&mut self, fd: RawFd) -> io::Result<()> {
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
            handling.to_remove.push(fd);
        } else {
            self.registered.remove(&fd);
        }
        Ok(())
    }
}
