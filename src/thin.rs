//! Thin pointer implementations.

use std::alloc::{self, Layout};
use std::marker::PhantomData;
use std::mem::{self, size_of};
use std::ops::Deref;
use std::os::fd::{AsRawFd, RawFd};
use std::ptr::{self, NonNull};

#[cfg(feature = "mock")]
use crate::mock::MockEventp;
use crate::utils::unlikely;
use crate::{Eventp, EventpOps, Subscriber};

/// Similar to `Box<dyn Subscriber<Ep>>`, but the size of this type is only one usize.
///
/// Since epoll allows registering only a `u64` alongside the file descriptor,
/// only a thin pointer can be stored.
///
/// # Memory layout
///
/// ```text
/// +---------+---------+---------+-----------------+--------------------+
/// |  _pad_  |  raw fd |  _pad_  |       vptr      | dyn Subscriber<Ep> |
/// +---------+---------+---------+-----------------+--------------------+
/// ??      ptr-16    ptr-12    ptr-8               ↑                    ??
///                                                 |
///                            ThinBoxSubscriber { ptr }
/// ```
///
/// See [technical](crate::_technical) for more information.
pub struct ThinBoxSubscriber<Ep: EventpOps> {
    ptr: NonNull<u8>,
    _marker: PhantomData<dyn Subscriber<Ep>>,
}

impl<Ep> ThinBoxSubscriber<Ep>
where
    Ep: EventpOps,
{
    /// Allocates memory on the heap and then places `value` into it.
    ///
    /// # Panics
    ///
    /// - if combining the `(usize, usize)` header layout with `T`'s layout
    ///   overflows (`Layout::extend` returns `Err`);
    /// - if the heap allocation fails (via [`alloc::handle_alloc_error`]).
    pub fn new<T: Subscriber<Ep>>(value: T) -> Self {
        #[cfg(not(target_pointer_width = "64"))]
        compile_error!("Platforms with pointer width other than 64 are not supported.");

        // Verify trait object layout: first 8 bytes for the data pointer,
        // next 8 bytes for the vtable pointer.
        const _: () = assert!(size_of::<&dyn Subscriber<Eventp>>() == 16);
        #[cfg(feature = "mock")]
        const _: () = assert!(size_of::<&dyn Subscriber<MockEventp>>() == 16);

        // Obtain the fat pointer and extract the vtable address.
        let fat_ptr = &value as &dyn Subscriber<Ep>;
        let (_data_ptr, vptr) =
            unsafe { mem::transmute::<&dyn Subscriber<Ep>, (*const (), *const ())>(fat_ptr) };

        // Read the raw fd before allocating, so any panic from a
        // user-provided `AsFd` impl cannot leave a partially-initialized heap.
        let raw_fd = value.as_fd().as_raw_fd();

        // Create a new layout for the raw fd, vptr and data T.
        let (layout, value_offset) = Layout::new::<(usize, usize)>()
            .extend(Layout::new::<T>())
            .expect("Failed to create combined layout");

        let ptr = {
            // SAFETY: Layout has a non-zero size, because it contains
            // at least two usizes.
            let ptr = unsafe { alloc::alloc(layout) };
            if ptr.is_null() {
                alloc::handle_alloc_error(layout);
            }

            // SAFETY: Points to a valid location because the previous allocation succeeded.
            let ptr = unsafe { ptr.add(value_offset) };
            unsafe { NonNull::new_unchecked(ptr) }
        };

        let mut ret = Self {
            ptr,
            _marker: PhantomData,
        };

        // Fill it with the data. No operation may unwind.

        *ret.raw_fd_mut() = raw_fd;
        *ret.vptr_mut() = vptr;

        // Move the value into the allocated location. No drop occurs.
        // SAFETY: data_ptr is valid and aligned for writes.
        unsafe { ret.ptr.as_ptr().cast::<T>().write(value) };

        ret
    }

    /// Allocates memory on the heap and then moves `value` into it.
    /// The original [Box] will be consumed.
    ///
    /// # Panics
    ///
    /// - if combining the `(usize, usize)` header layout with the value's layout
    ///   overflows (`Layout::extend` returns `Err`);
    /// - if the heap allocation fails (via [`alloc::handle_alloc_error`]).
    pub fn from_box_dyn(value: Box<dyn Subscriber<Ep>>) -> Self {
        // Obtain the fat pointer and extract the vtable address.
        let fat_ptr = value.deref();
        let (_data_ptr, vptr) =
            unsafe { mem::transmute::<&dyn Subscriber<Ep>, (*const (), *const ())>(fat_ptr) };

        // Get the layout of original data.
        let value_layout = Layout::for_value(value.deref());

        // Read the raw fd before allocating, so any panic from a
        // user-provided `AsFd` impl cannot leave a partially-initialized heap.
        let raw_fd = value.as_fd().as_raw_fd();

        // Create a new layout for the raw fd, vptr and data.
        let (layout, value_offset) = Layout::new::<(usize, usize)>()
            .extend(value_layout)
            .expect("Failed to create combined layout");

        let ptr = {
            // SAFETY: Layout has a non-zero size, because it contains
            // at least two usizes.
            let ptr = unsafe { alloc::alloc(layout) };
            if ptr.is_null() {
                alloc::handle_alloc_error(layout);
            }

            // SAFETY: Points to a valid location because the previous allocation succeeded.
            let ptr = unsafe { ptr.add(value_offset) };
            unsafe { NonNull::new_unchecked(ptr) }
        };

        let mut ret = Self {
            ptr,
            _marker: PhantomData,
        };

        // Fill it with the data. No operation may unwind.

        *ret.raw_fd_mut() = raw_fd;
        *ret.vptr_mut() = vptr;

        // Move the value into the allocated location. No drop occurs.
        let value = Box::into_raw(value) as *mut u8;
        // SAFETY: `src` and `dst` are valid and aligned. Because they are from
        // different allocation, so not overlapped.
        unsafe {
            ret.ptr
                .as_ptr()
                .cast::<u8>()
                .copy_from_nonoverlapping(value, value_layout.size())
        };
        // SAFETY: `GlobalAlloc` is the allocator of this value and `value_layout` is valid.
        unsafe { alloc::dealloc(value, value_layout) };

        ret
    }

    pub(crate) fn raw_fd_ref(&self) -> &RawFd {
        // SAFETY: See memory layout of docs of this type.
        unsafe { &*self.ptr.as_ptr().sub(2 * size_of::<usize>()).cast() }
    }

    fn raw_fd_mut(&mut self) -> &mut RawFd {
        // SAFETY: See memory layout of docs of this type.
        unsafe { &mut *self.ptr.as_ptr().sub(2 * size_of::<usize>()).cast() }
    }

    fn vptr_ref(&self) -> &*const () {
        // SAFETY: See memory layout of docs of this type.
        unsafe { &*self.ptr.as_ptr().sub(size_of::<usize>()).cast() }
    }

    fn vptr_mut(&mut self) -> &mut *const () {
        // SAFETY: See memory layout of docs of this type.
        unsafe { &mut *self.ptr.as_ptr().sub(size_of::<usize>()).cast() }
    }

    fn is_subscriber_dropped(&self) -> bool {
        *self.raw_fd_ref() == -1
    }

    fn mark_subscriber_dropped(&mut self) {
        *self.raw_fd_mut() = -1;
    }

    /// Dereferences to a trait object regardless of the raw-fd sentinel.
    ///
    /// The returned reference is always layout-valid (its address and vtable
    /// point at the heap slot of the original `T`), so the caller may always
    /// use it as the argument to `Layout::for_value`, `ptr::drop_in_place`, or
    /// other operations that only inspect the trait object's layout.
    ///
    /// # Safety
    ///
    /// - The caller must not invoke any method on the returned trait object
    ///   when the subscriber has already been dropped in place (i.e. when
    ///   `raw_fd == -1` after a prior `drop_in_place`/`mark_subscriber_dropped`).
    /// - The caller must respect Rust's aliasing rules: while this reference
    ///   exists, no other mutable reference to the same subscriber may exist.
    unsafe fn deref(&self) -> &dyn Subscriber<Ep> {
        let data_ptr = self.ptr.as_ptr().cast();
        let vptr = *self.vptr_ref();

        // SAFETY: `data_ptr` and `vptr` were written together in `new` /
        // `from_box_dyn`, so reassembling them yields a fat pointer whose layout
        // matches `*mut dyn Subscriber<Ep>`.
        let fat_ptr = unsafe {
            mem::transmute::<(*const (), *const ()), *mut dyn Subscriber<Ep>>((data_ptr, vptr))
        };
        // SAFETY: The heap slot is still allocated (it is only freed in `Drop`),
        // and the caller's `# Safety` contract above guarantees no aliasing
        // mutable reference exists.
        unsafe { &mut *fat_ptr }
    }

    /// Same as [`Self::deref()`], but obtains a mutable reference.
    unsafe fn deref_mut(&mut self) -> &mut dyn Subscriber<Ep> {
        let data_ptr = self.ptr.as_ptr().cast();
        let vptr = *self.vptr_ref();

        // SAFETY: `data_ptr` and `vptr` were written together in `new` /
        // `from_box_dyn`, so reassembling them yields a fat pointer whose layout
        // matches `*mut dyn Subscriber<Ep>`.
        let fat_ptr = unsafe {
            mem::transmute::<(*const (), *const ()), *mut dyn Subscriber<Ep>>((data_ptr, vptr))
        };
        // SAFETY: The heap slot is still allocated (it is only freed in `Drop`),
        // and the caller's `# Safety` contract above guarantees no aliasing
        // mutable reference exists.
        unsafe { &mut *fat_ptr }
    }

    pub(crate) fn try_deref(&self) -> Option<&dyn Subscriber<Ep>> {
        if unlikely(self.is_subscriber_dropped()) {
            return None;
        }
        // SAFETY: `raw_fd != -1`, so the subscriber has not been dropped in place;
        // the caller will receive a normal `&mut dyn Subscriber` and is free to
        // invoke methods on it. Aliasing is enforced by `&mut self`.
        unsafe { Some(self.deref()) }
    }

    pub(crate) fn try_deref_mut(&mut self) -> Option<&mut dyn Subscriber<Ep>> {
        if unlikely(self.is_subscriber_dropped()) {
            return None;
        }
        // SAFETY: `raw_fd != -1`, so the subscriber has not been dropped in place;
        // the caller will receive a normal `&mut dyn Subscriber` and is free to
        // invoke methods on it. Aliasing is enforced by `&mut self`.
        unsafe { Some(self.deref_mut()) }
    }

    /// Moves the subscriber out of this thin allocation into a `Box<dyn Subscriber<Ep>>`.
    ///
    /// Returns `None` if the subscriber has already been dropped in place
    /// (via the internal deferred-drop machinery), in which case the value is
    /// no longer recoverable.
    ///
    /// The thin allocation is released as part of this call; the returned
    /// `Box` owns a freshly-allocated heap slot whose layout matches the
    /// underlying concrete type, so it is fully compatible with the global
    /// allocator's `Box<dyn _>` deallocation path.
    ///
    /// # Panics
    ///
    /// - if allocating the destination `Box` fails (via
    ///   [`alloc::handle_alloc_error`]).
    pub fn into_box_dyn(mut self) -> Option<Box<dyn Subscriber<Ep>>> {
        if self.is_subscriber_dropped() {
            return None;
        }

        // SAFETY: The subscriber has not been dropped in place, so its value
        // is still live. We only inspect the trait object's layout / vtable
        // here -- we do not invoke any user method on it -- so the
        // `deref_mut` contract is upheld.
        let value_ref = unsafe { self.deref_mut() };
        let value_layout = Layout::for_value(value_ref);
        let value_ptr = value_ref as *mut dyn Subscriber<Ep> as *mut u8;
        let vptr = *self.vptr_ref();

        // Allocate a fresh slot sized exactly for the concrete value, so the
        // resulting `Box` can be deallocated by the global allocator using
        // the same layout it would have used for `Box::new(value)`.
        //
        // For a ZST we must NOT call `alloc::alloc` (it requires a non-zero
        // layout). Instead, mirror what `Box::<T>::new` does for ZSTs and use
        // a properly-aligned dangling pointer. `copy_nonoverlapping` of zero
        // bytes is a no-op and accepts any aligned pointer, and `Box::<T>`
        // for a ZST never calls the deallocator.
        //
        // The `align as *mut u8` cast produces a pointer with no provenance;
        // under strict provenance this is only sound for zero-sized accesses,
        // which is exactly the regime we use it in. Once MSRV reaches 1.84
        // this can be replaced with `ptr::without_provenance_mut(align)`.
        let dst: *mut u8 = if value_layout.size() == 0 {
            value_layout.align() as *mut u8
        } else {
            // SAFETY: `value_layout` is non-zero here.
            let p = unsafe { alloc::alloc(value_layout) };
            if p.is_null() {
                alloc::handle_alloc_error(value_layout);
            }
            p
        };

        // SAFETY: When `value_layout.size() > 0`, `dst` was just allocated
        // for `value_layout` and `value_ptr` points at a live value of that
        // layout in a different allocation, so the regions cannot overlap.
        // When `value_layout.size() == 0`, this is a no-op and the aligned
        // dangling pointers are valid for zero-sized reads/writes.
        unsafe {
            ptr::copy_nonoverlapping(value_ptr, dst, value_layout.size());
        }

        // Mark the slot as already-dropped BEFORE handing ownership over, so
        // that `Drop for ThinBoxSubscriber` only runs the deallocation path
        // and does not double-drop the value we just moved out.
        self.mark_subscriber_dropped();

        // SAFETY: `dst` and `vptr` were paired together from the same trait
        // object, so reassembling them yields a valid `*mut dyn Subscriber<Ep>`
        // pointing at storage compatible with what `Box<dyn Subscriber<Ep>>`
        // expects: either a global-allocator slot of `value_layout` (non-ZST)
        // or an aligned dangling pointer (ZST), matching `Box::<T>::new`'s
        // own conventions in both cases.
        let fat_ptr = unsafe {
            mem::transmute::<(*const (), *const ()), *mut dyn Subscriber<Ep>>((
                dst.cast::<()>(),
                vptr,
            ))
        };
        Some(unsafe { Box::from_raw(fat_ptr) })
    }

    /// Drops the subscriber in place and marks the slot so subsequent
    /// `try_deref_mut` calls return `None`.
    pub(crate) fn drop_in_place(&mut self) {
        if self.is_subscriber_dropped() {
            return;
        }

        // The order matters: we mark the slot BEFORE running the destructor so
        // that even if `T::drop` triggers re-entrancy (e.g. it ends up calling
        // back through this `ThinBoxSubscriber`), `try_deref_mut` will refuse
        // to hand out a reference to a half-destructed value.
        self.mark_subscriber_dropped();

        // SAFETY: We just verified `raw_fd != -1` above, so the subscriber has
        // not already been dropped. We use the returned reference only to obtain
        // a `*mut` for `ptr::drop_in_place` -- no method is invoked on it after
        // the destructor runs, so the `# Safety` contract of `deref_mut` is
        // upheld even though `raw_fd` has just been set to `-1`.
        let value = unsafe { self.deref_mut() };
        let value_ptr = value as *mut _;
        unsafe { ptr::drop_in_place(value_ptr) };
    }
}

impl<Ep> From<Box<dyn Subscriber<Ep>>> for ThinBoxSubscriber<Ep>
where
    Ep: EventpOps,
{
    fn from(value: Box<dyn Subscriber<Ep>>) -> Self {
        Self::from_box_dyn(value)
    }
}

impl<Ep> TryFrom<ThinBoxSubscriber<Ep>> for Box<dyn Subscriber<Ep>>
where
    Ep: EventpOps,
{
    type Error = ThinBoxSubscriber<Ep>;

    /// Converts a [`ThinBoxSubscriber`] back into a `Box<dyn Subscriber<Ep>>`.
    ///
    /// Returns the original `ThinBoxSubscriber` as the error if the
    /// subscriber has already been dropped in place and is no longer
    /// recoverable.
    fn try_from(value: ThinBoxSubscriber<Ep>) -> Result<Self, Self::Error> {
        if value.is_subscriber_dropped() {
            return Err(value);
        }
        // SAFETY: We just verified the subscriber is still alive, so
        // `into_box_dyn` is guaranteed to return `Some`.
        Ok(unsafe { value.into_box_dyn().unwrap_unchecked() })
    }
}

impl<Ep: EventpOps> Drop for ThinBoxSubscriber<Ep> {
    fn drop(&mut self) {
        struct DropGuard<Ep: EventpOps> {
            ptr: NonNull<u8>,
            value_layout: Layout,
            _marker: PhantomData<dyn Subscriber<Ep>>,
        }

        impl<Ep: EventpOps> Drop for DropGuard<Ep> {
            fn drop(&mut self) {
                unsafe {
                    // SAFETY: Layout must have been computable if we're in drop.
                    let (layout, value_offset) = Layout::new::<(usize, usize)>()
                        .extend(self.value_layout)
                        .unwrap_unchecked();

                    // SAFETY: `GlobalAlloc` is the allocator of this space and layout is valid.
                    alloc::dealloc(self.ptr.as_ptr().sub(value_offset), layout);
                }
            }
        }

        let ptr = self.ptr;
        let is_subscriber_dropped = self.is_subscriber_dropped();

        // SAFETY: We only need a fat pointer here to recover `Layout::for_value`
        // (which inspects the vtable for size/align, not the data) and to feed
        // `ptr::drop_in_place` below. We do not invoke any method on the trait
        // object, so the `deref_mut` contract is upheld in both states.
        let value = unsafe { self.deref_mut() };
        let value_ptr = value as *mut _;
        let value_layout = Layout::for_value(value);

        // `_guard` will deallocate the memory when dropped, even if `drop_in_place` unwinds.
        let _guard = DropGuard::<Ep> {
            ptr,
            value_layout,
            _marker: PhantomData,
        };
        if !is_subscriber_dropped {
            // SAFETY: The subscriber has not been dropped in place yet, so its
            // value is still live and must be destructed exactly once here.
            unsafe { ptr::drop_in_place(value_ptr) };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::os::fd::{AsFd, BorrowedFd};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nix::sys::eventfd::{EfdFlags, EventFd};

    use super::*;
    use crate::epoll::EpollFlags;
    use crate::subscriber::{Handler, HasInterest};
    use crate::{Event, Eventp, Interest, Pinned};

    /// A subscriber backed by a real `eventfd`, parameterized by alignment via
    /// a zero-sized `repr(align(N))` tag and an arbitrary payload `P`.
    ///
    /// `Align` controls the resulting `align_of::<TestSub<_, _>>()` so we can
    /// exercise the offset-padding logic between the (raw_fd, vptr) header and
    /// the value slot.
    #[repr(C)]
    struct TestSub<Align: Copy, P> {
        _align: Align,
        eventfd: EventFd,
        interest: Cell<Interest>,
        on_handle: P,
        on_drop: DropTracker,
    }

    /// Tracks whether the subscriber's destructor has run, and optionally
    /// panics from inside `Drop` to exercise the `DropGuard` path.
    struct DropTracker {
        counter: &'static AtomicUsize,
        panic_in_drop: bool,
    }

    impl Drop for DropTracker {
        fn drop(&mut self) {
            self.counter.fetch_add(1, Ordering::SeqCst);
            if self.panic_in_drop {
                panic!("intentional panic from DropTracker");
            }
        }
    }

    impl<Align: Copy, P> AsFd for TestSub<Align, P> {
        fn as_fd(&self) -> BorrowedFd<'_> {
            self.eventfd.as_fd()
        }
    }

    impl<Align: Copy, P> HasInterest for TestSub<Align, P> {
        fn interest(&self) -> &Cell<Interest> {
            &self.interest
        }
    }

    impl<Align, P> Handler<Eventp> for TestSub<Align, P>
    where
        Align: Copy,
        P: FnMut(),
    {
        fn handle(&mut self, _event: Event, _eventp: Pinned<'_, Eventp>) {
            (self.on_handle)();
        }
    }

    fn new_eventfd() -> EventFd {
        EventFd::from_flags(EfdFlags::EFD_CLOEXEC | EfdFlags::EFD_NONBLOCK).unwrap()
    }

    fn make_sub<Align: Copy + Default, P: FnMut()>(
        on_handle: P,
        counter: &'static AtomicUsize,
    ) -> TestSub<Align, P> {
        TestSub {
            _align: Align::default(),
            eventfd: new_eventfd(),
            interest: Cell::new(Interest::new(EpollFlags::empty())),
            on_handle,
            on_drop: DropTracker {
                counter,
                panic_in_drop: false,
            },
        }
    }

    /// Counter helper: each test uses its own static so tests can run in
    /// parallel without stepping on each other.
    macro_rules! drop_counter {
        () => {{
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            COUNTER.store(0, Ordering::SeqCst);
            &COUNTER
        }};
    }

    #[test]
    fn new_then_drop_runs_destructor_exactly_once() {
        let counter = drop_counter!();
        let sub = make_sub::<(), _>(|| {}, counter);

        let thin = ThinBoxSubscriber::<Eventp>::new(sub);
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        drop(thin);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn raw_fd_ref_returns_subscribers_fd() {
        let counter = drop_counter!();
        let sub = make_sub::<(), _>(|| {}, counter);
        let expected_fd = sub.eventfd.as_fd().as_raw_fd();

        let thin = ThinBoxSubscriber::<Eventp>::new(sub);
        assert_eq!(*thin.raw_fd_ref(), expected_fd);
    }

    #[test]
    fn try_deref_mut_dispatches_to_handler() {
        let counter = drop_counter!();
        let call_count = std::rc::Rc::new(Cell::new(0u32));
        let cc = call_count.clone();
        let sub = make_sub::<(), _>(move || cc.set(cc.get() + 1), counter);

        let mut thin = ThinBoxSubscriber::<Eventp>::new(sub);

        // Build a fresh `Eventp` purely to satisfy the `Pinned` parameter.
        // `handle()` here only calls `on_handle`, never touching the reactor.
        let mut ep = Eventp::default();
        // SAFETY: `ep` lives until end of scope and is never moved afterward.
        let pinned = Pinned(unsafe { std::pin::Pin::new_unchecked(&mut ep) });

        let s = thin.try_deref_mut().expect("must deref while alive");
        s.handle(Event::new(EpollFlags::empty()), pinned);
        assert_eq!(call_count.get(), 1);
    }

    #[test]
    fn drop_in_place_marks_slot_and_runs_destructor() {
        let counter = drop_counter!();
        let sub = make_sub::<(), _>(|| {}, counter);

        let mut thin = ThinBoxSubscriber::<Eventp>::new(sub);
        assert!(!thin.is_subscriber_dropped());

        thin.drop_in_place();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(thin.is_subscriber_dropped());
        assert!(thin.try_deref_mut().is_none());

        // Dropping the thin pointer afterwards must NOT run the destructor again.
        drop(thin);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn drop_in_place_is_idempotent() {
        let counter = drop_counter!();
        let sub = make_sub::<(), _>(|| {}, counter);

        let mut thin = ThinBoxSubscriber::<Eventp>::new(sub);
        thin.drop_in_place();
        thin.drop_in_place();
        thin.drop_in_place();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn drop_guard_releases_heap_when_destructor_panics() {
        let counter = drop_counter!();

        let sub = TestSub::<(), _> {
            _align: (),
            eventfd: new_eventfd(),
            interest: Cell::new(Interest::new(EpollFlags::empty())),
            on_handle: || {},
            on_drop: DropTracker {
                counter,
                panic_in_drop: true,
            },
        };

        let thin = ThinBoxSubscriber::<Eventp>::new(sub);

        // Drop must unwind, but the `DropGuard` inside `Drop for
        // ThinBoxSubscriber` should still release the heap allocation. If it
        // didn't, Miri / a leak sanitizer would catch it; we at least verify
        // the destructor counter ticks exactly once.
        let result = catch_unwind(AssertUnwindSafe(move || drop(thin)));
        assert!(result.is_err(), "drop must propagate the panic");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    /// Builds and round-trips a subscriber whose alignment is `$align`.
    /// Exercises the padding the layout code inserts between the (raw_fd,
    /// vptr) header and the value when `align_of::<T>() > align_of::<usize>()`.
    macro_rules! align_roundtrip_test {
        ($name:ident, $align:literal) => {
            #[test]
            fn $name() {
                #[derive(Copy, Clone, Default)]
                #[repr(align($align))]
                struct A;

                let counter = drop_counter!();
                let sub = make_sub::<A, _>(|| {}, counter);
                let expected_fd = sub.eventfd.as_fd().as_raw_fd();

                assert!(std::mem::align_of::<TestSub<A, fn()>>() >= $align);

                let mut thin = ThinBoxSubscriber::<Eventp>::new(sub);
                assert_eq!(*thin.raw_fd_ref(), expected_fd);

                // Dispatching through the vtable must reach the right value
                // even when there is padding between the header and the data.
                let mut ep = Eventp::default();
                // SAFETY: `ep` lives until end of scope and is never moved.
                let pinned = Pinned(unsafe { std::pin::Pin::new_unchecked(&mut ep) });
                let s = thin.try_deref_mut().unwrap();
                s.handle(Event::new(EpollFlags::empty()), pinned);

                drop(thin);
                assert_eq!(counter.load(Ordering::SeqCst), 1);
            }
        };
    }

    align_roundtrip_test!(roundtrip_align_8, 8);
    align_roundtrip_test!(roundtrip_align_16, 16);
    align_roundtrip_test!(roundtrip_align_32, 32);
    align_roundtrip_test!(roundtrip_align_64, 64);

    #[test]
    fn from_box_dyn_matches_new() {
        let counter = drop_counter!();
        let sub = make_sub::<(), _>(|| {}, counter);
        let expected_fd = sub.eventfd.as_fd().as_raw_fd();

        let boxed: Box<dyn Subscriber<Eventp>> = Box::new(sub);
        let thin = ThinBoxSubscriber::<Eventp>::from_box_dyn(boxed);
        assert_eq!(*thin.raw_fd_ref(), expected_fd);

        drop(thin);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn from_impl_delegates_to_from_box_dyn() {
        let counter = drop_counter!();
        let sub = make_sub::<(), _>(|| {}, counter);

        let boxed: Box<dyn Subscriber<Eventp>> = Box::new(sub);
        let thin: ThinBoxSubscriber<Eventp> = boxed.into();
        drop(thin);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    /// A subscriber whose `AsFd::as_fd` always panics. Used to verify that
    /// `ThinBoxSubscriber::{new, from_box_dyn}` read the fd *before*
    /// allocating, so that an unwind cannot leave a partially-initialized
    /// heap slot for `Drop` to crash on.
    ///
    /// `Drop` is instrumented so we can assert that the value itself was
    /// dropped exactly once on unwind (not zero times -> leak, and not twice
    /// -> double-drop / UB).
    struct PanickingFdSub {
        interest: Cell<Interest>,
        drops: &'static AtomicUsize,
    }

    impl PanickingFdSub {
        fn new(drops: &'static AtomicUsize) -> Self {
            Self {
                interest: Cell::new(Interest::new(EpollFlags::empty())),
                drops,
            }
        }
    }

    impl Drop for PanickingFdSub {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl AsFd for PanickingFdSub {
        fn as_fd(&self) -> BorrowedFd<'_> {
            panic!("intentional panic from PanickingFdSub::as_fd");
        }
    }

    impl HasInterest for PanickingFdSub {
        fn interest(&self) -> &Cell<Interest> {
            &self.interest
        }
    }

    impl Handler<Eventp> for PanickingFdSub {
        fn handle(&mut self, _event: Event, _eventp: Pinned<'_, Eventp>) {}
    }

    /// Regression test for the partial-construction unsoundness: if
    /// `value.as_fd()` panics inside `ThinBoxSubscriber::new`, the panic must
    /// propagate cleanly -- without segfaulting from a `Drop` that runs over
    /// uninitialized memory, and without leaking `value`.
    #[test]
    fn new_does_not_segfault_when_as_fd_panics() {
        let drops = drop_counter!();
        let sub = PanickingFdSub::new(drops);

        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = ThinBoxSubscriber::<Eventp>::new(sub);
        }));
        assert!(result.is_err(), "as_fd panic must propagate");
        // `sub` was moved into `new`, so its Drop must run exactly once on
        // unwind. If the fix regresses (heap allocated before reading the fd),
        // either the process segfaults or this counter ends up at 0.
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn into_box_dyn_round_trips_value_and_runs_destructor_once() {
        let counter = drop_counter!();
        let sub = make_sub::<(), _>(|| {}, counter);
        let expected_fd = sub.eventfd.as_fd().as_raw_fd();

        let thin = ThinBoxSubscriber::<Eventp>::new(sub);
        let boxed = thin.into_box_dyn().expect("alive subscriber must convert");
        // Conversion itself must not run the destructor.
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert_eq!(boxed.as_fd().as_raw_fd(), expected_fd);

        drop(boxed);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    /// A zero-sized subscriber. Exercises the ZST branch of `into_box_dyn`,
    /// which must NOT call `alloc::alloc` with a zero layout (UB) and must
    /// produce a `Box` whose deallocation path matches `Box::<Self>::new`.
    ///
    /// The subscriber holds no state, so it cannot own an `EventFd`; we use
    /// stdin as a borrowed fd just to satisfy `AsFd`, and a leaked
    /// `Cell<Interest>` to satisfy `HasInterest`. Neither is exercised here.
    struct ZstSub;

    impl AsFd for ZstSub {
        fn as_fd(&self) -> BorrowedFd<'_> {
            // SAFETY: stdin is open for the lifetime of the process; we only
            // need a borrowed fd token and never perform I/O on it.
            unsafe { BorrowedFd::borrow_raw(0) }
        }
    }

    impl HasInterest for ZstSub {
        fn interest(&self) -> &Cell<Interest> {
            // `Cell` is not `Sync`, so it cannot live in a `static`. Leak a
            // `Box` once instead; the resulting `&'static Cell<Interest>` is
            // good for the rest of the process. Per-call leaks would be
            // wasteful, so cache the pointer in a `OnceLock<usize>`.
            use std::sync::OnceLock;
            static SLOT: OnceLock<usize> = OnceLock::new();
            let addr = *SLOT.get_or_init(|| {
                let leaked: &'static Cell<Interest> =
                    Box::leak(Box::new(Cell::new(Interest::new(EpollFlags::empty()))));
                leaked as *const Cell<Interest> as usize
            });
            // SAFETY: `addr` was produced from `Box::leak`, which yields a
            // valid pointer with `'static` lifetime. `Cell` is `!Sync` but
            // we only ever read `addr` here, not the cell's contents, and
            // the test never accesses it concurrently.
            unsafe { &*(addr as *const Cell<Interest>) }
        }
    }

    impl Handler<Eventp> for ZstSub {
        fn handle(&mut self, _event: Event, _eventp: Pinned<'_, Eventp>) {}
    }

    #[test]
    fn into_box_dyn_handles_zst_subscriber() {
        assert_eq!(std::mem::size_of::<ZstSub>(), 0);

        let thin = ThinBoxSubscriber::<Eventp>::new(ZstSub);
        let boxed = thin.into_box_dyn().expect("alive ZST must convert");
        // Dropping the `Box` must not call the global allocator with a
        // zero-sized layout. If it does, Miri / a sanitizer will flag it.
        drop(boxed);
    }

    #[test]
    fn into_box_dyn_round_trips_through_thin_again() {
        let counter = drop_counter!();
        let sub = make_sub::<(), _>(|| {}, counter);
        let expected_fd = sub.eventfd.as_fd().as_raw_fd();

        let thin1 = ThinBoxSubscriber::<Eventp>::new(sub);
        let boxed = thin1.into_box_dyn().unwrap();
        let thin2 = ThinBoxSubscriber::<Eventp>::from_box_dyn(boxed);
        assert_eq!(*thin2.raw_fd_ref(), expected_fd);

        drop(thin2);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn into_box_dyn_returns_none_after_drop_in_place() {
        let counter = drop_counter!();
        let sub = make_sub::<(), _>(|| {}, counter);

        let mut thin = ThinBoxSubscriber::<Eventp>::new(sub);
        thin.drop_in_place();
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        assert!(thin.into_box_dyn().is_none());
        // Returning `None` must not run the destructor a second time.
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn try_from_thin_returns_err_after_drop_in_place() {
        let counter = drop_counter!();
        let sub = make_sub::<(), _>(|| {}, counter);

        let mut thin = ThinBoxSubscriber::<Eventp>::new(sub);
        thin.drop_in_place();

        let recovered: Result<Box<dyn Subscriber<Eventp>>, _> = thin.try_into();
        assert!(recovered.is_err());
        // The destructor must still have run exactly once (from drop_in_place).
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    /// Same regression check, but for the `from_box_dyn` path.
    #[test]
    fn from_box_dyn_does_not_segfault_when_as_fd_panics() {
        let drops = drop_counter!();
        let boxed: Box<dyn Subscriber<Eventp>> = Box::new(PanickingFdSub::new(drops));

        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = ThinBoxSubscriber::<Eventp>::from_box_dyn(boxed);
        }));
        assert!(result.is_err(), "as_fd panic must propagate");
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
