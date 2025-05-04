use std::mem;
use std::mem::needs_drop;
use std::ptr;
use std::ptr::NonNull;

/// Like `Option<Box<dyn Any>>`, but a single pointer in size.
#[derive(Default)]
pub(crate) struct ThinBoxAny(
    /// A pointer to a function that will be called on drop. The function argument receives a
    /// pointer to itself.
    Option<NonNull<unsafe fn(*mut ())>>,
);

impl ThinBoxAny {
    /// Heap-allocate some data, returning a pointer and the `ThinBoxAny` controlling its lifetime.
    pub fn new<T: 'static>(data: T) -> (ThinBoxAny, *mut T) {
        if size_of::<T>() == 0 {
            let ptr = ptr::dangling_mut::<T>();

            if needs_drop::<T>() {
                // Make sure we don't double-drop the type.
                mem::forget(data);

                // The function that will be called on drop. We just materialize a value of the
                // type (which is okay since it's zero-sized) and drop it.
                let drop_function: &'static unsafe fn(*mut ()) =
                    &const { |_| drop(unsafe { ptr::dangling_mut::<T>().read() }) };

                (Self(Some(NonNull::from(drop_function))), ptr)
            } else {
                (Self(None), ptr)
            }
        } else {
            // `repr(C)` ensures that a `*mut Heap<T>` can be converted to a
            // `*mut unsafe fn(*mut ())`.
            #[repr(C)]
            struct Heap<T> {
                drop: unsafe fn(*mut ()),
                data: T,
            }
            let ptr = NonNull::from(Box::leak(Box::new(Heap {
                drop: |ptr| drop(unsafe { Box::from_raw(ptr.cast::<Heap<T>>()) }),
                data,
            })));
            let this = Self(Some(ptr.cast::<unsafe fn(*mut ())>()));
            (this, unsafe { &mut (*ptr.as_ptr()).data })
        }
    }

    /// Heap-allocate some data if it is `Some`, and return both a pointer to the data and the
    /// `ThinBoxAny` that controls its lifetime.
    pub fn new_option<T: 'static>(data: Option<T>) -> (ThinBoxAny, Option<*mut T>) {
        match data {
            Some(data) => {
                let (boxed, ptr) = Self::new(data);
                (boxed, Some(ptr))
            }
            None => (ThinBoxAny::default(), None),
        }
    }
}

impl Drop for ThinBoxAny {
    fn drop(&mut self) {
        if let Some(ptr) = self.0 {
            unsafe { ptr.read()(ptr.as_ptr().cast()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThinBoxAny;
    use std::cell::Cell;

    thread_local!(static COUNTER: Cell<u32> = const { Cell::new(0) });

    struct Helper<T>(T);
    impl<T> Drop for Helper<T> {
        fn drop(&mut self) {
            COUNTER.set(COUNTER.get() + 1);
        }
    }

    #[test]
    fn zst_with_drop() {
        COUNTER.set(0);
        let (boxed, _) = ThinBoxAny::new(Helper(()));
        assert_eq!(COUNTER.get(), 0);
        drop(boxed);
        assert_eq!(COUNTER.get(), 1);
    }

    #[test]
    fn non_zst_with_drop() {
        COUNTER.set(0);
        let (boxed, data) = ThinBoxAny::new(Helper(5_u32));
        assert_eq!(unsafe { (*data).0 }, 5);
        assert_eq!(COUNTER.get(), 0);
        drop(boxed);
        assert_eq!(COUNTER.get(), 1);
    }
}
