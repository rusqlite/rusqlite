// Internal utilities
pub(crate) mod param_cache;
mod small_cstr;
pub(crate) use param_cache::ParamIndexCache;
pub(crate) use small_cstr::SmallCString;

// Doesn't use any modern features or vtab stuff, but is only used by them.
mod sqlite_string;
pub(crate) use sqlite_string::{alloc, SqliteMallocString};

#[cfg(any(feature = "collation", feature = "functions", feature = "vtab"))]
pub(crate) unsafe extern "C" fn free_boxed_value<T>(p: *mut std::ffi::c_void) {
    drop(Box::from_raw(p.cast::<T>()));
}

use crate::Result;
use std::ffi::CStr;

pub enum Named<'a> {
    Small(SmallCString),
    C(&'a CStr),
}
impl std::ops::Deref for Named<'_> {
    type Target = CStr;
    #[inline]
    fn deref(&self) -> &CStr {
        match self {
            Named::Small(s) => s.as_cstr(),
            Named::C(s) => s,
        }
    }
}

/// Database, table, column, collation, function, module, vfs name
pub trait Name: std::fmt::Debug {
    /// As C string
    fn as_cstr(&self) -> Result<Named<'_>>;
}
impl Name for &str {
    fn as_cstr(&self) -> Result<Named<'_>> {
        let ss = SmallCString::new(self)?;
        Ok(Named::Small(ss))
    }
}
impl Name for &CStr {
    #[inline]
    fn as_cstr(&self) -> Result<Named<'_>> {
        Ok(Named::C(self))
    }
}
