use crate::Result;
use crate::error::error_from_handle;
use crate::ffi::{
    self, SQLITE_OK, SQLITE_STATIC, SQLITE_TRANSIENT, SQLITE_UTF8, sqlite3_destructor_type,
    sqlite3_stmt, sqlite3_uint64,
};
#[cfg(any(feature = "functions", feature = "vtab"))]
use crate::ffi::{sqlite3_context, sqlite3_value};

use crate::pragma::Sql;
#[cfg(feature = "pointer")]
use std::ffi::CStr;
use std::ffi::{c_char, c_int, c_uchar, c_void};
use std::mem;

/// `sqlite3_stmt` or `sqlite3_context`
pub enum Assign<'v> {
    /// Statement parameter
    Stmt {
        /// Statement
        s: *mut sqlite3_stmt,
        /// nth parameter
        n: c_int,
        /// Allocator
        #[cfg(feature = "bumpalo")]
        bump: &'v bumpalo::Bump,
    },
    /// SQL function or virtual table result
    #[cfg(any(feature = "functions", feature = "vtab"))]
    Ctx((*mut sqlite3_context, &'v [*mut sqlite3_value])),
    /// Pragma parameter
    Pragma(&'v mut Sql),
    /// Test
    #[cfg(test)]
    U,
}
#[cfg(test)]
pub(crate) const SINK: Assign = Assign::U;

impl Assign<'_> {
    /// error handling
    pub fn decode_result(self, code: c_int) -> Result<()> {
        if code == SQLITE_OK {
            return Ok(());
        }
        Err(match self {
            Self::Stmt { s, .. } => unsafe { error_from_handle(ffi::sqlite3_db_handle(s), code) },
            #[cfg(any(feature = "functions", feature = "vtab"))]
            Self::Ctx(x) => unsafe { error_from_handle(ffi::sqlite3_context_db_handle(x.0), code) },
            Self::Pragma(_) => unreachable!(),
            #[cfg(test)]
            Self::U => unreachable!(),
        })
    }

    /// `sqlite3_bind_null` or `sqlite3_result_null`
    pub fn assign_null(self) -> Result<()> {
        match self {
            Self::Stmt { s, n, .. } => self.decode_result(unsafe { ffi::sqlite3_bind_null(s, n) }),
            #[cfg(any(feature = "functions", feature = "vtab"))]
            Self::Ctx(x) => unsafe {
                ffi::sqlite3_result_null(x.0);
                Ok(())
            },
            Self::Pragma(_) => Err(err!(ffi::SQLITE_MISUSE, "Unsupported value \"NULL\"")),
            #[cfg(test)]
            Self::U => Ok(()),
        }
    }

    /// `sqlite3_bind_int64` or `sqlite3_result_int64`
    pub fn assign_int(self, i: i64) -> Result<()> {
        match self {
            Self::Stmt { s, n, .. } => {
                self.decode_result(unsafe { ffi::sqlite3_bind_int64(s, n, i) })
            }
            #[cfg(any(feature = "functions", feature = "vtab"))]
            Self::Ctx(x) => unsafe {
                ffi::sqlite3_result_int64(x.0, i);
                Ok(())
            },
            Self::Pragma(sql) => {
                sql.push_int(i);
                Ok(())
            }
            #[cfg(test)]
            Self::U => Ok(()),
        }
    }

    /// `sqlite3_bind_double` or `sqlite3_result_double`
    pub fn assign_real(self, r: f64) -> Result<()> {
        match self {
            Self::Stmt { s, n, .. } => {
                self.decode_result(unsafe { ffi::sqlite3_bind_double(s, n, r) })
            }
            #[cfg(any(feature = "functions", feature = "vtab"))]
            Self::Ctx(x) => unsafe {
                ffi::sqlite3_result_double(x.0, r);
                Ok(())
            },
            Self::Pragma(sql) => {
                sql.push_real(r);
                Ok(())
            }
            #[cfg(test)]
            Self::U => Ok(()),
        }
    }

    /// like `sqlite3_bind_zeroblob64` or `sqlite3_result_zeroblob64` for text
    pub fn assign_empty_text(self) -> Result<()> {
        match self {
            Self::Stmt { s, n, .. } => self.decode_result(unsafe {
                ffi::sqlite3_bind_text64(
                    s,
                    n,
                    "".as_ptr().cast::<c_char>(),
                    0,
                    SQLITE_STATIC(),
                    SQLITE_UTF8 as _,
                )
            }),
            #[cfg(any(feature = "functions", feature = "vtab"))]
            Self::Ctx(x) => unsafe {
                ffi::sqlite3_result_text64(
                    x.0,
                    "".as_ptr().cast::<c_char>(),
                    0,
                    SQLITE_STATIC(),
                    SQLITE_UTF8 as _,
                );
                Ok(())
            },
            Self::Pragma(sql) => {
                sql.push_string_literal("");
                Ok(())
            }
            #[cfg(test)]
            Self::U => Ok(()),
        }
    }
    /// `sqlite3_bind_text64` or `sqlite3_result_text64`
    pub fn assign_text(self, s: &str, destructor: sqlite3_destructor_type) -> Result<()> {
        self.assign_text_slice(s, destructor)
    }
    /// `sqlite3_bind_text64` or `sqlite3_result_text64`
    ///
    /// # Safety
    /// `b` should be NULL or a pointer to a well-formed UTF8 string of `len` bytes
    pub unsafe fn assign_raw_text(
        self,
        t: *const c_char,
        len: sqlite3_uint64,
        destructor: sqlite3_destructor_type,
        encoding: c_uchar,
    ) -> Result<()> {
        if len == 0 {
            destroy(t as _, destructor);
            self.assign_empty_text()
        } else {
            match self {
                Self::Stmt { s, n, .. } => self.decode_result(unsafe {
                    ffi::sqlite3_bind_text64(s, n, t, len, destructor, encoding)
                }),
                #[cfg(any(feature = "functions", feature = "vtab"))]
                Self::Ctx(x) => unsafe {
                    ffi::sqlite3_result_text64(x.0, t, len, destructor, encoding);
                    Ok(())
                },
                Self::Pragma(sql) => unsafe {
                    let slice = std::slice::from_raw_parts(t as *const u8, len as _);
                    sql.push_string_literal(str::from_utf8(slice)?);
                    Ok(())
                },
                #[cfg(test)]
                Self::U => {
                    destroy(t as _, destructor);
                    Ok(())
                }
            }
        }
    }
    /// Like `assign_text` with `SQLITE_TRANSIENT`
    #[inline]
    pub fn assign_transient_text<T: AsRef<str>>(self, s: T) -> Result<()> {
        self.assign_text(s.as_ref(), SQLITE_TRANSIENT())
    }
    /// Like `assign_text` with a byte slice
    pub fn assign_text_slice<T: AsRef<[u8]>>(
        self,
        s: T,
        destructor: sqlite3_destructor_type,
    ) -> Result<()> {
        unsafe {
            self.assign_raw_text(
                s.as_ref().as_ptr().cast::<c_char>(),
                s.as_ref().len() as _,
                destructor,
                SQLITE_UTF8 as _,
            )
        }
    }

    /// Try to avoid allocating twice (only for statement with `bumpalo`):
    /// - first in Rust while converting `v` to `String`
    /// - then in SQLite for ensuring that the transient Rust `String` is not used after free
    pub fn write_fmt<T: std::fmt::Display>(self, v: T) -> Result<()> {
        match self {
            #[cfg(feature = "bumpalo")]
            Assign::Stmt { bump, .. } => {
                use std::fmt::Write as _;
                let mut buf = bumpalo::collections::string::String::new_in(bump);
                buf.write_fmt(format_args!("{v}"))
                    .map_err(|err| crate::Error::ToSqlConversionFailure(err.into()))?;
                self.assign_text(&buf, SQLITE_STATIC())
            }
            _ => self.assign_transient_text(format!("{v}")),
        }
    }

    /// `sqlite3_bind_blob64` or `sqlite3_result_blob64`
    #[inline]
    pub fn assign_blob(self, b: &[u8], destructor: sqlite3_destructor_type) -> Result<()> {
        unsafe { self.assign_raw_blob(b.as_ptr().cast::<c_void>(), b.len() as _, destructor) }
    }
    /// `sqlite3_bind_blob64` or `sqlite3_result_blob64`
    ///
    /// # Safety
    /// `b` should be NULL or a valid pointer to `len` bytes
    pub unsafe fn assign_raw_blob(
        self,
        b: *const c_void,
        len: sqlite3_uint64,
        destructor: sqlite3_destructor_type,
    ) -> Result<()> {
        if len == 0 {
            destroy(b as _, destructor);
            self.assign_zeroblob(0)
        } else {
            match self {
                Self::Stmt { s, n, .. } => self.decode_result({
                    unsafe { ffi::sqlite3_bind_blob64(s, n, b, len, destructor) }
                }),
                #[cfg(any(feature = "functions", feature = "vtab"))]
                Self::Ctx(x) => unsafe {
                    ffi::sqlite3_result_blob64(x.0, b, len, destructor);
                    Ok(())
                },
                Self::Pragma(_) => Err(err!(ffi::SQLITE_MISUSE, "Unsupported value \"BLOB\"")),
                #[cfg(test)]
                Self::U => {
                    destroy(b as _, destructor);
                    Ok(())
                }
            }
        }
    }
    /// Like `assign_blob` with `SQLITE_TRANSIENT`
    #[inline]
    pub fn assign_transient_blob<T: AsRef<[u8]>>(self, b: T) -> Result<()> {
        self.assign_blob(b.as_ref(), SQLITE_TRANSIENT())
    }

    /// `sqlite3_bind_zeroblob64` or `sqlite3_result_zeroblob64`
    pub fn assign_zeroblob(self, len: u64) -> Result<()> {
        match self {
            Self::Stmt { s, n, .. } => {
                self.decode_result(unsafe { ffi::sqlite3_bind_zeroblob64(s, n, len) })
            }
            #[cfg(any(feature = "functions", feature = "vtab"))]
            Self::Ctx(x) => self.decode_result(unsafe { ffi::sqlite3_result_zeroblob64(x.0, len) }),
            Self::Pragma(_) => Err(err!(ffi::SQLITE_MISUSE, "Unsupported value \"BLOB\"")),
            #[cfg(test)]
            Self::U => Ok(()),
        }
    }

    /// `sqlite3_result_value`
    #[cfg(feature = "functions")]
    pub fn assign_arg(self, idx: usize) -> Result<()> {
        match self {
            Self::Stmt { .. } => Err(err!(ffi::SQLITE_MISUSE, "Unsupported value")),
            #[cfg(any(feature = "functions", feature = "vtab"))]
            Self::Ctx(x) => unsafe {
                ffi::sqlite3_result_value(x.0, x.1[idx]);
                Ok(())
            },
            Self::Pragma(_) => Err(err!(ffi::SQLITE_MISUSE, "Unsupported value \"ARG\"")),
            #[cfg(test)]
            Self::U => Ok(()),
        }
    }

    /// `sqlite3_bind_pointer` or `sqlite3_result_pointer`
    ///
    /// # Safety
    ///
    #[cfg(feature = "pointer")]
    pub unsafe fn assign_ptr(
        self,
        ptr: *mut c_void,
        ptr_type: &'static CStr,
        destructor: sqlite3_destructor_type,
    ) -> Result<()> {
        match self {
            Self::Stmt { s, n, .. } => self.decode_result(unsafe {
                ffi::sqlite3_bind_pointer(s, n, ptr, ptr_type.as_ptr(), destructor)
            }),
            #[cfg(any(feature = "functions", feature = "vtab"))]
            Self::Ctx(x) => unsafe {
                ffi::sqlite3_result_pointer(x.0, ptr, ptr_type.as_ptr(), destructor);
                Ok(())
            },
            Self::Pragma(_) => Err(err!(ffi::SQLITE_MISUSE, "Unsupported value \"PTR\"")),
            #[cfg(test)]
            Self::U => {
                destroy(ptr, destructor);
                Ok(())
            }
        }
    }
}

fn destroy(ptr: *mut c_void, destructor: sqlite3_destructor_type) {
    if let Some(d) = destructor {
        unsafe {
            #[expect(clippy::transmutes_expressible_as_ptr_casts)] // for Miri
            if mem::transmute::<unsafe extern "C" fn(*mut c_void), isize>(d) == -1 {
                return; // SQLITE_TRANSIENT
            }
            d(ptr);
        }
    }
}

#[cfg(test)]
mod test {
    use std::ptr;

    use crate::{Result, ffi::SQLITE_TRANSIENT, types::SINK};

    #[test]
    fn assign_empty_text() -> Result<()> {
        SINK.assign_text("", SQLITE_TRANSIENT())
    }

    #[test]
    fn assign_empty_blob() -> Result<()> {
        SINK.assign_blob("".as_bytes(), SQLITE_TRANSIENT())
    }

    #[test]
    fn destroy() {
        super::destroy(ptr::null_mut(), None);
        super::destroy(ptr::null_mut(), SQLITE_TRANSIENT());
    }
}
