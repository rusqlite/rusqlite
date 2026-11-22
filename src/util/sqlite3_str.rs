//! Wrapper for `sqlite3_str`
use std::{
    ffi::{c_char, c_int},
    io::Write as _,
};

use crate::{Result, error::check, ffi};

pub(crate) struct Sqlite3Str {
    ptr: *mut ffi::sqlite3_str,
}

// Space to hold this string must be obtained
// from an SQLite memory allocation function
pub(crate) fn alloc<D: std::fmt::Display + ?Sized>(
    db: *mut ffi::sqlite3,
    d: &D,
) -> Result<*mut c_char> {
    let mut str = Sqlite3Str::new(db);
    write!(&mut str, "{}", d).map_err(|e| {
        e.downcast()
            .unwrap_or_else(|e| err!(ffi::SQLITE_IOERR, "{}", e))
    })?;
    str.into_raw().map(|raw| raw.0)
}

impl Sqlite3Str {
    pub fn new(db: *mut ffi::sqlite3) -> Self {
        Sqlite3Str {
            ptr: unsafe { ffi::sqlite3_str_new(db) },
        }
    }
    pub fn into_raw(self) -> Result<(*mut c_char, ffi::sqlite3_uint64)> {
        check(unsafe { ffi::sqlite3_str_errcode(self.ptr) })?;
        let len = self.len();
        let ptr = unsafe { ffi::sqlite3_str_finish(self.ptr) };
        std::mem::forget(self);
        Ok((ptr, len as ffi::sqlite3_uint64))
    }
    pub fn len(&self) -> c_int {
        unsafe { ffi::sqlite3_str_length(self.ptr) }
    }
}

impl std::io::Write for Sqlite3Str {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(pos) = buf.iter().position(|&b| b == b'\0') {
            let err = err!(
                ffi::SQLITE_MISUSE,
                "data provided contains an interior nul byte at pos {}",
                pos
            );
            return Err(std::io::Error::other(err));
        }
        let len = usize::try_from(self.len()).unwrap();
        unsafe {
            ffi::sqlite3_str_append(
                self.ptr,
                buf.as_ptr().cast(),
                c_int::try_from(buf.len()).map_err(|e| {
                    let err = err!(ffi::SQLITE_TOOBIG, "{}", e);
                    std::io::Error::other(err)
                })?,
            );
        }
        Ok(usize::try_from(self.len())
            .unwrap()
            .checked_sub(len)
            .unwrap())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Sqlite3Str {
    fn drop(&mut self) {
        unsafe { ffi::sqlite3_free(ffi::sqlite3_str_finish(self.ptr).cast()) }
    }
}

#[cfg(all(test, not(miri)))]
mod test {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use std::ffi::CStr;

    use crate::{Error, Result, ffi};

    #[test]
    fn null_error() -> Result<()> {
        let err = super::alloc(std::ptr::null_mut(), "el\0barto").unwrap_err();
        std::assert_matches!(
            err,
            Error::SqliteFailure(
                ffi::Error {
                    code: ffi::ErrorCode::ApiMisuse,
                    ..
                },
                Some(msg)
            ) if msg == "data provided contains an interior nul byte at pos 2"
        );
        Ok(())
    }

    #[test]
    fn alloc() -> Result<()> {
        let raw = super::alloc(std::ptr::null_mut(), "error")?;
        assert_eq!(unsafe { CStr::from_ptr(raw) }, c"error");
        unsafe { ffi::sqlite3_free(raw.cast()) };
        Ok(())
    }
}
