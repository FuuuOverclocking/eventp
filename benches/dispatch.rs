//! Dispatch micro-benchmarks: eventp vs event-manager vs mio (with a user-side
//! Token→Handler table).
//!
//! Run with:
//!     cargo bench --bench dispatch
//!     cargo bench --bench dispatch -- dispatch_one_single_fd
//! HTML report: target/criterion/report/index.html
//!
//! All three reactors are exercised through `eventfd` sources to keep socket /
//! pipe I/O syscalls out of the measured path; only the dispatch glue differs.
//!
//! # Interpreting the numbers
//!
//! The absolute per-event time (~1.1 µs in groups 1 and 2) is dominated by the
//! kernel-side cost of one `epoll_wait`, one `eventfd_write` (the bench fires
//! the event) and one `eventfd_read` (the handler drains it). Those three
//! syscalls add up to roughly the full µs and are the same across all three
//! backends. The signal we care about is the **delta between backends**:
//! that is the actual dispatch overhead.
//!
//! Typical observed deltas on a quiet x86_64 host:
//! - group 1 (1 fd / sub):  event_manager ≈ eventp + 50 ns;  mio ≈ eventp + 25 ns
//! - group 2 (4 fds / sub): event_manager ≈ eventp + 75 ns  (the +25 ns vs
//!   group 1 is the third HashMap lookup in `process` — see technical doc §1.1)
//! - group 3 (per-event amortised): event_manager ≈ eventp + 60 ns
//!
//! These deltas sit on top of a ~1 µs floor that no userspace dispatcher can
//! avoid. Don't compare absolutes; compare the per-row deltas.

use std::cell::Cell;
use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::rc::Rc;
use std::time::{Duration, Instant};

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use nix::sys::eventfd::{EfdFlags, EventFd};

use eventp::epoll::{EpollCreateFlags, EpollTimeout};
use eventp::tri_subscriber::WithHandler;
use eventp::{Eventp, Subscriber};

use event_manager::{EventManager, EventOps, EventSet, Events, MutEventSubscriber, SubscriberOps};

use mio::unix::SourceFd;
use mio::{Events as MioEvents, Interest, Poll, Token};

use rustc_hash::FxHashMap;

// ---------- shared fixture ----------

fn new_eventfd() -> EventFd {
    EventFd::from_flags(EfdFlags::EFD_CLOEXEC | EfdFlags::EFD_NONBLOCK).unwrap()
}

#[inline]
fn fire(efd: &EventFd) {
    efd.write(1).expect("eventfd write");
}

#[inline]
fn drain(efd: &EventFd) {
    let _ = efd.read();
}

// An owned counter shared between the harness (which asserts the dispatch
// actually fired) and the per-event handler closures.
type Counter = Rc<Cell<u64>>;

// ===================================================================
// eventp side
// ===================================================================

mod eventp_impl {
    use super::*;

    pub struct Harness {
        pub reactor: Eventp,
        // Owns the writer end of each eventfd so the benchmark loop can fire
        // events without the subscriber giving up its own fd.
        pub writers: Vec<EventFd>,
        pub counter: Counter,
    }

    /// Build a reactor with `n` subscribers, each watching its own eventfd.
    /// The buffer capacity is sized to `cap` so a single `run_once` can drain
    /// any batch the bench fires.
    pub fn build(n: usize, cap: usize) -> Harness {
        let mut reactor =
            Eventp::new(cap.max(1), EpollCreateFlags::EPOLL_CLOEXEC).expect("Eventp::new");
        let counter: Counter = Rc::new(Cell::new(0));

        let mut writers = Vec::with_capacity(n);
        for _ in 0..n {
            let efd_for_sub = new_eventfd();
            // Dup the fd into a separate writer; both EventFd's point at the
            // same kernel-side eventfd object, so writing on `writer` wakes
            // the one registered with eventp.
            let dup = efd_for_sub
                .as_fd()
                .try_clone_to_owned()
                .expect("dup eventfd");
            let writer = unsafe { EventFd::from_owned_fd(dup) };
            writers.push(writer);

            let cnt = counter.clone();
            eventp::interest()
                .read()
                .with_fd(efd_for_sub)
                .with_handler(move |efd: &mut EventFd| {
                    drain(efd);
                    cnt.set(cnt.get() + 1);
                })
                .register_into(&mut reactor)
                .expect("eventp register");
        }

        Harness {
            reactor,
            writers,
            counter,
        }
    }

    /// Variant for the multi-fd bench: `n` "logical subscribers", each
    /// represented by `m` independent TriSubscribers (eventp's model is
    /// 1 fd = 1 subscriber). Returns the harness plus the per-logical-sub
    /// list of writer fds so the bench can fire one chosen fd of one chosen
    /// logical sub.
    pub fn build_multi(n: usize, m: usize, cap: usize) -> (Harness, Vec<Vec<EventFd>>) {
        let mut reactor =
            Eventp::new(cap.max(1), EpollCreateFlags::EPOLL_CLOEXEC).expect("Eventp::new");
        let counter: Counter = Rc::new(Cell::new(0));

        let mut grouped_writers: Vec<Vec<EventFd>> = Vec::with_capacity(n);
        for _ in 0..n {
            let mut group_writers = Vec::with_capacity(m);
            for _ in 0..m {
                let efd_for_sub = new_eventfd();
                let dup = efd_for_sub
                    .as_fd()
                    .try_clone_to_owned()
                    .expect("dup eventfd");
                let writer = unsafe { EventFd::from_owned_fd(dup) };
                group_writers.push(writer);

                let cnt = counter.clone();
                eventp::interest()
                    .read()
                    .with_fd(efd_for_sub)
                    .with_handler(move |efd: &mut EventFd| {
                        drain(efd);
                        cnt.set(cnt.get() + 1);
                    })
                    .register_into(&mut reactor)
                    .expect("eventp register");
            }
            grouped_writers.push(group_writers);
        }

        // Flatten writers separately so Harness still has the full set,
        // and return the grouping so the bench can pick (n_i, m_j).
        let flat_writers: Vec<EventFd> = grouped_writers
            .iter()
            .flatten()
            .map(|w| {
                let dup = w.as_fd().try_clone_to_owned().expect("dup");
                unsafe { EventFd::from_owned_fd(dup) }
            })
            .collect();

        (
            Harness {
                reactor,
                writers: flat_writers,
                counter,
            },
            grouped_writers,
        )
    }
}

// ===================================================================
// event-manager side
// ===================================================================

mod em_impl {
    use super::*;

    /// Single-fd subscriber: owns one EventFd. `process` drains it directly,
    /// so only the two HashMap lookups in EventManager's dispatch path
    /// (fd→SubscriberId, SubscriberId→Subscriber) are exercised — no
    /// third lookup is needed because the owned fd is right there as a field.
    pub struct CounterSub {
        pub event_fd: EventFd,
        pub cnt: Counter,
    }

    impl MutEventSubscriber for CounterSub {
        fn process(&mut self, _events: Events, _ops: &mut EventOps) {
            drain(&self.event_fd);
            self.cnt.set(self.cnt.get() + 1);
        }
        fn init(&mut self, ops: &mut EventOps) {
            ops.add(Events::new(
                &BorrowedFdAdapter(self.event_fd.as_raw_fd()),
                EventSet::IN,
            ))
            .expect("em add");
        }
    }

    /// Multi-fd subscriber: owns M EventFds keyed by RawFd in an internal
    /// HashMap. `process` receives only a RawFd (`events.fd()`), so the
    /// handler MUST do a third HashMap lookup to recover the owned fd —
    /// this is exactly the cost the eventp technical doc §1.1 identifies.
    pub struct MultiCounterSub {
        pub fds: HashMap<RawFd, EventFd>,
        pub cnt: Counter,
    }

    impl MutEventSubscriber for MultiCounterSub {
        fn process(&mut self, events: Events, _ops: &mut EventOps) {
            // The third HashMap lookup — forced by the (fd, owned_fd) split.
            if let Some(efd) = self.fds.get_mut(&events.fd()) {
                drain(efd);
                self.cnt.set(self.cnt.get() + 1);
            }
        }
        fn init(&mut self, ops: &mut EventOps) {
            for fd in self.fds.values() {
                ops.add(Events::new(
                    &BorrowedFdAdapter(fd.as_raw_fd()),
                    EventSet::IN,
                ))
                .expect("em add");
            }
        }
    }

    // event-manager's `Events::new` takes `&impl AsRawFd`. We want to feed it a
    // plain RawFd without depending on vmm_sys_util::EventFd, so wrap it.
    pub struct BorrowedFdAdapter(pub RawFd);
    impl std::os::fd::AsRawFd for BorrowedFdAdapter {
        fn as_raw_fd(&self) -> RawFd {
            self.0
        }
    }

    pub type Manager = EventManager<Box<dyn MutEventSubscriber>>;

    pub struct Harness {
        pub manager: Manager,
        pub writers: Vec<EventFd>,
        pub counter: Counter,
    }

    pub fn build(n: usize, cap: usize) -> Harness {
        let mut manager = EventManager::new_with_capacity(cap.max(1)).expect("EM::new");
        let counter: Counter = Rc::new(Cell::new(0));

        let mut writers = Vec::with_capacity(n);
        for _ in 0..n {
            let efd_for_sub = new_eventfd();
            let dup = efd_for_sub
                .as_fd()
                .try_clone_to_owned()
                .expect("dup eventfd");
            writers.push(unsafe { EventFd::from_owned_fd(dup) });

            let sub: Box<dyn MutEventSubscriber> = Box::new(CounterSub {
                event_fd: efd_for_sub,
                cnt: counter.clone(),
            });
            manager.add_subscriber(sub);
        }

        Harness {
            manager,
            writers,
            counter,
        }
    }

    /// Returns the harness and the grouped writers (n × m) so the bench can
    /// pick one specific fd to fire.
    pub fn build_multi(n: usize, m: usize, cap: usize) -> (Harness, Vec<Vec<EventFd>>) {
        let mut manager = EventManager::new_with_capacity(cap.max(1)).expect("EM::new");
        let counter: Counter = Rc::new(Cell::new(0));

        let mut grouped: Vec<Vec<EventFd>> = Vec::with_capacity(n);
        for _ in 0..n {
            let mut fds = HashMap::with_capacity(m);
            let mut group_writers = Vec::with_capacity(m);
            for _ in 0..m {
                let efd_for_sub = new_eventfd();
                let dup = efd_for_sub
                    .as_fd()
                    .try_clone_to_owned()
                    .expect("dup eventfd");
                group_writers.push(unsafe { EventFd::from_owned_fd(dup) });
                fds.insert(efd_for_sub.as_raw_fd(), efd_for_sub);
            }
            grouped.push(group_writers);

            let sub: Box<dyn MutEventSubscriber> = Box::new(MultiCounterSub {
                fds,
                cnt: counter.clone(),
            });
            manager.add_subscriber(sub);
        }

        // Flatten a duplicate list for Harness.writers (not actually used in
        // the multi-fd bench path; bench fires through `grouped`).
        let flat = Vec::new();

        (
            Harness {
                manager,
                writers: flat,
                counter,
            },
            grouped,
        )
    }
}

// ===================================================================
// mio side — with a user-supplied Token→Handler FxHashMap
// ===================================================================

mod mio_impl {
    use super::*;

    pub struct Harness {
        pub poll: Poll,
        pub events: MioEvents,
        // Closures capture their own writer fd; FxHashMap is the fastest a
        // mio user can reasonably write without resorting to unsafe tricks.
        pub table: FxHashMap<Token, Box<dyn FnMut()>>,
        // Keep registered fds alive for the lifetime of the harness.
        pub _owned_fds: Vec<EventFd>,
        pub writers: Vec<EventFd>,
        pub counter: Counter,
    }

    pub fn build(n: usize, cap: usize) -> Harness {
        let poll = Poll::new().expect("mio Poll::new");
        let events = MioEvents::with_capacity(cap.max(1));
        let mut table: FxHashMap<Token, Box<dyn FnMut()>> =
            FxHashMap::with_capacity_and_hasher(n, Default::default());
        let counter: Counter = Rc::new(Cell::new(0));
        let mut owned = Vec::with_capacity(n);
        let mut writers = Vec::with_capacity(n);

        for i in 0..n {
            let efd = new_eventfd();
            let raw = efd.as_raw_fd();
            poll.registry()
                .register(&mut SourceFd(&raw), Token(i), Interest::READABLE)
                .expect("mio register");

            // Each closure captures its own EventFd (dup'd from the
            // registered one) so drain can happen without an extra lookup.
            let drain_fd =
                unsafe { EventFd::from_owned_fd(efd.as_fd().try_clone_to_owned().expect("dup")) };
            let cnt = counter.clone();
            table.insert(
                Token(i),
                Box::new(move || {
                    drain(&drain_fd);
                    cnt.set(cnt.get() + 1);
                }),
            );

            let writer_dup = efd.as_fd().try_clone_to_owned().expect("dup");
            writers.push(unsafe { EventFd::from_owned_fd(writer_dup) });
            owned.push(efd);
        }

        Harness {
            poll,
            events,
            table,
            _owned_fds: owned,
            writers,
            counter,
        }
    }

    pub fn build_multi(n: usize, m: usize, cap: usize) -> (Harness, Vec<Vec<EventFd>>) {
        let poll = Poll::new().expect("mio Poll::new");
        let events = MioEvents::with_capacity(cap.max(1));
        let mut table: FxHashMap<Token, Box<dyn FnMut()>> =
            FxHashMap::with_capacity_and_hasher(n * m, Default::default());
        let counter: Counter = Rc::new(Cell::new(0));
        let mut owned = Vec::with_capacity(n * m);
        let mut grouped: Vec<Vec<EventFd>> = Vec::with_capacity(n);

        let mut tok = 0usize;
        for _ in 0..n {
            let mut gw = Vec::with_capacity(m);
            for _ in 0..m {
                let efd = new_eventfd();
                let raw = efd.as_raw_fd();
                poll.registry()
                    .register(&mut SourceFd(&raw), Token(tok), Interest::READABLE)
                    .expect("mio register");

                let drain_fd = unsafe {
                    EventFd::from_owned_fd(efd.as_fd().try_clone_to_owned().expect("dup"))
                };
                let cnt = counter.clone();
                table.insert(
                    Token(tok),
                    Box::new(move || {
                        drain(&drain_fd);
                        cnt.set(cnt.get() + 1);
                    }),
                );

                let writer_dup = efd.as_fd().try_clone_to_owned().expect("dup");
                gw.push(unsafe { EventFd::from_owned_fd(writer_dup) });
                owned.push(efd);
                tok += 1;
            }
            grouped.push(gw);
        }

        (
            Harness {
                poll,
                events,
                table,
                _owned_fds: owned,
                writers: Vec::new(),
                counter,
            },
            grouped,
        )
    }

    /// One poll + dispatch round, equivalent to `Eventp::run_once_with_timeout(0)`
    /// / `EventManager::run_with_timeout(0)`. Polls with zero timeout so it
    /// returns immediately after dispatching whatever is ready.
    pub fn run_once(h: &mut Harness) {
        h.poll
            .poll(&mut h.events, Some(Duration::from_secs(0)))
            .expect("mio poll");
        for ev in h.events.iter() {
            if let Some(handler) = h.table.get_mut(&ev.token()) {
                handler();
            }
        }
    }
}

// ===================================================================
// helpers
// ===================================================================

#[inline]
fn run_once_eventp(reactor: &mut Eventp) {
    reactor
        .run_once_with_timeout(EpollTimeout::from(0u16))
        .expect("eventp run_once");
}

#[inline]
fn run_once_em(manager: &mut em_impl::Manager) {
    let _ = manager.run_with_timeout(0).expect("em run");
}

// ===================================================================
// group 1: dispatch_one_ready_single_fd
// ===================================================================

// N=100_000 is large enough to push event-manager's HashMap entries out of L2
// (each entry is ~24 B, so 100k entries ≈ 2.4 MB), making the SipHash cache-miss
// effect actually visible above noise.
const SINGLE_FD_NS: &[usize] = &[1, 10, 100, 1_000, 10_000, 100_000];

fn bench_dispatch_one_single_fd(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch_one_single_fd");
    group.throughput(Throughput::Elements(1));

    for &n in SINGLE_FD_NS {
        // -------- eventp --------
        group.bench_with_input(BenchmarkId::new("eventp", n), &n, |b, &n| {
            let mut h = eventp_impl::build(n, n.max(1));
            // Fire the *same* fd each iteration; subscriber drains it so the
            // kernel-side counter resets, preventing level-triggered re-fire.
            let target = h.writers.len() / 2;
            b.iter(|| {
                fire(&h.writers[target]);
                run_once_eventp(&mut h.reactor);
                black_box(h.counter.get());
            });
            assert!(h.counter.get() > 0, "eventp: dispatch never fired");
        });

        // -------- event-manager --------
        group.bench_with_input(BenchmarkId::new("event_manager", n), &n, |b, &n| {
            let mut h = em_impl::build(n, n.max(1));
            let target = h.writers.len() / 2;
            b.iter(|| {
                fire(&h.writers[target]);
                run_once_em(&mut h.manager);
                black_box(h.counter.get());
            });
            assert!(h.counter.get() > 0, "event-manager: dispatch never fired");
        });

        // -------- mio + user table --------
        group.bench_with_input(BenchmarkId::new("mio_with_table", n), &n, |b, &n| {
            let mut h = mio_impl::build(n, n.max(1));
            let target = h.writers.len() / 2;
            b.iter(|| {
                fire(&h.writers[target]);
                mio_impl::run_once(&mut h);
                black_box(h.counter.get());
            });
            assert!(h.counter.get() > 0, "mio: dispatch never fired");
        });
    }

    group.finish();
}

// ===================================================================
// group 2: dispatch_one_ready_multi_fd (M = 4)
// ===================================================================

const MULTI_FD_NS: &[usize] = &[100, 1_000, 10_000];
const M: usize = 4;

fn bench_dispatch_one_multi_fd(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch_one_multi_fd_M4");
    group.throughput(Throughput::Elements(1));

    for &n in MULTI_FD_NS {
        let cap = (n * M).max(1);

        // -------- eventp --------
        group.bench_with_input(BenchmarkId::new("eventp", n), &n, |b, _| {
            let (mut h, grouped) = eventp_impl::build_multi(n, M, cap);
            let target_n = grouped.len() / 2;
            let target_m = M / 2;
            b.iter(|| {
                fire(&grouped[target_n][target_m]);
                run_once_eventp(&mut h.reactor);
                black_box(h.counter.get());
            });
            assert!(h.counter.get() > 0, "eventp multi: dispatch never fired");
        });

        // -------- event-manager (the 3rd HashMap lookup happens inside `process`) --------
        group.bench_with_input(BenchmarkId::new("event_manager", n), &n, |b, _| {
            let (mut h, grouped) = em_impl::build_multi(n, M, cap);
            let target_n = grouped.len() / 2;
            let target_m = M / 2;
            b.iter(|| {
                fire(&grouped[target_n][target_m]);
                run_once_em(&mut h.manager);
                black_box(h.counter.get());
            });
            assert!(h.counter.get() > 0, "em multi: dispatch never fired");
        });

        // -------- mio --------
        group.bench_with_input(BenchmarkId::new("mio_with_table", n), &n, |b, _| {
            let (mut h, grouped) = mio_impl::build_multi(n, M, cap);
            let target_n = grouped.len() / 2;
            let target_m = M / 2;
            b.iter(|| {
                fire(&grouped[target_n][target_m]);
                mio_impl::run_once(&mut h);
                black_box(h.counter.get());
            });
            assert!(h.counter.get() > 0, "mio multi: dispatch never fired");
        });
    }

    group.finish();
}

// ===================================================================
// group 3: dispatch_all_ready (single-fd, N events per iter)
// ===================================================================

const ALL_READY_NS: &[usize] = &[16, 64, 256, 1024];

fn bench_dispatch_all_ready(c: &mut Criterion) {
    let mut group = c.benchmark_group("dispatch_all_ready");

    for &n in ALL_READY_NS {
        group.throughput(Throughput::Elements(n as u64));
        let cap = n.max(1);

        group.bench_with_input(BenchmarkId::new("eventp", n), &n, |b, _| {
            let mut h = eventp_impl::build(n, cap);
            b.iter(|| {
                for w in &h.writers {
                    fire(w);
                }
                run_once_eventp(&mut h.reactor);
                black_box(h.counter.get());
            });
        });

        group.bench_with_input(BenchmarkId::new("event_manager", n), &n, |b, _| {
            let mut h = em_impl::build(n, cap);
            b.iter(|| {
                for w in &h.writers {
                    fire(w);
                }
                run_once_em(&mut h.manager);
                black_box(h.counter.get());
            });
        });

        group.bench_with_input(BenchmarkId::new("mio_with_table", n), &n, |b, _| {
            let mut h = mio_impl::build(n, cap);
            b.iter(|| {
                for w in &h.writers {
                    fire(w);
                }
                mio_impl::run_once(&mut h);
                black_box(h.counter.get());
            });
        });
    }

    group.finish();
}

// ===================================================================
// group 4: register / unregister single-op
// ===================================================================

const REG_NS: &[usize] = &[10, 1_000];

fn bench_register(c: &mut Criterion) {
    let mut group = c.benchmark_group("register_one");

    // Use `iter_custom` to time *only* the registration call, not the
    // build-N-subs setup that produces the fixture. `iter_batched` is unsafe
    // here because at large N (10³+) the setup cost is comparable to or
    // larger than the routine, and criterion's batching does not always
    // keep them apart cleanly (see commit history for details).
    for &n in REG_NS {
        group.bench_with_input(BenchmarkId::new("eventp", n), &n, |b, &n| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let mut h = eventp_impl::build(n, (n + 1).max(1));
                    let efd = new_eventfd();
                    let start = Instant::now();
                    eventp::interest()
                        .read()
                        .with_fd(efd)
                        .with_handler(|_efd: &mut EventFd| {})
                        .register_into(&mut h.reactor)
                        .unwrap();
                    total += start.elapsed();
                    drop(h);
                }
                total
            });
        });

        group.bench_with_input(BenchmarkId::new("event_manager", n), &n, |b, &n| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let mut h = em_impl::build(n, (n + 1).max(1));
                    let efd = new_eventfd();
                    let cnt = h.counter.clone();
                    let sub: Box<dyn MutEventSubscriber> =
                        Box::new(em_impl::CounterSub { event_fd: efd, cnt });
                    let start = Instant::now();
                    h.manager.add_subscriber(sub);
                    total += start.elapsed();
                    drop(h);
                }
                total
            });
        });

        group.bench_with_input(BenchmarkId::new("mio_with_table", n), &n, |b, &n| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let mut tok_counter: usize = 1_000_000_000;
                for _ in 0..iters {
                    let mut h = mio_impl::build(n, (n + 1).max(1));
                    let efd = new_eventfd();
                    let raw = efd.as_raw_fd();
                    tok_counter = tok_counter.wrapping_add(1);
                    let tok = Token(tok_counter);
                    let start = Instant::now();
                    h.poll
                        .registry()
                        .register(&mut SourceFd(&raw), tok, Interest::READABLE)
                        .unwrap();
                    h.table.insert(tok, Box::new(|| {}));
                    total += start.elapsed();
                    h._owned_fds.push(efd);
                    drop(h);
                }
                total
            });
        });
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(50)
        // 5 s per case is enough for the 50-100 ns deltas to clear noise on
        // a quiet host; on a busy host this won't help — pin CPUs first.
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(5));
    targets =
        bench_dispatch_one_single_fd,
        bench_dispatch_one_multi_fd,
        bench_dispatch_all_ready,
        bench_register,
}
criterion_main!(benches);
