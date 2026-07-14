use std::{
    ffi::{CString, c_void},
    rc::Rc,
};

use crate::{ffi::SQLITE_UTF8, util::free_boxed_value};

/// Bytes that can passed to SQLite without copying
pub trait OwnedBytes {
    /// zero for blob
    const FLAGS: u64;
    /// Bytes count.
    /// This does not include the nul terminator if any.
    fn length(&self) -> usize;
    /// From Rust to SQLite pointer
    fn into_ptr(self) -> *mut c_void;
    /// sqlite3_destructor_type
    /// # Safety
    /// This function is unsafe because improper use may lead to
    /// memory problems.
    unsafe fn destroy(p: *mut c_void);
}

impl OwnedBytes for CString {
    #[cfg(feature = "modern_sqlite")]
    const FLAGS: u64 = (SQLITE_UTF8 | crate::ffi::SQLITE_UTF8_ZT) as _;
    #[cfg(not(feature = "modern_sqlite"))]
    const FLAGS: u64 = SQLITE_UTF8 as _;

    fn length(&self) -> usize {
        self.count_bytes()
    }

    fn into_ptr(self) -> *mut c_void {
        CString::into_raw(self) as *mut _
    }

    unsafe fn destroy(p: *mut c_void) {
        drop(unsafe { CString::from_raw(p as *mut _) });
    }
}
impl OwnedBytes for Rc<str> {
    const FLAGS: u64 = SQLITE_UTF8 as _;

    fn length(&self) -> usize {
        self.len()
    }

    fn into_ptr(self) -> *mut c_void {
        Rc::into_raw(self) as *mut _
    }

    unsafe fn destroy(p: *mut c_void) {
        unsafe { Rc::decrement_strong_count(p.cast::<*const str>()) };
    }
}
impl OwnedBytes for Rc<[u8]> {
    const FLAGS: u64 = 0;

    fn length(&self) -> usize {
        self.len()
    }

    fn into_ptr(self) -> *mut c_void {
        Rc::into_raw(self) as *mut _
    }

    unsafe fn destroy(p: *mut c_void) {
        unsafe { Rc::decrement_strong_count(p.cast::<*const [u8]>()) };
    }
}
impl<const N: usize> OwnedBytes for Box<[u8; N]> {
    const FLAGS: u64 = 0;

    fn length(&self) -> usize {
        self.len()
    }

    fn into_ptr(self) -> *mut c_void {
        Box::into_raw(self) as *mut _
    }

    unsafe fn destroy(p: *mut c_void) {
        unsafe { free_boxed_value::<[u8; N]>(p) };
    }
}

/* TODO
pub enum OwnedValue<T: OwnedBytes> {
    Null,
    Int(i64),
    Real(f64),
    Text(T),
    Blob(T),
}
*/

#[cfg(test)]
mod test {
    use super::OwnedBytes as _;
    use std::ffi::CString;
    use std::rc::Rc;

    #[test]
    fn cstring() {
        let cs = CString::new("Hello, world!").unwrap();
        assert_eq!(13, cs.length());
        let ptr = cs.into_ptr();
        unsafe { CString::destroy(ptr) };
    }

    #[test]
    fn rc_str() {
        let rs: Rc<str> = "Hello, world!".into();
        assert_eq!(13, rs.length());
        let ptr = rs.into_ptr();
        unsafe { Rc::<str>::destroy(ptr) };
    }

    #[test]
    fn rc_u8() {
        let rs: Rc<str> = "Hello, world!".into();
        let rb: Rc<[u8]> = rs.into();
        assert_eq!(13, rb.length());
        let ptr = rb.into_ptr();
        unsafe { Rc::<[u8]>::destroy(ptr) };
    }

    #[test]
    fn box_u8() {
        const N: usize = 2;
        let b: Box<[u8]> = Box::from([1, 2]);
        let b: Box<[u8; N]> = b.try_into().unwrap();
        assert_eq!(2, b.length());
        let ptr = b.into_ptr();
        unsafe {
            Box::<[u8; N]>::destroy(ptr);
        }
    }
}
