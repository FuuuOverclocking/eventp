use std::cell::Cell;
use std::collections::HashMap;
use std::mem::{self, MaybeUninit};
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::time::Duration;
use std::{io, ptr};

use nix::libc;
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};

pub struct Eventp<S> {
    // Drop order: `registered` firstly, then `epoll`.
    registered: HashMap<RawFd, Box<S>>,
    epoll: Epoll,
    buf: Vec<MaybeUninit<EpollEvent>>,
    handling: Option<Handling>,
}

struct Handling {
    fd: RawFd,
    to_remove: Vec<RawFd>,
}

impl<S> Eventp<S> {
    pub fn new() -> io::Result<Self> {
        Self::with_flags(EpollCreateFlags::EPOLL_CLOEXEC)
    }

    pub fn with_flags(flags: EpollCreateFlags) -> io::Result<Self> {
        Ok(Self {
            epoll: Epoll::new(flags).map_err(io::Error::from)?,
            registered: Default::default(),
            buf: vec![MaybeUninit::uninit(); 256],
            handling: None,
        })
    }
}

impl<S> Eventp<S>
where
    S: Subscriber<S>,
{
    pub fn add(&mut self, mut subscriber: Box<S>) -> io::Result<()> {
        let raw_fd = subscriber.fd().as_fd().as_raw_fd();
        let interests = subscriber.interests().get();

        let addr = subscriber.as_mut() as *mut S;
        let epoll_event = EpollEvent::new(interests, addr as u64);

        self.epoll.add(subscriber.fd(), epoll_event)?;
        self.registered.insert(raw_fd, subscriber);

        Ok(())
    }

    pub fn modify(&mut self, fd: RawFd, interests: EpollFlags) -> io::Result<()> {
        let subscriber = self
            .registered
            .get_mut(&fd)
            .ok_or(io::Error::new(io::ErrorKind::NotFound, "fd not registered"))?;
        let addr = subscriber.as_mut() as *mut S;
        let mut epoll_event = EpollEvent::new(interests, addr as u64);

        self.epoll.modify(subscriber.fd(), &mut epoll_event)?;
        subscriber.interests().set(interests);

        Ok(())
    }

    pub fn delete(&mut self, fd: RawFd) -> io::Result<()> {
        if unsafe {
            libc::epoll_ctl(
                self.epoll.0.as_raw_fd(),
                libc::EPOLL_CTL_DEL,
                fd,
                ptr::null_mut(),
            )
        } == -1
        {
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
            panic!("Recursive call to run().");
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
            let addr = ev.data() as *mut S;
            let subscriber = unsafe { &mut *addr };
            unsafe {
                self.handling.as_mut().unwrap_unchecked().fd = subscriber.fd().as_fd().as_raw_fd();
            }

            subscriber.handle(self, ev.events());
        }
        let handling = unsafe { self.handling.take().unwrap_unchecked() };
        for fd in handling.to_remove {
            self.registered.remove(&fd);
        }

        Ok(())
    }
}

pub trait Subscriber<S>: WithFd + WithInterests + Handler<S> {}

pub trait WithFd {
    type Fd: AsFd;

    fn fd(&self) -> Self::Fd;
}

pub trait WithInterests {
    fn interests(&self) -> &Cell<EpollFlags>;
}

pub trait Handler<S> {
    fn handle(&mut self, eventp: &mut Eventp<S>, events: EpollFlags);
}

/*
eventp.add(lt::read(server, handle));

fn handle(
server: &mut Server,
readable
writable
read_closed
write_closed
error
remove
modify
add
remove_self
)
*/
