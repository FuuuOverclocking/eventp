use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::mem::{self, MaybeUninit};
use std::os::fd::{AsFd, AsRawFd, RawFd};

use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};

pub struct Eventp<S> {
    // Drop order: `registered` firstly, then `epoll`.
    registered: HashMap<RawFd, Box<S>>,
    epoll: Epoll,
    buf: Vec<MaybeUninit<EpollEvent>>,
    handling: Option<(RawFd, bool)>,
}

impl<S> Default for Eventp<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Eventp<S> {
    fn new() -> Self {
        Self::with_flags(EpollCreateFlags::EPOLL_CLOEXEC)
    }

    fn with_flags(flags: EpollCreateFlags) -> Self {
        Self {
            epoll: Epoll::new(flags).unwrap(),
            registered: Default::default(),
            buf: vec![MaybeUninit::uninit(); 256],
            handling: None,
        }
    }
}

impl<S> Eventp<S>
where
    S: Subscriber<S>,
{
    fn add(&mut self, mut subscriber: Box<S>) {
        let raw_fd = subscriber.fd().as_fd().as_raw_fd();
        if self.registered.contains_key(&raw_fd) {
            panic!();
        }

        let interests = subscriber.interests().get();
        let addr = subscriber.as_mut() as *mut S;
        let epoll_event = EpollEvent::new(interests, addr as u64);

        self.epoll.add(subscriber.fd(), epoll_event).unwrap();
        self.registered.insert(raw_fd, subscriber);
    }

    fn modify(&mut self, fd: RawFd, interests: EpollFlags) {
        let subscriber = self.registered.get_mut(&fd).expect("Subscriber not found");
        let addr = subscriber.as_mut() as *mut S;
        let mut epoll_event = EpollEvent::new(interests, addr as u64);

        self.epoll
            .modify(subscriber.fd(), &mut epoll_event)
            .unwrap();
        subscriber.interests().set(interests);
    }

    fn delete(&mut self, fd: RawFd) {
        if let Some(handling) = &mut self.handling {
            if handling.0 == fd {
                handling.1 = true;
                return;
            }
        }

        let subscriber = self.registered.remove(&fd).expect("Subscriber not found");
        self.epoll.delete(subscriber.fd()).unwrap();
    }

    fn run(&mut self) {
        assert!(self.handling.is_none());

        // Use `BorrowedBuf` instead, once it becomes stable.
        let buf: &mut [MaybeUninit<EpollEvent>] = &mut self.buf;
        let buf: &mut [EpollEvent] = unsafe { mem::transmute(buf) };

        let n = self.epoll.wait(buf, EpollTimeout::NONE).unwrap();
        let buf = &buf[..n];

        for ev in buf {
            let addr = ev.data() as *mut S;
            let subscriber = unsafe { &mut *addr };
            let fd = subscriber.fd().as_fd().as_raw_fd();
            self.handling = Some((fd, false));

            subscriber.handle(self, ev.events());

            if self.handling.unwrap().1 {
                let subscriber = self.registered.remove(&fd).expect("Subscriber not found");
                self.epoll.delete(subscriber.fd()).unwrap();
            }
        }
        self.handling = None;
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
