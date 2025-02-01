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
