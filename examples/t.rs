#![allow(unused)]
#![allow(clippy::type_complexity)]

use std::{net::TcpListener, pin::Pin};

use eventp::{Eventp, EventpOps, Subscriber, Subscriber1, ThinBoxSubscriber};

fn main() {}

fn test(
    a: Subscriber1<
        TcpListener,
        (&mut TcpListener,),
        fn(&mut TcpListener),
    >,
    mut ep: Eventp,
) {
    ep.add(ThinBoxSubscriber::new(a));
}
