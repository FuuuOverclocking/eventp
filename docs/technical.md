# Technical

English | [中文](crate::_technical_zh)

`Eventp` is a zero-overhead event dispatch mechanism with a clean, test-friendly
API. This page tells the story of how it gets there.

---
## 1. From mio, to event-manager, to eventp

[mio](https://docs.rs/mio/latest/mio/) is a thin, cross-platform wrapper over
`epoll`/`kqueue`/IOCP. You ask it to watch fds, it tells you which ones are
ready, you `match` on a `Token` (a `usize` you picked) to figure out what to
do. It is essentially "raw `epoll` with a portable accent" — see
[mio's tcp_server example](https://github.com/tokio-rs/mio/blob/master/examples/tcp_server.rs)
for the flavor.

[event-manager](https://docs.rs/event-manager/latest/event_manager/) goes a
step further: it adds a real *subscription* layer. Each fd is owned by a
`Subscriber` object that knows how to handle its own events; the dispatch
table is mutable at runtime; new sources can be registered from inside a
handler. This is a much nicer programming model for large projects (think
rust-vmm), and the
[basic example](https://github.com/rust-vmm/event-manager?tab=readme-ov-file#basic-single-thread-subscriber)
shows the kind of code you actually want to write.

So far, so good. But there is a price.

### 1.1 The price: three HashMap lookups per event

When a `Subscriber`'s handler fires, it usually wants to do two things:

1. Read or write its own data (`&mut self`).
2. Mutate the reactor — add a new connection, remove itself, change interest
   flags (`&mut Reactor`).

These two `&mut`s overlap, because `Subscriber` *lives inside* `Reactor`. Rust
refuses to compile that. The straightforward workaround is to give up
co-locating them and shuffle ownership around. event-manager does exactly
that, with **four** `HashMap`s in a three-layer structure:

![event-manager](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/event-manager.svg)

The two `&mut`s now come from genuinely distinct objects, and the borrow
checker is appeased. The cost is three `HashMap` lookups per dispatched event.

It gets a little worse: the maps use `std::collections::HashMap`, whose
default hasher is SipHash 1-3 — a HashDoS-resistant hash, which is great for
HTTP headers, but our keys are *small integer fds handed out by the kernel*.
There is no attacker. We are paying for armor against a threat that does not
exist.

### 1.2 The hidden bomb: fd reuse and ghost events

Routing events by `RawFd` makes a whole class of ABA bugs **all too easy
to trip over**. POSIX specifies that `open(2)`, `accept(2)`, `socket(2)`,
`pipe(2)` and friends return **the lowest-numbered fd not currently in use
by this process**. So the moment a fd is closed, its integer is the
*first candidate* for the next fd you open. Reuse is the norm, not the
exception.

Consider the kind of sequence fd-keyed dispatch invites:

1. Subscriber `A` holds `fd = 7`, registered into the reactor; the dispatch
   table has a row keyed by `7`.
2. `A`'s destructor (or some deeper chain it triggers) closes `fd = 7` but
   forgets to unregister.
3. The process later `accept`s a new connection. The kernel hands back the
   number `7` for it. The application registers it as subscriber `B`.
4. epoll fires. The reactor looks up `fd = 7`, lands on `A`'s entry, and
   **dispatches the event to a corpse**.

This class of bug — let us call it the *ghost event* — has three flavors of
nasty:

- **Silent**. The compiler can't see it. Unit tests almost never reproduce
  it. It only shows up in production, on a busy day, with a postmortem.
- **It crosses ownership boundaries**. Even after `A`'s storage has been
  freed and recycled for something else, the stale `RawFd → subscriber id`
  row still exists. Events get routed to whoever happens to occupy that
  memory now. Have fun.
- **It's not really the user's fault**. The API *shape* encourages
  "close-then-remove" ordering, especially when `close` is invoked deep
  inside a `Drop` chain. Pushing this invariant onto users is a design
  smell.

### 1.3 The eventp insight

`epoll_ctl(2)` lets you attach an arbitrary 8-byte payload (`epoll_data_t`)
to every registered fd. When the event fires, `epoll_wait(2)` hands the same
payload back. Semantically, it's a free-form "context pointer" slot —
that's literally how the man page recommends using it.

So: **put the heap address of the handler object in there**. When the event
fires, we transmute the `u64` back to a pointer, do one virtual call, and
we're in user code. No hashing. No lookup. One `callq`. Done.

This also vaporizes the ghost-event class entirely: routing now follows the
object pointer the kernel hands back, not a `RawFd` lookup. The fd integer
being reused is irrelevant — different fd, different registration, different
pointer. And `Eventp::delete` is wired so that releasing the subscriber and
calling `EPOLL_CTL_DEL` are inseparable, which means *"forget to remove"* is
no longer an option the API even exposes.

Of course, none of this is free. To make it work we need to solve three
Rust-specific puzzles, and that's what the rest of this document is about:

1. `&dyn Trait` is 16 bytes on 64-bit. It doesn't fit in a `u64`.
2. Handing `&mut Reactor` to a handler that itself lives inside the reactor
   is a textbook double-mutable-borrow.
3. Handlers may mutate the reactor (add, modify, delete — possibly themselves)
   while a batch of events is mid-dispatch. We need this to be sound.


---

## 2. Slimming Down Fat Pointers: `ThinBoxSubscriber`

### 2.1 Why runtime polymorphism (and why that's a problem)

You might ask: why not just parameterize the reactor over `T: Subscriber` and
let monomorphization sort it out? In practice — VMM-style codebases especially
— roughly 90% of real reactors hold subscribers of *many* different concrete
types: a control eventfd, a TCP listener, a bunch of TCP connections, a
serial console, a vsock channel, … The moment a generic parameter shows up in
the reactor type, it virally infects every owner all the way to `fn main`.
Trait objects are the practical answer, and we will pay the one indirect
call.

The trouble is Rust's trait-object representation:

<figure style="display: inline-block;">
<img src="https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/pointer-meta.svg" alt="Rust fat pointer" />
<figcaption style="text-align: center;">Rust fat pointer</figcaption>
</figure>

<figure style="display: inline-block;">
<img src="https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/cpp-vptr.svg" alt="C++ single-inheritance vptr layout" />
<figcaption style="text-align: center;">C++ single-inheritance</figcaption>
</figure>

Rust's `&dyn Trait` is a **fat pointer**: data pointer + vtable pointer,
16 bytes on a 64-bit target. That is 8 bytes too many for `epoll_data_t`.

### 2.2 Insight: don't be afraid of the allocator

`rustc`'s memory layout is a default, not a prison. If we manage the
allocation ourselves, nothing stops us from putting the vtable pointer
**inside** the object, C++-style. Then a pointer to the object is just one
word — and that one word goes straight into `epoll_event.data`.

### 2.3 First sketch

Let's build it step by step.

```rust,ignore
pub struct ThinBoxSubscriber {
    ptr: NonNull<u8>,
    _marker: PhantomData<dyn Subscriber>,
}

impl ThinBoxSubscriber {
    pub fn new<T: Subscriber>(value: T) -> Self {
        todo!()
    }
}
```

#### Step 1: cordon off the exotic cases

We only support 64-bit Linux. Anything else is a compile error:

```rust,ignore
#[cfg(not(target_pointer_width = "64"))]
compile_error!("Platforms with pointer width other than 64 are not supported.");
```

With that, we can nail down a fact at *compile* time:

```rust,ignore
const _: () = assert!(size_of::<&dyn Subscriber>() == 16);
```

If a future toolchain ever changes trait-object layout, the build fails on
the spot — no silent miscompile.

#### Step 2: pry the vtable out of a fat pointer

A fat pointer *is* a `(data, vtable)` pair in memory. So we `transmute` it:

```rust,ignore
let fat_ptr = &value as &dyn Subscriber;
let (_data_ptr, vptr) = unsafe {
    mem::transmute::<&dyn Subscriber, (*const (), *const ())>(fat_ptr)
};
```

Now we want a heap layout that starts with the vptr:

<figure style="display: inline-block;">
<img src="https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/step-2.svg" alt="initial layout: vptr followed by T" />
<figcaption style="text-align: center;">First attempt: <code>(vptr, T)</code></figcaption>
</figure>

**Small but fatal: the alignment hole.** If `T` has an alignment greater than `usize`
(think `#[repr(align(16))]` or a struct containing `__m128`), the compiler quietly inserts
padding between `vptr` and `value`:

![step-2-align-issue](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/step-2-align-issue.svg)

So `value` is *not* at `ptr + size_of::<usize>()`. Our deref math is off.
Cue undefined behavior.

**Trick: keep `vptr` adjacent to `value`, let padding fall outside.** Use
[`Layout::extend`] to compose a one-`usize` header (which will hold the
vtable pointer) with the layout of `T`. The allocator returns the offset of
`T` for free, and inserts any padding *before* the header instead of between
the header and `T`:

[`Layout::extend`]: core::alloc::Layout::extend

```rust,ignore
let (layout, value_offset) = Layout::new::<usize>()
    .extend(Layout::new::<T>())
    .expect("Failed to create combined layout");
```

We then make `ptr` point at `T`, and read `vptr` at the fixed negative
offset `ptr - 8`.

> Exercise: why is the vptr offset valid? (hint: align rules of repr C)

![step-2-align-issue-solved](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/step-2-align-issue-solved.svg)

#### Step 3: allocate, place, point

```rust,ignore
let ptr = unsafe {
    let raw = alloc::alloc(layout);
    if raw.is_null() { alloc::handle_alloc_error(layout); }
    NonNull::new_unchecked(raw.add(value_offset))   // point at T, not at the allocation
};
unsafe {
    ptr.as_ptr().sub(size_of::<usize>())            // vptr slot
       .cast::<*const ()>().write(vptr);
    ptr.as_ptr().cast::<T>().write(value);          // move T in
}
```

`Deref` is the same trick in reverse — read the vptr from `ptr - 8`,
combine it with `ptr` into a fat pointer, hand out `&mut dyn Subscriber<Ep>`.

### 2.4 Drop, panic-safely

Drop is where it gets fun. We have to:

1. Run `T`'s destructor.
2. `dealloc` the heap slot.

What if step 1 panics? Per
[panic-in-drop discussion](https://github.com/Amanieu/rfcs/blob/panic-in-drop/text/0000-panic-in-drop.md)
(the RFC was withdrawn, but the behavior stands), a panic inside `Drop`
unwinds. If we naively wrote `drop_in_place(value); dealloc(ptr)`, an unwind
through step 1 would skip step 2 — and leak.

The trick is the classic *guard-inside-Drop* pattern: hand the deallocation
responsibility to a local struct whose own `Drop` is unconditional:

```rust,ignore
let _guard = DropGuard { ptr, value_layout, _marker: PhantomData };
unsafe { ptr::drop_in_place(value_ptr) };  // may panic
// _guard.drop() runs in either path and calls alloc::dealloc.
```

The same pattern shows up in [`Vec`] and most other RAII containers — but
this is one of the rare cases where you actually need to write it yourself.

[`Vec`]: std::vec::Vec

### 2.5 The differences vs. the real code

The real [src/thin.rs](https://github.com/FuuuOverclocking/eventp/blob/main/src/thin.rs)
is slightly fancier than what's above:

- **The header also stores `raw_fd`** (next to `vptr`). This avoids some virtual
  calls to `as_fd()`. It also serves as a sentinel, where a value of -1 indicates
  that the `value` has been `drop_in_place`d but the heap slot is still alive.
  We will use this in §4 to make reentrant deletion sound.
- **`Subscriber<Ep>` is generic over the reactor type** (so that the mock
  reactor can plug into the same `ThinBoxSubscriber<MockEventp>`). It's
  uniform churn, not interesting on its own.
- **`from_box_dyn`** lets you convert an *already type-erased*
  `Box<dyn Subscriber<Ep>>` into a `ThinBoxSubscriber`.

### 2.6 Why this kills the fd-reuse bug for free

Routing now goes:

```text
epoll_wait → ev.data() (u64) → reinterpret as &mut dyn Subscriber<Ep>
```

There is no `RawFd → subscriber` map on the dispatch path. The kernel hands
back the exact heap address you registered, so the only way a "ghost"
subscriber could receive an event is if its heap slot were deallocated
behind `epoll`'s back — and the only API that removes a subscriber
(`Eventp::delete`) is the same one that calls `EPOLL_CTL_DEL`. The two are
welded together; you cannot have one without the other.

---

## 3. The Double Mutable Borrow

### 3.1 The interface we wish we could write

What we'd like in user code is brutally obvious:

```rust,ignore
trait Subscriber {
    fn handle(&mut self, reactor: &mut Eventp);
}
```

What the borrow checker thinks of it:

```text
error[E0499]: cannot borrow `*reactor` as mutable more than once at a time
```

…because `*self` *lives inside* `reactor.registered`, and you have just
asked for two `&mut`s that overlap. event-manager's response was the
three-layer HashMap structure from §1; the cost was 3 lookups per event.
We'd rather not pay that.

### 3.2 Approaching the problem from the other side

Let's invert the framing. Suppose we have:

```rust,ignore
use rustc_hash::FxHashMap;  // fast hasher, no DoS resistance (we don't need it)

struct Eventp {
    registered: FxHashMap<RawFd, ThinBoxSubscriber>,
    // ...
}
```

Suppose we accept "splitting" `&mut Eventp` into two logical halves:

- `&mut subscriber_i` — the one currently dispatching
- `&mut (Eventp − subscriber_i)` — everything else

We *know*, by §2, that `ThinBoxSubscriber` is just a pointer. The actual
subscriber bytes live on *another* heap allocation that the map merely
references. So when we pluck out `&mut (self.registered[fd].deref())` and hand it to
`Subscriber::handle`, the only thing that can invalidate it is something
that frees or moves the heap slot under us.

Now, what could `&mut Eventp` actually *do* to that heap slot, during the
handler call? Three things:

1. **Public field access** (`reactor.registered = ...`). Easy to forbid: don't
   expose any fields as `pub`.
2. **Public method calls** (`reactor.some_method(&mut self)`). Annoying, but
   *we* control the method set. We can just not expose anything dangerous.
3. **`mem::replace`, `mem::take`, `*reactor = new_reactor`**. 💥 The old
   `Eventp` is destructed *right now*, including the entire `registered`
   map, including the heap slot we were currently inside. The `&mut self`
   that the handler holds is suddenly pointing into freed memory.

Categories 1 and 2 are in our hands. Category 3 is the actual showstopper.

### 3.3 Descending deeper into the dark arts: `Pin`

We need a way to hand the handler "something with `&mut Eventp`-ish powers,
**but with category 3 surgically removed**". Fortunately, Rust has already
been here. When async/await was being designed, [`Future`] faced the exact
same crisis — a `Future` returned by `async fn` is a self-referential state
machine, and `mem::replace`-ing it would invalidate its own internal
pointers. The fix, after a lot of debate and a lot of documentation, was
[`Pin`].

[`Future`]: core::future::Future
[`Pin`]: core::pin

Skipping the
[sixteen chapters of Pin documentation](core::pin):
the only thing it does that matters here is that safe code **cannot turn
`Pin<&mut T>` back into `&mut T`** unless `T: Unpin`. Inherent methods on the
pinned type can use `unsafe` internally to project back to `&mut T`, but
those methods are written by the type's author and can be chosen to never
move the value out.

So: mark `Eventp` as `!Unpin` (one `PhantomPinned` field is enough), and
hand the handler a `Pin<&mut Eventp>`. Category 3 is gone. Safe user code
*cannot* `mem::replace` the reactor.

```rust,ignore
struct Eventp {
    registered: FxHashMap<RawFd, ThinBoxSubscriber>,
    _pinned: PhantomPinned,
    // ...
}

trait Subscriber {
    fn handle(&mut self, reactor: Pin<&mut Eventp>);
    //                            ^^^^^^^^^^^^^^^^
    //              "you can use it, but you cannot make it stop existing"
}
```

Before you cheer: keep [The Problem With Single-threaded Shared
Mutability](https://manishearth.github.io/blog/2015/05/17/the-problem-with-shared-mutability/)
in mind on the way back. The thing that makes this safe isn't `Pin` waving a
wand; it's the *specific* set of methods we expose on the pinned reactor,
which we will deliberately keep tiny.

### 3.4 [`Pinned<'_, Ep>`](crate::Pinned): the deliberately narrow API

Rather than handing out `Pin<&mut Eventp>` directly (which would let users
call any inherent method we ever add to `Pin<&mut Eventp>` later), we wrap
it in a newtype that has *exactly* the three methods corresponding to
`epoll_ctl(2)`:

```rust,ignore
pub struct Pinned<'a, Ep>(pub Pin<&'a mut Ep>);

impl<'a, Ep: EventpOps> Pinned<'a, Ep> {
    pub fn add(&mut self, sub: ThinBoxSubscriber<Ep>) -> io::Result<()> { ... }
    pub fn modify(&mut self, fd: RawFd, interest: Interest) -> io::Result<()> { ... }
    pub fn delete(&mut self, fd: RawFd) -> io::Result<()> { ... }
}
```

These are exactly the three `EPOLL_CTL_*` operations, and *nothing else*.
`run_once`, `into_inner`, `Drop`, `Default`, you name it — all unreachable
from inside a handler. The reactor cannot be moved, cannot be replaced,
cannot even re-enter `epoll_wait`. The blast radius of "what a handler can
do to the reactor" is by construction the same as the blast radius of three
syscalls.

### 3.5 What `!Unpin` actually guarantees (a small precision note)

A subtle point that's easy to misread: `!Unpin` does **not** guarantee that
the `registered` map "doesn't move in memory" — `FxHashMap` will happily
rehash and shuffle its internal buckets when you `add` a new subscriber.
What `!Unpin` guarantees is that *the `Eventp` struct itself* cannot be
moved or replaced, and therefore its `registered` *field* is not swapped out
from under us.

The actual reason the in-flight `&mut Subscriber` stays valid across a
rehash is §2's indirection: the map only stores `ThinBoxSubscriber` (a
single word), and the *subscriber bytes live on a separate heap allocation*.
Rehashing moves the one-word handle, not the bytes it points at. The
handler's `&mut self` continues to point at the same heap address.

In other words: §2 and §3 work together. The thin pointer gives us pointer
stability across rehashes, and `Pin` gives us pointer stability against
`mem::replace`. Either alone would not be enough.

---

## 4. Handler internals: re-entrancy and the `Handling` state machine

§3 explained why `&mut Eventp` is safe to hand out (in narrowed form). It
left open the harder question: what may handlers actually *do* with it
without invalidating the in-flight subscriber reference?

### 4.1 Per-operation hazard analysis

`epoll_wait` returns up to N ready events; we dispatch them one by one.
While handler `i` runs, it may call back into the reactor. For each
operation we must ask: could this corrupt the loop?

| Operation in handler          | Risk                                                                              | Resolution                                |
| ----------------------------- | --------------------------------------------------------------------------------- | ----------------------------------------- |
| `add(new_sub)`                | `FxHashMap` rehash. But thin pointers are stable; new sub isn't in this batch.    | Allow.                                    |
| `modify(other, ..)`           | Updates kernel state + a `Cell<Interest>` inside the sub. Touches nothing else.   | Allow.                                    |
| `delete(other)`               | `other`'s event may also be in this batch — naive `dealloc` ⇒ dangling pointer.   | Drop in place now, defer free to batch end. |
| `delete(self)`                | `&mut self` is still live; can't drop now. But fd won't reappear in this batch.   | Mark `drop_current = true`; reap at tail. |
| `run_once_with_timeout(...)`  | Would clobber the dispatch state and re-enter `epoll_wait`.                       | **Panic.**                                |

The state for all of this is one tiny struct:

```rust,ignore
struct Handling {
    fd: RawFd,                                      // who's running right now
    drop_current: bool,                             // self-delete requested
    deferred_drop: Vec<ThinBoxSubscriber<Eventp>>,  // dropped-in-place, awaiting dealloc
}
```

`self.handling` is `Some` *iff* we're inside a dispatch batch. Entering
`run_once_with_timeout` while it's already `Some` panics — that's how we
forbid reentrant `run_once` ([src/lib.rs:285-322](../../src/eventp/lib.rs.html#285-322)).

### 4.2 The two flavours of `delete`

```rust,ignore
fn delete(&mut self, fd: RawFd) -> io::Result<()> {
    // epoll_ctl(EPOLL_CTL_DEL) — same for every path
    ...
    if let Some(h) = &mut self.handling {
        if h.fd == fd {
            // (A) self-delete: registry entry stays put until loop tail
            h.drop_current = true;
        } else {
            // (B) cross-delete: pop from registry, run user destructor now
            //     (so fd/socket handles release immediately), but keep the
            //     heap slot alive until end of batch.
            let mut sub = self.registered.remove(&fd).unwrap();
            sub.drop_in_place();
            h.deferred_drop.push(sub);
        }
    } else {
        // (C) not in dispatch: just remove
        self.registered.remove(&fd);
    }
    Ok(())
}
```

This produces one user-visible quirk worth pinning in a test:

- **Cross-delete then re-add the same fd in the same handler → works.**
  The registry entry was removed in (B), so the new `add` doesn't collide.
- **Self-delete then re-add the same fd in the same handler → `AlreadyExists`.**
  Self-delete only flips a flag; the registry entry is still there.

Both are pinned by tests ([handler_can_re_add_other_fd_after_delete](../../src/eventp/lib.rs.html#781),
[self_delete_then_re_add_same_fd_returns_already_exists](../../src/eventp/lib.rs.html#869)),
so any future change is visible and deliberate.

### 4.3 `ThinBoxSubscriber`, augmented with a sentinel

§2's `drop_in_place` story needs one more piece. When (B) runs the user
destructor early, the heap slot still exists — but it's logically
"already dropped". If `epoll_wait` reported both A and B in the same batch
and the dispatch loop later reconstructs B's thin pointer from
`ev.data()`, we must *not* re-run the user's `handle`.

So the layout grows one more field — the `raw fd` slot promised back in §2:

```text
+---------+---------+---------+---------+--------------------+
|  _pad_  |  raw fd |  _pad_  |  vptr   | dyn Subscriber<Ep> |
+---------+---------+---------+---------+--------------------+
          ptr-16             ptr-8      ↑
                              ThinBoxSubscriber { ptr }
```

It pulls double duty:

- **Fast-path fd read.** The dispatch loop wants to record "who's running"
  in `handling.fd` *before* calling `handle()`. With the cached fd, that's
  a single load — no vtable dance.
- **Dropped-in-place sentinel.** `drop_in_place` writes `raw_fd = -1`
  *before* calling the user destructor (so a re-entrant access during
  `T::drop` sees the "dead" state), and `try_deref_mut` returns `None`
  whenever it sees `-1` ([src/thin.rs:189-246](../../src/eventp/thin.rs.html#189-246)).

The dispatch loop wraps each reconstructed thin pointer in `ManuallyDrop`
([src/lib.rs:333-336](../../src/eventp/lib.rs.html#333-336)). The real owner is the
registry (or `deferred_drop`); even if the handler panics on the way out,
this local can't double-free.

### 4.4 The batch tail

After the loop, we `take()` `self.handling` to `None`. Dropping the
`Handling` drops the `deferred_drop` vector, which drops each
`ThinBoxSubscriber`, which finally calls `alloc::dealloc`. All the
in-place-dropped subscribers from (B) get their heap slots released
exactly here. Any `drop_current`-flagged subscribers were already
removed from the registry inline after each handler returned.

---

## 5. Builder & DI: throwing away the boilerplate

Tired of writing a `struct + AsFd + HasInterest + Handler` quartet *and* a
mock quartet for every fd you want to watch? Same. Let's see how far the
type system can carry us.

### 5.1 What the user writes

```rust,ignore
eventp::interest()                           // empty Interest
    .edge_triggered()                        // builder methods on Interest
    .read()
    .with_fd(listener)                       // (Interest, Fd)
    .with_handler(on_connection)             // → TriSubscriber
    .register_into(&mut reactor)?;           // calls Eventp::add

fn on_connection(
    listener:    &mut impl Accept,
    mut reactor: Pinned<impl EventpOps>,
) { ... }
```

No subscriber struct. No trait impls. The handler is a plain `fn` (or
closure), with whatever parameters it actually needs, in **whatever order**
it pleases.

### 5.2 Two halves of the builder, dual-trait style

There's no `Builder<T>` here. `with_fd` and `with_handler` are trait methods
that turn one tuple type into another, and they happen to commute:

```rust,ignore
impl<Args, F> WithFd      for (Interest, FnHandler<Args, F>) { type Out<Fd> = TriSubscriber<Fd, Args, F>; ... }
impl<Fd: AsFd> WithHandler for (Interest, Fd)                { type Out<Args, F> = TriSubscriber<Fd, Args, F>; ... }
```

Whichever you call first works; both paths converge on
`TriSubscriber<Fd, Args, F>`. The `Subscriber<Ep>` trait has a blanket
impl over `AsFd + HasInterest + Handler<Ep>`, so the resulting type plugs
straight into `register_into`.

### 5.3 Parameter injection: the macro factory

A handler can take any subset of `{ &mut Fd, Event, Interest, Pinned<'_, Ep> }`
in any order. To make this possible without proc-macros, the library writes
out **all 65 impls** by hand via a `macro_rules!` factory
(1 nullary + 4·P(4,1) + P(4,2) + P(4,3) + P(4,4) = 1 + 4 + 12 + 24 + 24 = 65;
see [src/tri_subscriber.rs:143-253](../../src/eventp/tri_subscriber.rs.html#143-253)).

Two small things make this work:

- **Signature lock-in via `PhantomData<fn(Args)>`.** Rust technically lets
  you `impl FnMut<A>` multiple times for the same type. `FnHandler<Args, F>`
  carries an `Args` type parameter, so `(fd, event)` and `(event, fd)`
  become *different* `Args`, and the corresponding `Handler` impls don't
  overlap.
- **TT-muncher accumulator** inside `impl_handler!` walks the parameter
  list left-to-right, building the call's argument list as it goes — the
  classic `macro_rules!` pattern for n-ary code generation.

### 5.4 Testing for almost free

Because handlers are plain functions and reactor methods go through the
`EventpOps` trait, your test is just:

```rust,ignore
fn on_connection<Ep: EventpOps>(listener: &mut impl Accept, mut reactor: Pinned<Ep>) { ... }

#[test]
fn accepts_then_registers_stream() {
    let mut mock_accept  = MockAccept::new();    // ← only mock what you used
    let mut mock_reactor = MockEventp::new();

    mock_accept.expect_accept().returning(...);
    mock_reactor.expect_add().times(1).returning(|_| Ok(()));

    on_connection(&mut mock_accept, pinned!(mock_reactor));
}
```

`MockEventp` is generated by [`mockall`](https://docs.rs/mockall) — see
[`src/mock.rs`](../../src/eventp/mock.rs.html) — and the `pinned!` macro pins it on the
stack without `Box::pin` ceremony ([src/pinned.rs:82-86](../../src/eventp/pinned.rs.html#82-86)).
Parameters you never inject in `fn handle` need no mocks at all.

For a complete end-to-end test suite written in this style, see
[`examples/echo-server.rs`](https://github.com/FuuuOverclocking/eventp/blob/main/examples/echo-server.rs).

---

## 6. The zero-cost dispatch path, verified

Let's see what `Eventp::run_once_with_timeout` actually compiles to. The
following is the inner dispatch loop from a `--release` build of the echo
server (lightly annotated):

```text
; for ev in buf:
   17b8c: mov  rdi, [r14 + r15 + 0x4]   ; rdi  = ev.data  (the subscriber addr)
   17b91: mov  eax, [rdi - 0x10]        ; eax  = *raw_fd_ref()        ← no vtable
   17b94: mov  [r12], eax               ; handling.fd = eax

;     if !is_subscriber_dropped:
   17b98: cmp  eax, -1                  ; raw_fd == -1 ?
   17b9b: je   .skip                    ; predicted not-taken via hand-rolled `unlikely`

;         s.handle(Event::from(ev), Pinned(...))
   17b9d: mov  rax, [rdi - 0x8]         ; rax = vptr
   17ba1: mov  esi, [r14 + r15]         ; esi = ev.events  (Event::from)
   17ba5: mov  rdx, rbx                 ; rdx = &mut self  (the Pinned)
   17ba8: call [rax + 0x30]             ; one indirect call — the handler

;     if handling.drop_current { ... }
   17bab: cmp  byte ptr [rbx + 0x34], 0
   17baf: je   .next_event              ; common case: nothing to do
```

That's it. Per event we have: one load of the user-data word, one load of
the cached fd, one branch (predicted away), one load of the vtable slot,
one indirect call. No hash, no allocation, no `Token → Handler` lookup, no
trampoline.

Compare this to the `event-manager` shape: SipHash 1-3 + three
`HashMap::get_mut` calls + a `Box<dyn>` deref, on every single event. The
difference isn't a constant factor; it's an axis.

### A few quieter optimisations supporting that

- **`FxHashMap`.** Keys are kernel-issued small integers; SipHash is pure
  overhead. ([src/lib.rs:134](../../src/eventp/lib.rs.html#134))
- **`MaybeUninit<EpollEvent>` event buffer.** Allocate `capacity` slots,
  `set_len` to `capacity` without initialising, then re-slice to the first
  `n` that `epoll_wait` wrote. `EpollEvent` is a POD wrapper around
  `libc::epoll_event`. ([src/lib.rs:201-219](../../src/eventp/lib.rs.html#201-219))
- **`hint::unreachable_unchecked()`** in the dispatch loop tells LLVM that
  `self.handling` is provably `None` at one specific point, saving a drop
  check. ([src/lib.rs:308-322](../../src/eventp/lib.rs.html#308-322))
- **Hand-rolled `unlikely`** using `checked_div(0)` — a known trick for
  giving the optimiser a branch hint without depending on unstable
  intrinsics. ([src/thin.rs:230-237](../../src/eventp/thin.rs.html#230-237))
- **`mem::transmute_copy`** instead of `transmute` when laundering a thin
  pointer into a `usize`, because we still need the original value to move
  it into the registry. ([src/lib.rs:383](../../src/eventp/lib.rs.html#383))
- **Direct `libc::epoll_ctl` for `EPOLL_CTL_DEL`**, because `nix`'s wrapper
  insists on an `AsFd` source — which we may not have, if the source was
  already dropped. The fd number is all the kernel needs.
  ([src/lib.rs:456-463](../../src/eventp/lib.rs.html#456-463))

### Runtime measurements

The disassembly above is the microscope. Here is the clock.

The harness lives in [`benches/dispatch.rs`](https://github.com/FuuuOverclocking/eventp/blob/main/benches/dispatch.rs).
Three reactors are driven through `eventfd` sources so that one round of
fire-and-drain involves the same three syscalls (`epoll_wait`,
`eventfd_write`, `eventfd_read`) regardless of dispatcher: **eventp**,
**mio** (plus a 30-line `FxHashMap<Token, Box<dyn FnMut()>>` user table —
the shape any mio user actually writes), and **event-manager**. Anything
else would be measuring kernel I/O, not dispatch.

**Host:** Intel Xeon Platinum 8163 @ 2.50 GHz (Skylake-SP, 33 MB L3 shared),
Linux 5.10.134, rustc 1.95.0; `cargo bench` with `lto=true` and
`codegen-units=1` (see `[profile.bench]` in `Cargo.toml`). Not a
CPU-pinned, isolated host — read the deltas, not the absolutes.

#### One ready event among N registered, single fd per subscriber

![dispatch one event with one fd per subscriber](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/bench-dispatch-one-single-fd.svg)

| N        | eventp     | event-manager | mio + FxHashMap | em − ep |
|----------|------------|---------------|-----------------|---------|
| 1        | 1.126 µs   | 1.165 µs      | 1.133 µs        | +39 ns  |
| 10       | 1.112      | 1.163         | 1.136           | +51 ns  |
| 100      | 1.114      | 1.165         | 1.138           | +51 ns  |
| 1 000    | 1.108      | 1.159         | 1.130           | +51 ns  |
| 10 000   | 1.103      | 1.157         | 1.127           | +54 ns  |
| 100 000  | 1.127      | 1.179         | 1.153           | +52 ns  |

Three things to read off:

1. **Dispatch is O(1) for all three.** Each row's median moves by less
   than 25 ns from N=1 to N=10,000. None of these designs have a "look
   up the handler" cost that grows with the registry.
2. **The bump at N=100,000 is shared.** Every backend slows down by
   ~25 ns together. If this were HashMap cache pressure, only
   event-manager would feel it; the fact that all three move in lockstep
   pins the cost on the kernel side — the epoll interest set's internal
   data structure feeling 100k entries, not anything in user space.
3. **The flat ~50 ns gap is two SipHash lookups.** event-manager's hot
   path does `fd_dispatch.get(fd)` followed by
   `subscribers.get_mut_unchecked(id)`; both are
   `std::collections::HashMap` (SipHash 1-3). mio sits ~25 ns above
   eventp — one FxHash lookup. FxHash is roughly 2× faster than SipHash,
   and the numbers line up.

#### Where the third HashMap actually fires

The `dispatch_one_multi_fd_M4` group registers four eventfds per logical
subscriber — the natural shape of a virtio device, a vsock backend, or
anything multiplexing several signal fds.

![dispatch one event with four fds per subscriber](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/bench-dispatch-one-multi-fd.svg)

| N (subs) | eventp     | event-manager | mio        | em − ep |
|----------|------------|---------------|------------|---------|
| 100      | 1.109 µs   | 1.212 µs      | 1.161 µs   | +103 ns |
| 1 000    | 1.125      | 1.207         | 1.147      | +82 ns  |
| 10 000   | 1.125      | 1.209         | 1.159      | +84 ns  |

eventp and mio are essentially unchanged from the single-fd case.
event-manager picks up another ~30 ns on top of its existing 50 ns —
**exactly the third lookup §1.1 promised**. With four fds per
subscriber, `process(events: Events, ...)` only sees the `RawFd`, so to
call `read` on the right owned `EventFd` the handler has to do
`self.fds.get_mut(&events.fd())` itself. There is no clean way out of
this in the event-manager API short of `unsafe` and a bare-`RawFd`
storage strategy. eventp doesn't pay it because the fd object lives on
the subscriber as a field, handed to the handler as `&mut Fd` through
the dependency injection of §5.

#### Per-event amortised throughput

`dispatch_all_ready`: N subscribers, all fired together, one `run_once`
to drain the batch.

![per-event amortised throughput](https://raw.githubusercontent.com/FuuuOverclocking/eventp/refs/heads/main/docs/images/bench-dispatch-all-ready.svg)

| N      | eventp ns/event | event-manager ns/event | mio ns/event |
|--------|-----------------|------------------------|--------------|
| 16     | 804             | 856                    | 828          |
| 64     | 809             | 862                    | 833          |
| 256    | 806             | 866                    | 837          |
| 1 024  | 817             | 896                    | 855          |

Per-core throughput: eventp ≈ **1.24 M events/s**, event-manager
≈ 1.16 M, mio + FxHashMap ≈ 1.20 M.

The em−ep delta widens from +52 ns at N=16 to +79 ns at N=1024 — a small
extra +27 ns. That is event-manager's HashMap entries spilling out of
the L1 data cache (1024 entries × ~24 bytes ≈ 24 KB, just past 32 KB
L1d on this host). eventp has no hashtable to miss.

#### A note on the absolute numbers

The kernel's three syscalls are roughly 1.05 µs of that 1.1 µs total —
~95% of one event today. So picking eventp over event-manager moves
4–7% of one event in this synthetic eventfd benchmark. That is a small
win on its own.

The interesting axis is forward, not present: when the syscall floor
goes down (io_uring with `IORING_SETUP_IOPOLL`, batched ring polling,
busy-poll on a NAPI device, kernel bypass) the dispatch overhead this
section measures is what's left. At that point the same 50 ns is the
lion's share, not a rounding error. eventp is shaped for that future,
not today's "syscall is everything" regime.

---

## 7. Known limitations

- **`Eventp` is `!Send`.** Cross-thread access goes through the
  [`remote_endpoint`](mod@crate::remote_endpoint) module, which sends
  closures into the reactor over an `eventfd` + MPSC channel. Making
  `Eventp` itself `Send` would require revisiting several of the unsafe
  invariants in §3-§4 and is not currently planned.
- **64-bit Linux only.** Both are checked at compile time
  ([src/lib.rs:1-11](../../src/eventp/lib.rs.html#1-11), [src/thin.rs:48-49](../../src/eventp/thin.rs.html#48-49));
  porting to 32-bit would mean giving up the "stash the address in `u64`"
  trick, which is the entire point of the library.
