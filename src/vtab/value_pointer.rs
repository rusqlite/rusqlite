//! Provides support for the SQLite pointer passing interface
//! see <https://sqlite.org/bindptr.html>
use std::any::Any;
use std::ffi::{c_void, CStr};
use std::rc::Rc;

#[derive(Debug, Clone)]
/// Raw pointer for passing values
pub struct ValuePointer {
    pub(crate) value: Rc<dyn Any>,
    pub(crate) pointer_type: &'static CStr,
}

impl PartialEq for ValuePointer {
    fn eq(&self, other: &Self) -> bool {
        self.pointer_type == other.pointer_type
            && std::ptr::addr_eq(Rc::as_ptr(&self.value), Rc::as_ptr(&other.value))
    }
}

pub(crate) unsafe extern "C" fn free_pointer(p: *mut c_void) {
    Rc::decrement_strong_count(p as *const dyn Any);
}
