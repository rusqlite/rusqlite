use std::{
    ffi::{CStr, CString, c_char, c_void},
    rc::Rc,
};

use crate::{
    ffi::{SQLITE_UTF8, sqlite3_destructor_type},
    util::free_boxed_value,
};

/// Raw value to be passed to SQLite (`sqlite3_bind_*` or `sqlite3_result_*`)
pub enum RawValue {
    //Null,
    //Integer(i64),
    //Real(f64),
    /// Pointer passing interface
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
        /// Text encoding
        flags: u64,
        /// `ptr` destructor
        destroy: sqlite3_destructor_type,
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

impl From<CString> for RawValue {
    fn from(value: CString) -> Self {
        unsafe extern "C" fn free_cstring(p: *mut std::ffi::c_void) {
            drop(unsafe { CString::from_raw(p as *mut _) });
        }
        #[cfg(feature = "modern_sqlite")]
        let flags: u64 = (SQLITE_UTF8 | crate::ffi::SQLITE_UTF8_ZT) as _;
        #[cfg(not(feature = "modern_sqlite"))]
        let flags: u64 = SQLITE_UTF8 as _;
        let bytes = value.count_bytes();
        RawValue::Text {
            ptr: value.into_raw() as *const _,
            bytes,
            flags,
            destroy: Some(free_cstring),
        }
    }
}
impl From<Rc<str>> for RawValue {
    fn from(value: Rc<str>) -> Self {
        unsafe extern "C" fn free_rc_str(p: *mut std::ffi::c_void) {
            unsafe { Rc::decrement_strong_count(p.cast::<*const str>()) };
        }
        let rb: Rc<[u8]> = value.into(); // TODO Validate: necessary ?
        let bytes = rb.len();
        RawValue::Text {
            ptr: Rc::into_raw(rb) as *const _,
            bytes,
            flags: SQLITE_UTF8 as _,
            destroy: Some(free_rc_str),
        }
    }
}
impl From<Rc<[u8]>> for RawValue {
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
        assert_eq!(flags, SQLITE_UTF8 as _);
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
