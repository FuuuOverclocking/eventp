use std::alloc::{self, Layout};
use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::ptr::{self, NonNull};

#[cfg(feature = "mock")]
use crate::MockEventp;
use crate::{Eventp, EventpLike, Subscriber};

#[cfg(not(target_pointer_width = "64"))]
compile_error!("Platforms with pointer width other than 64 are not supported.");

pub struct ThinBoxSubscriber<E: EventpLike> {
    ptr: NonNull<u8>,
    _marker: PhantomData<dyn Subscriber<E>>,
}

impl<E> ThinBoxSubscriber<E>
where
    E: EventpLike,
{
    pub fn new<S: Subscriber<E>>(value: S) -> Self {
        if size_of::<S>() == 0 {
            panic!("ZST not supported");
        } else {
            const DYN_SUBSCRIBER_SIZE: usize = size_of::<&dyn Subscriber<Eventp>>();
            const _: () = assert!(DYN_SUBSCRIBER_SIZE == 16);

            #[cfg(feature = "mock")]
            const DYN_SUBSCRIBER_SIZE_MOCK: usize = size_of::<&dyn Subscriber<MockEventp>>();
            #[cfg(feature = "mock")]
            const _: () = assert!(DYN_SUBSCRIBER_SIZE_MOCK == 16);

            let fat_ptr = &value as &dyn Subscriber<E>;
            let vtable_ptr =
                unsafe { mem::transmute::<&dyn Subscriber<E>, (usize, usize)>(fat_ptr).1 };

            let (layout, value_offset) = Layout::new::<usize>()
                .extend(Layout::new::<S>())
                .expect("Failed to create combined layout");

            unsafe {
                let ptr = {
                    // SAFETY: layout has a non-zero size, because S is not ZST.
                    let ptr = alloc::alloc(layout);
                    if ptr.is_null() {
                        alloc::handle_alloc_error(layout);
                    }

                    let ptr = ptr.add(value_offset);
                    NonNull::new_unchecked(ptr)
                };

                ptr.as_ptr().sub(8).cast::<usize>().write(vtable_ptr);
                ptr.as_ptr().cast::<S>().write(value);

                Self {
                    ptr,
                    _marker: PhantomData,
                }
            }
        }
    }

    #[allow(clippy::boxed_local)]
    pub fn from_box<S: Subscriber<E>>(value: Box<S>) -> Self {
        // Take down from heap firstly.
        Self::new(*value)
    }

    pub fn from_box_dyn(value: Box<dyn Subscriber<E>>) -> Self {
        Self::from(value)
    }

    const fn meta(&self) -> *mut u8 {
        //  Safety:
        //  - At least 8 bytes are allocated ahead of the pointer.
        //  - We know that Meta will be aligned because the middle pointer is aligned to the greater
        //    of the alignment of the header and the data and the header size includes the padding
        //    needed to align the header. Subtracting the header size from the aligned data pointer
        //    will always result in an aligned header pointer, it just may not point to the
        //    beginning of the allocation.
        unsafe { self.ptr.as_ptr().sub(8) }
    }

    const fn value(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }
}

impl<E: EventpLike> Deref for ThinBoxSubscriber<E> {
    type Target = dyn Subscriber<E>;

    fn deref(&self) -> &Self::Target {
        let value = self.value();
        let metadata = unsafe { self.meta().cast::<usize>().read() };
        unsafe {
            let fat_ptr =
                mem::transmute::<(*mut u8, usize), *const dyn Subscriber<E>>((value, metadata));
            &*fat_ptr
        }
    }
}

impl<E: EventpLike> DerefMut for ThinBoxSubscriber<E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let value = self.value();
        let metadata = unsafe { self.meta().cast::<usize>().read() };
        unsafe {
            let fat_ptr =
                mem::transmute::<(*mut u8, usize), *mut dyn Subscriber<E>>((value, metadata));
            &mut *fat_ptr
        }
    }
}

impl<E: EventpLike> Drop for ThinBoxSubscriber<E> {
    fn drop(&mut self) {
        struct DropGuard<E: EventpLike> {
            ptr: NonNull<u8>,
            value_layout: Layout,
            _marker: PhantomData<dyn Subscriber<E>>,
        }

        impl<E: EventpLike> Drop for DropGuard<E> {
            fn drop(&mut self) {
                // All ZST are allocated statically.
                if self.value_layout.size() == 0 {
                    return;
                }

                unsafe {
                    // SAFETY: Layout must have been computable if we're in drop
                    let (layout, value_offset) = Layout::new::<usize>()
                        .extend(self.value_layout)
                        .unwrap_unchecked();

                    // Since we only allocate for non-ZSTs, the layout size cannot be zero.
                    debug_assert!(layout.size() != 0);
                    alloc::dealloc(self.ptr.as_ptr().sub(value_offset), layout);
                }
            }
        }

        unsafe {
            let value = self.deref_mut();
            let value_ptr = value as *mut _;

            let value_layout = Layout::for_value(value);

            // `_guard` will deallocate the memory when dropped, even if `drop_in_place` unwinds.
            let _guard = DropGuard::<E> {
                ptr: self.ptr,
                value_layout,
                _marker: PhantomData,
            };
            ptr::drop_in_place(value_ptr);
        }
    }
}

impl<S, E> From<S> for ThinBoxSubscriber<E>
where
    S: Subscriber<E>,
    E: EventpLike,
{
    fn from(value: S) -> Self {
        Self::new(value)
    }
}

impl<E> From<Box<dyn Subscriber<E>>> for ThinBoxSubscriber<E>
where
    E: EventpLike,
{
    fn from(old_value: Box<dyn Subscriber<E>>) -> Self {
        let fat_ptr = old_value.deref();
        let vtable_ptr = unsafe { mem::transmute::<&dyn Subscriber<E>, (usize, usize)>(fat_ptr).1 };

        let value_layout = Layout::for_value(old_value.deref());
        if value_layout.size() == 0 {
            panic!("ZST not supported");
        }

        let (layout, value_offset) = Layout::new::<usize>()
            .extend(value_layout)
            .expect("Failed to create combined layout");

        unsafe {
            let ptr = {
                // SAFETY: layout has a non-zero size, because S is not ZST.
                let ptr = alloc::alloc(layout);
                if ptr.is_null() {
                    alloc::handle_alloc_error(layout);
                }

                let ptr = ptr.add(value_offset);
                NonNull::new_unchecked(ptr)
            };

            let old_value = Box::into_raw(old_value) as *mut u8;
            ptr::copy_nonoverlapping(old_value, ptr.as_ptr(), value_layout.size());
            alloc::dealloc(old_value, value_layout);

            ptr.as_ptr().sub(8).cast::<usize>().write(vtable_ptr);

            Self {
                ptr,
                _marker: PhantomData,
            }
        }
    }
}
