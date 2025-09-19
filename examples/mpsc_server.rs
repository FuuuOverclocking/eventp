use std::cell::Cell;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::{Arc, mpsc};

use eventp::{
    DynSubscriber, Eventp, EventpDyn, EventpOps, Handler, IntoBox, Subscriber, WithInterests,
};
use nix::sys::epoll::EpollFlags;
use nix::sys::eventfd::{EfdFlags, EventFd};

struct Server {
    rx: mpsc::Receiver<(String, oneshot::Sender<String>)>,
    eventfd: Arc<EventFd>,
}

struct Client {
    tx: mpsc::Sender<(String, oneshot::Sender<String>)>,
    eventfd: Arc<EventFd>,
}

fn new_cs() -> io::Result<(Client, Server)> {
    let (tx, rx) = mpsc::channel();
    let eventfd = Arc::new(EventFd::from_flags(
        EfdFlags::EFD_NONBLOCK | EfdFlags::EFD_CLOEXEC,
    )?);

    let client = Client {
        tx: tx.clone(),
        eventfd: Arc::clone(&eventfd),
    };

    let server = Server { rx, eventfd };

    Ok((client, server))
}

struct MySubscriber {
    server: Server,
    interests: Cell<EpollFlags>,
}

impl AsRawFd for MySubscriber {
    fn as_raw_fd(&self) -> RawFd {
        self.server.eventfd.as_raw_fd()
    }
}

impl WithInterests for MySubscriber {
    fn interests(&self) -> &Cell<EpollFlags> {
        &self.interests
    }
}

impl Handler for MySubscriber {
    type Eventp = Eventp<MySubscriber>;

    fn handle(&mut self, events: EpollFlags, eventp: &mut Eventp<MySubscriber>) {
        let _ = self.server.eventfd.read();

        loop {
            let (req, respond) = match self.server.rx.try_recv() {
                Ok(x) => x,
                Err(_) => break,
            };
            respond.send(format!("Hello, {req}")).ok();
        }
    }
}

fn main() {
    let (client, server) = new_cs().unwrap();
    let my_subscriber = MySubscriber {
        server,
        interests: Cell::new(EpollFlags::EPOLLIN | EpollFlags::EPOLLET),
    };
    let mut eventp = EventpDyn::new().unwrap();
    eventp
        .add(Box::new(my_subscriber) as Box<DynSubscriber>)
        .unwrap();
}
