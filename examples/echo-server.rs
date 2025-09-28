use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsFd, AsRawFd};

use eventp::tri_subscriber::WithHandler;
use eventp::{interest, Event, Eventp, EventpOps, Interest, Pinned, Subscriber};

// Set up an echo server on port 3000.
#[rustfmt::skip]
fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:3000")?;
    listener.set_nonblocking(true)?;

    let mut eventp = Eventp::default(); // Internally it creates an epoll fd.
    interest()
        .read()                         // Interested in readable events, i.e. new connections.
        .with_fd(listener)
        .with_handler(on_connection)
        .register_into(&mut eventp)?;

    eventp.run_forever()                // Loop: epoll_wait, dispatch event.
}

#[rustfmt::skip]
fn on_connection(
    listener: &mut impl Accept,         // Will receive `TcpListener`. To make it testable, we define a trait below.
    mut eventp: Pinned<impl EventpOps>, // Will receive `Pinned<Eventp>`.
) {
    let (stream, _) = listener.accept().expect("accept failed");

    interest()
        .edge_triggered()
        .read()                         // Interested in readable events, edge triggered.
        .with_fd(stream)
        .with_handler(on_data)
        .register_into(&mut eventp)
        .expect("add to epoll failed");
}

#[rustfmt::skip]
fn on_data(
    // Rustacean Dependency Injection. Place any parameters you like, in any order.
    _interest: Interest,                // Previously registered interests.
    mut eventp: Pinned<impl EventpOps>,
    ev: Event,                          // The triggered event.
    stream: &mut (impl Read + Write + AsFd),
) {
    if ev.is_error() || ev.is_hangup() {
        eventp
            .delete(stream.as_fd().as_raw_fd())
            .expect("delete from epoll failed");
    }
    if !ev.is_readable() {
        return;
    }

    let mut buf = [0; 512];
    loop {
        match stream.read(&mut buf) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                return;
            }
            Err(_) | Ok(0) => {
                eventp
                    .delete(stream.as_fd().as_raw_fd())
                    .expect("delete from epoll failed");
                return;
            }
            Ok(n) => stream.write_all(&buf[..n]).expect("write failed"),
        }
    }
}

#[cfg_attr(feature = "mock", mockall::automock(type Stream = MockStream;))]
trait Accept {
    type Stream: Read + Write + AsFd;

    fn accept(&self) -> io::Result<(Self::Stream, SocketAddr)>;
}

impl Accept for TcpListener {
    type Stream = TcpStream;

    fn accept(&self) -> io::Result<(Self::Stream, SocketAddr)> {
        let (stream, addr) = self.accept()?;
        stream.set_nonblocking(true)?;

        Ok((stream, addr))
    }
}

#[cfg(feature = "mock")]
mockall::mock! {
    pub Stream {}

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    }
    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
        fn flush(&mut self) -> io::Result<()>;
    }
    impl AsFd for Stream {
        fn as_fd(&self) -> std::os::fd::BorrowedFd<'_>;
    }
}

#[cfg(all(test, feature = "mock"))]
mod tests {
    use std::io::ErrorKind;
    use std::os::fd::BorrowedFd;

    use eventp::epoll::EpollFlags;
    use eventp::{pinned, MockEventp};
    use mockall::predicate::*;

    use super::*;

    #[test]
    fn test_on_connection_success() {
        // 1. Setup
        let mut mock_listener = MockAccept::new();
        let mut mock_eventp = MockEventp::new();

        mock_listener.expect_accept().returning(|| {
            let stream = MockStream::new();
            let addr = "127.0.0.1:12345".parse().unwrap();
            Ok((stream, addr))
        });

        mock_eventp
            .expect_add()
            .with(always())
            .times(1)
            .returning(|_| Ok(()));

        // 2. Act
        on_connection(&mut mock_listener, pinned!(mock_eventp));
    }

    #[test]
    fn test_on_stream_read_and_write() {
        // 1. Setup
        let mut mock_stream = MockStream::new();
        let mock_eventp = MockEventp::new();
        let mut seq = mockall::Sequence::new();

        let data = b"hello";
        let mut read_buf = [0u8; 1024];
        read_buf[..data.len()].copy_from_slice(data);

        mock_stream
            .expect_read()
            .times(1)
            .in_sequence(&mut seq)
            .returning(move |buf| {
                buf[..data.len()].copy_from_slice(data);
                Ok(data.len())
            });

        mock_stream
            .expect_write()
            .with(eq(data.as_slice()))
            .times(1)
            .in_sequence(&mut seq)
            .returning(|buf| Ok(buf.len()));

        mock_stream
            .expect_read()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_| Err(io::Error::new(ErrorKind::WouldBlock, "no more data")));

        // 2. Act
        on_data(
            Interest::default(),
            pinned!(mock_eventp),
            EpollFlags::EPOLLIN.into(),
            &mut mock_stream,
        );
    }

    #[test]
    fn test_on_stream_read_eof_closes_connection() {
        // 1. Setup
        let mut mock_stream = MockStream::new();
        let mut mock_eventp = MockEventp::new();
        let fd = 42;

        mock_stream
            .expect_as_fd()
            .returning(move || unsafe { BorrowedFd::borrow_raw(fd) });
        mock_stream.expect_read().times(1).returning(|_| Ok(0)); // EOF

        mock_eventp
            .expect_delete()
            .with(eq(fd))
            .times(1)
            .returning(|_| Ok(()));

        // 2. Act
        on_data(
            Interest::default(),
            pinned!(mock_eventp),
            EpollFlags::EPOLLIN.into(),
            &mut mock_stream,
        );
    }

    #[test]
    fn test_on_stream_read_error_closes_connection() {
        // 1. Setup
        let mut mock_stream = MockStream::new();
        let mut mock_eventp = MockEventp::new();
        let fd = 43;

        mock_stream
            .expect_as_fd()
            .returning(move || unsafe { BorrowedFd::borrow_raw(fd) });
        mock_stream
            .expect_read()
            .times(1)
            .returning(|_| Err(io::Error::new(ErrorKind::Other, "a real error")));

        mock_eventp
            .expect_delete()
            .with(eq(fd))
            .times(1)
            .returning(|_| Ok(()));

        // 2. Act
        on_data(
            Interest::default(),
            pinned!(mock_eventp),
            EpollFlags::EPOLLIN.into(),
            &mut mock_stream,
        );
    }

    #[test]
    fn test_on_stream_hup_or_err_event_closes_connection() {
        // 1. Setup
        let mut mock_stream = MockStream::new();
        let mut mock_eventp = MockEventp::new();
        let fd = 44;

        mock_stream
            .expect_as_fd()
            .returning(move || unsafe { BorrowedFd::borrow_raw(fd) });
        mock_stream.expect_read().never();
        mock_stream.expect_write().never();

        mock_eventp
            .expect_delete()
            .with(eq(fd))
            .times(1)
            .returning(|_| Ok(()));

        // 2. Act
        on_data(
            Interest::default(),
            pinned!(mock_eventp),
            (EpollFlags::EPOLLHUP | EpollFlags::EPOLLERR).into(),
            &mut mock_stream,
        );
    }
}
