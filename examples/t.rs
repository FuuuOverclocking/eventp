#![allow(unused)]
#![allow(clippy::type_complexity)]

use std::net::TcpListener;
use std::pin::Pin;

use eventp::{Event, Eventp, EventpOps, Subscriber, ThinBoxSubscriber, TriSubscriber};

fn main() {}

fn test(a: TriSubscriber<TcpListener, (Event,), fn(Event)>, mut ep: Eventp) {
    ep.add(ThinBoxSubscriber::new(a));
}
