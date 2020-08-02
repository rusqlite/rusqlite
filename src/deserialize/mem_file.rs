use std::ptr::NonNull;
use std::{alloc, fmt, iter, ops, ptr, slice};

use crate::ffi;

/// Memory file with serialized database content owned by SQLite.
/// Used for [`crate::Connection::serialize`] and [`crate::Connection::deserialize`].
/// This looks like `Vec<u8>` - a growable vector of bytes - but
/// memory allocation is handled by `sqlite3_malloc64`, `sqlite3_realloc64`,
/// `sqlite3_msize` and `sqlite3_free`.
///
/// It is named after the private struct `MemFile` in
/// [`sqlite/src/memdb.c`](https://www.sqlite.org/src/doc/trunk/src/memdb.c).
///
/// ```
/// # use rusqlite::deserialize::MemFile;
/// let mut serialized = MemFile::new();
/// serialized.extend(vec![1, 2, 3]);
/// assert_eq!(serialized[1], 2);
/// ```
pub struct MemFile {
    data: NonNull<u8>,
    len: usize,
    cap: usize,
}

impl Drop for MemFile {
    fn drop(&mut self) {
        if self.cap > 0 {
            unsafe { ffi::sqlite3_free(self.data.as_ptr() as *mut _) };
        }
    }
}

impl MemFile {
    /// Constructs a new, empty `MemFile`.
    ///
    /// The vector will not allocate until elements are pushed onto it.
    pub fn new() -> Self {
        unsafe { Self::from_non_null(NonNull::dangling(), 0, 0) }
    }

    /// Constructs a new, empty `MemFile` with the specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut file = Self::new();
        file.reserve(capacity);
        file
    }

    /// Creates a `MemFile` directly from the raw components of another vector.
    ///
    /// # Safety
    ///
    /// This is highly unsafe, due to the number of invariants that aren't
    /// checked:
    ///
    /// * `ptr` needs to have been previously allocated via `sqlite3_malloc64`
    /// * `length` needs to be less than or equal to `capacity`.
    /// * `capacity` needs to be the capacity that the pointer was allocated with.
    ///
    /// The ownership of `ptr` is effectively transferred to the
    /// `MemFile` which may then deallocate, reallocate or change the
    /// contents of memory pointed to by the pointer at will. Ensure
    /// that nothing else uses the pointer after calling this
    /// function.
    pub unsafe fn from_raw_parts(ptr: *mut u8, length: usize, capacity: usize) -> Self {
        Self::from_non_null(NonNull::new_unchecked(ptr), length, capacity)
    }

    pub(crate) unsafe fn from_non_null(data: NonNull<u8>, len: usize, cap: usize) -> Self {
        debug_assert!(len <= cap);
        MemFile { data, len, cap }
    }

    /// Copies and appends all bytes in a slice to the `MemFile`.
    pub fn extend_from_slice(&mut self, other: &[u8]) {
        let len = other.len();
        self.reserve(len);
        unsafe { ptr::copy_nonoverlapping(other.as_ptr(), self.data.as_ptr().add(self.len), len) };
        self.len += len;
    }

    /// Reserves capacity for at least `additional` more bytes to be inserted
    /// in the given `MemFile`. After calling `reserve`, capacity will be
    /// greater than or equal to `self.len() + additional`. Does nothing if
    /// capacity is already sufficient.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity overflows `usize`.
    pub fn reserve(&mut self, additional: usize) {
        let new_len = self.len + additional;
        if new_len > self.cap {
            self.set_capacity(new_len);
        }
    }

    /// Shrinks the capacity of the `MemFile` as much as possible.
    pub fn shrink_to_fit(&mut self) {
        self.set_capacity(self.len)
    }

    /// Resizes the allocation.
    fn set_capacity(&mut self, cap: usize) {
        if self.cap == cap {
            return;
        }
        if cap == 0 {
            *self = Self::new();
            return;
        }
        unsafe {
            let p = if self.cap == 0 {
                ffi::sqlite3_malloc64(cap as _)
            } else {
                ffi::sqlite3_realloc64(self.data.as_ptr() as _, cap as _)
            };
            self.data = NonNull::new(p)
                .unwrap_or_else(|| {
                    alloc::handle_alloc_error(alloc::Layout::from_size_align_unchecked(cap, 1))
                })
                .cast();
            self.cap = ffi::sqlite3_msize(self.data.as_ptr() as _) as _;
            debug_assert!(self.cap >= cap);
        };
    }

    /// Set `len`, the size of the file.
    /// # Safety
    /// This can expose uninitialized memory when increasing the length.
    /// `len` must not overflows the capacity.
    pub unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= self.cap, "len overflows capacity");
        self.len = len;
    }

    /// The number of allocated bytes.
    pub fn capacity(&self) -> usize {
        self.cap
    }
}

impl iter::Extend<u8> for MemFile {
    fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
        let mut iter = iter.into_iter();
        self.reserve(iter.size_hint().0);
        while let Some(byte) = iter.next() {
            let index = self.len;
            self.len += 1;
            self.reserve(iter.size_hint().0);
            self[index] = byte;
        }
    }
}

impl Clone for MemFile {
    fn clone(&self) -> Self {
        let mut c = MemFile::new();
        c.extend_from_slice(&self);
        c
    }
}

impl ops::Deref for MemFile {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.data.as_ptr(), self.len) }
    }
}
impl ops::DerefMut for MemFile {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.data.as_ptr(), self.len) }
    }
}

impl Default for MemFile {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MemFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemFile")
            .field("len", &self.len)
            .field("cap", &self.cap)
            .finish()
    }
}
