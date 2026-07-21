#[cfg(feature = "pointer")]
use std::ffi::CStr;
use std::{
    ffi::{CString, c_char, c_void},
    rc::Rc,
};

use crate::{
    ffi::{SQLITE_UTF8, sqlite3_destructor_type},
    util::free_boxed_value,
};

/// Raw value to be passed to SQLite (`sqlite3_bind_*` or `sqlite3_result_*`)
#[derive(Clone, Copy, Debug)]
pub enum RawValue {
    //Null,
    //Integer(i64),
    //Real(f64),
    /// Pointer passing interface
    #[cfg(feature = "pointer")]
    Pointer {
        /// Raw pointer
        ptr: *const c_void,
        /// Point type name
        ptr_type: &'static CStr,
        /// `ptr` destructor
        destroy: sqlite3_destructor_type,
    },
    /// UTF-8
    Text {
        /// Raw pointer to a byte array
        ptr: *const c_char,
        /// Bytes count.
        /// This does not include the nul terminator if any.
        bytes: usize,
        /// `ptr` destructor
        destroy: sqlite3_destructor_type,
        /// Text encoding
        flags: u8,
    },
    /// BLOB
    Blob {
        /// Raw pointer to a byte array
        ptr: *const c_void,
        /// Bytes count.
        bytes: usize,
        /// `ptr` destructor
        destroy: sqlite3_destructor_type,
    },
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
            ptr: Rc::into_raw(value.0) as *const _,
            ptr_type: value.1,
            destroy: Some(free_rc::<T>),
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
            ptr: Box::into_raw(value.0) as *const _,
            ptr_type: value.1,
            destroy: Some(free_boxed_value::<T>),
        }
    }
}

impl From<CString> for RawValue {
    /// Pass a `CString` as UTF-8 slice to SQLite
    ///
    /// # Warning
    /// Leak memory if an error happens before the returned pointer is bound to an SQLite statement.
    fn from(value: CString) -> Self {
        unsafe extern "C" fn free_cstring(p: *mut std::ffi::c_void) {
            drop(unsafe { CString::from_raw(p as *mut _) });
        }
        #[cfg(feature = "modern_sqlite")]
        let flags: u8 = (SQLITE_UTF8 | crate::ffi::SQLITE_UTF8_ZT) as _;
        #[cfg(not(feature = "modern_sqlite"))]
        let flags: u8 = SQLITE_UTF8 as _;
        let bytes = value.count_bytes();
        RawValue::Text {
            ptr: value.into_raw() as *const _,
            bytes,
            destroy: Some(free_cstring),
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
        unsafe extern "C" fn free_rc_str(p: *mut std::ffi::c_void) {
            unsafe { Rc::decrement_strong_count(p.cast::<*const str>()) };
        }
        let rb: Rc<[u8]> = value.into(); // TODO Validate: necessary ?
        let bytes = rb.len();
        RawValue::Text {
            ptr: Rc::into_raw(rb) as *const _,
            bytes,
            destroy: Some(free_rc_str),
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
        unsafe extern "C" fn free_rc_slice(p: *mut std::ffi::c_void) {
            unsafe { Rc::decrement_strong_count(p.cast::<*const [u8]>()) };
        }
        let bytes = value.len();
        RawValue::Blob {
            ptr: Rc::into_raw(value) as *const _,
            bytes,
            destroy: Some(free_rc_slice),
        }
    }
}
impl<const N: usize> From<Box<[u8; N]>> for RawValue {
    /// Pass a `Box<[u8; N]>` as a BLOB to SQLite
    ///
    /// # Warning
    /// Leak memory if an error happens before the returned pointer is bound to an SQLite statement.
    fn from(value: Box<[u8; N]>) -> Self {
        let bytes = value.len();
        RawValue::Blob {
            ptr: Box::into_raw(value) as *const _,
            bytes,
            destroy: Some(free_boxed_value::<[u8; N]>),
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
            destroy,
        } = rv
        else {
            panic!("RawValue::Pointer expected");
        };
        assert_eq!(ptr_type, c"rc");
        unsafe { destroy.unwrap()(ptr as *mut _) };
    }

    #[test]
    #[cfg(feature = "pointer")]
    fn box_ptr() {
        let data = Box::new("box".to_owned());
        let rv = RawValue::from((data, c"box"));
        let RawValue::Pointer {
            ptr,
            ptr_type,
            destroy,
        } = rv
        else {
            panic!("RawValue::Pointer expected");
        };
        assert_eq!(ptr_type, c"box");
        unsafe { destroy.unwrap()(ptr as *mut _) };
    }

    #[test]
    fn cstring() {
        let cs = CString::new("Hello, world!").unwrap();
        let rv = RawValue::from(cs);
        let RawValue::Text {
            ptr,
            bytes,
            destroy,
            ..
        } = rv
        else {
            panic!("RawValue::Text expected");
        };
        assert_eq!(bytes, 13);
        unsafe { destroy.unwrap()(ptr as *mut _) };
    }

    #[test]
    fn rc_str() {
        let rs: Rc<str> = "Hello, world!".into();
        let rv = RawValue::from(rs);
        let RawValue::Text {
            ptr,
            bytes,
            flags,
            destroy,
        } = rv
        else {
            panic!("RawValue::Text expected");
        };
        assert_eq!(bytes, 13);
        assert_eq!(flags, SQLITE_UTF8 as u8);
        unsafe { destroy.unwrap()(ptr as *mut _) };
    }

    #[test]
    fn rc_u8() {
        let rs: Rc<str> = "Hello, world!".into();
        let rb: Rc<[u8]> = rs.into();
        let rv = RawValue::from(rb);
        let RawValue::Blob {
            ptr,
            bytes,
            destroy,
        } = rv
        else {
            panic!("RawValue::Blob expected");
        };
        assert_eq!(bytes, 13);
        unsafe { destroy.unwrap()(ptr as *mut _) };
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
            destroy,
        } = rv
        else {
            panic!("RawValue::Blob expected");
        };
        assert_eq!(bytes, 2);
        unsafe { destroy.unwrap()(ptr as *mut _) };
    }
}
