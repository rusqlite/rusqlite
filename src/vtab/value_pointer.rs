//! Provides support for the SQLite pointer passing interface
//! see <https://sqlite.org/bindptr.html>
use std::any::Any;
use std::ffi::{c_void, CStr};
use std::marker::PhantomData;
use std::rc::Rc;

use crate::types::ToSqlOutput;

/// Represents a pointer type that can be passed via SQLite's pointer passing interface
#[derive(Debug, Copy, Clone)]
pub struct PointerType<T> {
    type_name: &'static CStr,
    marker: PhantomData<T>,
}

impl<T: 'static> PointerType<T> {
    /// Creates a `PointerType<T>` for pointers to values of type `T`.
    /// Note: the type_name must be unique as this is used by SQLite's runtime pointer type checking
    pub const fn new(type_name: &'static CStr) -> Self {
        Self {
            type_name,
            marker: PhantomData,
        }
    }

    /// Return the type name string provided in `PointerType::new`.
    pub fn get_type_name(&self) -> &'static CStr {
        self.type_name
    }

    pub(crate) unsafe extern "C" fn free_pointer(p: *mut c_void) {
        Rc::decrement_strong_count(p as *const T);
    }

    /// Assists converting from an `Rc<T>` to ToSqlOutput.
    /// This function should be called from a ToSql impl.
    pub fn to_sql(&self, value: &Rc<T>) -> ToSqlOutput<'_> {
        ToSqlOutput::ValuePointer(ValuePointer {
            value: value.clone(),
            pointer_type_name: self.type_name,
            free_pointer: Self::free_pointer,
        })
    }
}

#[derive(Debug, Clone)]
/// Raw pointer for passing values
pub struct ValuePointer {
    pub(crate) value: Rc<dyn Any>,
    pub(crate) pointer_type_name: &'static CStr,
    pub(crate) free_pointer: unsafe extern "C" fn(p: *mut c_void),
}

impl PartialEq for ValuePointer {
    fn eq(&self, other: &Self) -> bool {
        self.pointer_type_name == other.pointer_type_name
            && std::ptr::addr_eq(Rc::as_ptr(&self.value), Rc::as_ptr(&other.value))
    }
}
