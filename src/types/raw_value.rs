/// SQLite protected value
use std::{
    ffi::{c_int, c_uint},
    marker::PhantomData,
    ops::{Index, IndexMut},
    slice,
};

use crate::{
    Error, Result, ffi,
    types::{FromSql, FromSqlError, Type, ValueRef},
    util,
};

/// Wrapper to [`ffi::sqlite3_value`]s
pub struct Values<'a> {
    pub(crate) args: &'a [*mut ffi::sqlite3_value],
}

impl Values<'_> {
    pub(crate) unsafe fn from_raw_parts(argv: *mut *mut ffi::sqlite3_value, argc: c_int) -> Self {
        let args = unsafe { util::from_raw_parts(argv, argc) };
        Values { args }
    }
    #[cfg(feature = "functions")]
    pub(crate) fn empty() -> Self {
        Values { args: &[] }
    }

    /// Returns the number of values.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.args.len()
    }

    /// Returns `true` if there is no value.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.args.is_empty()
    }

    /// Returns value at `idx`
    #[inline]
    pub fn get<T: FromSql>(&self, idx: usize) -> Result<T> {
        self[idx].get(idx)
    }

    /// Returns the subtype of value at `idx`.
    ///
    /// # Failure
    ///
    /// Will panic if `idx` is greater than or equal to
    /// [`self.len()`](Values::len).
    #[inline]
    #[must_use]
    pub fn get_subtype(&self, idx: usize) -> c_uint {
        self[idx].subtype()
    }

    /// Return raw pointer at `idx`
    /// # Safety
    /// This function is unsafe because it uses raw pointer and cast
    // `sqlite3_value_type` returns `SQLITE_NULL` for pointer.
    // So it seems not possible to enhance `ValueRef::from_value`.
    #[cfg(feature = "pointer")]
    #[must_use]
    pub unsafe fn get_pointer<'a, T: 'static>(
        &self,
        idx: usize,
        ptr_type: &'static std::ffi::CStr,
    ) -> Option<&'a T> {
        unsafe { self[idx].get_pointer(ptr_type) }
    }

    /// Turns `Values` into an iterator.
    #[inline]
    #[must_use]
    pub fn iter(&self) -> ValueIter<'_> {
        ValueIter {
            iter: self.args.iter(),
        }
    }
}

impl<'a> Index<usize> for Values<'a> {
    type Output = RawValue;

    fn index(&self, index: usize) -> &Self::Output {
        let arg = self.args[index];
        unsafe { RawValue::from_ptr(arg) }
    }
}
impl<'a> IndexMut<usize> for Values<'a> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let arg = self.args[index];
        unsafe { RawValue::from_ptr(arg) }
    }
}

impl<'a> IntoIterator for &'a Values<'a> {
    type IntoIter = ValueIter<'a>;
    type Item = ValueRef<'a>;

    #[inline]
    fn into_iter(self) -> ValueIter<'a> {
        self.iter()
    }
}

/// [`Values`] iterator.
pub struct ValueIter<'a> {
    iter: slice::Iter<'a, *mut ffi::sqlite3_value>,
}

impl<'a> Iterator for ValueIter<'a> {
    type Item = ValueRef<'a>;

    #[inline]
    fn next(&mut self) -> Option<ValueRef<'a>> {
        self.iter
            .next()
            .map(|&raw| unsafe { ValueRef::from_value(raw) })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

/// SQLite protected value
#[repr(transparent)]
pub struct RawValue {
    ptr: ffi::sqlite3_value,
    phantom: PhantomData<*mut ffi::sqlite3_value>,
}

impl RawValue {
    #[inline]
    pub(crate) unsafe fn from_ptr<'a>(ptr: *mut ffi::sqlite3_value) -> &'a mut Self {
        unsafe { &mut *(ptr as *mut Self) }
    }
    fn as_mut_ptr(&mut self) -> *mut ffi::sqlite3_value {
        &mut self.ptr as *mut ffi::sqlite3_value
    }
    fn as_ptr(&self) -> *mut ffi::sqlite3_value {
        &self.ptr as *const ffi::sqlite3_value as *mut _
    }

    /// Returns the datatype code
    pub fn data_type(&self) -> Type {
        Type::from(unsafe { ffi::sqlite3_value_type(self.as_ptr()) })
    }

    /// Attempts to apply numeric affinity to the value.
    pub fn numeric_type(&mut self) -> Type {
        Type::from(unsafe { ffi::sqlite3_value_numeric_type(self.as_mut_ptr()) })
    }

    /// Returns the subtype
    #[inline]
    pub fn subtype(&self) -> c_uint {
        unsafe { ffi::sqlite3_value_subtype(self.as_ptr()) }
    }

    /// Returns the value as a `T`.
    ///
    /// `idx` is used only for error reporting.
    ///
    /// # Failure
    ///
    /// Will return Err if the underlying SQLite type cannot be converted to a
    /// `T`.
    pub fn get<T: FromSql>(&self, idx: usize) -> Result<T> {
        let value = self.get_raw();
        FromSql::column_result(value).map_err(|err| match err {
            FromSqlError::InvalidType => Error::InvalidParameterType(idx, value.data_type()),
            FromSqlError::OutOfRange(i) => Error::IntegralValueOutOfRange(idx, i),
            FromSqlError::Utf8Error(err) => Error::Utf8Error(idx, err),
            FromSqlError::Other(err) => {
                Error::FromSqlConversionFailure(idx, value.data_type(), err)
            }
            FromSqlError::InvalidBlobSize { .. } => {
                Error::FromSqlConversionFailure(idx, value.data_type(), Box::new(err))
            }
        })
    }

    /// Returns  as a `ValueRef`.
    #[inline]
    #[must_use]
    pub fn get_raw(&self) -> ValueRef<'_> {
        ValueRef::from(self)
    }

    /// Returns the pointer
    /// # Safety
    /// This function is unsafe because it uses raw pointer and cast
    #[cfg(feature = "pointer")]
    pub unsafe fn get_pointer<'a, T: 'static>(
        &self,
        ptr_type: &'static std::ffi::CStr,
    ) -> Option<&'a T> {
        debug_assert_eq!(
            unsafe { ffi::sqlite3_value_type(self.as_ptr()) },
            ffi::SQLITE_NULL
        );
        unsafe {
            ffi::sqlite3_value_pointer(self.as_ptr(), ptr_type.as_ptr())
                .cast::<T>()
                .as_ref()
        }
    }
}

impl<'a> From<&'a RawValue> for ValueRef<'a> {
    fn from(val: &'a RawValue) -> Self {
        unsafe { ValueRef::from_value(val.as_ptr()) }
    }
}

impl Type {
    pub(crate) fn from(i: i32) -> Self {
        use crate::ffi;
        match i {
            ffi::SQLITE_INTEGER => Type::Integer,
            ffi::SQLITE_FLOAT => Type::Real,
            ffi::SQLITE_TEXT => Type::Text,
            ffi::SQLITE_BLOB => Type::Blob,
            ffi::SQLITE_NULL => Type::Null,
            _ => unreachable!(),
        }
    }
}

#[cfg(all(test, not(miri)))]
mod test {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use crate::{Connection, Result};

    #[test]
    #[cfg(feature = "functions")]
    fn values() -> Result<()> {
        use crate::functions::FunctionFlags;
        use crate::types::Type;

        let db = Connection::open_in_memory()?;
        db.create_scalar_function(
            "test",
            3,
            FunctionFlags::SQLITE_UTF8
                | FunctionFlags::SQLITE_DETERMINISTIC
                | FunctionFlags::SQLITE_SUBTYPE,
            |ctx| {
                use crate::types::ValueRef;

                assert_eq!(ctx.len(), ctx.into_iter().count());
                // int
                let rv0 = &ctx[0];
                assert_eq!(rv0.data_type(), Type::Integer);
                assert_eq!(rv0.subtype(), 0);
                assert_eq!(rv0.get::<u8>(0)?, 1);
                let v = ValueRef::from(rv0);
                assert_eq!(v, ValueRef::Integer(1));
                // null
                let rv1 = &ctx[1];
                assert_eq!(rv1.data_type(), Type::Null);
                assert_eq!(rv1.subtype(), 0);
                assert!(rv1.get::<u8>(1).is_err());
                let v = ValueRef::from(rv1);
                assert_eq!(v, ValueRef::Null);
                // str
                let rv2 = &ctx[2];
                assert_eq!(rv2.data_type(), Type::Text);
                assert_eq!(rv2.subtype(), 0);
                assert_eq!(rv2.get::<String>(2)?, "str");
                let v = ValueRef::from(rv2);
                assert_eq!(v, ValueRef::Text(b"str"));
                // mut
                let rv2 = &mut ctx[2];
                assert_eq!(rv2.numeric_type(), Type::Text);
                Ok(true)
            },
        )?;
        let result: bool = db.query_row("SELECT test(1, NULL, 'str')", [], |r| r.get(0))?;
        assert!(result);
        Ok(())
    }
}
