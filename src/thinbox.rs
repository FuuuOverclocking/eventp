use std::alloc::{self, Layout};
use std::error::Error;
use std::fmt::{self, Debug, Display, Formatter};
use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::ptr::{self, NonNull};

use crate::subscriber::Subscriber;

#[cfg(not(target_pointer_width = "64"))]
compile_error!("Platforms with pointer width other than 64 are not supported.");

pub struct ThinBox<T: ?Sized> {
    ptr: NonNull<u8>,
    _marker: PhantomData<T>,
}

unsafe impl<T: ?Sized + Send> Send for ThinBox<T> {}

/// `ThinBox<T>` is `Sync` if `T` is `Sync` because the data is owned.
unsafe impl<T: ?Sized + Sync> Sync for ThinBox<T> {}

impl ThinBox<dyn Subscriber> {
    pub fn new_unsize<S: Subscriber>(value: S) -> Self {
        if size_of::<S>() == 0 {
            panic!("ZST not supported");
        } else {
            const DYN_SUBSCRIBER_SIZE: usize = size_of::<&dyn Subscriber>();
            debug_assert_eq!(DYN_SUBSCRIBER_SIZE, 16);

            let fat_ptr = &value as &dyn Subscriber;
            let vtable_ptr =
                unsafe { mem::transmute::<&dyn Subscriber, (usize, usize)>(fat_ptr).1 };

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

                let this = Self {
                    ptr,
                    _marker: PhantomData,
                };

                this.meta().cast::<usize>().write(vtable_ptr);
                this.value().cast::<S>().write(value);

                this
            }
        }
    }

    fn meta(&self) -> *mut u8 {
        //  Safety:
        //  - At least 8 bytes are allocated ahead of the pointer.
        //  - We know that Meta will be aligned because the middle pointer is aligned to the greater
        //    of the alignment of the header and the data and the header size includes the padding
        //    needed to align the header. Subtracting the header size from the aligned data pointer
        //    will always result in an aligned header pointer, it just may not point to the
        //    beginning of the allocation.
        unsafe { self.ptr.as_ptr().sub(8) }
    }

    fn value(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }
}

impl Deref for ThinBox<dyn Subscriber> {
    type Target = dyn Subscriber;

    fn deref(&self) -> &Self::Target {
        let value = self.value();
        let metadata = self.meta();
        unsafe {
            let fat_ptr = mem::transmute::<_, *const dyn Subscriber>((value, metadata));
            &*fat_ptr
        }
    }
}

impl DerefMut for ThinBox<dyn Subscriber> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let value = self.value();
        let metadata = self.meta();
        unsafe {
            let fat_ptr = mem::transmute::<_, *mut dyn Subscriber>((value, metadata));
            &mut *fat_ptr
        }
    }
}

impl<T: ?Sized> Drop for ThinBox<T> {
    fn drop(&mut self) {
        unsafe {
            let value = self.deref_mut();
            let value = value as *mut T;
            self.with_header().drop::<T>(value);
        }
    }
}
