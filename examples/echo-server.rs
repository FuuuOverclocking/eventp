use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};

use eventp::{interests, EventP, Subscriber};
use nix::sys::epoll::EpollFlags;

fn main() -> io::Result<()> {
    let listener = UnixListener::bind("/tmp/echo.sock")?;
    listener.set_nonblocking(true)?;

    let mut eventp = EventP::default();
    interests()
        .edge_triggered()
        .read()
        .with_fd(listener)
        .finish(on_connection)
        .register_into(&mut eventp)?;

    loop {
        eventp.run()?;
    }
}

fn on_connection(listener: &mut UnixListener, eventp: &mut EventP) {
    let (stream, _) = listener.accept().expect("accept failed");
    stream
        .set_nonblocking(true)
        .expect("set nonblocking failed");

    interests()
        .edge_triggered()
        .read()
        .with_fd(stream)
        .finish(on_stream)
        .register_into(eventp)
        .expect("add to epoll failed");
}

fn on_stream(stream: &mut UnixStream, events: EpollFlags, eventp: &mut EventP) {
    if events.contains(EpollFlags::EPOLLIN) {
        let mut buf = [0; 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => {
                    eventp
                        .delete(stream.as_raw_fd())
                        .expect("delete from epoll failed");
                    return;
                }
                Ok(n) => stream.write_all(&buf[..n]).expect("write failed"),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return;
                }
                Err(e) => {
                    eprintln!("{}", e);
                    eventp
                        .delete(stream.as_raw_fd())
                        .expect("delete from epoll failed");
                }
            }
        }
    }
    if events.intersects(EpollFlags::EPOLLHUP | EpollFlags::EPOLLERR) {
        eventp
            .delete(stream.as_raw_fd())
            .expect("delete from epoll failed");
    }
}
