use std::cell::Cell;
use std::collections::HashMap;
use std::mem::{self, MaybeUninit};
use std::ops::DerefMut;
use std::os::fd::{AsRawFd, RawFd};
use std::{io, ptr};

use nix::libc;
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use vptr::{ThinBox, ThinRefMut};

/// Dark magic to determine if type is `Sized`.
const fn is_sized<T: ?Sized>() -> bool {
    size_of::<&T>() == size_of::<&()>()
}

fn epoll_ctl(epfd: &Epoll, op: i32, fd: RawFd, mut event: Option<EpollEvent>) -> io::Result<()> {
    let event = match &mut event {
        Some(ev) => ev,
        None => ptr::null_mut(),
    };
    let ret = unsafe { libc::epoll_ctl(epfd.0.as_raw_fd(), op, fd, event as *mut _) };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub struct Eventp<S: ?Sized> {
    // Drop order: `registered` firstly, then `epoll`.
    registered: HashMap<RawFd, (Box<S>, u64)>,
    epoll: Epoll,
    buf: Vec<MaybeUninit<EpollEvent>>,
    handling: Option<Handling>,
}

struct Handling {
    fd: RawFd,
    to_remove: Vec<RawFd>,
}

impl<S: ?Sized> Eventp<S> {
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

impl<S: ?Sized> Eventp<S>
where
    S: Subscriber<S>,
{
    pub fn add<T>(&mut self, mut subscriber: T) -> io::Result<()>
    where
        T: AsThinPtrMut + IntoBox<S>,
    {
        let addr = subscriber.as_thin_ptr_mut() as u64;

        let subscriber = subscriber.into_box();
        let raw_fd = subscriber.as_raw_fd();
        let interests = subscriber.interests().get();

        let epoll_event = EpollEvent::new(interests, addr);

        epoll_ctl(&self.epoll, libc::EPOLL_CTL_ADD, raw_fd, Some(epoll_event))?;
        self.registered.insert(raw_fd, (subscriber, addr));

        Ok(())
    }

    pub fn modify(&mut self, fd: RawFd, interests: EpollFlags) -> io::Result<()> {
        let (subscriber, addr) = self
            .registered
            .get_mut(&fd)
            .ok_or(io::Error::new(io::ErrorKind::NotFound, "fd not registered"))?;
        let epoll_event = EpollEvent::new(interests, *addr);
        epoll_ctl(&self.epoll, libc::EPOLL_CTL_MOD, fd, Some(epoll_event))?;
        subscriber.interests().set(interests);

        Ok(())
    }

    pub fn delete(&mut self, fd: RawFd) -> io::Result<()> {
        epoll_ctl(&self.epoll, libc::EPOLL_CTL_DEL, fd, None)?;
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
            let addr = ev.data();
            // Deep dark magic!
            let subscriber: &mut S = unsafe {
                if is_sized::<S>() {
                    mem::transmute_copy(&addr)
                } else {
                    let mut thin_ref: ThinRefMut<'_, S> = mem::transmute(addr);
                    mem::transmute::<&mut S, &mut S>(thin_ref.deref_mut())
                }
            };
            unsafe {
                self.handling.as_mut().unwrap_unchecked().fd = subscriber.as_raw_fd();
            }

            subscriber.handle(ev.events(), self);
        }
        let handling = unsafe { self.handling.take().unwrap_unchecked() };
        for fd in handling.to_remove {
            self.registered.remove(&fd);
        }

        Ok(())
    }
}

pub trait AsThinPtrMut {
    fn as_thin_ptr_mut(&mut self) -> usize;
}

pub trait IntoBox<T: ?Sized> {
    fn into_box(self) -> Box<T>;
}

impl<T> AsThinPtrMut for Box<T> {
    fn as_thin_ptr_mut(&mut self) -> usize {
        self.as_mut() as *mut _ as usize
    }
}

impl<T> IntoBox<T> for Box<T> {
    fn into_box(self) -> Box<T> {
        self
    }
}

#[cfg(feature = "vptr")]
impl<T> AsThinPtrMut for ThinBox<T>
where
    T: ?Sized + 'static,
{
    fn as_thin_ptr_mut(&mut self) -> usize {
        let ptr = ThinBox::as_thin_ref_mut(self);
        unsafe { mem::transmute(ptr) }
    }
}

#[cfg(feature = "vptr")]
impl<T> IntoBox<T> for ThinBox<T>
where
    T: ?Sized + 'static,
{
    fn into_box(self) -> Box<T> {
        ThinBox::into_box(self)
    }
}

pub trait Subscriber<S>: Handler<S> + WithInterests + AsRawFd
where
    S: ?Sized,
{
}

pub trait WithInterests {
    fn interests(&self) -> &Cell<EpollFlags>;
}

pub trait Handler<S>
where
    S: ?Sized,
{
    fn handle(&mut self, events: EpollFlags, eventp: &mut Eventp<S>);
}
