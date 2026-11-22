// This is used when either vtab or modern-sqlite is on. Different methods are
// used in each feature. Avoid having to track this for each function. We will
// still warn for anything that's not used by either, though.
#![cfg_attr(not(feature = "vtab"), allow(dead_code))]
use crate::ffi;
use std::ffi::{CStr, c_char};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// A string we own that's allocated on the SQLite heap. Automatically calls
/// `sqlite3_free` when dropped.
///
/// It has identical representation to a nonnull `*mut c_char`, so you can use
/// it transparently as one. It's nonnull, so Option<SqliteMallocString> can be
/// used for nullable ones (it's still just one pointer).
///
/// Most strings shouldn't use this! Only places where the string needs to be
/// freed with `sqlite3_free`. This includes `sqlite3_extended_sql` results,
/// some error message pointers... Note that misuse is extremely dangerous!
///
/// Note that this is *not* a lossless interface. Incoming strings with internal
/// NULs are modified, and outgoing strings which are non-UTF8 are modified.
/// This seems unavoidable -- it tries very hard to not panic.
#[repr(transparent)]
pub(crate) struct SqliteMallocString {
    ptr: NonNull<c_char>,
    _boo: PhantomData<Box<[c_char]>>,
}
// This is owned data for a primitive type, and thus it's safe to implement
// these. That said, nothing needs them, and they make things easier to misuse.

// unsafe impl Send for SqliteMallocString {}
// unsafe impl Sync for SqliteMallocString {}

impl SqliteMallocString {
    /// SAFETY: Caller must be certain that `m` a nul-terminated c string
    /// allocated by `sqlite3_malloc64`, and that SQLite expects us to free it!
    #[inline]
    pub(crate) unsafe fn from_raw_nonnull(ptr: NonNull<c_char>) -> Self {
        Self {
            ptr,
            _boo: PhantomData,
        }
    }

    /// SAFETY: Caller must be certain that `m` a nul-terminated c string
    /// allocated by `sqlite3_malloc64`, and that SQLite expects us to free it!
    #[inline]
    pub(crate) unsafe fn from_raw(ptr: *mut c_char) -> Option<Self> {
        unsafe { NonNull::new(ptr).map(|p| Self::from_raw_nonnull(p)) }
    }

    /// Borrow the pointer behind `self`. We still manage it when this function
    /// returns. If you want to relinquish ownership, use `into_raw`.
    #[inline]
    pub(crate) fn as_ptr(&self) -> *const c_char {
        self.ptr.as_ptr()
    }

    #[inline]
    pub(crate) fn as_cstr(&self) -> &CStr {
        unsafe { CStr::from_ptr(self.as_ptr()) }
    }

    #[inline]
    pub(crate) fn to_string_lossy(&self) -> std::borrow::Cow<'_, str> {
        self.as_cstr().to_string_lossy()
    }
}

impl Drop for SqliteMallocString {
    #[inline]
    fn drop(&mut self) {
        unsafe { ffi::sqlite3_free(self.ptr.as_ptr().cast()) };
    }
}
