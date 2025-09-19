use std::os::fd::{AsRawFd, RawFd};
use std::{io, ptr};

use nix::libc;
use nix::sys::epoll::{Epoll, EpollEvent};

/// Dark magic to determine if type is `Sized`.
pub const fn is_sized<T: ?Sized>() -> bool {
    size_of::<&T>() == size_of::<&()>()
}

pub fn epoll_ctl(
    epfd: &Epoll,
    op: i32,
    fd: RawFd,
    mut event: Option<EpollEvent>,
) -> io::Result<()> {
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
