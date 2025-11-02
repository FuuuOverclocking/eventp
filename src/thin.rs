//! Thin pointer implementations.

use std::alloc::{self, Layout};
use std::marker::PhantomData;
use std::mem::{self, size_of};
use std::ops::{Deref, DerefMut};
use std::os::fd::AsRawFd;
use std::ptr::{self, NonNull};

#[cfg(feature = "mock")]
use crate::mock::MockEventp;
use crate::{Eventp, EventpOps, Subscriber};

#[cfg(not(target_pointer_width = "64"))]
compile_error!("Platforms with pointer width other than 64 are not supported.");

/// Similar to `Box<dyn Subscriber<Ep>>`, but the size is only one pointer width.
///
/// Since epoll allows registering only a `u64` alongside the file descriptor, fat pointers
/// cannot be stored, which is why this type was created. See [technical](crate::_technical)
/// for more information.
pub struct ThinBoxSubscriber<Ep: EventpOps> {
    ptr: NonNull<u8>,
    _marker: PhantomData<dyn Subscriber<Ep>>,
}

impl<Ep> ThinBoxSubscriber<Ep>
where
    Ep: EventpOps,
{
    pub fn new<S: Subscriber<Ep>>(value: S) -> Self {
        if size_of::<S>() == 0 {
            panic!("ZST not supported");
        }

        const DYN_SUBSCRIBER_SIZE: usize = size_of::<&dyn Subscriber<Eventp>>();
        const _: () = assert!(DYN_SUBSCRIBER_SIZE == 16);

        #[cfg(feature = "mock")]
        const DYN_SUBSCRIBER_SIZE_MOCK: usize = size_of::<&dyn Subscriber<MockEventp>>();
        #[cfg(feature = "mock")]
        const _: () = assert!(DYN_SUBSCRIBER_SIZE_MOCK == 16);

        let fat_ptr = &value as &dyn Subscriber<Ep>;
        let vtable_ptr =
            unsafe { mem::transmute::<&dyn Subscriber<Ep>, (usize, usize)>(fat_ptr).1 };

        let (layout, value_offset) = Layout::new::<(usize, usize)>()
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

            ptr.as_ptr()
                .sub(2 * size_of::<usize>())
                .cast::<i32>()
                .write(value.as_fd().as_raw_fd());
            ptr.as_ptr()
                .sub(size_of::<usize>())
                .cast::<usize>()
                .write(vtable_ptr);
            ptr.as_ptr().cast::<S>().write(value);

            Self {
                ptr,
                _marker: PhantomData,
            }
        }
    }

    pub fn from_box<S: Subscriber<Ep>>(value: Box<S>) -> Self {
        Self::from(value)
    }

    pub fn from_box_dyn(value: Box<dyn Subscriber<Ep>>) -> Self {
        Self::from(value)
    }

    pub(crate) const fn raw_fd(&self) -> i32 {
        unsafe {
            self.ptr
                .as_ptr()
                .sub(2 * size_of::<usize>())
                .cast::<i32>()
                .read()
        }
    }

    const fn meta(&self) -> usize {
        //  Safety:
        //  - At least 8 bytes are allocated ahead of the pointer.
        //  - We know that Meta will be aligned because the middle pointer is aligned to the greater
        //    of the alignment of the header and the data and the header size includes the padding
        //    needed to align the header. Subtracting the header size from the aligned data pointer
        //    will always result in an aligned header pointer, it just may not point to the
        //    beginning of the allocation.
        unsafe {
            self.ptr
                .as_ptr()
                .sub(size_of::<usize>())
                .cast::<usize>()
                .read()
        }
    }

    const fn value(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }
}

impl<Ep: EventpOps> Deref for ThinBoxSubscriber<Ep> {
    type Target = dyn Subscriber<Ep>;

    fn deref(&self) -> &Self::Target {
        let value = self.value();
        let metadata = self.meta();
        unsafe {
            let fat_ptr =
                mem::transmute::<(*mut u8, usize), *const dyn Subscriber<Ep>>((value, metadata));
            &*fat_ptr
        }
    }
}

impl<Ep: EventpOps> DerefMut for ThinBoxSubscriber<Ep> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let value = self.value();
        let metadata = self.meta();
        unsafe {
            let fat_ptr =
                mem::transmute::<(*mut u8, usize), *mut dyn Subscriber<Ep>>((value, metadata));
            &mut *fat_ptr
        }
    }
}

impl<Ep: EventpOps> AsRef<dyn Subscriber<Ep>> for ThinBoxSubscriber<Ep> {
    fn as_ref(&self) -> &(dyn Subscriber<Ep> + 'static) {
        self.deref()
    }
}

impl<Ep: EventpOps> AsMut<dyn Subscriber<Ep>> for ThinBoxSubscriber<Ep> {
    fn as_mut(&mut self) -> &mut (dyn Subscriber<Ep> + 'static) {
        self.deref_mut()
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
                // All ZST are allocated statically.
                if self.value_layout.size() == 0 {
                    return;
                }

                unsafe {
                    // SAFETY: Layout must have been computable if we're in drop
                    let (layout, value_offset) = Layout::new::<(usize, usize)>()
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
            let _guard = DropGuard::<Ep> {
                ptr: self.ptr,
                value_layout,
                _marker: PhantomData,
            };
            ptr::drop_in_place(value_ptr);
        }
    }
}

impl<S, Ep> From<Box<S>> for ThinBoxSubscriber<Ep>
where
    S: Subscriber<Ep>,
    Ep: EventpOps,
{
    fn from(value: Box<S>) -> Self {
        Self::new(*value)
    }
}

impl<Ep> From<Box<dyn Subscriber<Ep>>> for ThinBoxSubscriber<Ep>
where
    Ep: EventpOps,
{
    fn from(old_value: Box<dyn Subscriber<Ep>>) -> Self {
        let raw_fd = old_value.as_fd().as_raw_fd();

        let fat_ptr = old_value.deref();
        let vtable_ptr =
            unsafe { mem::transmute::<&dyn Subscriber<Ep>, (usize, usize)>(fat_ptr).1 };

        let value_layout = Layout::for_value(old_value.deref());
        if value_layout.size() == 0 {
            panic!("ZST not supported");
        }

        let (layout, value_offset) = Layout::new::<(usize, usize)>()
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

            ptr.as_ptr()
                .sub(2 * size_of::<usize>())
                .cast::<i32>()
                .write(raw_fd);
            ptr.as_ptr()
                .sub(size_of::<usize>())
                .cast::<usize>()
                .write(vtable_ptr);

            Self {
                ptr,
                _marker: PhantomData,
            }
        }
    }
}
