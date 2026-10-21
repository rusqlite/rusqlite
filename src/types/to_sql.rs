use super::{Assign, Null, Value, ValueRef};
#[cfg(feature = "pointer")]
use crate::util::free_boxed_value;
use crate::{Error, Result, ffi};
use std::borrow::Cow;
use std::ffi::{CStr, CString};
use std::rc::Rc;

/// A trait for types that can be converted into SQLite values. Returns
/// [`crate::Error::ToSqlConversionFailure`] if the conversion fails.
pub trait ToSql {
    /// Converts Rust value to SQLite value
    fn to_sql(&self, a: Assign) -> Result<()>;
    /// by-value
    fn into_sql(self, a: Assign) -> Result<()>
    where
        Self: Sized,
    {
        self.to_sql(a)
    }
}

impl ToSql for ValueRef<'_> {
    fn to_sql(&self, a: Assign) -> Result<()> {
        match self {
            ValueRef::Null => a.assign_null(),
            ValueRef::Integer(i) => a.assign_int(*i),
            ValueRef::Real(r) => a.assign_real(*r),
            ValueRef::Text(t) => a.assign_text_slice(t, ffi::SQLITE_TRANSIENT()),
            ValueRef::Blob(b) => a.assign_transient_blob(*b),
        }
    }
}

impl<T: ToSql + ToOwned + ?Sized> ToSql for Cow<'_, T> {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        self.as_ref().to_sql(a)
    }
}

impl<T: ToSql + ?Sized> ToSql for Box<T> {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        self.as_ref().to_sql(a)
    }
}

impl<T: ToSql + ?Sized> ToSql for Rc<T> {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        self.as_ref().to_sql(a)
    }
}

impl<T: ToSql + ?Sized> ToSql for std::sync::Arc<T> {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        self.as_ref().to_sql(a)
    }
}

// We should be able to use a generic impl like this:
//
// impl<T: Copy> ToSql for T where T: Into<Value> {
//     fn to_sql(&self, a: Assign) -> Result<()> {
//         (*self).into().to_sql(a)
//     }
// }
//
// instead of the following macro, but this runs afoul of
// https://github.com/rust-lang/rust/issues/30191 and reports conflicting
// implementations even when there aren't any.

macro_rules! from_i64(
    ($t:ty) => (
        impl ToSql for $t {
            #[inline]
            fn to_sql(&self, a: Assign) -> Result<()> {
                a.assign_int(i64::from(*self))
            }
            #[inline]
            fn into_sql(self, a: Assign) -> Result<()> {
                a.assign_int(i64::from(self))
            }
        }
    );
    (non_zero $t:ty) => (
        impl ToSql for $t {
            #[inline]
            fn to_sql(&self, a: Assign) -> Result<()> {
                a.assign_int(self.get().into())
            }
            #[inline]
            fn into_sql(self, a: Assign) -> Result<()> {
                a.assign_int(self.get().into())
            }
        }
    )
);

impl ToSql for Null {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        a.assign_null()
    }
    #[inline]
    fn into_sql(self, a: Assign) -> Result<()> {
        a.assign_null()
    }
}
from_i64!(bool);
from_i64!(i8);
from_i64!(i16);
from_i64!(i32);
from_i64!(i64);
from_i64!(u8);
from_i64!(u16);
from_i64!(u32);

impl ToSql for f64 {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        a.assign_real(*self)
    }
    #[inline]
    fn into_sql(self, a: Assign) -> Result<()> {
        a.assign_real(self)
    }
}
impl ToSql for f32 {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        a.assign_real((*self).into())
    }
    #[inline]
    fn into_sql(self, a: Assign) -> Result<()> {
        a.assign_real(self.into())
    }
}

from_i64!(non_zero std::num::NonZeroI8);
from_i64!(non_zero std::num::NonZeroI16);
from_i64!(non_zero std::num::NonZeroI32);
from_i64!(non_zero std::num::NonZeroI64);
from_i64!(non_zero std::num::NonZeroU8);
from_i64!(non_zero std::num::NonZeroU16);
from_i64!(non_zero std::num::NonZeroU32);

#[cfg(feature = "i128_blob")]
impl ToSql for i128 {
    fn to_sql(&self, a: Assign) -> Result<()> {
        // We store these biased (e.g. with the most significant bit flipped)
        // so that comparisons with negative numbers work properly.
        a.assign_transient_blob(i128::to_be_bytes(self ^ (1_i128 << 127)))
    }
}

#[cfg(feature = "i128_blob")]
impl ToSql for std::num::NonZeroI128 {
    fn to_sql(&self, a: Assign) -> Result<()> {
        self.get().to_sql(a)
    }
}

#[cfg(feature = "uuid")]
impl ToSql for uuid::Uuid {
    fn to_sql(&self, a: Assign) -> Result<()> {
        a.assign_transient_blob(self.as_bytes())
    }
}

macro_rules! try_from_i64 {
    ($t:ty) => {
        impl ToSql for $t {
            #[inline]
            fn to_sql(&self, a: Assign) -> Result<()> {
                a.assign_int(i64::try_from(*self).map_err(
                    // TODO: Include the values in the error message.
                    |err| Error::ToSqlConversionFailure(err.into()),
                )?)
            }
            #[inline]
            fn into_sql(self, a: Assign) -> Result<()> {
                a.assign_int(i64::try_from(self).map_err(
                    // TODO: Include the values in the error message.
                    |err| Error::ToSqlConversionFailure(err.into()),
                )?)
            }
        }
    };
    (non_zero $t:ty) => {
        impl ToSql for $t {
            #[inline]
            fn to_sql(&self, a: Assign) -> Result<()> {
                a.assign_int(i64::try_from(self.get()).map_err(
                    // TODO: Include the values in the error message.
                    |err| Error::ToSqlConversionFailure(err.into()),
                )?)
            }
            #[inline]
            fn into_sql(self, a: Assign) -> Result<()> {
                a.assign_int(i64::try_from(self.get()).map_err(
                    // TODO: Include the values in the error message.
                    |err| Error::ToSqlConversionFailure(err.into()),
                )?)
            }
        }
    };
}

try_from_i64!(isize);
try_from_i64!(non_zero std::num::NonZeroIsize);

// Special implementations for usize and u64 because these conversions can fail.
#[cfg(feature = "fallible_uint")]
try_from_i64!(u64);
#[cfg(feature = "fallible_uint")]
try_from_i64!(usize);
#[cfg(feature = "fallible_uint")]
try_from_i64!(non_zero std::num::NonZeroU64);
#[cfg(feature = "fallible_uint")]
try_from_i64!(non_zero std::num::NonZeroUsize);

impl<T: ?Sized> ToSql for &'_ T
where
    T: ToSql,
{
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        (*self).to_sql(a)
    }
}

impl ToSql for String {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        a.assign_transient_text(self)
    }
}

impl ToSql for str {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        a.assign_transient_text(self)
    }
}

impl ToSql for Vec<u8> {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        a.assign_transient_blob(self)
    }
}

impl<const N: usize> ToSql for [u8; N] {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        a.assign_transient_blob(self)
    }
}

impl ToSql for [u8] {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        a.assign_transient_blob(self)
    }
}

impl ToSql for Value {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        match self {
            Value::Null => a.assign_null(),
            Value::Integer(i) => a.assign_int(*i),
            Value::Real(r) => a.assign_real(*r),
            Value::Text(t) => a.assign_transient_text(t),
            Value::Blob(b) => a.assign_transient_blob(b),
        }
    }
}

impl<T: ToSql> ToSql for Option<T> {
    #[inline]
    fn to_sql(&self, a: Assign) -> Result<()> {
        match *self {
            None => a.assign_null(),
            Some(ref t) => t.to_sql(a),
        }
    }
    #[inline]
    fn into_sql(self, a: Assign) -> Result<()> {
        match self {
            None => a.assign_null(),
            Some(ref t) => t.into_sql(a),
        }
    }
}

#[cfg(feature = "pointer")]
impl<T> ToSql for (Rc<T>, &'static CStr) {
    fn to_sql(&self, _: Assign) -> Result<()> {
        Err(err!(ffi::SQLITE_MISUSE, "Pointer must be passed by value"))
    }
    /// Pass a `Rc` as a raw pointer to SQLite
    fn into_sql(self, a: Assign) -> Result<()> {
        unsafe extern "C" fn free_rc<T>(p: *mut std::ffi::c_void) {
            unsafe { Rc::decrement_strong_count(p.cast::<T>()) };
        }
        unsafe { a.assign_ptr(Rc::into_raw(self.0) as _, self.1, Some(free_rc::<T>)) }
    }
}
#[cfg(feature = "pointer")]
impl<T> ToSql for (Box<T>, &'static CStr) {
    fn to_sql(&self, _: Assign) -> Result<()> {
        Err(err!(ffi::SQLITE_MISUSE, "Pointer must be passed by value"))
    }
    /// Pass a `Rc` as a raw pointer to SQLite
    fn into_sql(self, a: Assign) -> Result<()> {
        unsafe {
            a.assign_ptr(
                Box::into_raw(self.0) as _,
                self.1,
                Some(free_boxed_value::<T>),
            )
        }
    }
}
#[cfg(feature = "pointer")]
impl ToSql for (*mut std::ffi::c_void, &'static CStr) {
    fn to_sql(&self, a: Assign) -> Result<()> {
        unsafe { a.assign_ptr(self.0, self.1, None) }
    }
    /// Pass a `Rc` as a raw pointer to SQLite
    fn into_sql(self, a: Assign) -> Result<()> {
        unsafe { a.assign_ptr(self.0, self.1, None) }
    }
}

impl ToSql for CString {
    fn to_sql(&self, a: Assign) -> Result<()> {
        #[cfg(feature = "modern_sqlite")]
        let flags: u8 = (ffi::SQLITE_UTF8 | ffi::SQLITE_UTF8_ZT) as _;
        #[cfg(not(feature = "modern_sqlite"))]
        let flags: u8 = ffi::SQLITE_UTF8 as _;
        unsafe {
            a.assign_raw_text(
                self.as_ptr(),
                self.count_bytes() as _,
                ffi::SQLITE_TRANSIENT(),
                flags,
            )
        }
    }
    /// Pass a `CString` as UTF-8 slice to SQLite
    fn into_sql(self, a: Assign) -> Result<()> {
        unsafe extern "C" fn free_cstring(p: *mut std::ffi::c_void) {
            drop(unsafe { CString::from_raw(p as *mut _) });
        }
        #[cfg(feature = "modern_sqlite")]
        let flags: u8 = (ffi::SQLITE_UTF8 | ffi::SQLITE_UTF8_ZT) as _;
        #[cfg(not(feature = "modern_sqlite"))]
        let flags: u8 = ffi::SQLITE_UTF8 as _;
        let bytes = self.count_bytes();
        unsafe { a.assign_raw_text(self.into_raw(), bytes as _, Some(free_cstring), flags) }
    }
}
impl ToSql for &'static CStr {
    fn to_sql(&self, a: Assign) -> Result<()> {
        #[cfg(feature = "modern_sqlite")]
        let flags: u8 = (ffi::SQLITE_UTF8 | ffi::SQLITE_UTF8_ZT) as _;
        #[cfg(not(feature = "modern_sqlite"))]
        let flags: u8 = ffi::SQLITE_UTF8 as _;
        unsafe {
            a.assign_raw_text(
                self.as_ptr(),
                self.count_bytes() as _,
                ffi::SQLITE_STATIC(),
                flags,
            )
        }
    }
}

#[cfg(test)]
mod test {
    use std::ffi::CString;
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::ToSql;
    use crate::Result;
    use crate::types::assign::SINK;

    fn is_to_sql<T: ToSql>() {}

    #[test]
    fn test_integral_types() {
        is_to_sql::<i8>();
        is_to_sql::<i16>();
        is_to_sql::<i32>();
        is_to_sql::<i64>();
        is_to_sql::<isize>();
        is_to_sql::<u8>();
        is_to_sql::<u16>();
        is_to_sql::<u32>();
        #[cfg(feature = "fallible_uint")]
        is_to_sql::<u64>();
        #[cfg(feature = "fallible_uint")]
        is_to_sql::<usize>();
    }

    #[test]
    fn test_nonzero_types() {
        is_to_sql::<std::num::NonZeroI8>();
        is_to_sql::<std::num::NonZeroI16>();
        is_to_sql::<std::num::NonZeroI32>();
        is_to_sql::<std::num::NonZeroI64>();
        is_to_sql::<std::num::NonZeroIsize>();
        is_to_sql::<std::num::NonZeroU8>();
        is_to_sql::<std::num::NonZeroU16>();
        is_to_sql::<std::num::NonZeroU32>();
        #[cfg(feature = "fallible_uint")]
        is_to_sql::<std::num::NonZeroU64>();
        #[cfg(feature = "fallible_uint")]
        is_to_sql::<std::num::NonZeroUsize>();
    }

    #[test]
    fn test_u8_array() -> Result<()> {
        let a: [u8; 99] = [0u8; 99];
        let _a: &[&dyn ToSql] = crate::params![a];
        ToSql::to_sql(&a, SINK)
    }

    #[test]
    fn test_cow_str() -> Result<()> {
        use std::borrow::Cow;
        let s = "str";
        let cow: Cow<str> = Cow::Borrowed(s);
        cow.to_sql(SINK)?;
        let cow: Cow<str> = Cow::Owned::<str>(String::from(s));
        cow.to_sql(SINK)?;
        // Ensure this compiles.
        let _p: &[&dyn ToSql] = crate::params![cow];
        Ok(())
    }

    #[test]
    fn test_box_dyn() -> Result<()> {
        let s: Box<dyn ToSql> = Box::new("Hello world!");
        let _s: &[&dyn ToSql] = crate::params![s];
        ToSql::to_sql(&s, SINK)
    }

    #[test]
    fn test_box_deref() -> Result<()> {
        let s: Box<str> = "Hello world!".into();
        let _s: &[&dyn ToSql] = crate::params![s];
        s.to_sql(SINK)
    }

    #[test]
    fn test_box_direct() -> Result<()> {
        let s: Box<str> = "Hello world!".into();
        let _s: &[&dyn ToSql] = crate::params![s];
        ToSql::to_sql(&s, SINK)
    }

    #[test]
    fn test_cells() -> Result<()> {
        use std::{rc::Rc, sync::Arc};

        let source_str: Box<str> = "Hello world!".into();

        let s: Rc<Box<str>> = Rc::new(source_str.clone());
        let _s: &[&dyn ToSql] = crate::params![s];
        s.to_sql(SINK)?;

        let s: Arc<Box<str>> = Arc::new(source_str.clone());
        let _s: &[&dyn ToSql] = crate::params![s];
        s.to_sql(SINK)?;

        let s: Arc<str> = Arc::from(&*source_str);
        let _s: &[&dyn ToSql] = crate::params![s];
        s.to_sql(SINK)?;

        let s: Arc<dyn ToSql> = Arc::new(source_str.clone());
        let _s: &[&dyn ToSql] = crate::params![s];
        s.to_sql(SINK)?;

        let s: Rc<str> = Rc::from(&*source_str);
        let _s: &[&dyn ToSql] = crate::params![s];
        s.to_sql(SINK)?;

        let s: Rc<dyn ToSql> = Rc::new(source_str);
        let _s: &[&dyn ToSql] = crate::params![s];
        s.to_sql(SINK)
    }

    #[cfg(feature = "i128_blob")]
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_i128() -> Result<()> {
        use crate::Connection;
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (i128 BLOB, desc TEXT)")?;
        db.execute(
            "
            INSERT INTO foo(i128, desc) VALUES
                (?1, 'zero'),
                (?2, 'neg one'), (?3, 'neg two'),
                (?4, 'pos one'), (?5, 'pos two'),
                (?6, 'min'), (?7, 'max')",
            [0i128, -1i128, -2i128, 1i128, 2i128, i128::MIN, i128::MAX],
        )?;

        let mut stmt = db.prepare("SELECT i128, desc FROM foo ORDER BY i128 ASC")?;

        let res = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i128>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        assert_eq!(
            res,
            &[
                (i128::MIN, "min".to_owned()),
                (-2, "neg two".to_owned()),
                (-1, "neg one".to_owned()),
                (0, "zero".to_owned()),
                (1, "pos one".to_owned()),
                (2, "pos two".to_owned()),
                (i128::MAX, "max".to_owned()),
            ]
        );
        Ok(())
    }

    #[cfg(feature = "i128_blob")]
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_non_zero_i128() -> Result<()> {
        use std::num::NonZeroI128;
        macro_rules! nz {
            ($x:expr) => {
                NonZeroI128::new($x).unwrap()
            };
        }

        let db = crate::Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (i128 BLOB, desc TEXT)")?;
        db.execute(
            "INSERT INTO foo(i128, desc) VALUES
                (?1, 'neg one'), (?2, 'neg two'),
                (?3, 'pos one'), (?4, 'pos two'),
                (?5, 'min'), (?6, 'max')",
            [
                nz!(-1),
                nz!(-2),
                nz!(1),
                nz!(2),
                nz!(i128::MIN),
                nz!(i128::MAX),
            ],
        )?;
        let mut stmt = db.prepare("SELECT i128, desc FROM foo ORDER BY i128 ASC")?;

        let res = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<(NonZeroI128, String)>, _>>()?;

        assert_eq!(
            res,
            &[
                (nz!(i128::MIN), "min".to_owned()),
                (nz!(-2), "neg two".to_owned()),
                (nz!(-1), "neg one".to_owned()),
                (nz!(1), "pos one".to_owned()),
                (nz!(2), "pos two".to_owned()),
                (nz!(i128::MAX), "max".to_owned()),
            ]
        );
        let err = db.query_row("SELECT ?1", [0i128], |row| row.get::<_, NonZeroI128>(0));
        assert_eq!(err, Err(crate::Error::IntegralValueOutOfRange(0, 0)));
        Ok(())
    }

    #[cfg(feature = "uuid")]
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_uuid() -> Result<()> {
        use crate::{Connection, params};
        use uuid::Uuid;

        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (id BLOB CHECK(length(id) = 16), label TEXT);")?;

        let id = Uuid::new_v4();

        db.execute(
            "INSERT INTO foo (id, label) VALUES (?1, ?2)",
            params![id, "target"],
        )?;

        let mut stmt = db.prepare("SELECT id, label FROM foo WHERE id = ?1")?;

        let mut rows = stmt.query(params![id])?;
        let row = rows.next()?.unwrap();

        let found_id: Uuid = row.get_unwrap(0);
        let found_label: String = row.get_unwrap(1);

        assert_eq!(found_id, id);
        assert_eq!(found_label, "target");
        Ok(())
    }

    #[test]
    #[cfg(feature = "pointer")]
    fn rc_ptr() -> Result<()> {
        use std::rc::Rc;
        let rc = Rc::new("rc".to_owned());
        (rc, c"rc").into_sql(SINK)
    }

    #[test]
    #[cfg(feature = "pointer")]
    fn box_ptr() -> Result<()> {
        let data = Box::new("box".to_owned());
        (data, c"box").into_sql(SINK)
    }

    #[test]
    fn cstring() -> Result<()> {
        let cs = CString::new("Hello, world!")?;
        cs.into_sql(SINK)
    }

    #[test]
    fn empty_cstring() -> Result<()> {
        let cs = CString::new("")?;
        cs.into_sql(SINK)
    }

    #[test]
    fn static_cstr() -> Result<()> {
        let slice = c"Hello, world!";
        slice.into_sql(SINK)
    }
}
