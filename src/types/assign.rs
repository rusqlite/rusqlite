use crate::Result;
use crate::error::{Error, error_from_handle};
use crate::ffi::{
    self, SQLITE_OK, SQLITE_STATIC, SQLITE_TRANSIENT, SQLITE_UTF8, sqlite3_destructor_type,
    sqlite3_stmt, sqlite3_uint64,
};
#[cfg(any(feature = "functions", feature = "vtab"))]
use crate::ffi::{sqlite3_context, sqlite3_value};

use std::ffi::{c_char, c_int, c_uchar, c_void};

mod sealed {
    use crate::ffi;
    use std::ffi::c_int;
    pub trait Sealed {}
    impl Sealed for (*mut ffi::sqlite3_stmt, c_int) {}
    #[cfg(any(feature = "functions", feature = "vtab"))]
    impl Sealed for (*mut ffi::sqlite3_context, &[*mut ffi::sqlite3_value]) {}
    #[cfg(test)]
    impl Sealed for () {}
}

/// `sqlite3_stmt` or `sqlite3_context`
pub trait Assign: sealed::Sealed + Sized {
    /// error handling
    fn decode_result(self, code: c_int) -> Result<()> {
        if code == SQLITE_OK {
            return Ok(());
        }
        Err(self.error(code))
    }
    /// error handling
    fn error(self, code: c_int) -> Error;
    /// `sqlite3_bind_null` or `sqlite3_result_null`
    fn assign_null(self) -> Result<()>;
    /// `sqlite3_bind_int64` or `sqlite3_result_int64`
    fn assign_int(self, i: i64) -> Result<()>;
    /// `sqlite3_bind_double` or `sqlite3_result_double`
    fn assign_real(self, r: f64) -> Result<()>;
    /// like `sqlite3_bind_zeroblob64` or `sqlite3_result_zeroblob64` for text
    fn assign_empty_text(self) -> Result<()>;
    /// `sqlite3_bind_text64` or `sqlite3_result_text64`
    fn assign_text(self, s: &[u8], destructor: sqlite3_destructor_type) -> Result<()> {
        self.assign_raw_text(
            s.as_ptr().cast::<c_char>(),
            s.len() as _,
            destructor,
            SQLITE_UTF8 as _,
        )
    }
    /// `sqlite3_bind_text64` or `sqlite3_result_text64`
    fn assign_raw_text(
        self,
        s: *const c_char,
        len: sqlite3_uint64,
        destructor: sqlite3_destructor_type,
        encoding: c_uchar,
    ) -> Result<()>;
    /// Like `assign_text` with `SQLITE_TRANSIENT`
    #[inline]
    fn assign_transient_text(self, s: &[u8]) -> Result<()> {
        self.assign_text(s, SQLITE_TRANSIENT())
    }
    /// `sqlite3_bind_blob64` or `sqlite3_result_blob64`
    #[inline]
    fn assign_blob(self, b: &[u8], destructor: sqlite3_destructor_type) -> Result<()> {
        self.assign_raw_blob(b.as_ptr().cast::<c_void>(), b.len() as _, destructor)
    }
    /// `sqlite3_bind_text64` or `sqlite3_result_text64`
    fn assign_raw_blob(
        self,
        b: *const c_void,
        len: sqlite3_uint64,
        destructor: sqlite3_destructor_type,
    ) -> Result<()>;
    /// Like `assign_blob` with `SQLITE_TRANSIENT`
    #[inline]
    fn assign_transient_blob(self, b: &[u8]) -> Result<()> {
        self.assign_blob(b, SQLITE_TRANSIENT())
    }
    /// `sqlite3_bind_zeroblob64` or `sqlite3_result_zeroblob64`
    fn assign_zeroblob(self, len: u64) -> Result<()>;
    /// `sqlite3_result_value`
    #[cfg(feature = "functions")]
    fn assign_arg(self, idx: usize) -> Result<()>;
    /// `sqlite3_bind_pointer` or `sqlite3_result_pointer`
    #[cfg(feature = "pointer")]
    fn assign_ptr(
        self,
        ptr: *mut c_void,
        ptr_type: &'static std::ffi::CStr,
        destructor: sqlite3_destructor_type,
    ) -> Result<()>;
}

impl Assign for (*mut sqlite3_stmt, c_int) {
    #[cold]
    fn error(self, code: c_int) -> Error {
        unsafe { error_from_handle(ffi::sqlite3_db_handle(self.0), code) }
    }

    fn assign_null(self) -> Result<()> {
        self.decode_result(unsafe { ffi::sqlite3_bind_null(self.0, self.1) })
    }

    fn assign_int(self, i: i64) -> Result<()> {
        self.decode_result(unsafe { ffi::sqlite3_bind_int64(self.0, self.1, i) })
    }

    fn assign_real(self, r: f64) -> Result<()> {
        self.decode_result(unsafe { ffi::sqlite3_bind_double(self.0, self.1, r) })
    }

    fn assign_empty_text(self) -> Result<()> {
        self.decode_result(unsafe {
            ffi::sqlite3_bind_text64(
                self.0,
                self.1,
                "".as_ptr().cast::<c_char>(),
                0,
                SQLITE_STATIC(),
                SQLITE_UTF8 as _,
            )
        })
    }

    fn assign_raw_text(
        self,
        s: *const c_char,
        len: sqlite3_uint64,
        destructor: sqlite3_destructor_type,
        encoding: c_uchar,
    ) -> Result<()> {
        if len == 0 {
            destroy(s as _, destructor);
            self.assign_empty_text()
        } else {
            self.decode_result(unsafe {
                ffi::sqlite3_bind_text64(self.0, self.1, s, len, destructor, encoding)
            })
        }
    }
    fn assign_raw_blob(
        self,
        b: *const c_void,
        len: sqlite3_uint64,
        destructor: sqlite3_destructor_type,
    ) -> Result<()> {
        self.decode_result(if len == 0 {
            destroy(b as _, destructor);
            unsafe { ffi::sqlite3_bind_zeroblob64(self.0, self.1, 0) }
        } else {
            unsafe { ffi::sqlite3_bind_blob64(self.0, self.1, b, len, destructor) }
        })
    }
    fn assign_zeroblob(self, len: u64) -> Result<()> {
        self.decode_result(unsafe { ffi::sqlite3_bind_zeroblob64(self.0, self.1, len) })
    }

    #[cfg(feature = "functions")]
    fn assign_arg(self, _: usize) -> Result<()> {
        Err(err!(ffi::SQLITE_MISUSE, "Unsupported value"))
    }

    #[cfg(feature = "pointer")]
    fn assign_ptr(
        self,
        ptr: *mut c_void,
        ptr_type: &'static std::ffi::CStr,
        destructor: sqlite3_destructor_type,
    ) -> Result<()> {
        self.decode_result(unsafe {
            ffi::sqlite3_bind_pointer(self.0, self.1, ptr, ptr_type.as_ptr(), destructor)
        })
    }
}

#[cfg(any(feature = "functions", feature = "vtab"))]
impl Assign for (*mut sqlite3_context, &[*mut sqlite3_value]) {
    #[cold]
    fn error(self, code: c_int) -> Error {
        unsafe { error_from_handle(ffi::sqlite3_context_db_handle(self.0), code) }
    }

    #[inline]
    fn assign_null(self) -> Result<()> {
        unsafe { ffi::sqlite3_result_null(self.0) };
        Ok(())
    }

    #[inline]
    fn assign_int(self, i: i64) -> Result<()> {
        unsafe { ffi::sqlite3_result_int64(self.0, i) };
        Ok(())
    }

    #[inline]
    fn assign_real(self, r: f64) -> Result<()> {
        unsafe { ffi::sqlite3_result_double(self.0, r) };
        Ok(())
    }

    fn assign_empty_text(self) -> Result<()> {
        unsafe {
            ffi::sqlite3_result_text64(
                self.0,
                "".as_ptr().cast::<c_char>(),
                0,
                SQLITE_STATIC(),
                SQLITE_UTF8 as _,
            );
        }
        Ok(())
    }

    fn assign_raw_text(
        self,
        s: *const c_char,
        len: sqlite3_uint64,
        destructor: sqlite3_destructor_type,
        encoding: c_uchar,
    ) -> Result<()> {
        if len == 0 {
            destroy(s as _, destructor);
            self.assign_empty_text()
        } else {
            unsafe {
                ffi::sqlite3_result_text64(self.0, s, len, destructor, encoding);
            };
            Ok(())
        }
    }
    fn assign_raw_blob(
        self,
        b: *const c_void,
        len: sqlite3_uint64,
        destructor: sqlite3_destructor_type,
    ) -> Result<()> {
        if len == 0 {
            destroy(b as _, destructor);
            let code = unsafe { ffi::sqlite3_result_zeroblob64(self.0, 0) };
            self.decode_result(code)
        } else {
            unsafe {
                ffi::sqlite3_result_blob64(self.0, b, len, destructor);
            };
            Ok(())
        }
    }
    #[inline]
    fn assign_zeroblob(self, len: u64) -> Result<()> {
        let code = unsafe { ffi::sqlite3_result_zeroblob64(self.0, len) };
        self.decode_result(code)
    }

    #[cfg(feature = "functions")]
    fn assign_arg(self, idx: usize) -> Result<()> {
        unsafe { ffi::sqlite3_result_value(self.0, self.1[idx]) };
        Ok(())
    }

    #[cfg(feature = "pointer")]
    #[inline]
    fn assign_ptr(
        self,
        ptr: *mut c_void,
        ptr_type: &'static std::ffi::CStr,
        destructor: sqlite3_destructor_type,
    ) -> Result<()> {
        unsafe { ffi::sqlite3_result_pointer(self.0, ptr, ptr_type.as_ptr(), destructor) };
        Ok(())
    }
}

fn destroy(ptr: *mut c_void, destructor: sqlite3_destructor_type) {
    #[expect(unpredictable_function_pointer_comparisons)]
    if destructor == SQLITE_TRANSIENT() {
        return;
    }
    if let Some(d) = destructor {
        unsafe { d(ptr) }
    }
}

#[cfg(test)]
impl Assign for () {
    fn error(self, _: c_int) -> Error {
        unreachable!()
    }

    fn assign_null(self) -> Result<()> {
        Ok(())
    }

    fn assign_int(self, _: i64) -> Result<()> {
        Ok(())
    }

    fn assign_real(self, _: f64) -> Result<()> {
        Ok(())
    }

    fn assign_empty_text(self) -> Result<()> {
        Ok(())
    }

    fn assign_raw_text(
        self,
        s: *const c_char,
        _: sqlite3_uint64,
        destructor: sqlite3_destructor_type,
        _: c_uchar,
    ) -> Result<()> {
        destroy(s as _, destructor);
        Ok(())
    }

    fn assign_raw_blob(
        self,
        b: *const c_void,
        _: sqlite3_uint64,
        destructor: sqlite3_destructor_type,
    ) -> Result<()> {
        destroy(b as _, destructor);
        Ok(())
    }

    fn assign_zeroblob(self, _: u64) -> Result<()> {
        Ok(())
    }

    #[cfg(feature = "functions")]
    fn assign_arg(self, _: usize) -> Result<()> {
        Ok(())
    }

    #[cfg(feature = "pointer")]
    fn assign_ptr(
        self,
        ptr: *mut c_void,
        _: &'static std::ffi::CStr,
        destructor: sqlite3_destructor_type,
    ) -> Result<()> {
        destroy(ptr, destructor);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::ptr;

    use crate::{Result, ffi::SQLITE_TRANSIENT, types::Assign};

    #[test]
    fn assign_empty_text() -> Result<()> {
        ().assign_text("".as_bytes(), SQLITE_TRANSIENT())
    }

    #[test]
    fn assign_empty_blob() -> Result<()> {
        ().assign_blob("".as_bytes(), SQLITE_TRANSIENT())
    }

    #[test]
    fn destroy() {
        super::destroy(ptr::null_mut(), None);
        super::destroy(ptr::null_mut(), SQLITE_TRANSIENT());
    }
}
