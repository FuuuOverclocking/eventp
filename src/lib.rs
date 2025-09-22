mod builder;
mod interests;
mod subscriber;
mod thinbox;

use std::collections::HashMap;
use std::mem::{self, MaybeUninit, transmute};
use std::os::fd::{AsRawFd, RawFd};
use std::{io, ptr};

use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};

pub use crate::builder::{FdWithInterests, Subscriber1, Subscriber2};
pub use crate::interests::{Interests, interests};
pub use crate::subscriber::{Handler, Subscriber, WithInterests};
pub use crate::thinbox::ThinBoxSubscriber;

const DEFAULT_CAPACITY: usize = 256;

pub struct EventP {
    registered: HashMap<RawFd, ThinBoxSubscriber>,
    epoll: Epoll,
    buf: Vec<MaybeUninit<EpollEvent>>,
    handling: Option<Handling>,
}

struct Handling {
    fd: RawFd,
    to_remove: Vec<RawFd>,
}

impl EventP {
    pub fn new(capacity: usize, flags: EpollCreateFlags) -> io::Result<Self> {
        let mut buf = Vec::with_capacity(capacity);
        unsafe { buf.set_len(capacity) };

        Ok(Self {
            epoll: Epoll::new(flags).map_err(io::Error::from)?,
            registered: Default::default(),
            buf,
            handling: None,
        })
    }
}

impl Default for EventP {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY, EpollCreateFlags::EPOLL_CLOEXEC)
            .expect("Failed to create epoll instance")
    }
}

impl EventP {
    pub fn add(&mut self, subscriber: ThinBoxSubscriber) -> io::Result<()> {
        let raw_fd = subscriber.as_fd().as_raw_fd();
        let interests = subscriber.interests().get();

        let addr = unsafe { mem::transmute_copy::<_, usize>(&subscriber) };
        let epoll_event = EpollEvent::new(interests, addr as u64);

        self.epoll.add(subscriber.as_fd(), epoll_event)?;
        self.registered.insert(raw_fd, subscriber);

        Ok(())
    }

    pub fn modify(&mut self, fd: RawFd, interests: EpollFlags) -> io::Result<()> {
        let subscriber = self
            .registered
            .get(&fd)
            .ok_or(io::Error::new(io::ErrorKind::NotFound, "fd not registered"))?;
        let addr = unsafe { mem::transmute_copy::<_, usize>(subscriber) };
        let mut epoll_event = EpollEvent::new(interests, addr as u64);

        self.epoll.modify(subscriber.as_fd(), &mut epoll_event)?;
        subscriber.interests().set(interests);

        Ok(())
    }

    pub fn delete(&mut self, fd: RawFd) -> io::Result<()> {
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

    pub fn run(&mut self) -> io::Result<()> {
        self.run_with_timeout(EpollTimeout::NONE)
    }

    pub fn run_with_timeout(&mut self, timeout: EpollTimeout) -> io::Result<()> {
        if self.handling.is_some() {
            panic!("Recursive call to EventP::run_with_timeout");
        }

        // Use `BorrowedBuf` instead, once it becomes stable.
        let buf: &mut [MaybeUninit<EpollEvent>] = &mut self.buf;
        let buf: &mut [EpollEvent] = unsafe { mem::transmute(buf) };

        let n = self.epoll.wait(buf, timeout)?;
        let buf = &buf[..n];

        self.handling = Some(Handling {
            fd: -1,
            to_remove: vec![],
        });
        for ev in buf {
            let addr = ev.data() as usize;
            let mut subscriber = unsafe { transmute::<usize, ThinBoxSubscriber>(addr) };
            unsafe {
                self.handling.as_mut().unwrap_unchecked().fd = subscriber.as_fd().as_raw_fd();
            }

            subscriber.handle(ev.events(), self);
            mem::forget(subscriber);
        }
        let handling = unsafe { self.handling.take().unwrap_unchecked() };

        for fd in handling.to_remove {
            self.registered.remove(&fd);
        }

        Ok(())
    }
}
