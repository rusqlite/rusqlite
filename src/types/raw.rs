#[cfg(feature = "pointer")]
use std::ffi::CStr;
use std::{
    ffi::{CString, c_char, c_void},
    num::NonZeroUsize,
    rc::Rc,
};

use crate::{
    ffi::{SQLITE_STATIC, SQLITE_TRANSIENT, SQLITE_UTF8, sqlite3_destructor_type},
    types::{Value, ValueRef},
    util::free_boxed_value,
};

/// Raw value to be passed to SQLite (`sqlite3_bind_*` or `sqlite3_result_*`)
#[derive(Clone, Copy, Debug)]
pub enum RawValue {
    /// `NULL` value.
    Null,
    /// Signed integer.
    Integer(i64),
    /// Floating point number.
    Real(f64),
    /// Pointer passing interface
    #[cfg(feature = "pointer")]
    Pointer {
        /// Raw pointer
        ptr: *const c_void,
        /// Point type name
        ptr_type: &'static CStr,
        /// `ptr` destructor
        destructor: sqlite3_destructor_type,
    },
    /// n-th arg of an SQL scalar function
    #[cfg(feature = "functions")]
    Arg(usize),
    /// UTF-8
    Text {
        /// Raw pointer to a byte array
        ptr: *const c_char,
        /// Bytes count.
        /// This does not include the nul terminator if any.
        bytes: NonZeroUsize,
        /// `ptr` destructor
        destructor: sqlite3_destructor_type,
        /// Text encoding
        flags: u8,
    },
    /// Empty Text
    EmptyText,
    /// BLOB
    Blob {
        /// Raw pointer to a byte array
        ptr: *const c_void,
        /// Bytes count.
        bytes: NonZeroUsize,
        /// `ptr` destructor
        destructor: sqlite3_destructor_type,
    },
    /// A BLOB of the given length that is filled with
    /// zeroes.
    ZeroBlob(u64),
}

impl<'a> From<ValueRef<'a>> for RawValue {
    fn from(value: ValueRef<'a>) -> Self {
        match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(i) => Self::Integer(i),
            ValueRef::Real(r) => Self::Real(r),
            ValueRef::Text(t) => {
                if let Some(bytes) = NonZeroUsize::new(t.len()) {
                    Self::Text {
                        ptr: t.as_ptr() as _,
                        bytes,
                        destructor: SQLITE_TRANSIENT(),
                        flags: SQLITE_UTF8 as _,
                    }
                } else {
                    Self::EmptyText
                }
            }
            ValueRef::Blob(b) => {
                if let Some(bytes) = NonZeroUsize::new(b.len()) {
                    Self::Blob {
                        ptr: b.as_ptr() as _,
                        bytes,
                        destructor: SQLITE_TRANSIENT(),
                    }
                } else {
                    Self::ZeroBlob(0)
                }
            }
        }
    }
}

impl From<Value> for RawValue {
    fn from(value: Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Integer(i) => Self::Integer(i),
            Value::Real(r) => Self::Real(r),
            Value::Text(t) => {
                if let Some(bytes) = NonZeroUsize::new(t.len()) {
                    Self::Text {
                        ptr: t.as_ptr() as _,
                        bytes,
                        destructor: SQLITE_TRANSIENT(),
                        flags: SQLITE_UTF8 as _,
                    }
                } else {
                    Self::EmptyText
                }
            }
            Value::Blob(b) => {
                if let Some(bytes) = NonZeroUsize::new(b.len()) {
                    Self::Blob {
                        ptr: b.as_ptr() as _,
                        bytes,
                        destructor: SQLITE_TRANSIENT(),
                    }
                } else {
                    Self::ZeroBlob(0)
                }
            }
        }
    }
}

#[cfg(feature = "pointer")]
impl<T> From<(Rc<T>, &'static CStr)> for RawValue {
    /// Pass a `Rc` as a raw pointer to SQLite
    ///
    /// # Warning
    /// Leak memory if an error happens before the returned pointer is bound to an SQLite statement.
    fn from(value: (Rc<T>, &'static CStr)) -> Self {
        unsafe extern "C" fn free_rc<T>(p: *mut std::ffi::c_void) {
            unsafe {
                Rc::decrement_strong_count(p.cast::<T>());
            }
        }
        RawValue::Pointer {
            ptr: Rc::into_raw(value.0) as _,
            ptr_type: value.1,
            destructor: Some(free_rc::<T>),
        }
    }
}

#[cfg(feature = "pointer")]
impl<T> From<(Box<T>, &'static CStr)> for RawValue {
    /// Pass a `Box` as a raw pointer to SQLite
    ///
    /// # Warning
    /// Leak memory if an error happens before the returned pointer is bound to an SQLite statement.
    fn from(value: (Box<T>, &'static CStr)) -> Self {
        RawValue::Pointer {
            ptr: Box::into_raw(value.0) as _,
            ptr_type: value.1,
            destructor: Some(free_boxed_value::<T>),
        }
    }
}

impl From<CString> for RawValue {
    /// Pass a `CString` as UTF-8 slice to SQLite
    ///
    /// # Warning
    /// Leak memory if an error happens before the returned pointer is bound to an SQLite statement.
    fn from(value: CString) -> Self {
        if value.is_empty() {
            return Self::EmptyText;
        }
        unsafe extern "C" fn free_cstring(p: *mut std::ffi::c_void) {
            drop(unsafe { CString::from_raw(p as *mut _) });
        }
        #[cfg(feature = "modern_sqlite")]
        let flags: u8 = (SQLITE_UTF8 | crate::ffi::SQLITE_UTF8_ZT) as _;
        #[cfg(not(feature = "modern_sqlite"))]
        let flags: u8 = SQLITE_UTF8 as _;
        let bytes = value.count_bytes();
        RawValue::Text {
            ptr: value.into_raw() as _,
            bytes: NonZeroUsize::new(bytes).unwrap(),
            destructor: Some(free_cstring),
            flags,
        }
    }
}
impl From<Rc<str>> for RawValue {
    /// Pass a `Rc<str>` as UTF-8 slice to SQLite
    ///
    /// # Warning
    /// Leak memory if an error happens before the returned pointer is bound to an SQLite statement.
    fn from(value: Rc<str>) -> Self {
        if value.is_empty() {
            return Self::EmptyText;
        }
        unsafe extern "C" fn free_rc_str(p: *mut std::ffi::c_void) {
            unsafe { Rc::decrement_strong_count(p.cast::<*const str>()) };
        }
        let rb: Rc<[u8]> = value.into(); // TODO Validate: necessary ?
        let bytes = rb.len();
        RawValue::Text {
            ptr: Rc::into_raw(rb) as _,
            bytes: NonZeroUsize::new(bytes).unwrap(),
            destructor: Some(free_rc_str),
            flags: SQLITE_UTF8 as _,
        }
    }
}
impl From<&'static str> for RawValue {
    fn from(value: &'static str) -> Self {
        if value.is_empty() {
            return Self::EmptyText;
        }
        let bytes = value.len();
        RawValue::Text {
            ptr: value.as_ptr() as _,
            bytes: NonZeroUsize::new(bytes).unwrap(),
            destructor: SQLITE_STATIC(),
            flags: SQLITE_UTF8 as _,
        }
    }
}

impl From<Rc<[u8]>> for RawValue {
    /// Pass a `Rc<[u8]>` as a BLOB to SQLite
    ///
    /// # Warning
    /// Leak memory if an error happens before the returned pointer is bound to an SQLite statement.
    fn from(value: Rc<[u8]>) -> Self {
        if value.is_empty() {
            return Self::ZeroBlob(0);
        }
        unsafe extern "C" fn free_rc_slice(p: *mut std::ffi::c_void) {
            unsafe { Rc::decrement_strong_count(p.cast::<*const [u8]>()) };
        }
        let bytes = value.len();
        RawValue::Blob {
            ptr: Rc::into_raw(value) as _,
            bytes: NonZeroUsize::new(bytes).unwrap(),
            destructor: Some(free_rc_slice),
        }
    }
}
impl<const N: usize> From<Box<[u8; N]>> for RawValue {
    /// Pass a `Box<[u8; N]>` as a BLOB to SQLite
    ///
    /// # Warning
    /// Leak memory if an error happens before the returned pointer is bound to an SQLite statement.
    fn from(value: Box<[u8; N]>) -> Self {
        if value.is_empty() {
            return Self::ZeroBlob(0);
        }
        let bytes = value.len();
        RawValue::Blob {
            ptr: Box::into_raw(value) as _,
            bytes: NonZeroUsize::new(bytes).unwrap(),
            destructor: Some(free_boxed_value::<[u8; N]>),
        }
    }
}
impl From<&'static [u8]> for RawValue {
    fn from(value: &'static [u8]) -> Self {
        if value.is_empty() {
            return Self::ZeroBlob(0);
        }
        let bytes = value.len();
        RawValue::Blob {
            ptr: value.as_ptr() as _,
            bytes: NonZeroUsize::new(bytes).unwrap(),
            destructor: SQLITE_STATIC(),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::ffi::SQLITE_UTF8;

    use super::RawValue;
    use std::ffi::CString;
    use std::rc::Rc;

    #[test]
    #[cfg(feature = "pointer")]
    fn rc_ptr() {
        let rc = std::rc::Rc::new("rc".to_owned());
        let rv = RawValue::from((rc, c"rc"));
        let RawValue::Pointer {
            ptr,
            ptr_type,
            destructor,
        } = rv
        else {
            panic!("RawValue::Pointer expected");
        };
        assert_eq!(ptr_type, c"rc");
        unsafe { destructor.unwrap()(ptr as *mut _) };
    }

    #[test]
    #[cfg(feature = "pointer")]
    fn box_ptr() {
        let data = Box::new("box".to_owned());
        let rv = RawValue::from((data, c"box"));
        let RawValue::Pointer {
            ptr,
            ptr_type,
            destructor,
        } = rv
        else {
            panic!("RawValue::Pointer expected");
        };
        assert_eq!(ptr_type, c"box");
        unsafe { destructor.unwrap()(ptr as *mut _) };
    }

    #[test]
    fn cstring() {
        let cs = CString::new("Hello, world!").unwrap();
        let rv = RawValue::from(cs);
        let RawValue::Text {
            ptr,
            bytes,
            destructor,
            ..
        } = rv
        else {
            panic!("RawValue::Text expected");
        };
        assert_eq!(bytes.get(), 13);
        unsafe { destructor.unwrap()(ptr as *mut _) };
    }

    #[test]
    fn empty_cstring() {
        let cs = CString::new("").unwrap();
        let rv = RawValue::from(cs);
        let RawValue::EmptyText = rv else {
            panic!("RawValue::EmptyText expected");
        };
    }

    #[test]
    fn rc_str() {
        let rs: Rc<str> = "Hello, world!".into();
        let rv = RawValue::from(rs);
        let RawValue::Text {
            ptr,
            bytes,
            flags,
            destructor,
        } = rv
        else {
            panic!("RawValue::Text expected");
        };
        assert_eq!(bytes.get(), 13);
        assert_eq!(flags, SQLITE_UTF8 as u8);
        unsafe { destructor.unwrap()(ptr as *mut _) };
    }

    #[test]
    fn empty_rc_str() {
        let rs: Rc<str> = "".into();
        let rv = RawValue::from(rs);
        let RawValue::EmptyText = rv else {
            panic!("RawValue::EmptyText expected");
        };
    }

    #[test]
    fn static_str() {
        let str = "Hello, world!";
        let rv = RawValue::from(str);
        let RawValue::Text {
            ptr,
            bytes,
            flags,
            destructor,
        } = rv
        else {
            panic!("RawValue::Text expected");
        };
        assert_eq!(ptr, str.as_ptr() as *const _);
        assert_eq!(bytes.get(), str.len());
        assert_eq!(flags, SQLITE_UTF8 as u8);
        assert!(destructor.is_none());
    }

    #[test]
    fn rc_u8() {
        let rs: Rc<str> = "Hello, world!".into();
        let rb: Rc<[u8]> = rs.into();
        let rv = RawValue::from(rb);
        let RawValue::Blob {
            ptr,
            bytes,
            destructor,
        } = rv
        else {
            panic!("RawValue::Blob expected");
        };
        assert_eq!(bytes.get(), 13);
        unsafe { destructor.unwrap()(ptr as *mut _) };
    }

    #[test]
    fn box_u8() {
        const N: usize = 2;
        let b: Box<[u8]> = Box::from([1, 2]);
        let b: Box<[u8; N]> = b.try_into().unwrap();
        let rv = RawValue::from(b);
        let RawValue::Blob {
            ptr,
            bytes,
            destructor,
        } = rv
        else {
            panic!("RawValue::Blob expected");
        };
        assert_eq!(bytes.get(), 2);
        unsafe { destructor.unwrap()(ptr as *mut _) };
    }

    #[test]
    fn static_slice() {
        let slice = "Hello, world!".as_bytes();
        let rv = RawValue::from(slice);
        let RawValue::Blob {
            ptr,
            bytes,
            destructor,
        } = rv
        else {
            panic!("RawValue::Blob expected");
        };
        assert_eq!(ptr, slice.as_ptr() as *const _);
        assert_eq!(bytes.get(), slice.len());
        assert!(destructor.is_none());
    }
}
