use std::ffi::{CStr, CString, c_char};
use std::rc::Rc;

use super::{Assign, Null, ToSql, ToSqlOutput, Value, ValueRef};
use crate::ffi::{SQLITE_STATIC, SQLITE_TRANSIENT, SQLITE_UTF8};
use crate::util::free_boxed_value;
use crate::{Error, Result};

/// A by-value conversion trait
pub trait IntoSql {
    /// Converts Rust value to SQLite
    fn into_sql<A: Assign>(self, stmt_or_ctx: A) -> Result<()>;
}
impl<T: ToSql + ?Sized> IntoSql for &T {
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        self.to_sql()?.into_sql(a)
    }
}
impl IntoSql for Null {
    #[inline]
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        a.assign_null()
    }
}
impl IntoSql for i64 {
    #[inline]
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        a.assign_int(self)
    }
}

macro_rules! from_i64(
    ($t:ty) => (
        impl IntoSql for $t {
            #[inline]
            fn into_sql<A: Assign>(self, a: A) -> Result<()> {
                a.assign_int(i64::from(self))
            }
        }
        impl IntoSql for Box<$t> {
            #[inline]
            fn into_sql<A: Assign>(self, a: A) -> Result<()> {
                a.assign_int(i64::from(*self))
            }
        }
    );
    (non_zero $t:ty) => (
        impl IntoSql for $t {
            #[inline]
            fn into_sql<A: Assign>(self, a: A) -> Result<()> {
                a.assign_int(self.get().into())
            }
        }
    )
);

from_i64!(bool);
from_i64!(i8);
from_i64!(i16);
from_i64!(i32);
from_i64!(u8);
from_i64!(u16);
from_i64!(u32);
from_i64!(non_zero std::num::NonZeroI8);
from_i64!(non_zero std::num::NonZeroI16);
from_i64!(non_zero std::num::NonZeroI32);
from_i64!(non_zero std::num::NonZeroI64);
from_i64!(non_zero std::num::NonZeroU8);
from_i64!(non_zero std::num::NonZeroU16);
from_i64!(non_zero std::num::NonZeroU32);

macro_rules! try_from_i64 {
    ($t:ty) => {
        impl IntoSql for $t {
            #[inline]
            fn into_sql<A: Assign>(self, a: A) -> Result<()> {
                a.assign_int(i64::try_from(self).map_err(
                    // TODO: Include the values in the error message.
                    |err| Error::ToSqlConversionFailure(err.into()),
                )?)
            }
        }
    };
    (non_zero $t:ty) => {
        impl IntoSql for $t {
            #[inline]
            fn into_sql<A: Assign>(self, a: A) -> Result<()> {
                a.assign_int(i64::try_from(self.get()).map_err(
                    // TODO: Include the values in the error message.
                    |err| Error::ToSqlConversionFailure(err.into()),
                )?)
            }
        }
    };
}

try_from_i64!(isize);
#[cfg(feature = "fallible_uint")]
try_from_i64!(u64);
#[cfg(feature = "fallible_uint")]
try_from_i64!(usize);
#[cfg(feature = "fallible_uint")]
try_from_i64!(non_zero std::num::NonZeroU64);
#[cfg(feature = "fallible_uint")]
try_from_i64!(non_zero std::num::NonZeroUsize);

impl IntoSql for f64 {
    #[inline]
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        a.assign_real(self)
    }
}
impl IntoSql for f32 {
    #[inline]
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        a.assign_real(self.into())
    }
}

impl IntoSql for String {
    #[inline]
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        a.assign_transient_text(self.as_str())
    }
}

impl IntoSql for Vec<u8> {
    #[inline]
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        a.assign_transient_blob(self.as_slice())
    }
}

impl IntoSql for ToSqlOutput<'_> {
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        match self {
            ToSqlOutput::Borrowed(value_ref) => match value_ref {
                ValueRef::Null => a.assign_null(),
                ValueRef::Integer(i) => a.assign_int(i),
                ValueRef::Real(r) => a.assign_real(r),
                ValueRef::Text(s) => unsafe {
                    a.assign_raw_text(
                        s.as_ptr().cast::<c_char>(),
                        s.len() as _,
                        SQLITE_TRANSIENT(),
                        SQLITE_UTF8 as _,
                    )
                },
                ValueRef::Blob(b) => a.assign_transient_blob(b),
            },
            ToSqlOutput::Owned(value) => value.into_sql(a),
            #[cfg(feature = "blob")]
            ToSqlOutput::ZeroBlob(len) => a.assign_zeroblob(len),
            #[cfg(feature = "functions")]
            ToSqlOutput::Arg(i) => a.assign_arg(i),
            #[cfg(feature = "pointer")]
            ToSqlOutput::Pointer(p) => unsafe { a.assign_ptr(p.0 as _, p.1, p.2) },
        }
    }
}

impl IntoSql for Value {
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        match self {
            Value::Null => a.assign_null(),
            Value::Integer(i) => a.assign_int(i),
            Value::Real(r) => a.assign_real(r),
            Value::Text(s) => a.assign_transient_text(s.as_str()),
            Value::Blob(b) => a.assign_transient_blob(b.as_slice()),
        }
    }
}

impl<T: IntoSql> IntoSql for Option<T> {
    #[inline]
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        match self {
            None => a.assign_null(),
            Some(t) => t.into_sql(a),
        }
    }
}

unsafe extern "C" fn free_rc<T>(p: *mut std::ffi::c_void) {
    unsafe { Rc::decrement_strong_count(p.cast::<T>()) };
}

#[cfg(feature = "pointer")]
impl<T> IntoSql for (Rc<T>, &'static CStr) {
    /// Pass a `Rc` as a raw pointer to SQLite
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        unsafe { a.assign_ptr(Rc::into_raw(self.0) as _, self.1, Some(free_rc::<T>)) }
    }
}

#[cfg(feature = "pointer")]
impl<T> IntoSql for (Box<T>, &'static CStr) {
    /// Pass a `Box` as a raw pointer to SQLite
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        unsafe {
            a.assign_ptr(
                Box::into_raw(self.0) as _,
                self.1,
                Some(free_boxed_value::<T>),
            )
        }
    }
}

impl IntoSql for CString {
    /// Pass a `CString` as UTF-8 slice to SQLite
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        unsafe extern "C" fn free_cstring(p: *mut std::ffi::c_void) {
            drop(unsafe { CString::from_raw(p as *mut _) });
        }
        #[cfg(feature = "modern_sqlite")]
        let flags: u8 = (SQLITE_UTF8 | crate::ffi::SQLITE_UTF8_ZT) as _;
        #[cfg(not(feature = "modern_sqlite"))]
        let flags: u8 = SQLITE_UTF8 as _;
        let bytes = self.count_bytes();
        unsafe { a.assign_raw_text(self.into_raw(), bytes as _, Some(free_cstring), flags) }
    }
}

impl IntoSql for &'static CStr {
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        #[cfg(feature = "modern_sqlite")]
        let flags: u8 = (SQLITE_UTF8 | crate::ffi::SQLITE_UTF8_ZT) as _;
        #[cfg(not(feature = "modern_sqlite"))]
        let flags: u8 = SQLITE_UTF8 as _;
        unsafe {
            a.assign_raw_text(
                self.as_ptr(),
                self.count_bytes() as _,
                SQLITE_STATIC(),
                flags,
            )
        }
    }
}

impl IntoSql for Rc<str> {
    /// Pass a `Rc<str>` as UTF-8 slice to SQLite
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        if self.is_empty() {
            return a.assign_empty_text();
        }
        let bytes = self.len();
        unsafe {
            a.assign_raw_text(
                Rc::into_raw(self) as _,
                bytes as _,
                Some(free_rc::<*const str>),
                SQLITE_UTF8 as _,
            )
        }
    }
}
impl IntoSql for Rc<[u8]> {
    /// Pass a `Rc<[u8]>` as a BLOB to SQLite
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        if self.is_empty() {
            return a.assign_zeroblob(0);
        }
        let bytes = self.len();
        unsafe {
            a.assign_raw_blob(
                Rc::into_raw(self) as _,
                bytes as _,
                Some(free_rc::<*const [u8]>),
            )
        }
    }
}

impl<const N: usize> IntoSql for Box<[u8; N]> {
    /// Pass a `Box<[u8; N]>` as a BLOB to SQLite
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        let bytes = self.len();
        unsafe {
            a.assign_raw_blob(
                Box::into_raw(self) as _,
                bytes as _,
                Some(free_boxed_value::<[u8; N]>),
            )
        }
    }
}

#[cfg(feature = "i128_blob")]
impl IntoSql for i128 {
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        // We store these biased (e.g. with the most significant bit flipped)
        // so that comparisons with negative numbers work properly.
        a.assign_transient_blob(&i128::to_be_bytes(self ^ (1_i128 << 127)))
    }
}

#[cfg(feature = "uuid")]
impl IntoSql for uuid::Uuid {
    fn into_sql<A: Assign>(self, a: A) -> Result<()> {
        a.assign_transient_blob(self.as_bytes().as_slice())
    }
}

#[cfg(test)]
mod test {
    use crate::Result;

    use super::IntoSql as _;
    use std::ffi::CString;
    use std::rc::Rc;

    #[test]
    #[cfg(feature = "pointer")]
    fn rc_ptr() -> Result<()> {
        let rc = Rc::new("rc".to_owned());
        rc.into_sql(())
    }

    #[test]
    #[cfg(feature = "pointer")]
    fn box_ptr() -> Result<()> {
        let data = Box::new("box".to_owned());
        (data, c"box").into_sql(())
    }

    #[test]
    fn cstring() -> Result<()> {
        let cs = CString::new("Hello, world!")?;
        cs.into_sql(())
    }

    #[test]
    fn empty_cstring() -> Result<()> {
        let cs = CString::new("")?;
        cs.into_sql(())
    }

    #[test]
    fn static_cstr() -> Result<()> {
        let slice = c"Hello, world!";
        slice.into_sql(())
    }

    #[test]
    fn rc_str() -> Result<()> {
        let rs: Rc<str> = "Hello, world!".into();
        rs.into_sql(())
    }

    #[test]
    fn empty_rc_str() -> Result<()> {
        let rs: Rc<str> = "".into();
        rs.into_sql(())
    }

    #[test]
    fn empty_rc_blob() -> Result<()> {
        let rs: Rc<str> = "".into();
        let rb: Rc<[u8]> = rs.into();
        rb.into_sql(())
    }

    #[test]
    fn static_str() -> Result<()> {
        let str = "Hello, world!";
        str.into_sql(())
    }

    #[test]
    fn rc_u8() -> Result<()> {
        let rs: Rc<str> = "Hello, world!".into();
        let rb: Rc<[u8]> = rs.into();
        rb.into_sql(())
    }

    #[test]
    fn box_u8() -> Result<()> {
        const N: usize = 2;
        let b: Box<[u8]> = Box::from([1, 2]);
        let b: Box<[u8; N]> = b.try_into().unwrap();
        b.into_sql(())
    }

    #[test]
    fn static_slice() -> Result<()> {
        let slice = "Hello, world!".as_bytes();
        slice.into_sql(())
    }
}
